use super::decision::RuntimeDecisionError;
use super::response_parser::{
    parse_runtime_response_envelope_diagnostic, ParsedRuntimeResponse, RuntimeResponse,
    RuntimeResponseParseError, RESPONSE_ACTIVITY_PAIRS,
};
use super::{
    payload_ordering_contract_lines, response_boundary_contract_lines,
    tool_path_selection_contract_lines,
};
use crate::tool::tool_spec;

pub const MAX_REPAIR_ATTEMPTS: u16 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairRequest {
    pub attempt: u16,
    pub max_attempts: u16,
    pub source: &'static str,
    pub failure_signature: String,
    pub prompt: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairLimitReached {
    pub attempts: u16,
    pub max_attempts: u16,
    pub failure_signature: String,
    pub reason: RepairLimitReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepairLimitReason {
    MaxAttemptsReached,
}

impl RepairLimitReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MaxAttemptsReached => "max_attempts_reached",
        }
    }
}

pub struct RepairLoop {
    max_attempts: u16,
}

impl RepairLoop {
    pub fn new(max_attempts: u16) -> Self {
        Self { max_attempts }
    }

    pub fn default_local() -> Self {
        Self::new(MAX_REPAIR_ATTEMPTS)
    }

    pub fn max_attempts(&self) -> u16 {
        self.max_attempts
    }

    pub fn next_request_with_raw(
        &self,
        attempts: u16,
        error: &RuntimeResponseParseError,
        raw_response: Option<&str>,
    ) -> Result<RepairRequest, RepairLimitReached> {
        let diagnostic = RepairDiagnostic::parse_error(error, raw_response);
        self.next_request_for_diagnostic(attempts, diagnostic)
    }

    pub fn next_request_for_runtime_decision(
        &self,
        attempts: u16,
        parsed: &ParsedRuntimeResponse,
        error: &RuntimeDecisionError,
        raw_response: &str,
    ) -> Result<RepairRequest, RepairLimitReached> {
        let diagnostic = RepairDiagnostic::runtime_decision_error(parsed, error, raw_response);
        self.next_request_for_diagnostic(attempts, diagnostic)
    }

