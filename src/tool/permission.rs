use serde_json::Value;

use crate::config::RuntimeConfig;
use crate::llm::{Activity, RuntimeDecision};

pub enum PermissionDecision {
    Allow,
    Ask(PermissionRequest),
    Deny(PermissionDenial),
}

impl PermissionDecision {
    pub fn branch(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask(_) => "ask",
            Self::Deny(_) => "deny",
        }
    }
}

pub struct PermissionRequest {
    pub title: String,
    pub reason: String,
    pub action: String,
    pub details: String,
}

pub struct PermissionDenial {
    pub reason: String,
    pub message: String,
}

pub struct PermissionGate;

impl PermissionGate {
    pub fn evaluate(config: &RuntimeConfig, decision: &RuntimeDecision) -> PermissionDecision {
        match decision {
            RuntimeDecision::ToolCandidatePending {
                activity,
                tool_name,
                arguments,
                summary,
            } => evaluate_explore(config, *activity, tool_name, arguments, summary),
            RuntimeDecision::ApprovalNeeded {
                activity,
                tool_name,
                arguments,
                change_preview,
                reason,
            } => evaluate_approval_needed(*activity, tool_name, arguments, change_preview, reason),
            _ => PermissionDecision::Allow,
        }
    }
}

fn evaluate_explore(
    config: &RuntimeConfig,
    activity: Activity,
    tool_name: &str,
    arguments: &Value,
    summary: &str,
) -> PermissionDecision {
    match tool_name {
        "list_files" | "search_text" | "read_file" | "inspect_git" => PermissionDecision::Allow,
        "web_search" | "web_fetch" => {
            if !config.web.enabled {
                return PermissionDecision::Deny(PermissionDenial {
                    reason: "web_disabled".to_owned(),
                    message: "web/network tool is disabled by configuration".to_owned(),
                });
            }

            PermissionDecision::Ask(PermissionRequest {
                title: "Approval required".to_owned(),
                reason: "Network access requires user approval.".to_owned(),
                action: format!("{tool_name} ({})", activity.as_str()),
                details: format!("summary: {summary}\narguments: {arguments}"),
            })
        }
        _ => PermissionDecision::Deny(PermissionDenial {
            reason: "unsupported_explore_tool".to_owned(),
            message: format!("Explore tool is not available in permission branch: {tool_name}"),
        }),
    }
}

fn evaluate_approval_needed(
    activity: Activity,
    tool_name: &str,
    arguments: &Value,
    change_preview: &Option<crate::llm::ChangePreview>,
    reason: &str,
) -> PermissionDecision {
    if tool_name == "run_command" {
        if let Some(denial) = hard_safety_command_denial(arguments) {
            return PermissionDecision::Deny(denial);
        }
    }

    PermissionDecision::Ask(PermissionRequest {
        title: "Approval required".to_owned(),
        reason: approval_reason(activity).to_owned(),
        action: format!("{tool_name} ({})", activity.as_str()),
        details: approval_details(reason, arguments, change_preview),
    })
}

fn approval_details(
    reason: &str,
    arguments: &Value,
    change_preview: &Option<crate::llm::ChangePreview>,
) -> String {
    match change_preview {
        Some(preview) => format!(
            "reason: {reason}\n{}\narguments: {arguments}",
            preview.details()
        ),
        None => format!("reason: {reason}\narguments: {arguments}"),
    }
}

fn approval_reason(activity: Activity) -> &'static str {
    match activity {
        Activity::Change => "Change tools require approval before preview or execution.",
        Activity::Execute => "Command execution requires approval before execution.",
        Activity::Configure => "Configuration changes require approval before applying.",
        _ => "This tool requires approval before continuing.",
    }
}

fn hard_safety_command_denial(arguments: &Value) -> Option<PermissionDenial> {
    let argv = arguments
        .as_object()
        .and_then(|object| object.get("argv"))
        .and_then(Value::as_array)?;
    let command = argv.first().and_then(Value::as_str)?;

    if matches!(command, "sudo" | "su" | "kill" | "killall") {
        return Some(PermissionDenial {
            reason: "hard_safety_command".to_owned(),
            message: format!("{command} is blocked by hard safety limits"),
        });
    }

    if command == "rm" && rm_has_recursive_force(argv) {
        return Some(PermissionDenial {
            reason: "hard_safety_command".to_owned(),
            message: "recursive force delete is blocked by hard safety limits".to_owned(),
        });
    }

    None
}

fn rm_has_recursive_force(argv: &[Value]) -> bool {
    let flags = argv
        .iter()
        .skip(1)
        .filter_map(Value::as_str)
        .filter(|value| value.starts_with('-'))
        .collect::<Vec<_>>();
    let recursive = flags
        .iter()
        .any(|flag| flag.contains('r') || flag.contains('R'));
    let force = flags.iter().any(|flag| flag.contains('f'));
    recursive && force
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{PermissionDecision, PermissionGate};
    use crate::config::RuntimeConfig;
    use crate::llm::{Activity, RuntimeDecision};

    fn config() -> RuntimeConfig {
        RuntimeConfig::default_local("target/tool-permission-test.toml".into())
    }

    #[test]
    fn allows_local_explore_tool() {
        let decision = RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name: "read_file".to_owned(),
            arguments: json!({"path":"README.md","start_line":1,"max_lines":40}),
            summary: "read".to_owned(),
        };

        assert!(matches!(
            PermissionGate::evaluate(&config(), &decision),
            PermissionDecision::Allow
        ));
    }

    #[test]
    fn asks_for_web_explore_when_web_enabled() {
        let decision = RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name: "web_search".to_owned(),
            arguments: json!({"query":"rust","max_results":3}),
            summary: "search web".to_owned(),
        };

        assert!(matches!(
            PermissionGate::evaluate(&config(), &decision),
            PermissionDecision::Ask(_)
        ));
    }

    #[test]
    fn denies_recursive_force_delete_command() {
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Execute,
            tool_name: "run_command".to_owned(),
            arguments: json!({"argv":["rm","-rf","target"],"cwd":".","timeout_ms":30000}),
            change_preview: None,
            reason: "delete".to_owned(),
        };

        assert!(matches!(
            PermissionGate::evaluate(&config(), &decision),
            PermissionDecision::Deny(_)
        ));
    }
}
