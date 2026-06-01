use std::borrow::Cow;
use std::collections::HashSet;

use super::response_parser::{
    unwrap_whole_markdown_fence, Activity, ParsedRuntimeResponse, PlanOperation, RuntimeAnswer,
    RuntimePlan, RuntimePlanItem, RuntimeResponse, RuntimeToolCandidate,
};
use crate::tool::{normalize_tool_arguments, tool_spec, ToolName};
use serde_json::{Map, Value};

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
    PlanCandidate {
        message: String,
        items: Vec<RuntimePlanItem>,
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
    pub payload_body: String,
}

impl ChangePreview {
    pub fn details(&self) -> String {
        let mut details = format!(
            "patch target: {}\noperation: {}\nadditions: {}\ndeletions: {}\npayload_id: {}",
            self.target_path,
            self.operation.as_str(),
            self.additions,
            self.deletions,
            self.payload_id
        );
        if let Ok(targets) = self.target_summaries() {
            if targets.len() > 1 {
                details.push_str("\ntargets:");
                for target in targets {
                    details.push_str(&format!(
                        "\n- {} {} (+{} -{})",
                        target.operation.as_str(),
                        target.target_path,
                        target.additions,
                        target.deletions
                    ));
                }
            }
        }
        details
    }

    pub fn target_summaries(&self) -> Result<Vec<ChangeTargetPreview>, String> {
        parse_apply_patch_target_sections(&self.payload_body).map(|sections| {
            sections
                .into_iter()
                .map(|section| section.preview)
                .collect()
        })
    }

