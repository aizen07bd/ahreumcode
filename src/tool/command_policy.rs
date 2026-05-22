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

    pub fn approval_details(arguments: &Value) -> Option<CommandApprovalDetails> {
        let candidate = CommandCandidate::from_arguments(arguments)?;
        let capability = match classify_command(&candidate) {
            CommandPolicyDecision::ApprovalRequired { capability } => capability,
            CommandPolicyDecision::ManualOnly { capability, .. } => capability,
        };
        Some(CommandApprovalDetails {
            capability,
            original_argv: candidate.argv(),
            parsed_argv: candidate.argv(),
            cwd: arguments
                .as_object()
                .and_then(|object| object.get("cwd"))
                .and_then(Value::as_str)
                .unwrap_or(".")
                .to_owned(),
            timeout_ms: arguments
                .as_object()
                .and_then(|object| object.get("timeout_ms"))
                .and_then(Value::as_u64),
            risk_markers: command_risk_markers(&candidate),
            persistent_approval_allowed: persistent_approval_allowed(&candidate),
        })
    }
}

pub struct CommandApprovalDetails {
    pub capability: CommandCapability,
    pub original_argv: Vec<String>,
    pub parsed_argv: Vec<String>,
    pub cwd: String,
    pub timeout_ms: Option<u64>,
    pub risk_markers: Vec<String>,
    pub persistent_approval_allowed: bool,
}

impl CommandApprovalDetails {
    pub fn render(&self) -> String {
        let risk_markers = if self.risk_markers.is_empty() {
            "none".to_owned()
        } else {
            self.risk_markers.join(", ")
        };
        format!(
            "command_capability: {}\noriginal_argv: {}\nparsed_argv: {}\ncwd: {}\ntimeout_ms: {}\npersistent_approval_allowed: {}\nrisk_markers: {}",
            self.capability.as_str(),
            render_argv(&self.original_argv),
            render_argv(&self.parsed_argv),
            self.cwd,
            self.timeout_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            self.persistent_approval_allowed,
            risk_markers
        )
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
        if !is_plain_command_part(program) {
            return None;
        }
        let args = argv
            .iter()
            .skip(1)
            .map(|value| value.as_str().filter(|part| is_plain_command_part(part)))
            .collect::<Option<Vec<_>>>()?;

        Some(Self { program, args })
    }

    fn argv(&self) -> Vec<String> {
        let mut argv = vec![self.program.to_owned()];
        argv.extend(self.args.iter().map(|arg| (*arg).to_owned()));
        argv
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

    if is_mutation_command(candidate) {
        return manual_only(
            CommandCapability::Mutation,
            "mutation command requires a dry-run contract",
        );
    }

    match approved_capability(candidate) {
        Some(capability) => CommandPolicyDecision::ApprovalRequired { capability },
        None => manual_only(
            CommandCapability::Unknown,
            "command is outside the approved execution contract",
        ),
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
    ) || (candidate.program == "git"
        && candidate
            .args
            .first()
            .is_some_and(|arg| matches!(*arg, "push" | "pull" | "fetch" | "clone")))
}

fn is_mutation_command(candidate: &CommandCandidate<'_>) -> bool {
    match candidate.program {
        "mkdir" | "touch" | "cp" | "mv" => true,
        "sed" => candidate
            .args
            .iter()
            .any(|arg| *arg == "-i" || arg.starts_with("-i")),
        "git" => candidate.args.first().is_some_and(|arg| {
            matches!(
                *arg,
                "add"
                    | "am"
                    | "apply"
                    | "bisect"
                    | "branch"
                    | "checkout"
                    | "cherry-pick"
                    | "clean"
                    | "commit"
                    | "merge"
                    | "mv"
                    | "rebase"
                    | "reset"
                    | "restore"
                    | "revert"
                    | "rm"
                    | "stash"
                    | "switch"
                    | "tag"
                    | "worktree"
            )
        }),
        _ => false,
    }
}

fn approved_capability(candidate: &CommandCandidate<'_>) -> Option<CommandCapability> {
    match candidate.program {
        "pwd" if candidate.args.is_empty() => Some(CommandCapability::ReadOnly),
        "cargo" if is_approved_cargo(candidate) => Some(CommandCapability::BuildTest),
        "git" if is_approved_git_read(candidate) => Some(CommandCapability::ReadOnly),
        _ => None,
    }
}

fn is_approved_cargo(candidate: &CommandCandidate<'_>) -> bool {
    matches!(
        candidate.args.as_slice(),
        ["--version"] | ["check"] | ["test"] | ["test", "--no-run"] | ["fmt", "--check"]
    )
}

fn is_approved_git_read(candidate: &CommandCandidate<'_>) -> bool {
    matches!(
        candidate.args.as_slice(),
        ["status"]
            | ["status", "--short"]
            | ["status", "--porcelain"]
            | ["diff"]
            | ["diff", "--stat"]
            | ["diff", "--name-only"]
    )
}

fn command_risk_markers(candidate: &CommandCandidate<'_>) -> Vec<String> {
    let mut markers = Vec::new();
    for (index, value) in candidate.argv().iter().enumerate() {
        if value.chars().any(|character| !character.is_ascii()) {
            markers.push(format!("non_ascii_argv[{index}]"));
        }
        if value.chars().any(is_hidden_or_control) {
            markers.push(format!("hidden_or_control_argv[{index}]"));
        }
    }
    if !persistent_approval_allowed(candidate) {
        markers.push("broad_persistent_prefix_denied".to_owned());
    }
    markers
}

fn is_hidden_or_control(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{200B}'
                | '\u{200C}'
                | '\u{200D}'
                | '\u{2060}'
                | '\u{FEFF}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
        )
}

