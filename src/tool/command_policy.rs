use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandCapability {
    ReadOnly,
    BuildTest,
    ProcessControl,
    Mutation,
    DestructiveFilesystem,
    SystemLevel,
    ExternalService,
    HighLoad,
    Unknown,
}

impl CommandCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::BuildTest => "build_test",
            Self::ProcessControl => "process_control",
            Self::Mutation => "mutation",
            Self::DestructiveFilesystem => "destructive_filesystem",
            Self::SystemLevel => "system_level",
            Self::ExternalService => "external_service",
            Self::HighLoad => "high_load",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPolicyDecision {
    ApprovalRequired {
        capability: CommandCapability,
    },
    ManualOnly {
        capability: CommandCapability,
        reason: &'static str,
    },
}

pub struct CommandPolicy;

impl CommandPolicy {
    pub fn evaluate(arguments: &Value) -> CommandPolicyDecision {
        let Some(candidate) = CommandCandidate::from_arguments(arguments) else {
            return CommandPolicyDecision::ManualOnly {
                capability: CommandCapability::Unknown,
                reason: "command arguments could not be classified",
            };
        };

        classify_command(&candidate)
    }
}

struct CommandCandidate<'a> {
    program: &'a str,
    args: Vec<&'a str>,
}

impl<'a> CommandCandidate<'a> {
    fn from_arguments(arguments: &'a Value) -> Option<Self> {
        let argv = arguments
            .as_object()
            .and_then(|object| object.get("argv"))
            .and_then(Value::as_array)?;
        let program = argv.first().and_then(Value::as_str)?;
        if program.trim().is_empty() {
            return None;
        }

        Some(Self {
            program,
            args: argv.iter().skip(1).filter_map(Value::as_str).collect(),
        })
    }

    fn has_arg(&self, expected: &str) -> bool {
        self.args.iter().any(|arg| *arg == expected)
    }

    fn has_any_arg(&self, expected: &[&str]) -> bool {
        expected.iter().any(|value| self.has_arg(value))
    }

    fn has_flag_containing(&self, expected: char) -> bool {
        self.args
            .iter()
            .filter(|arg| arg.starts_with('-'))
            .any(|arg| arg.chars().skip(1).any(|value| value == expected))
    }
}

fn classify_command(candidate: &CommandCandidate<'_>) -> CommandPolicyDecision {
    if is_system_level(candidate) {
        return manual_only(CommandCapability::SystemLevel, "system-level command");
    }

    if is_process_control(candidate) {
        return manual_only(CommandCapability::ProcessControl, "process-control command");
    }

    if is_destructive_filesystem(candidate) {
        return manual_only(
            CommandCapability::DestructiveFilesystem,
            "destructive filesystem command",
        );
    }

    if is_high_load(candidate) {
        return manual_only(CommandCapability::HighLoad, "high-load command");
    }

    if is_external_service(candidate) {
        return manual_only(
            CommandCapability::ExternalService,
            "external-service command",
        );
    }

    CommandPolicyDecision::ApprovalRequired {
        capability: approval_capability(candidate),
    }
}

fn manual_only(capability: CommandCapability, reason: &'static str) -> CommandPolicyDecision {
    CommandPolicyDecision::ManualOnly { capability, reason }
}

fn is_system_level(candidate: &CommandCandidate<'_>) -> bool {
    matches!(
        candidate.program,
        "sudo"
            | "su"
            | "doas"
            | "shutdown"
            | "reboot"
            | "halt"
            | "poweroff"
            | "mkfs"
            | "mount"
            | "umount"
            | "launchctl"
            | "systemctl"
            | "chown"
            | "chgrp"
            | "chmod"
    )
}

fn is_process_control(candidate: &CommandCandidate<'_>) -> bool {
    matches!(candidate.program, "kill" | "killall" | "pkill")
}

fn is_destructive_filesystem(candidate: &CommandCandidate<'_>) -> bool {
    match candidate.program {
        "rm" => {
            candidate.has_flag_containing('r')
                || candidate.has_flag_containing('R')
                || candidate.has_flag_containing('f')
        }
        "find" => candidate.has_any_arg(&["-delete", "-exec"]),
        "dd" | "shred" | "wipefs" => true,
        _ => false,
    }
}

fn is_high_load(candidate: &CommandCandidate<'_>) -> bool {
    matches!(
        candidate.program,
        "yes" | "stress" | "stress-ng" | "ab" | "wrk"
    )
}

fn is_external_service(candidate: &CommandCandidate<'_>) -> bool {
    matches!(
        candidate.program,
        "curl" | "wget" | "ssh" | "scp" | "rsync" | "docker" | "kubectl" | "terraform"
    )
}

fn approval_capability(candidate: &CommandCandidate<'_>) -> CommandCapability {
    match candidate.program {
        "ls" | "pwd" | "cat" | "head" | "tail" | "sed" | "grep" | "rg" | "git" => {
            CommandCapability::ReadOnly
        }
        "cargo" | "npm" | "pnpm" | "yarn" | "make" | "go" | "pytest" | "python" | "python3" => {
            CommandCapability::BuildTest
        }
        "mkdir" | "touch" | "cp" | "mv" => CommandCapability::Mutation,
        _ => CommandCapability::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{CommandCapability, CommandPolicy, CommandPolicyDecision};

    #[test]
    fn classifies_safe_verification_as_approval_required() {
        assert_eq!(
            CommandPolicy::evaluate(&json!({"argv":["cargo","test"],"cwd":".","timeout_ms":30000})),
            CommandPolicyDecision::ApprovalRequired {
                capability: CommandCapability::BuildTest
            }
        );
    }

    #[test]
    fn classifies_recursive_delete_as_manual_only() {
        assert_eq!(
            CommandPolicy::evaluate(
                &json!({"argv":["rm","-rf","target"],"cwd":".","timeout_ms":30000})
            ),
            CommandPolicyDecision::ManualOnly {
                capability: CommandCapability::DestructiveFilesystem,
                reason: "destructive filesystem command"
            }
        );
    }

    #[test]
    fn classifies_system_command_as_manual_only() {
        assert_eq!(
            CommandPolicy::evaluate(
                &json!({"argv":["sudo","cargo","test"],"cwd":".","timeout_ms":30000})
            ),
            CommandPolicyDecision::ManualOnly {
                capability: CommandCapability::SystemLevel,
                reason: "system-level command"
            }
        );
    }

    #[test]
    fn classifies_external_service_command_as_manual_only() {
        assert_eq!(
            CommandPolicy::evaluate(
                &json!({"argv":["curl","https://example.com"],"cwd":".","timeout_ms":30000})
            ),
            CommandPolicyDecision::ManualOnly {
                capability: CommandCapability::ExternalService,
                reason: "external-service command"
            }
        );
    }
}