    pub fn split_by_target(&self) -> Result<Vec<ChangePreview>, String> {
        parse_apply_patch_target_sections(&self.payload_body).map(|sections| {
            sections
                .into_iter()
                .map(|section| ChangePreview {
                    payload_id: self.payload_id.clone(),
                    target_path: section.preview.target_path,
                    operation: section.preview.operation,
                    additions: section.preview.additions,
                    deletions: section.preview.deletions,
                    payload_body: section.payload_body,
                })
                .collect()
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeTargetPreview {
    pub target_path: String,
    pub operation: PatchOperation,
    pub additions: u16,
    pub deletions: u16,
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
            Self::PlanCandidate { .. } => "plan_candidate",
            Self::ToolCandidatePending { .. } => "tool_candidate_pending",
            Self::ApprovalNeeded { .. } => "approval_needed",
        }
    }

    pub fn activity(&self) -> Option<Activity> {
        match self {
            Self::ToolCandidatePending { activity, .. } | Self::ApprovalNeeded { activity, .. } => {
                Some(*activity)
            }
            Self::PlanCandidate { .. } => Some(Activity::None),
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
            RuntimeResponse::Plan(response) => classify_plan(response),
            RuntimeResponse::Tool(candidate) => classify_tool_candidate(candidate, parsed),
        }
    }
}

fn classify_plan(response: &RuntimePlan) -> Result<RuntimeDecision, RuntimeDecisionError> {
    if !response.plan_items.iter().any(is_executable_plan_item) {
        return Err(RuntimeDecisionError::invalid_arguments(
            "plan requires at least one executable workspace item; advisory, summary, explanation, or team-perspective responses must use answer",
        ));
    }

    Ok(RuntimeDecision::PlanCandidate {
        message: response.message.clone(),
        items: response.plan_items.clone(),
        reason: response.reason.clone(),
    })
}

fn is_executable_plan_item(item: &RuntimePlanItem) -> bool {
    match item.operation {
        PlanOperation::Read
        | PlanOperation::Create
        | PlanOperation::Update
        | PlanOperation::Delete => item
            .target
            .as_deref()
            .is_some_and(|target| !target.is_empty()),
        PlanOperation::Execute => true,
        PlanOperation::Verify | PlanOperation::Answer => false,
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
    let arguments = normalize_candidate_arguments(candidate)?;

    match candidate.activity {
        Activity::Explore => Ok(RuntimeDecision::ToolCandidatePending {
            activity: candidate.activity,
            tool_name: candidate.tool_name.clone(),
            arguments,
            summary: candidate.message.clone(),
        }),
        Activity::Change => classify_change_candidate(candidate, parsed, arguments),
        Activity::Execute | Activity::Configure => Ok(RuntimeDecision::ApprovalNeeded {
            activity: candidate.activity,
            tool_name: candidate.tool_name.clone(),
            arguments,
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
    arguments: Value,
) -> Result<RuntimeDecision, RuntimeDecisionError> {
    let mut change_preview = None;
    if candidate.tool_name == ToolName::ApplyPatch.as_str() {
        let payload_id = required_str(arguments_value_object(&arguments)?, "payload_id")?;
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

        let patch_body = normalize_apply_patch_payload_body(&payload.body);
        change_preview = Some(parse_apply_patch_preview(payload_id, patch_body.as_ref())?);
    }

    Ok(RuntimeDecision::ApprovalNeeded {
        activity: candidate.activity,
        tool_name: candidate.tool_name.clone(),
        arguments,
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

fn normalize_candidate_arguments(
    candidate: &RuntimeToolCandidate,
) -> Result<Value, RuntimeDecisionError> {
    let Some(tool_name) = ToolName::parse(candidate.tool_name.as_str()) else {
        return Err(RuntimeDecisionError::invalid_tool(format!(
            "unknown tool: {}",
            candidate.tool_name
        )));
    };

    normalize_tool_arguments(tool_name, &candidate.arguments)
        .map_err(RuntimeDecisionError::invalid_arguments)
}

fn validate_non_empty_plain_string(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !contains_control_char(value)
}

fn contains_control_char(value: &str) -> bool {
    value.chars().any(char::is_control)
}

fn arguments_value_object(arguments: &Value) -> Result<&Map<String, Value>, RuntimeDecisionError> {
    arguments.as_object().ok_or_else(|| {
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

fn normalize_apply_patch_payload_body(body: &str) -> Cow<'_, str> {
    let trimmed = unwrap_whole_markdown_fence(body.trim()).trim();
    let Some(start) = trimmed.find("*** Begin Patch") else {
        let Some(target_start) = first_apply_patch_target_index(trimmed) else {
            return Cow::Borrowed(trimmed);
        };
        let patch_body = if let Some(end_start) = trimmed[target_start..].rfind("*** End Patch") {
            let end = target_start + end_start;
            trimmed[target_start..end].trim()
        } else {
            trimmed[target_start..].trim()
        };
        if count_apply_patch_targets(patch_body) > 0 {
            return Cow::Owned(format!("*** Begin Patch\n{patch_body}\n*** End Patch"));
        }
        return Cow::Borrowed(trimmed);
    };
    let Some(end_start) = trimmed[start..].rfind("*** End Patch") else {
        let patch_body = trimmed[start..].trim();
        if count_apply_patch_targets(patch_body) > 0 {
            return Cow::Owned(format!("{patch_body}\n*** End Patch"));
        }
        return Cow::Borrowed(trimmed);
    };
    let end = start + end_start + "*** End Patch".len();
    Cow::Borrowed(trimmed[start..end].trim())
}

fn first_apply_patch_target_index(body: &str) -> Option<usize> {
    ["*** Add File: ", "*** Update File: ", "*** Delete File: "]
        .into_iter()
        .filter_map(|marker| body.find(marker))
        .min()
}

fn parse_apply_patch_preview(
    payload_id: &str,
    body: &str,
) -> Result<ChangePreview, RuntimeDecisionError> {
    let sections =
        parse_apply_patch_target_sections(body).map_err(RuntimeDecisionError::invalid_arguments)?;
    let first = sections
        .first()
        .expect("parse_apply_patch_target_sections returns at least one target");
    let target_path = if sections.len() == 1 {
        first.preview.target_path.clone()
    } else {
        let paths = sections
            .iter()
            .map(|section| section.preview.target_path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{} targets: {paths}", sections.len())
    };
    let additions = sum_patch_counts(sections.iter().map(|section| section.preview.additions))?;
    let deletions = sum_patch_counts(sections.iter().map(|section| section.preview.deletions))?;

    Ok(ChangePreview {
        payload_id: payload_id.to_owned(),
        target_path,
        operation: first.preview.operation,
        additions,
        deletions,
        payload_body: body.to_owned(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PatchTargetSection {
    preview: ChangeTargetPreview,
    payload_body: String,
}

fn parse_apply_patch_target_sections(body: &str) -> Result<Vec<PatchTargetSection>, String> {
    let lines = body.lines().collect::<Vec<_>>();
    if lines.first() != Some(&"*** Begin Patch") || lines.last() != Some(&"*** End Patch") {
        return Err(
            "apply_patch payload must begin with *** Begin Patch and end with *** End Patch"
                .to_owned(),
        );
    }

    let mut starts = Vec::new();
    for (index, line) in lines
        .iter()
        .enumerate()
        .take(lines.len().saturating_sub(1))
        .skip(1)
    {
        if parse_target_header(line).is_some() {
            starts.push(index);
        }
    }
    if starts.is_empty() {
        return Err("apply_patch payload target was not found".to_owned());
    }

    let mut seen = HashSet::new();
    let mut sections = Vec::new();
    for (position, target_index) in starts.iter().copied().enumerate() {
        let next_index = starts.get(position + 1).copied().unwrap_or(lines.len() - 1);
        let (operation, target_path) = parse_target_header(lines[target_index])
            .expect("target index came from parse_target_header");
        if !validate_non_empty_plain_string(target_path) {
            return Err(format!(
                "apply_patch target must be a non-empty path string: {target_path}"
            ));
        }
        if !seen.insert(target_path.to_owned()) {
            return Err(format!(
                "apply_patch target path is duplicated: {target_path}"
            ));
        }

        let body_lines = &lines[target_index + 1..next_index];
        let normalized_body_lines = normalize_target_body_lines(operation, body_lines)?;
        let normalized_body_refs = normalized_body_lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let count_lines = normalized_body_refs.as_slice();
        let additions = count_patch_lines(count_lines, '+').map_err(|error| error.message)?;
        let deletions = count_patch_lines(count_lines, '-').map_err(|error| error.message)?;
        if operation == PatchOperation::Update && additions == 0 && deletions == 0 {
            return Err(
                "Update File patch must include at least one added or removed line".to_owned(),
            );
        }
        if operation == PatchOperation::Delete && !body_lines.is_empty() {
            return Err("Delete File patch must not include patch body lines".to_owned());
        }

        let mut payload_lines = vec!["*** Begin Patch".to_owned(), lines[target_index].to_owned()];
        payload_lines.extend(normalized_body_lines);
        payload_lines.push("*** End Patch".to_owned());
        sections.push(PatchTargetSection {
            preview: ChangeTargetPreview {
                target_path: target_path.to_owned(),
                operation,
                additions,
                deletions,
            },
            payload_body: payload_lines.join("\n"),
        });
    }

    Ok(sections)
}

fn parse_target_header(line: &str) -> Option<(PatchOperation, &str)> {
    line.strip_prefix("*** Add File: ")
        .map(|path| (PatchOperation::Add, path))
        .or_else(|| {
            line.strip_prefix("*** Update File: ")
                .map(|path| (PatchOperation::Update, path))
        })
        .or_else(|| {
            line.strip_prefix("*** Delete File: ")
                .map(|path| (PatchOperation::Delete, path))
        })
}

fn normalize_target_body_lines(
    operation: PatchOperation,
    body_lines: &[&str],
) -> Result<Vec<String>, String> {
    match operation {
        PatchOperation::Add => body_lines
            .iter()
            .map(|line| {
                if parse_target_header(line).is_some()
                    || *line == "*** Begin Patch"
                    || *line == "*** End Patch"
                {
                    return Err(format!(
                        "Add File patch body contains a patch control line: {line}"
                    ));
                }
                Ok(line
                    .strip_prefix('+')
                    .map(|content| format!("+{content}"))
                    .unwrap_or_else(|| format!("+{line}")))
            })
            .collect(),
        PatchOperation::Update | PatchOperation::Delete => {
            Ok(body_lines.iter().map(|line| (*line).to_owned()).collect())
        }
    }
}

fn sum_patch_counts(mut counts: impl Iterator<Item = u16>) -> Result<u16, RuntimeDecisionError> {
    counts
        .try_fold(0u32, |sum, count| Ok(sum + u32::from(count)))
        .and_then(|sum| {
            u16::try_from(sum).map_err(|_| {
                RuntimeDecisionError::invalid_arguments("apply_patch line count too large")
            })
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
        Activity, ParsedRuntimeResponse, PlanOperation, RuntimeAnswer, RuntimeManifest,
        RuntimePayload, RuntimePlan, RuntimePlanItem, RuntimeResponse, RuntimeToolCandidate,
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
    fn classifies_plan_candidate_as_runtime_ledger() {
        let items = vec![
            RuntimePlanItem {
                operation: PlanOperation::Create,
                target: Some("web/index.html".to_owned()),
            },
            RuntimePlanItem {
                operation: PlanOperation::Create,
                target: Some("web/app.js".to_owned()),
            },
        ];
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Plan(RuntimePlan {
                activity: Activity::None,
                message: "planned targets".to_owned(),
                plan_items: items.clone(),
                reason: "multi-target request".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let decision = DecisionGate::classify(&parsed).expect("plan should classify");

        assert_eq!(
            decision,
            RuntimeDecision::PlanCandidate {
                message: "planned targets".to_owned(),
                items,
                reason: "multi-target request".to_owned(),
            }
        );
        assert_eq!(decision.kind(), "plan_candidate");
    }

    #[test]
    fn rejects_non_executable_plan_candidate() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Plan(RuntimePlan {
                activity: Activity::None,
                message: "advisory plan".to_owned(),
                plan_items: vec![
                    RuntimePlanItem {
                        operation: PlanOperation::Answer,
                        target: None,
                    },
                    RuntimePlanItem {
                        operation: PlanOperation::Verify,
                        target: None,
                    },
                ],
                reason: "advisory request".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("advisory plan should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
        assert!(error.message.contains("advisory"));
    }

    #[test]
    fn classifies_execute_plan_candidate_as_runtime_ledger() {
        let items = vec![RuntimePlanItem {
            operation: PlanOperation::Execute,
            target: None,
        }];
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Plan(RuntimePlan {
                activity: Activity::None,
                message: "planned command".to_owned(),
                plan_items: items.clone(),
                reason: "ordered execution".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let decision = DecisionGate::classify(&parsed).expect("execute plan should classify");

        assert_eq!(
            decision,
            RuntimeDecision::PlanCandidate {
                message: "planned command".to_owned(),
                items,
                reason: "ordered execution".to_owned(),
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
    fn rejects_integer_string_tool_arguments_before_runtime() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "search".to_owned(),
                tool_name: "search_text".to_owned(),
                arguments: json!({"path":"src","query":"RuntimeDecision","max_results":"20"}),
                reason: "inspect".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let error = DecisionGate::classify(&parsed).expect_err("numeric string should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
        assert_eq!(error.message, "invalid argument type or value: max_results");
    }

    #[test]
    fn fills_explore_bound_defaults_before_runtime() {
        let parsed = ParsedRuntimeResponse {
            response: RuntimeResponse::Tool(RuntimeToolCandidate {
                activity: Activity::Explore,
                message: "read".to_owned(),
                tool_name: "read_file".to_owned(),
                arguments: json!({"path":"Cargo.toml"}),
                reason: "inspect".to_owned(),
                manifest: manifest(),
            }),
            payloads: Vec::new(),
        };

        let decision = DecisionGate::classify(&parsed).expect("bounds should default");

        let RuntimeDecision::ToolCandidatePending { arguments, .. } = decision else {
            panic!("expected pending tool");
        };
        assert_eq!(
            arguments,
            json!({"path":"Cargo.toml","start_line":1,"max_lines":120})
        );
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
            panic!("change should produce approval with preview");
        };
        assert_eq!(preview.target_path, "src/main.rs");
        assert_eq!(preview.operation, PatchOperation::Update);
        assert_eq!(preview.additions, 1);
        assert_eq!(preview.deletions, 1);
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
    fn rejects_noop_update_patch_before_approval() {
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
                body: concat!(
                    "*** Begin Patch\n",
                    "*** Update File: sample.txt\n",
                    "@@\n",
                    " existing line\n",
                    "*** End Patch"
                )
                .to_owned(),
            }],
        };

        let error = DecisionGate::classify(&parsed).expect_err("noop update should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
        assert!(error.message.contains("at least one added or removed line"));
    }

    #[test]
    fn trims_apply_patch_payload_outer_whitespace_before_preview() {
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
                    "\n  *** Begin Patch\n*** Add File: fixture-target.txt\n+hello\n*** End Patch  \n"
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
        assert_eq!(preview.target_path, "fixture-target.txt");
        assert!(preview.payload_body.starts_with("*** Begin Patch"));
        assert!(preview.payload_body.ends_with("*** End Patch"));
    }

    #[test]
    fn unwraps_apply_patch_payload_whole_markdown_fence_before_preview() {
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
                body: concat!(
                    "```patch\n",
                    "*** Begin Patch\n",
                    "*** Add File: fixture-target.txt\n",
                    "+hello\n",
                    "*** End Patch\n",
                    "```"
                )
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
        assert_eq!(preview.target_path, "fixture-target.txt");
        assert!(preview.payload_body.starts_with("*** Begin Patch"));
    }

    #[test]
    fn extracts_apply_patch_marker_segment_from_payload_wrapper_text() {
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
                body: concat!(
                    "patch body:\n",
                    "*** Begin Patch\n",
                    "*** Add File: fixture-target.txt\n",
                    "+hello\n",
                    "*** End Patch\n",
                    "done"
                )
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
        assert_eq!(preview.target_path, "fixture-target.txt");
        assert_eq!(
            preview.payload_body,
            "*** Begin Patch\n*** Add File: fixture-target.txt\n+hello\n*** End Patch"
        );
    }

    #[test]
    fn wraps_single_target_apply_patch_payload_body_without_outer_markers() {
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
                body: concat!("*** Add File: fixture-target.txt\n", "+hello\n", "+world")
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
        assert_eq!(preview.target_path, "fixture-target.txt");
        assert_eq!(preview.additions, 2);
        assert_eq!(
            preview.payload_body,
            "*** Begin Patch\n*** Add File: fixture-target.txt\n+hello\n+world\n*** End Patch"
        );
    }

    #[test]
    fn wraps_single_target_apply_patch_payload_body_missing_begin_marker() {
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
                body: "*** Update File: src/main.rs\n@@\n-old\n+new\n*** End Patch".to_owned(),
            }],
        };

        let decision =
            DecisionGate::classify(&parsed).expect("single target patch should classify");

        let RuntimeDecision::ApprovalNeeded {
            change_preview: Some(preview),
            ..
        } = decision
        else {
            panic!("change should produce preview");
        };
        assert_eq!(
            preview.payload_body,
            "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old\n+new\n*** End Patch"
        );
    }

    #[test]
    fn completes_single_target_apply_patch_payload_body_missing_end_marker() {
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
                body: "*** Begin Patch\n*** Add File: fixture-target.txt\n+hello".to_owned(),
            }],
        };

        let decision =
            DecisionGate::classify(&parsed).expect("single target patch should classify");

        let RuntimeDecision::ApprovalNeeded {
            change_preview: Some(preview),
            ..
        } = decision
        else {
            panic!("change should produce preview");
        };
        assert_eq!(
            preview.payload_body,
            "*** Begin Patch\n*** Add File: fixture-target.txt\n+hello\n*** End Patch"
        );
    }

    #[test]
    fn accepts_multi_target_apply_patch_as_one_change_candidate() {
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
                body: concat!(
                    "*** Begin Patch\n",
                    "*** Update File: src/main.rs\n",
                    "@@\n",
                    "-old\n",
                    "+new\n",
                    "*** Update File: src/lib.rs\n",
                    "@@\n",
                    "-old\n",
                    "+new\n",
                    "*** End Patch"
                )
                .to_owned(),
            }],
        };

        let decision = DecisionGate::classify(&parsed).expect("multi target patch should classify");

        let RuntimeDecision::ApprovalNeeded {
            change_preview: Some(preview),
            ..
        } = decision
        else {
            panic!("change should produce preview");
        };
        assert_eq!(preview.target_path, "2 targets: src/main.rs, src/lib.rs");
        assert_eq!(preview.additions, 2);
        assert_eq!(preview.deletions, 2);
        assert_eq!(
            preview.target_summaries().expect("target summaries").len(),
            2
        );
    }

    #[test]
    fn wraps_multi_target_apply_patch_payload_body_without_outer_markers() {
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
                body: concat!(
                    "*** Add File: web/index.html\n",
                    "+<script src=\"game.js\"></script>\n",
                    "*** Add File: web/game.js\n",
                    "+console.log('ready');"
                )
                .to_owned(),
            }],
        };

        let decision = DecisionGate::classify(&parsed).expect("multi target patch should classify");

        let RuntimeDecision::ApprovalNeeded {
            change_preview: Some(preview),
            ..
        } = decision
        else {
            panic!("change should produce preview");
        };
        assert_eq!(
            preview.target_path,
            "2 targets: web/index.html, web/game.js"
        );
        assert_eq!(preview.additions, 2);
        assert!(preview.payload_body.starts_with("*** Begin Patch"));
        assert!(preview.payload_body.ends_with("*** End Patch"));
    }

    #[test]
    fn normalizes_add_file_body_lines_without_plus_prefix() {
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
                body: concat!(
                    "*** Begin Patch\n",
                    "*** Add File: web/index.html\n",
                    "<!doctype html>\n",
                    "<script src=\"game.js\"></script>\n",
                    "*** Add File: web/game.js\n",
                    "console.log('ready');\n",
                    "*** End Patch"
                )
                .to_owned(),
            }],
        };

        let decision = DecisionGate::classify(&parsed).expect("add body should normalize");

        let RuntimeDecision::ApprovalNeeded {
            change_preview: Some(preview),
            ..
        } = decision
        else {
            panic!("change should produce preview");
        };
        assert_eq!(preview.additions, 3);
        let target_previews = preview.split_by_target().expect("split targets");
        assert_eq!(
            target_previews[0].payload_body,
            concat!(
                "*** Begin Patch\n",
                "*** Add File: web/index.html\n",
                "+<!doctype html>\n",
                "+<script src=\"game.js\"></script>\n",
                "*** End Patch"
            )
        );
        assert_eq!(
            target_previews[1].payload_body,
            concat!(
                "*** Begin Patch\n",
                "*** Add File: web/game.js\n",
                "+console.log('ready');\n",
                "*** End Patch"
            )
        );
    }

    #[test]
    fn rejects_delete_patch_with_body_lines_before_approval() {
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
                body: concat!(
                    "*** Begin Patch\n",
                    "*** Delete File: src/main.rs\n",
                    "-old\n",
                    "*** End Patch"
                )
                .to_owned(),
            }],
        };

        let error = DecisionGate::classify(&parsed).expect_err("delete body should fail");

        assert_eq!(error.kind, RuntimeDecisionErrorKind::InvalidArguments);
        assert!(error.message.contains("must not include patch body lines"));
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
    fn preserves_parent_directory_tool_path_for_permission_policy() {
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

        let decision =
            DecisionGate::classify(&parsed).expect("external path policy should classify later");

        assert_eq!(decision.kind(), "tool_candidate_pending");
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
    fn classifies_registered_web_tool_as_pending() {
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

        let decision = DecisionGate::classify(&parsed).expect("registered web tool should pass");

        assert_eq!(decision.kind(), "tool_candidate_pending");
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