fn is_plain_command_part(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(is_hidden_or_control)
}

fn persistent_approval_allowed(candidate: &CommandCandidate<'_>) -> bool {
    !matches!(
        candidate.program,
        "bash" | "sh" | "zsh" | "python" | "python3" | "node" | "git"
    )
}

fn render_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|part| format!("{part:?}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{CommandCapability, CommandPolicy, CommandPolicyDecision};

    #[test]
    fn classifies_command_policy_matrix() {
        let cases = [
            (
                json!({"argv":["cargo","test"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ApprovalRequired {
                    capability: CommandCapability::BuildTest,
                },
            ),
            (
                json!({"argv":["git","status","--short"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ApprovalRequired {
                    capability: CommandCapability::ReadOnly,
                },
            ),
            (
                json!({"argv":["rm","-rf","target"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::DestructiveFilesystem,
                    reason: "destructive filesystem command",
                },
            ),
            (
                json!({"argv":["sudo","cargo","test"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::SystemLevel,
                    reason: "system-level command",
                },
            ),
            (
                json!({"argv":["curl","https://example.com"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::ExternalService,
                    reason: "external-service command",
                },
            ),
            (
                json!({"argv":["touch","index.html"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::Mutation,
                    reason: "mutation command requires a dry-run contract",
                },
            ),
            (
                json!({"argv":["git","reset","--hard"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::Mutation,
                    reason: "mutation command requires a dry-run contract",
                },
            ),
            (
                json!({"argv":["sed","-i","s/a/b/","file.txt"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::Mutation,
                    reason: "mutation command requires a dry-run contract",
                },
            ),
            (
                json!({"argv":["custom-tool"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::Unknown,
                    reason: "command is outside the approved execution contract",
                },
            ),
            (
                json!({"argv":["python3","script.py"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::Unknown,
                    reason: "command is outside the approved execution contract",
                },
            ),
            (
                json!({"argv":["git","show","HEAD"],"cwd":".","timeout_ms":30000}),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::Unknown,
                    reason: "command is outside the approved execution contract",
                },
            ),
        ];

        for (arguments, expected) in cases {
            assert_eq!(CommandPolicy::evaluate(&arguments), expected);
        }
    }

    #[test]
    fn approval_details_show_original_parsed_and_persistent_guard() {
        let details = CommandPolicy::approval_details(
            &json!({"argv":["git","status"],"cwd":".","timeout_ms":30000}),
        )
        .expect("details");

        let rendered = details.render();
        assert!(rendered.contains("original_argv: \"git\" \"status\""));
        assert!(rendered.contains("parsed_argv: \"git\" \"status\""));
        assert!(rendered.contains("persistent_approval_allowed: false"));
        assert!(rendered.contains("broad_persistent_prefix_denied"));
    }

    #[test]
    fn rejects_unclassifiable_argv_shapes_without_dropping_parts() {
        for arguments in [
            json!({"argv":["cargo","test",true],"cwd":".","timeout_ms":30000}),
            json!({"argv":["cargo","test\n"],"cwd":".","timeout_ms":30000}),
        ] {
            assert_eq!(
                CommandPolicy::evaluate(&arguments),
                CommandPolicyDecision::ManualOnly {
                    capability: CommandCapability::Unknown,
                    reason: "command arguments could not be classified"
                }
            );
        }
    }
}
