use serde_json::{Map, Value};
use std::path::{Component, Path};

use super::response_parser::{
    Activity, ParsedRuntimeResponse, RuntimeAnswer, RuntimeResponse, RuntimeToolCandidate,
};
use crate::tool::{
    tool_spec, validate_tool_arguments as validate_registry_tool_arguments, ToolName,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeDecision {
    Answer {
        summary: String,
        payload_body: Option<String>,
    },
    Clarify {
        message: String,
        reason: String,
    },
    Blocked {
        message: String,
        reason: String,
    },
    ToolCandidatePending {
        activity: Activity,
        tool_name: String,
        arguments: Value,
        summary: String,
    },
    ApprovalNeeded {
        activity: Activity,
        tool_name: String,
        arguments: Value,
        change_preview: Option<ChangePreview>,
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangePreview {
    pub payload_id: String,
    pub target_path: String,
    pub operation: PatchOperation,
    pub additions: u16,
    pub deletions: u16,
}

impl ChangePreview {
    pub fn details(&self) -> String {
        format!(
            "patch target: {}\noperation: {}\nadditions: {}\ndeletions: {}\npayload_id: {}",
            self.target_path,
            self.operation.as_str(),
            self.additions,
            self.deletions,
            self.payload_id
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatchOperation {
    Add,
    Update,
    Delete,
}

impl PatchOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

impl RuntimeDecision {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Answer { .. } => "answer",
            Self::Clarify { .. } => "clarify",
            Self::Blocked { .. } => "blocked",
            Self::ToolCandidatePending { .. } => "tool_candidate_pending",
            Self::ApprovalNeeded { .. } => "approval_needed",
        }
    }

    pub fn activity(&self) -> Option<Activity> {
        match self {
            Self::ToolCandidatePending { activity, .. } | Self::ApprovalNeeded { activity, .. } => {
                Some(*activity)
            }
            _ => None,
        }
    }

    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::ToolCandidatePending { tool_name, .. }
            | Self::ApprovalNeeded { tool_name, .. } => Some(tool_name),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDecisionError {
    pub kind: RuntimeDecisionErrorKind,
    pub message: String,
}

impl RuntimeDecisionError {
    fn invalid_tool(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeDecisionErrorKind::InvalidToolCandidate,
            message: message.into(),
        }
    }

    fn invalid_arguments(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeDecisionErrorKind::InvalidArguments,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeDecisionErrorKind {
    InvalidToolCandidate,
    InvalidArguments,
}

impl RuntimeDecisionErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidToolCandidate => "invalid_tool_candidate",
            Self::InvalidArguments => "invalid_arguments",
        }
    }
}

pub struct DecisionGate;

impl DecisionGate {
    pub fn classify(
        parsed: &ParsedRuntimeResponse,
    ) -> Result<RuntimeDecision, RuntimeDecisionError> {
        match &parsed.response {
            RuntimeResponse::Answer(response) => classify_answer(response, parsed),
            RuntimeResponse::Clarify(response) => Ok(RuntimeDecision::Clarify {
                message: response.message.clone(),
                reason: response.reason.clone(),
            }),
            RuntimeResponse::Blocked(response) => Ok(RuntimeDecision::Blocked {
                message: response.message.clone(),
                reason: response.reason.clone(),
            }),
            RuntimeResponse::Tool(candidate) => classify_tool_candidate(candidate, parsed),
        }
    }
}

fn classify_answer(
    response: &RuntimeAnswer,
    parsed: &ParsedRuntimeResponse,
) -> Result<RuntimeDecision, RuntimeDecisionError> {
    let Some(payload_id) = response.answer_payload_id.as_deref() else {
        return Ok(RuntimeDecision::Answer {
            summary: response.message.clone(),
            payload_body: None,
        });
    };
    let payload = parsed
        .payloads
        .iter()
        .find(|payload| payload.id == payload_id)
        .ok_or_else(|| {
            RuntimeDecisionError::invalid_arguments(format!(
                "answer_payload_id has no matching payload: {payload_id}"
            ))
        })?;

    Ok(RuntimeDecision::Answer {
        summary: response.message.clone(),
        payload_body: Some(payload.body.clone()),
    })
}

fn classify_tool_candidate(
    candidate: &RuntimeToolCandidate,
    parsed: &ParsedRuntimeResponse,
) -> Result<RuntimeDecision, RuntimeDecisionError> {
    validate_tool_activity(candidate)?;
    validate_tool_arguments(candidate)?;

    match candidate.activity {
        Activity::Explore => Ok(RuntimeDecision::ToolCandidatePending {
            activity: candidate.activity,
            tool_name: candidate.tool_name.clone(),
            arguments: candidate.arguments.clone(),
            summary: candidate.message.clone(),
        }),
        Activity::Change => classify_change_candidate(candidate, parsed),
        Activity::Execute | Activity::Configure => Ok(RuntimeDecision::ApprovalNeeded {
            activity: candidate.activity,
            tool_name: candidate.tool_name.clone(),
            arguments: candidate.arguments.clone(),
            change_preview: None,
            reason: candidate.reason.clone(),
        }),
        Activity::None | Activity::Ask => Err(RuntimeDecisionError::invalid_tool(format!(
            "tool cannot use activity {}",
            candidate.activity.as_str()
        ))),
    }
}

fn classify_change_candidate(
    candidate: &RuntimeToolCandidate,
    parsed: &ParsedRuntimeResponse,
) -> Result<RuntimeDecision, RuntimeDecisionError> {
    let mut change_preview = None;
    if candidate.tool_name == ToolName::ApplyPatch.as_str() {
        let payload_id = required_str(arguments_object(candidate)?, "payload_id")?;
        let payload = parsed
            .payloads
            .iter()
            .find(|payload| payload.id == payload_id)
            .ok_or_else(|| {
                RuntimeDecisionError::invalid_arguments(format!(
                    "payload_id has no matching payload: {payload_id}"
                ))
            })?;
        if payload.format != "apply_patch" {
            return Err(RuntimeDecisionError::invalid_arguments(format!(
                "apply_patch payload format must be apply_patch: {}",
                payload.format
            )));
        }

        let target_count = count_apply_patch_targets(&payload.body);
        if target_count != 1 {
            return Ok(RuntimeDecision::Clarify {
                message: "Patch target must be exactly one file before approval.".to_owned(),
                reason: format!("apply_patch target count was {target_count}"),
            });
        }
        change_preview = Some(parse_apply_patch_preview(payload_id, &payload.body)?);
    }

    Ok(RuntimeDecision::ApprovalNeeded {
        activity: candidate.activity,
        tool_name: candidate.tool_name.clone(),
        arguments: candidate.arguments.clone(),
        change_preview,
        reason: candidate.reason.clone(),
    })
}

fn validate_tool_activity(candidate: &RuntimeToolCandidate) -> Result<(), RuntimeDecisionError> {
    let expected = tool_spec(candidate.tool_name.as_str()).ok_or_else(|| {
        RuntimeDecisionError::invalid_tool(format!("unknown tool: {}", candidate.tool_name))
    })?;
    if expected.activity != candidate.activity {
        return Err(RuntimeDecisionError::invalid_tool(format!(
            "tool/activity mismatch: {}/{}",
            candidate.tool_name,
            candidate.activity.as_str()
        )));
    }

    Ok(())
}

fn validate_tool_arguments(candidate: &RuntimeToolCandidate) -> Result<(), RuntimeDecisionError> {
    let Some(tool_name) = ToolName::parse(candidate.tool_name.as_str()) else {
        return Err(RuntimeDecisionError::invalid_tool(format!(
            "unknown tool: {}",
            candidate.tool_name
        )));
    };

    validate_registry_tool_arguments(tool_name, &candidate.arguments)
        .map_err(RuntimeDecisionError::invalid_arguments)
}

fn validate_non_empty_plain_string(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !contains_control_char(value)
}

fn validate_workspace_relative_path(value: &str) -> bool {
    if !validate_non_empty_plain_string(value) {
        return false;
    }

    let path = Path::new(value);
    if path.is_absolute() {
        return false;
    }

    path.components().all(|component| {
        !matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    })
}

fn contains_control_char(value: &str) -> bool {
    value.chars().any(char::is_control)
}

fn arguments_object(
    candidate: &RuntimeToolCandidate,
) -> Result<&Map<String, Value>, RuntimeDecisionError> {
    candidate.arguments.as_object().ok_or_else(|| {
        RuntimeDecisionError::invalid_arguments("tool arguments must be a JSON object")
    })
}

fn required_str<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, RuntimeDecisionError> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| RuntimeDecisionError::invalid_arguments(format!("{name} must be a string")))
}

fn count_apply_patch_targets(body: &str) -> usize {
    body.lines()
        .filter(|line| {
            line.starts_with("*** Add File: ")
                || line.starts_with("*** Update File: ")
                || line.starts_with("*** Delete File: ")
        })
        .count()
}

fn parse_apply_patch_preview(
    payload_id: &str,
    body: &str,
) -> Result<ChangePreview, RuntimeDecisionError> {
    let lines = body.lines().collect::<Vec<_>>();
    if lines.first() != Some(&"*** Begin Patch") || lines.last() != Some(&"*** End Patch") {
        return Err(RuntimeDecisionError::invalid_arguments(
            "apply_patch payload must begin with *** Begin Patch and end with *** End Patch",
        ));
    }

    let mut target = None;
    for line in &lines {
        let candidate = line
            .strip_prefix("*** Add File: ")
            .map(|path| (PatchOperation::Add, path))
            .or_else(|| {
                line.strip_prefix("*** Update File: ")
                    .map(|path| (PatchOperation::Update, path))
            })
            .or_else(|| {
                line.strip_prefix("*** Delete File: ")
                    .map(|path| (PatchOperation::Delete, path))
            });
        if let Some((operation, path)) = candidate {
            target = Some((operation, path.to_owned()));
            break;
        }
    }

    let Some((operation, target_path)) = target else {
        return Err(RuntimeDecisionError::invalid_arguments(
            "apply_patch payload target was not found",
        ));
    };
    if !validate_workspace_relative_path(&target_path) {
        return Err(RuntimeDecisionError::invalid_arguments(format!(
            "apply_patch target must be workspace-relative: {target_path}"
        )));
    }

    let additions = count_patch_lines(&lines, '+')?;
    let deletions = count_patch_lines(&lines, '-')?;

    Ok(ChangePreview {
        payload_id: payload_id.to_owned(),
        target_path,
        operation,
        additions,
        deletions,
    })
}

fn count_patch_lines(lines: &[&str], marker: char) -> Result<u16, RuntimeDecisionError> {
    let count = lines.iter().filter(|line| line.starts_with(marker)).count();
    u16::try_from(count)
        .map_err(|_| RuntimeDecisionError::invalid_arguments("apply_patch line count too large"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{DecisionGate, PatchOperation, RuntimeDecision, RuntimeDecisionErrorKind};
    use crate::llm::response_parser::{
        Activity, ParsedRuntimeResponse, RuntimeAnswer, RuntimeManifest, RuntimePayload,
        RuntimeResponse, RuntimeToolCandidate,
    };

    fn manifest() -> RuntimeManifest {
        RuntimeManifest {
            tool_manifest_id: "ahreumcode.local-llm.tool-manifest.v1".to_owned(),
            tool_manifest_version: "1".to_owned(),
        }
    }

    #[test]
    fn classifies_answer() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Answer(RuntimeAnswer {
                activity: Activity::None,
                message: "done".to_owned(),
                answer_payload_id: None,
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let decision = DecisionGate::classify(&parsed).expect("answer should classify");

        assert_eq!(
            decision,
            RuntimeDecision::Answer {
                summary: "done".to_owned(),
                payload_body: None,
            }
        );
    }

    #[test]
    fn classifies_answer_summary_and_payload_body_separately() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Answer(RuntimeAnswer {
                activity: Activity::None,
                message: "summary".to_owned(),
                answer_payload_id: Some("answer_001".to_owned()),
                manifest: manifest(),
            }),
            payloads: vec![RuntimePayload {
                id: "answer_001".to_owned(),
                format: "markdown".to_owned(),
                body: "```typescript\nconsole.log(\"Hello\");\n```".to_owned(),
            }],
        };

        let decision = DecisionGate::classify(&parsed).expect("payload answer should classify");

        assert_eq!(
            decision,
            RuntimeDecision::Answer {
                summary: "summary".to_owned(),
                payload_body: Some("```typescript\nconsole.log(\"Hello\");\n```".to_owned()),
            }
        );
    }

    #[test]
    fn classifies_explore_tool_as_pending() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "need file".to_owned(),
                tool_name: "read_file".to_owned(),
                arguments: json!({"path":"README.md","start_line":1,"max_lines":80}),
                reason: "inspect".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let decision = DecisionGate::classify(&parsed).expect("tool should classify");

        assert_eq!(decision.kind(), "tool_candidate_pending");
    }

    #[test]
    fn classifies_change_tool_as_approval_needed() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Change,
                message: "patch ready".to_owned(),
                tool_name: "apply_patch".to_owned(),
                arguments: json!({"payload_id":"patch_001"}),
                reason: "change requested".to_owned(),
                manifest: manifest(),
            }),
            payloads: vec![RuntimePayload {
                id: "patch_001".to_owned(),
                format: "apply_patch".to_owned(),
                body: "*** Begin Patch\n*** Update File: src/main.rs\n*** End Patch".to_owned(),
            }],
        };

        let decision = DecisionGate::classify(&parsed).expect("change should classify");

        let RuntimeDecision::ApprovalNeeded {
            change_preview: Some(preview),
            ..
        } = decision
        else {
            panic!("change should produce approval with preview");
        };
        assert_eq!(preview.target_path, "src/main.rs");
        assert_eq!(preview.operation, PatchOperation::Update);
        assert_eq!(preview.additions, 0);
        assert_eq!(preview.deletions, 0);
    }

    #[test]
    fn extracts_apply_patch_preview_counts() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Change,
                message: "patch ready".to_owned(),
                tool_name: "apply_patch".to_owned(),
                arguments: json!({"payload_id":"patch_001"}),
                reason: "change requested".to_owned(),
                manifest: manifest(),
            }),
            payloads: vec![RuntimePayload {
                id: "patch_001".to_owned(),
                format: "apply_patch".to_owned(),
                body:
                    "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old\n+new\n*** End Patch"
                        .to_owned(),
            }],
        };

        let decision = DecisionGate::classify(&parsed).expect("change should classify");

        let RuntimeDecision::ApprovalNeeded {
            change_preview: Some(preview),
            ..
        } = decision
        else {
            panic!("change should produce preview");
        };
        assert_eq!(preview.additions, 1);
        assert_eq!(preview.deletions, 1);
    }

    #[test]
    fn rejects_malformed_apply_patch_payload() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Change,
                message: "patch ready".to_owned(),
                tool_name: "apply_patch".to_owned(),
                arguments: json!({"payload_id":"patch_001"}),
                reason: "change requested".to_owned(),
                manifest: manifest(),
            }),
            payloads: vec![RuntimePayload {
                id: "patch_001".to_owned(),
                format: "apply_patch".to_owned(),
                body: "*** Update File: src/main.rs\n*** End Patch".to_owned(),
            }],
        };

        let error = DecisionGate::classify(&parsed).expect_err("malformed patch should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
    }

    #[test]
    fn rejects_tool_activity_mismatch() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "patch ready".to_owned(),
                tool_name: "apply_patch".to_owned(),
                arguments: json!({"payload_id":"patch_001"}),
                reason: "change requested".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("mismatch should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidToolCandidate);
    }

    #[test]
    fn rejects_unknown_argument_field() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "need file".to_owned(),
                tool_name: "read_file".to_owned(),
                arguments: json!({"path":"README.md","start_line":1,"max_lines":80,"extra":true}),
                reason: "inspect".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("unknown argument should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
    }

    #[test]
    fn rejects_parent_directory_tool_path() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "need file".to_owned(),
                tool_name: "read_file".to_owned(),
                arguments: json!({"path":"../outside.md","start_line":1,"max_lines":80}),
                reason: "inspect".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("parent path should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
    }

    #[test]
    fn rejects_out_of_range_tool_limit() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "need file".to_owned(),
                tool_name: "read_file".to_owned(),
                arguments: json!({"path":"README.md","start_line":1,"max_lines":301}),
                reason: "inspect".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("limit should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
    }

    #[test]
    fn rejects_empty_command_argv() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Execute,
                message: "run command".to_owned(),
                tool_name: "run_command".to_owned(),
                arguments: json!({"argv":[],"cwd":".","timeout_ms":30000}),
                reason: "execute".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("empty argv should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
    }

    #[test]
    fn rejects_search_text_regex_argument() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "search".to_owned(),
                tool_name: "search_text".to_owned(),
                arguments: json!({"path":".","query":"fn main","use_regex":true,"max_results":20}),
                reason: "search".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("unknown option should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
    }

    #[test]
    fn rejects_unregistered_web_tool() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "search web".to_owned(),
                tool_name: "web_search".to_owned(),
                arguments: json!({"query":"rust","max_results":3}),
                reason: "search".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("unregistered tool should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidToolCandidate);
    }

    #[test]
    fn rejects_unregistered_git_scope() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "inspect git".to_owned(),
                tool_name: "inspect_git".to_owned(),
                arguments: json!({"scope":"diff_summary"}),
                reason: "inspect".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("unsupported scope should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
    }
}