    fn next_request_for_diagnostic(
        &self,
        attempts: u16,
        diagnostic: RepairDiagnostic,
    ) -> Result<RepairRequest, RepairLimitReached> {
        let failure_signature = diagnostic.failure_signature();
        if attempts >= self.max_attempts {
            return Err(RepairLimitReached {
                attempts,
                max_attempts: self.max_attempts,
                failure_signature,
                reason: RepairLimitReason::MaxAttemptsReached,
            });
        }

        let attempt = attempts + 1;
        Ok(RepairRequest {
            attempt,
            max_attempts: self.max_attempts,
            source: diagnostic.source,
            failure_signature,
            prompt: build_repair_prompt_from_diagnostic(&diagnostic, attempt, self.max_attempts),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RepairDiagnostic {
    source: &'static str,
    failure_kind: String,
    failure_message: String,
    raw_response: Option<String>,
    response_type: Option<String>,
    activity: Option<String>,
    tool_name: Option<String>,
    tool_schema_line: Option<&'static str>,
}

impl RepairDiagnostic {
    fn parse_error(error: &RuntimeResponseParseError, raw_response: Option<&str>) -> Self {
        let envelope = raw_response.and_then(parse_runtime_response_envelope_diagnostic);
        let tool_schema_line = envelope
            .as_ref()
            .and_then(|envelope| envelope.tool_name.as_deref())
            .and_then(tool_spec)
            .map(|spec| spec.schema_line);

        Self {
            source: "response_parse",
            failure_kind: error.kind.as_str().to_owned(),
            failure_message: error.message.clone(),
            raw_response: raw_response.map(str::to_owned),
            response_type: envelope
                .as_ref()
                .and_then(|envelope| envelope.response_type.clone()),
            activity: envelope
                .as_ref()
                .and_then(|envelope| envelope.activity.clone()),
            tool_name: envelope.and_then(|envelope| envelope.tool_name),
            tool_schema_line,
        }
    }

    fn runtime_decision_error(
        parsed: &ParsedRuntimeResponse,
        error: &RuntimeDecisionError,
        raw_response: &str,
    ) -> Self {
        let (response_type, activity, tool_name) = match &parsed.response {
            RuntimeResponse::Answer(response) => (
                Some("answer".to_owned()),
                Some(response.activity.as_str().to_owned()),
                None,
            ),
            RuntimeResponse::Clarify(response) => (
                Some("clarify".to_owned()),
                Some(response.activity.as_str().to_owned()),
                None,
            ),
            RuntimeResponse::Blocked(response) => (
                Some("blocked".to_owned()),
                Some(response.activity.as_str().to_owned()),
                None,
            ),
            RuntimeResponse::Plan(response) => (
                Some("plan".to_owned()),
                Some(response.activity.as_str().to_owned()),
                None,
            ),
            RuntimeResponse::Tool(candidate) => (
                Some("tool".to_owned()),
                Some(candidate.activity.as_str().to_owned()),
                Some(candidate.tool_name.clone()),
            ),
        };
        let tool_schema_line = tool_name
            .as_deref()
            .and_then(tool_spec)
            .map(|spec| spec.schema_line);

        Self {
            source: "runtime_decision",
            failure_kind: error.kind.as_str().to_owned(),
            failure_message: error.message.clone(),
            raw_response: Some(raw_response.to_owned()),
            response_type,
            activity,
            tool_name,
            tool_schema_line,
        }
    }

    fn failure_signature(&self) -> String {
        format!("{}:{}", self.failure_kind, self.failure_message)
    }

    fn required_activity(&self) -> Option<String> {
        if self.failure_kind == "invalid_tool_candidate"
            && self.failure_message.contains("tool/activity mismatch")
        {
            return self
                .tool_name
                .as_deref()
                .and_then(tool_spec)
                .map(|spec| spec.activity.as_str().to_owned())
                .or_else(|| self.activity.clone());
        }

        self.activity.clone()
    }
}

const APPLY_PATCH_REPAIR_CONTRACT_LINES: &[&str] = &[
    "Apply_patch repair focus:",
    "- Keep the failed response tool shape: response_type=tool, activity=Change, tool_name=apply_patch.",
    "- The JSON arguments object for apply_patch must contain payload_id only.",
    "- Do not put target path, patch operation, patch text, or file body text in JSON arguments.",
    "- Keep the existing payload_id when one was present; otherwise choose one payload id and use it in both arguments and AHREUM_PAYLOAD.",
    "- If the previous response contains <AHREUM_PAYLOAD id=\"X\" format=\"apply_patch\"> and arguments.payload_id is missing or different, set arguments to exactly {\"payload_id\":\"X\"}.",
    "- Do not remove arguments.payload_id or switch away from apply_patch to avoid repairing the patch payload.",
    "- The same payload_id must appear in arguments.payload_id and exactly one AHREUM_PAYLOAD id.",
    "- The AHREUM_PAYLOAD format must be \"apply_patch\".",
    "- The repaired response must start with <AHREUM_ACTION>, then the JSON envelope, then </AHREUM_ACTION>, then the matching AHREUM_PAYLOAD block.",
    "- The payload body must be one complete patch document beginning with *** Begin Patch and ending with *** End Patch.",
    "- The payload body may contain one or more target headers: Add File, Update File, or Delete File.",
    "- Encode each target path and operation only in its patch target header.",
    "- The payload body is the patch wrapper, not the final file body by itself.",
    "- Required payload skeleton, with placeholders replaced by the actual target and lines:",
    "<AHREUM_PAYLOAD id=\"patch_001\" format=\"apply_patch\">",
    "*** Begin Patch",
    "*** Update File: path/from/observed/request",
    "@@",
    " exact existing context line",
    "-exact old line",
    "+exact new line",
    "*** End Patch",
    "</AHREUM_PAYLOAD>",
    "- For a new file, use *** Add File: <requested workspace path> and prefix every created content line with +.",
    "- For Update File, each hunk must include at least one matching existing context line prefixed with space or one removal line prefixed with -; a hunk with only + lines is invalid.",
    "- For Update File, bare content lines are invalid. Use exactly one line marker per hunk line: space for existing context, - for removed existing text, + for added replacement text, or @@ for the hunk boundary.",
    "- The space, -, and + markers are the first character of the patch line, not separators. Do not add an extra space after - or + unless the file line itself starts with a space.",
    "- Do not wrap the patch body in markdown fences.",
    "- Preserve the intended patch operation unless the parser failure proves that operation invalid for the observed file state.",
];

const PLAN_REPAIR_CONTRACT_LINES: &[&str] = &[
    "Plan repair focus:",
    "- Use response_type=plan only for a multi-target execution ledger that the runtime should complete over later turns.",
    "- If the failed response is only a normal advisory or explanatory reply, convert it to response_type=answer with activity=None instead of forcing an unnecessary plan.",
    "- When preserving response_type=plan, plan_items must be a JSON array, never an object, string, map, or prose paragraph.",
    "- Every plan item must be a JSON object with operation and optional target only.",
    "- operation must be one of read, create, update, delete, execute, verify, answer.",
    "- read, create, update, and delete plan items require a concrete workspace-relative target when the target is known.",
    "- Plan responses must not contain tool_name, arguments, payload_id, patch text, file body text, command argv, or AHREUM_PAYLOAD blocks.",
];

#[cfg(test)]
fn build_repair_prompt(
    error: &RuntimeResponseParseError,
    attempt: u16,
    max_attempts: u16,
) -> String {
    build_repair_prompt_from_diagnostic(
        &RepairDiagnostic::parse_error(error, None),
        attempt,
        max_attempts,
    )
}

fn build_repair_prompt_from_diagnostic(
    diagnostic: &RepairDiagnostic,
    attempt: u16,
    max_attempts: u16,
) -> String {
    let mut lines = [
        "The previous assistant response did not satisfy the AhreumCode response contract.",
        "Regenerate the response for the same user intent and same candidate shape.",
        "",
        "Repair constraints:",
        "- Return exactly one response contract.",
        "- Return only valid JSON, or one AHREUM_ACTION block plus required AHREUM_PAYLOAD blocks.",
        "- Repair the exact failed response shown below; do not infer a fresh plan from the user prompt.",
        "- Do not add unknown fields.",
        "- Prefer plain JSON when the candidate is an Explore tool, clarify, blocked, or a short answer.",
        "- Explore tool candidates must not include AHREUM_PAYLOAD blocks.",
        "- If the answer contains code, markdown, or long prose, put it in answer_payload_id and AHREUM_PAYLOAD format=\"markdown\".",
        "- Do not put source, patch, or file body text inside JSON string fields.",
        "- Use payload_id and AHREUM_PAYLOAD blocks for source, patch, or file body text.",
        "- For payload reference failures, either add the exact required answer_payload_id or arguments.payload_id reference, or remove every unreferenced AHREUM_PAYLOAD block.",
        "- For payload reference failures, do not change existing non-payload tool arguments.",
        "- If the previous response already has response_type, activity, or tool_name, keep them unless the failure says that exact field is invalid.",
        "- Do not switch a tool candidate to clarify, blocked, or answer to avoid fixing validation errors.",
        "- Do not go backward to an earlier evidence-gathering tool when the failed response attempted a later Change, Execute, or Configure candidate.",
        "- For tool argument errors, keep the same tool_name and repair only arguments, message, or reason as needed.",
        "- For apply_patch repairs, do not return plain JSON alone; return one AHREUM_ACTION block plus exactly one matching AHREUM_PAYLOAD id with format=\"apply_patch\".",
        "- For apply_patch target-count errors, the payload body must contain one complete patch document with one or more Add File, Update File, or Delete File target headers.",
        "- For apply_patch target-count errors, do not preserve a bare file body; convert it into a patch document with the target header and patch line prefixes.",
        "- If a workspace fact can be checked by a safe Explore tool, do not use clarify.",
        "",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();

    lines.extend(
        response_boundary_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    lines.extend(
        payload_ordering_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    lines.extend(
        tool_path_selection_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    lines.push(String::new());
    lines.push("Allowed response_type/activity pairs:".to_owned());

    for pair in RESPONSE_ACTIVITY_PAIRS {
        lines.push(format!("- {}", pair.rule_text()));
    }

    lines.extend(
        [
            "",
            &format!("Repair source: {}", diagnostic.source),
            &format!("Repair attempt: {attempt}/{max_attempts}"),
            &format!("Failure kind: {}", diagnostic.failure_kind),
            &format!("Failure message: {}", diagnostic.failure_message),
        ]
        .into_iter()
        .map(str::to_owned),
    );

    if diagnostic.response_type.is_some()
        || diagnostic.activity.is_some()
        || diagnostic.tool_name.is_some()
    {
        lines.push("Required candidate shape:".to_owned());
        if let Some(response_type) = &diagnostic.response_type {
            lines.push(format!("- response_type: {response_type}"));
        }
        let required_activity = diagnostic.required_activity();
        if let Some(activity) = &required_activity {
            lines.push(format!("- activity: {activity}"));
        }
        if let Some(tool_name) = &diagnostic.tool_name {
            lines.push(format!("- tool_name: {tool_name}"));
        }
        if diagnostic.activity.as_ref() != required_activity.as_ref()
            && diagnostic
                .failure_message
                .contains("tool/activity mismatch")
        {
            lines.push(format!(
                "- previous activity {} is invalid for tool {}; use the registry activity {}.",
                diagnostic.activity.as_deref().unwrap_or("-"),
                diagnostic.tool_name.as_deref().unwrap_or("-"),
                required_activity.as_deref().unwrap_or("-")
            ));
        }
    }

    if let Some(schema_line) = diagnostic.tool_schema_line {
        lines.push("Exact tool argument schema for this repair:".to_owned());
        lines.push(format!("- {schema_line}"));
    }

    if diagnostic.tool_name.as_deref() == Some("apply_patch") {
        lines.extend(
            APPLY_PATCH_REPAIR_CONTRACT_LINES
                .iter()
                .map(|line| line.to_string()),
        );
    }

    if diagnostic.response_type.as_deref() == Some("plan")
        || diagnostic.failure_message.contains("plan_items")
        || diagnostic.failure_message.contains("plan operation")
    {
        lines.extend(
            PLAN_REPAIR_CONTRACT_LINES
                .iter()
                .map(|line| line.to_string()),
        );
    }

    if matches!(
        diagnostic.failure_kind.as_str(),
        "payload_validation_failed" | "partial_response"
    ) {
        lines.push("Payload repair focus:".to_owned());
        lines.push(
            "- Keep the same response_type, activity, tool_name, and non-payload arguments when they are present in the previous response."
                .to_owned(),
        );
        lines.push(
            "- If an AHREUM_PAYLOAD block remains, the JSON envelope must reference it with answer_payload_id or arguments.payload_id."
                .to_owned(),
        );
        lines.push(
            "- For apply_patch with one AHREUM_PAYLOAD id=\"X\", keep response_type=tool, activity=Change, tool_name=apply_patch, and set arguments to exactly {\"payload_id\":\"X\"}."
                .to_owned(),
        );
        lines.push(
            "- For response_type=tool with activity=Explore, remove every AHREUM_PAYLOAD block and keep the tool arguments as plain JSON."
                .to_owned(),
        );
        lines.push(
            "- For response_type=answer with one markdown payload, either add answer_payload_id with that exact payload id or remove the payload and keep a short answer in message."
                .to_owned(),
        );
        lines.push(
            "- If no payload is needed, remove every AHREUM_PAYLOAD block instead of leaving it unreferenced."
                .to_owned(),
        );
    }

    if let Some(raw_response) = &diagnostic.raw_response {
        lines.push("Previous raw assistant response to repair:".to_owned());
        lines.push("<AHREUM_PREVIOUS_RESPONSE>".to_owned());
        lines.push(raw_response.clone());
        lines.push("</AHREUM_PREVIOUS_RESPONSE>".to_owned());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        build_repair_prompt, payload_ordering_contract_lines, response_boundary_contract_lines,
        tool_path_selection_contract_lines, RepairLimitReason, RepairLoop,
    };
    use crate::llm::response_parser::{
        Activity, ParsedRuntimeResponse, RuntimeManifest, RuntimeResponse, RuntimeToolCandidate,
    };
    use crate::llm::{DecisionGate, RuntimeResponseParseError, RuntimeResponseParseErrorKind};
    use serde_json::json;

    fn expected_pair_lines() -> Vec<String> {
        crate::llm::response_parser::RESPONSE_ACTIVITY_PAIRS
            .iter()
            .map(|pair| format!("- {}", pair.rule_text()))
            .collect()
    }

    fn parse_error() -> RuntimeResponseParseError {
        RuntimeResponseParseError {
            kind: RuntimeResponseParseErrorKind::JsonParseFailed,
            message: "expected value".to_owned(),
        }
    }

    #[test]
    fn builds_repair_prompt_with_failure_reason() {
        let prompt = build_repair_prompt(&parse_error(), 1, 1);

        assert!(prompt.contains("same user intent"));
        assert!(prompt.contains("Repair the exact failed response"));
        assert!(prompt.contains("do not infer a fresh plan"));
        assert!(prompt.contains("Failure kind: json_parse_failed"));
        assert!(prompt.contains("Failure message: expected value"));
        assert!(prompt.contains("Repair attempt: 1/1"));
        assert!(prompt.contains("Allowed response_type/activity pairs:"));
        for boundary_line in response_boundary_contract_lines() {
            assert!(prompt.contains(boundary_line));
        }
        for payload_line in payload_ordering_contract_lines() {
            assert!(prompt.contains(payload_line));
        }
        for path_line in tool_path_selection_contract_lines() {
            assert!(prompt.contains(path_line));
        }
        for pair_line in expected_pair_lines() {
            assert!(prompt.contains(&pair_line));
        }
    }

    #[test]
    fn allows_first_repair_attempt() {
        let loop_state = RepairLoop::default_local();

        let request = loop_state
            .next_request_with_raw(0, &parse_error(), None)
            .expect("first repair should be allowed");

        assert_eq!(request.attempt, 1);
        assert_eq!(request.max_attempts, 2);
    }

    #[test]
    fn blocks_after_repair_limit() {
        let loop_state = RepairLoop::default_local();

        let limit = loop_state
            .next_request_with_raw(2, &parse_error(), None)
            .expect_err("third repair should be blocked");

        assert_eq!(limit.attempts, 2);
        assert_eq!(limit.reason, RepairLimitReason::MaxAttemptsReached);
    }

    #[test]
    fn raw_parse_repair_includes_previous_response() {
        let loop_state = RepairLoop::default_local();

        let request = loop_state
            .next_request_with_raw(0, &parse_error(), Some(r#"{"response_type":"tool"}"#))
            .expect("repair should be allowed");

        assert!(request.prompt.contains("<AHREUM_PREVIOUS_RESPONSE>"));
        assert!(request.prompt.contains(r#"{"response_type":"tool"}"#));
    }

    #[test]
    fn payload_parse_repair_preserves_extracted_tool_shape_and_schema() {
        let error = RuntimeResponseParseError {
            kind: RuntimeResponseParseErrorKind::PayloadValidationFailed,
            message: "payload block exists without payload_id reference".to_owned(),
        };
        let raw = r#"<AHREUM_ACTION>
{"response_type":"tool","activity":"Explore","message":"read","tool_name":"read_file","arguments":{"path":"Cargo.toml","start_line":1,"max_lines":120}}
</AHREUM_ACTION>
<AHREUM_PAYLOAD id="orphan" format="markdown">unused</AHREUM_PAYLOAD>"#;
        let loop_state = RepairLoop::default_local();

        let request = loop_state
            .next_request_with_raw(0, &error, Some(raw))
            .expect("payload repair should be allowed");

        assert!(request.prompt.contains("Payload repair focus:"));
        assert!(request.prompt.contains("- response_type: tool"));
        assert!(request.prompt.contains("- activity: Explore"));
        assert!(request.prompt.contains("- tool_name: read_file"));
        assert!(request.prompt.contains("read_file arguments"));
        assert!(request
            .prompt
            .contains("do not change existing non-payload tool arguments"));
        assert!(request
            .prompt
            .contains("Explore tool candidates must not include AHREUM_PAYLOAD blocks"));
        assert!(request
            .prompt
            .contains("remove every AHREUM_PAYLOAD block and keep the tool arguments"));
    }

    #[test]
    fn plan_shape_repair_includes_array_contract_and_answer_escape() {
        let error = RuntimeResponseParseError {
            kind: RuntimeResponseParseErrorKind::SchemaValidationFailed,
            message: "plan_items must be an array".to_owned(),
        };
        let raw = r#"{"response_type":"plan","activity":"None","message":"정리합니다.","tool_manifest_id":"ahreumcode.local-llm.tool-manifest.v1","tool_manifest_version":"1","plan_items":{"operation":"answer"}}"#;
        let loop_state = RepairLoop::default_local();

        let request = loop_state
            .next_request_with_raw(0, &error, Some(raw))
            .expect("plan repair should be allowed");

        assert!(request.prompt.contains("Plan repair focus:"));
        assert!(request.prompt.contains("plan_items must be a JSON array"));
        assert!(request
            .prompt
            .contains("convert it to response_type=answer"));
        assert!(request
            .prompt
            .contains("must not contain tool_name, arguments, payload_id"));
    }

    #[test]
    fn payload_parse_repair_reconstructs_apply_patch_payload_contract() {
        let error = RuntimeResponseParseError {
            kind: RuntimeResponseParseErrorKind::PayloadValidationFailed,
            message: "missing payload block: change_001".to_owned(),
        };
        let raw = r#"<AHREUM_ACTION>
{"response_type":"tool","activity":"Change","message":"create file","tool_name":"apply_patch","arguments":{"payload_id":"change_001"}}
</AHREUM_ACTION>"#;
        let loop_state = RepairLoop::default_local();

        let request = loop_state
            .next_request_with_raw(0, &error, Some(raw))
            .expect("payload repair should be allowed");

        assert_eq!(request.source, "response_parse");
        assert!(request.prompt.contains("Payload repair focus:"));
        assert!(request.prompt.contains("- tool_name: apply_patch"));
        assert!(request
            .prompt
            .contains("The same payload_id must appear in arguments.payload_id"));
        assert!(request
            .prompt
            .contains("set arguments to exactly {\"payload_id\":\"X\"}"));
        assert!(request.prompt.contains("format=\"apply_patch\""));
        assert!(request.prompt.contains("*** Begin Patch"));
        assert!(request.prompt.contains("*** End Patch"));
        assert!(request
            .prompt
            .contains("Do not remove arguments.payload_id or switch away from apply_patch"));
        assert!(request.prompt.contains("payload body is the patch wrapper"));
        assert!(request
            .prompt
            .contains("prefix every created content line with +"));
        assert!(request.prompt.contains("missing payload block: change_001"));
    }

    #[test]
    fn runtime_decision_repair_preserves_tool_shape_and_schema() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "read file".to_owned(),
                tool_name: "read_file".to_owned(),
                arguments: json!({"path":"README.md","start_line":1,"max_lines":301}),
                reason: "inspect".to_owned(),
                manifest: RuntimeManifest {
                    tool_manifest_id: "ahreumcode.local-llm.tool-manifest.v1".to_owned(),
                    tool_manifest_version: "1".to_owned(),
                },
            }),
            payloads: Vec::new(),
        };
        let error = DecisionGate::classify(&parsed).expect_err("limit should fail");
        let loop_state = RepairLoop::default_local();

        let request = loop_state
            .next_request_for_runtime_decision(0, &parsed, &error, r#"{"response_type":"tool"}"#)
            .expect("runtime decision repair should be allowed");

        assert_eq!(request.source, "runtime_decision");
        assert!(request.prompt.contains("Repair source: runtime_decision"));
        assert!(request.prompt.contains("- response_type: tool"));
        assert!(request.prompt.contains("- activity: Explore"));
        assert!(request.prompt.contains("- tool_name: read_file"));
        assert!(request.prompt.contains("read_file arguments"));
        assert!(request
            .prompt
            .contains("Do not switch a tool candidate to clarify"));
        assert!(request
            .prompt
            .contains("Do not go backward to an earlier evidence-gathering tool"));
    }

    #[test]
    fn runtime_decision_repair_enforces_apply_patch_split_contract() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Change,
                message: "create file".to_owned(),
                tool_name: "apply_patch".to_owned(),
                arguments: json!({
                    "payload_id":"patch_001",
                    "path":"fixture-target.txt",
                    "operation":"add"
                }),
                reason: "change requested".to_owned(),
                manifest: RuntimeManifest {
                    tool_manifest_id: "ahreumcode.local-llm.tool-manifest.v1".to_owned(),
                    tool_manifest_version: "1".to_owned(),
                },
            }),
            payloads: Vec::new(),
        };
        let error = DecisionGate::classify(&parsed).expect_err("unknown fields should fail");
        let loop_state = RepairLoop::default_local();

        let request = loop_state
            .next_request_for_runtime_decision(
                0,
                &parsed,
                &error,
                r#"{"response_type":"tool","activity":"Change","tool_name":"apply_patch","arguments":{"payload_id":"patch_001","path":"fixture-target.txt","operation":"add"}}"#,
            )
            .expect("runtime decision repair should be allowed");

        assert!(request.prompt.contains("Apply_patch repair focus:"));
        assert!(request
            .prompt
            .contains("apply_patch must contain payload_id only"));
        assert!(request
            .prompt
            .contains("target path, patch operation, patch text, or file body text"));
        assert!(request
            .prompt
            .contains("Encode each target path and operation only in its patch target header"));
        assert!(request
            .prompt
            .contains("one or more target headers: Add File, Update File, or Delete File"));
        assert!(request.prompt.contains("do not preserve a bare file body"));
        assert!(request
            .prompt
            .contains("Do not wrap the patch body in markdown fences"));
    }

    #[test]
    fn runtime_decision_repair_corrects_tool_activity_mismatch() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Execute,
                message: "change file".to_owned(),
                tool_name: "apply_patch".to_owned(),
                arguments: json!({"payload_id":"patch_001"}),
                reason: "change requested".to_owned(),
                manifest: RuntimeManifest {
                    tool_manifest_id: "ahreumcode.local-llm.tool-manifest.v1".to_owned(),
                    tool_manifest_version: "1".to_owned(),
                },
            }),
            payloads: Vec::new(),
        };
        let error = DecisionGate::classify(&parsed).expect_err("activity mismatch should fail");
        let loop_state = RepairLoop::default_local();

        let request = loop_state
            .next_request_for_runtime_decision(
                0,
                &parsed,
                &error,
                r#"{"response_type":"tool","activity":"Execute","tool_name":"apply_patch","arguments":{"payload_id":"patch_001"}}"#,
            )
            .expect("runtime decision repair should be allowed");

        assert!(request.prompt.contains("- activity: Change"));
        assert!(request
            .prompt
            .contains("previous activity Execute is invalid for tool apply_patch"));
        assert!(!request.prompt.contains("- activity: Execute"));
    }
}
