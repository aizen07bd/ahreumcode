use serde_json::{Map, Value};

use super::response_parser::{
    Activity, ParsedRuntimeResponse, RuntimeAnswer, RuntimeResponse, RuntimeToolCandidate,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeDecision {
    Answer {
        message: String,
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
        summary: String,
    },
    ApprovalNeeded {
        activity: Activity,
        tool_name: String,
        reason: String,
    },
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
            message: response.message.clone(),
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
        message: payload.body.clone(),
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
            summary: candidate.message.clone(),
        }),
        Activity::Change => classify_change_candidate(candidate, parsed),
        Activity::Execute | Activity::Configure => Ok(RuntimeDecision::ApprovalNeeded {
            activity: candidate.activity,
            tool_name: candidate.tool_name.clone(),
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
    if candidate.tool_name == "apply_patch" {
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
    }

    Ok(RuntimeDecision::ApprovalNeeded {
        activity: candidate.activity,
        tool_name: candidate.tool_name.clone(),
        reason: candidate.reason.clone(),
    })
}

fn validate_tool_activity(candidate: &RuntimeToolCandidate) -> Result<(), RuntimeDecisionError> {
    let expected = expected_activity(candidate.tool_name.as_str()).ok_or_else(|| {
        RuntimeDecisionError::invalid_tool(format!("unknown tool: {}", candidate.tool_name))
    })?;
    if expected != candidate.activity {
        return Err(RuntimeDecisionError::invalid_tool(format!(
            "tool/activity mismatch: {}/{}",
            candidate.tool_name,
            candidate.activity.as_str()
        )));
    }

    Ok(())
}

fn expected_activity(tool_name: &str) -> Option<Activity> {
    match tool_name {
        "list_files" | "find_files" | "search_text" | "read_file" | "inspect_git"
        | "web_search" | "web_fetch" => Some(Activity::Explore),
        "apply_patch" => Some(Activity::Change),
        "run_command" => Some(Activity::Execute),
        "add_provider" | "update_config" => Some(Activity::Configure),
        _ => None,
    }
}

fn validate_tool_arguments(candidate: &RuntimeToolCandidate) -> Result<(), RuntimeDecisionError> {
    match candidate.tool_name.as_str() {
        "list_files" => validate_arguments(
            candidate,
            &[
                ArgumentRule::string("path"),
                ArgumentRule::integer("max_depth"),
                ArgumentRule::integer("max_entries"),
            ],
        ),
        "find_files" => validate_arguments(
            candidate,
            &[
                ArgumentRule::string("path"),
                ArgumentRule::string("pattern"),
                ArgumentRule::integer("max_results"),
            ],
        ),
        "search_text" => validate_arguments(
            candidate,
            &[
                ArgumentRule::string("path"),
                ArgumentRule::string("query"),
                ArgumentRule::boolean("use_regex"),
                ArgumentRule::integer("max_results"),
            ],
        ),
        "read_file" => validate_arguments(
            candidate,
            &[
                ArgumentRule::string("path"),
                ArgumentRule::integer("start_line"),
                ArgumentRule::integer("max_lines"),
            ],
        ),
        "inspect_git" => validate_arguments(
            candidate,
            &[ArgumentRule::string_enum(
                "scope",
                &["status", "diff_summary", "recent_commits"],
            )],
        ),
        "web_search" => validate_arguments(
            candidate,
            &[
                ArgumentRule::string("query"),
                ArgumentRule::integer("max_results"),
            ],
        ),
        "web_fetch" => validate_arguments(
            candidate,
            &[
                ArgumentRule::string("url"),
                ArgumentRule::integer("max_bytes"),
            ],
        ),
        "apply_patch" => validate_arguments(candidate, &[ArgumentRule::string("payload_id")]),
        "run_command" => validate_arguments(
            candidate,
            &[
                ArgumentRule::string_array("argv"),
                ArgumentRule::string("cwd"),
                ArgumentRule::integer("timeout_ms"),
            ],
        ),
        "add_provider" => validate_arguments(
            candidate,
            &[
                ArgumentRule::string("provider_id"),
                ArgumentRule::string("base_url"),
                ArgumentRule::string("model"),
                ArgumentRule::integer("context_tokens"),
            ],
        ),
        "update_config" => validate_arguments(
            candidate,
            &[ArgumentRule::string("key_path"), ArgumentRule::any("value")],
        ),
        _ => Err(RuntimeDecisionError::invalid_tool(format!(
            "unknown tool: {}",
            candidate.tool_name
        ))),
    }
}

struct ArgumentRule<'a> {
    name: &'a str,
    kind: ArgumentKind<'a>,
}

impl<'a> ArgumentRule<'a> {
    fn string(name: &'a str) -> Self {
        Self {
            name,
            kind: ArgumentKind::String,
        }
    }

    fn integer(name: &'a str) -> Self {
        Self {
            name,
            kind: ArgumentKind::Integer,
        }
    }

    fn boolean(name: &'a str) -> Self {
        Self {
            name,
            kind: ArgumentKind::Boolean,
        }
    }

    fn string_array(name: &'a str) -> Self {
        Self {
            name,
            kind: ArgumentKind::StringArray,
        }
    }

    fn string_enum(name: &'a str, values: &'a [&'a str]) -> Self {
        Self {
            name,
            kind: ArgumentKind::StringEnum(values),
        }
    }

    fn any(name: &'a str) -> Self {
        Self {
            name,
            kind: ArgumentKind::Any,
        }
    }
}

enum ArgumentKind<'a> {
    String,
    Integer,
    Boolean,
    StringArray,
    StringEnum(&'a [&'a str]),
    Any,
}

fn validate_arguments(
    candidate: &RuntimeToolCandidate,
    rules: &[ArgumentRule<'_>],
) -> Result<(), RuntimeDecisionError> {
    let arguments = arguments_object(candidate)?;
    for key in arguments.keys() {
        if !rules.iter().any(|rule| rule.name == key) {
            return Err(RuntimeDecisionError::invalid_arguments(format!(
                "unknown argument field: {key}"
            )));
        }
    }

    for rule in rules {
        let value = arguments.get(rule.name).ok_or_else(|| {
            RuntimeDecisionError::invalid_arguments(format!("missing argument: {}", rule.name))
        })?;
        validate_argument_value(rule, value)?;
    }

    Ok(())
}

fn validate_argument_value(
    rule: &ArgumentRule<'_>,
    value: &Value,
) -> Result<(), RuntimeDecisionError> {
    let valid = match rule.kind {
        ArgumentKind::String => value.as_str().is_some(),
        ArgumentKind::Integer => value.as_i64().is_some(),
        ArgumentKind::Boolean => value.as_bool().is_some(),
        ArgumentKind::StringArray => value
            .as_array()
            .map(|items| items.iter().all(|item| item.as_str().is_some()))
            .unwrap_or(false),
        ArgumentKind::StringEnum(values) => value
            .as_str()
            .map(|actual| values.contains(&actual))
            .unwrap_or(false),
        ArgumentKind::Any => true,
    };

    if valid {
        Ok(())
    } else {
        Err(RuntimeDecisionError::invalid_arguments(format!(
            "invalid argument type or value: {}",
            rule.name
        )))
    }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{DecisionGate, RuntimeDecision, RuntimeDecisionErrorKind};
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
                message: "done".to_owned()
            }
        );
    }

    #[test]
    fn classifies_answer_payload_body_as_answer_message() {
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
                message: "```typescript\nconsole.log(\"Hello\");\n```".to_owned()
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

        assert_eq!(decision.kind(), "approval_needed");
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
}
