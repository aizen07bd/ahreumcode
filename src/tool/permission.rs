use serde_json::Value;

use crate::config::RuntimeConfig;
use crate::llm::{Activity, RuntimeDecision};

use crate::tool::{tool_spec, CommandPolicy, CommandPolicyDecision, ToolName, ToolPermission};

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
    _config: &RuntimeConfig,
    activity: Activity,
    tool_name: &str,
    arguments: &Value,
    summary: &str,
) -> PermissionDecision {
    let Some(spec) = tool_spec(tool_name) else {
        return PermissionDecision::Deny(PermissionDenial {
            reason: "unsupported_explore_tool".to_owned(),
            message: format!("Explore tool is not available in permission branch: {tool_name}"),
        });
    };

    if spec.activity != activity {
        return PermissionDecision::Deny(PermissionDenial {
            reason: "tool_activity_mismatch".to_owned(),
            message: format!("Tool activity does not match registry contract: {tool_name}"),
        });
    }

    match spec.permission {
        ToolPermission::Allow => PermissionDecision::Allow,
        ToolPermission::Ask => PermissionDecision::Ask(PermissionRequest {
            title: "Approval required".to_owned(),
            reason: approval_reason(activity).to_owned(),
            action: format!("{tool_name} ({})", activity.as_str()),
            details: format!("summary: {summary}\narguments: {arguments}"),
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
    let Some(spec) = tool_spec(tool_name) else {
        return PermissionDecision::Deny(PermissionDenial {
            reason: "unsupported_approval_tool".to_owned(),
            message: format!("Approval tool is not available in permission branch: {tool_name}"),
        });
    };

    if spec.activity != activity {
        return PermissionDecision::Deny(PermissionDenial {
            reason: "tool_activity_mismatch".to_owned(),
            message: format!("Tool activity does not match registry contract: {tool_name}"),
        });
    }

    if spec.permission != ToolPermission::Ask {
        return PermissionDecision::Deny(PermissionDenial {
            reason: "unexpected_approval_branch".to_owned(),
            message: format!("Tool does not require approval in registry contract: {tool_name}"),
        });
    }

    if tool_name == ToolName::RunCommand.as_str() {
        match CommandPolicy::evaluate(arguments) {
            CommandPolicyDecision::ApprovalRequired { .. } => {}
            CommandPolicyDecision::ManualOnly { capability, reason } => {
                return PermissionDecision::Deny(PermissionDenial {
                    reason: "manual_only_command".to_owned(),
                    message: format!(
                        "{reason}; capability={} is guidance-only and will not be executed by AhreumCode",
                        capability.as_str()
                    ),
                });
            }
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
    fn denies_tool_outside_runtime_contract() {
        let decision = RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name: "web_search".to_owned(),
            arguments: json!({"query":"rust","max_results":3}),
            summary: "unsupported web search".to_owned(),
        };

        assert!(matches!(
            PermissionGate::evaluate(&config(), &decision),
            PermissionDecision::Deny(_)
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

        let decision = PermissionGate::evaluate(&config(), &decision);

        let PermissionDecision::Deny(denial) = decision else {
            panic!("recursive delete should be manual-only");
        };
        assert_eq!(denial.reason, "manual_only_command");
        assert!(denial.message.contains("destructive_filesystem"));
    }

    #[test]
    fn asks_for_safe_verification_command() {
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Execute,
            tool_name: "run_command".to_owned(),
            arguments: json!({"argv":["cargo","test"],"cwd":".","timeout_ms":30000}),
            change_preview: None,
            reason: "verify".to_owned(),
        };

        assert!(matches!(
            PermissionGate::evaluate(&config(), &decision),
            PermissionDecision::Ask(_)
        ));
    }

    #[test]
    fn denies_external_service_command_as_manual_only() {
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Execute,
            tool_name: "run_command".to_owned(),
            arguments: json!({"argv":["curl","https://example.com"],"cwd":".","timeout_ms":30000}),
            change_preview: None,
            reason: "call external service".to_owned(),
        };

        let decision = PermissionGate::evaluate(&config(), &decision);

        let PermissionDecision::Deny(denial) = decision else {
            panic!("external service command should be manual-only");
        };
        assert_eq!(denial.reason, "manual_only_command");
        assert!(denial.message.contains("external_service"));
    }
}
