use std::path::{Component, Path, PathBuf};

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
            } => evaluate_approval_needed(
                config,
                *activity,
                tool_name,
                arguments,
                change_preview,
                reason,
            ),
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

    if matches!(
        ToolName::parse(tool_name),
        Some(ToolName::WebSearch | ToolName::WebFetch)
    ) && !config.web.enabled
    {
        return PermissionDecision::Deny(PermissionDenial {
            reason: "web_disabled".to_owned(),
            message: "Web/network tool is disabled by configuration".to_owned(),
        });
    }

    if let Some(denial) = external_path_denial(config, tool_name, arguments, None) {
        return PermissionDecision::Deny(denial);
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
    config: &RuntimeConfig,
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

    if let Some(denial) =
        external_path_denial(config, tool_name, arguments, change_preview.as_ref())
    {
        return PermissionDecision::Deny(denial);
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
        details: approval_details(tool_name, reason, arguments, change_preview),
    })
}

fn external_path_denial(
    config: &RuntimeConfig,
    tool_name: &str,
    arguments: &Value,
    change_preview: Option<&crate::llm::ChangePreview>,
) -> Option<PermissionDenial> {
    let raw = path_argument(tool_name, arguments, change_preview)?;
    if is_workspace_relative_path(raw) {
        return None;
    }
    let resolved = resolve_external_display(&config.workspace.root, raw);
    let sensitive = is_sensitive_external_path(raw, resolved.as_deref());
    let reason = if sensitive {
        "sensitive_external_path"
    } else {
        "external_path_manual_only"
    };
    Some(PermissionDenial {
        reason: reason.to_owned(),
        message: format!(
            "External path is not executable by the workspace tool runtime; original_path={raw}; resolved_path={}; sensitive={sensitive}",
            resolved.as_deref().unwrap_or("unresolved")
        ),
    })
}

fn path_argument<'a>(
    tool_name: &str,
    arguments: &'a Value,
    change_preview: Option<&'a crate::llm::ChangePreview>,
) -> Option<&'a str> {
    match ToolName::parse(tool_name) {
        Some(ToolName::ListFiles | ToolName::SearchText | ToolName::ReadFile) => {
            arguments.get("path").and_then(Value::as_str)
        }
        Some(ToolName::RunCommand) => arguments.get("cwd").and_then(Value::as_str),
        Some(ToolName::ApplyPatch) => change_preview.map(|preview| preview.target_path.as_str()),
        _ => None,
    }
}

fn is_workspace_relative_path(raw: &str) -> bool {
    if raw.is_empty() || raw.trim() != raw || raw.chars().any(char::is_control) {
        return false;
    }
    let path = Path::new(raw);
    !path.is_absolute()
        && path.components().all(|component| {
            !matches!(
                component,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
}

fn resolve_external_display(workspace_root: &str, raw: &str) -> Option<String> {
    let path = Path::new(raw);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        PathBuf::from(workspace_root).join(path)
    };
    if let Ok(resolved) = candidate.canonicalize() {
        return Some(resolved.display().to_string());
    }
    let parent = candidate.parent()?;
    let file_name = candidate.file_name()?;
    parent
        .canonicalize()
        .ok()
        .map(|parent| parent.join(file_name).display().to_string())
}

fn is_sensitive_external_path(raw: &str, resolved: Option<&str>) -> bool {
    [raw, resolved.unwrap_or("")].iter().any(|value| {
        value.starts_with("/etc/")
            || value == &"/etc"
            || value.starts_with("/private/etc/")
            || value.starts_with("/var/root/")
            || value.starts_with("/System/")
            || value.starts_with("/Library/Keychains/")
            || value.contains("/.ssh/")
            || value.contains("/.aws/")
            || value.contains("/.config/gh/")
            || value.ends_with("/.env")
            || value.contains("/.env.")
    })
}

fn approval_details(
    tool_name: &str,
    reason: &str,
    arguments: &Value,
    change_preview: &Option<crate::llm::ChangePreview>,
) -> String {
    if tool_name == ToolName::RunCommand.as_str() {
        if let Some(details) = CommandPolicy::approval_details(arguments) {
            return format!("reason: {reason}\n{}", details.render());
        }
    }

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
            tool_name: "unknown_tool".to_owned(),
            arguments: json!({"query":"rust","max_results":3}),
            summary: "unsupported tool".to_owned(),
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
    fn denies_web_tool_when_web_is_disabled() {
        let mut config = config();
        config.web.enabled = false;
        let decision = RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name: "web_fetch".to_owned(),
            arguments: json!({"url":"https://example.com","max_bytes":1000}),
            summary: "fetch web".to_owned(),
        };

        let decision = PermissionGate::evaluate(&config, &decision);

        let PermissionDecision::Deny(denial) = decision else {
            panic!("disabled web should be denied");
        };
        assert_eq!(denial.reason, "web_disabled");
    }

    #[test]
    fn denies_external_explore_path_with_original_and_resolved_path() {
        let decision = RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name: "read_file".to_owned(),
            arguments: json!({"path":"../outside.md","start_line":1,"max_lines":80}),
            summary: "read outside".to_owned(),
        };

        let decision = PermissionGate::evaluate(&config(), &decision);

        let PermissionDecision::Deny(denial) = decision else {
            panic!("external path should be denied");
        };
        assert_eq!(denial.reason, "external_path_manual_only");
        assert!(denial.message.contains("original_path=../outside.md"));
        assert!(denial.message.contains("resolved_path="));
    }

    #[test]
    fn denies_sensitive_absolute_path_without_approval() {
        let decision = RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name: "read_file".to_owned(),
            arguments: json!({"path":"/etc/passwd","start_line":1,"max_lines":80}),
            summary: "read sensitive".to_owned(),
        };

        let decision = PermissionGate::evaluate(&config(), &decision);

        let PermissionDecision::Deny(denial) = decision else {
            panic!("sensitive path should be denied");
        };
        assert_eq!(denial.reason, "sensitive_external_path");
        assert!(denial.message.contains("sensitive=true"));
    }

    #[test]
    fn denies_external_apply_patch_target() {
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Change,
            tool_name: "apply_patch".to_owned(),
            arguments: json!({"payload_id":"patch_001"}),
            change_preview: Some(crate::llm::ChangePreview {
                payload_id: "patch_001".to_owned(),
                target_path: "../outside.md".to_owned(),
                operation: crate::llm::PatchOperation::Add,
                additions: 1,
                deletions: 0,
                payload_body: "*** Begin Patch\n*** Add File: ../outside.md\n+hello\n*** End Patch"
                    .to_owned(),
            }),
            reason: "change outside".to_owned(),
        };

        let decision = PermissionGate::evaluate(&config(), &decision);

        let PermissionDecision::Deny(denial) = decision else {
            panic!("external change path should be denied");
        };
        assert_eq!(denial.reason, "external_path_manual_only");
    }

    #[test]
    fn asks_for_web_tool_when_web_is_enabled() {
        let mut config = config();
        config.web.enabled = true;
        let decision = RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name: "web_fetch".to_owned(),
            arguments: json!({"url":"https://example.com","max_bytes":1000}),
            summary: "fetch web".to_owned(),
        };

        assert!(matches!(
            PermissionGate::evaluate(&config, &decision),
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
