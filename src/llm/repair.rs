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
}

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
        "- For tool argument errors, keep the same tool_name and repair only arguments, message, or reason as needed.",
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
        if let Some(activity) = &diagnostic.activity {
            lines.push(format!("- activity: {activity}"));
        }
        if let Some(tool_name) = &diagnostic.tool_name {
            lines.push(format!("- tool_name: {tool_name}"));
        }
    }

    if let Some(schema_line) = diagnostic.tool_schema_line {
        lines.push("Exact tool argument schema for this repair:".to_owned());
        lines.push(format!("- {schema_line}"));
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

        assert!(request.prompt.contains("Repair source: runtime_decision"));
        assert!(request.prompt.contains("- response_type: tool"));
        assert!(request.prompt.contains("- activity: Explore"));
        assert!(request.prompt.contains("- tool_name: read_file"));
        assert!(request.prompt.contains("read_file arguments"));
        assert!(request
            .prompt
            .contains("Do not switch a tool candidate to clarify"));
    }
}
