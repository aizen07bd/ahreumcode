use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::command_policy::{CommandPolicy, CommandPolicyDecision};
use super::observation::{ToolErrorKind, ToolObservation};
use crate::tool::{normalize_tool_arguments, ToolName};

pub struct ApprovedCommand {
    pub arguments: Value,
    pub max_timeout_ms: u32,
}

pub fn execute_approved_command(
    workspace_root: &Path,
    command: ApprovedCommand,
) -> ToolObservation {
    let arguments = match normalize_tool_arguments(ToolName::RunCommand, &command.arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            return ToolObservation::failed(
                "run_command",
                None,
                ToolErrorKind::InvalidArguments,
                error,
            );
        }
    };
    if command.max_timeout_ms == 0 {
        return ToolObservation::failed(
            "run_command",
            target_from_arguments(&arguments),
            ToolErrorKind::InvalidArguments,
            "max_timeout_ms must be greater than zero",
        );
    }
    let policy = CommandPolicy::evaluate(&arguments);
    if let CommandPolicyDecision::ManualOnly { capability, reason } = policy {
        return ToolObservation::failed(
            "run_command",
            target_from_arguments(&arguments),
            ToolErrorKind::PermissionError,
            format!(
                "{reason}; capability={} is guidance-only and was not executed",
                capability.as_str()
            ),
        );
    }

    let Some(argv) = argv_from_arguments(&arguments) else {
        return ToolObservation::failed(
            "run_command",
            target_from_arguments(&arguments),
            ToolErrorKind::InvalidArguments,
            "argv must be a non-empty string array",
        );
    };
    let cwd = match command_cwd(workspace_root, &arguments) {
        Ok(cwd) => cwd,
        Err(error) => return error,
    };
    let requested_timeout_ms = arguments
        .as_object()
        .and_then(|object| object.get("timeout_ms"))
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(command.max_timeout_ms));
    let effective_timeout_ms = requested_timeout_ms.min(u64::from(command.max_timeout_ms));

    run_argv(
        workspace_root,
        &cwd,
        &argv,
        requested_timeout_ms,
        effective_timeout_ms,
    )
}

fn run_argv(
    workspace_root: &Path,
    cwd: &Path,
    argv: &[String],
    requested_timeout_ms: u64,
    effective_timeout_ms: u64,
) -> ToolObservation {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ToolObservation::failed(
                "run_command",
                Some(display_workspace_path(workspace_root, cwd)),
                ToolErrorKind::IoError,
                format!("command could not start: {error}"),
            );
        }
    };

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if started.elapsed() >= Duration::from_millis(effective_timeout_ms) {
                    let _ = child.kill();
                    let output = child.wait_with_output();
                    return match output {
                        Ok(output) => command_timeout_observation(
                            workspace_root,
                            cwd,
                            argv,
                            requested_timeout_ms,
                            effective_timeout_ms,
                            &output.stdout,
                            &output.stderr,
                        ),
                        Err(error) => ToolObservation::failed(
                            "run_command",
                            Some(display_workspace_path(workspace_root, cwd)),
                            ToolErrorKind::IoError,
                            format!("timed out command could not be collected: {error}"),
                        ),
                    };
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                return ToolObservation::failed(
                    "run_command",
                    Some(display_workspace_path(workspace_root, cwd)),
                    ToolErrorKind::IoError,
                    format!("command status could not be read: {error}"),
                );
            }
        }
    }

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => {
            return ToolObservation::failed(
                "run_command",
                Some(display_workspace_path(workspace_root, cwd)),
                ToolErrorKind::IoError,
                format!("command output could not be collected: {error}"),
            );
        }
    };

    let preview = command_preview(
        workspace_root,
        cwd,
        argv,
        requested_timeout_ms,
        effective_timeout_ms,
        &output.stdout,
        &output.stderr,
    );
    if output.status.success() {
        ToolObservation::succeeded(
            "run_command",
            Some(display_workspace_path(workspace_root, cwd)),
            Some(render_argv(argv)),
            preview,
            false,
            None,
            "approved command succeeded",
        )
    } else {
        let mut observation = ToolObservation::failed(
            "run_command",
            Some(display_workspace_path(workspace_root, cwd)),
            ToolErrorKind::ExecutionError,
            format!("approved command failed with exit status {}", output.status),
        );
        observation.preview = preview;
        observation.total_lines = observation.preview.len();
        observation.total_bytes = observation.preview.join("\n").len();
        observation
    }
}

fn command_timeout_observation(
    workspace_root: &Path,
    cwd: &Path,
    argv: &[String],
    requested_timeout_ms: u64,
    effective_timeout_ms: u64,
    stdout: &[u8],
    stderr: &[u8],
) -> ToolObservation {
    let mut observation = ToolObservation::failed(
        "run_command",
        Some(display_workspace_path(workspace_root, cwd)),
        ToolErrorKind::Timeout,
        format!("approved command timed out after {effective_timeout_ms} ms"),
    );
    observation.preview = command_preview(
        workspace_root,
        cwd,
        argv,
        requested_timeout_ms,
        effective_timeout_ms,
        stdout,
        stderr,
    );
    observation.total_lines = observation.preview.len();
    observation.total_bytes = observation.preview.join("\n").len();
    observation
}

fn command_preview(
    workspace_root: &Path,
    cwd: &Path,
    argv: &[String],
    requested_timeout_ms: u64,
    effective_timeout_ms: u64,
    stdout: &[u8],
    stderr: &[u8],
) -> Vec<String> {
    let mut lines = vec![
        format!("original_argv: {}", render_argv(argv)),
        format!("parsed_argv: {}", render_argv(argv)),
        format!("cwd: {}", display_workspace_path(workspace_root, cwd)),
        format!("requested_timeout_ms: {requested_timeout_ms}"),
        format!("effective_timeout_ms: {effective_timeout_ms}"),
    ];
    lines.extend(decode_lines("stdout", stdout));
    lines.extend(decode_lines("stderr", stderr));
    lines
}

fn argv_from_arguments(arguments: &Value) -> Option<Vec<String>> {
    let argv = arguments
        .as_object()
        .and_then(|object| object.get("argv"))
        .and_then(Value::as_array)?;
    argv.iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect()
}

fn command_cwd(workspace_root: &Path, arguments: &Value) -> Result<PathBuf, ToolObservation> {
    let raw = target_from_arguments(arguments).unwrap_or_else(|| ".".to_owned());
    validate_workspace_relative_path(&raw)?;
    let workspace_root = workspace_root.canonicalize().map_err(|error| {
        ToolObservation::failed(
            "run_command",
            Some(raw.clone()),
            ToolErrorKind::IoError,
            format!("workspace root cannot be resolved: {error}"),
        )
    })?;
    let candidate = workspace_root.join(&raw);
    let resolved = candidate.canonicalize().map_err(|error| {
        ToolObservation::failed(
            "run_command",
            Some(raw.clone()),
            ToolErrorKind::PathNotFound,
            format!("cwd cannot be resolved: {error}"),
        )
    })?;
    if !resolved.starts_with(&workspace_root) {
        return Err(ToolObservation::failed(
            "run_command",
            Some(raw),
            ToolErrorKind::PathOutsideWorkspace,
            "cwd resolved outside workspace",
        ));
    }
    if !resolved.is_dir() {
        return Err(ToolObservation::failed(
            "run_command",
            Some(raw),
            ToolErrorKind::NotADirectory,
            "cwd is not a directory",
        ));
    }
    Ok(resolved)
}

fn validate_workspace_relative_path(raw: &str) -> Result<(), ToolObservation> {
    let path = Path::new(raw);
    if raw.is_empty()
        || raw.trim() != raw
        || raw.chars().any(char::is_control)
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err(ToolObservation::failed(
            "run_command",
            Some(raw.to_owned()),
            ToolErrorKind::PathOutsideWorkspace,
            "cwd must stay inside the workspace",
        ));
    }
    Ok(())
}

fn target_from_arguments(arguments: &Value) -> Option<String> {
    arguments
        .as_object()
        .and_then(|object| object.get("cwd"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn display_workspace_path(workspace_root: &Path, resolved: &Path) -> String {
    workspace_root
        .canonicalize()
        .ok()
        .and_then(|root| resolved.strip_prefix(root).ok().map(Path::to_path_buf))
        .and_then(|path| path.to_str().map(str::to_owned))
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| ".".to_owned())
}

fn render_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|part| format!("{part:?}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_lines(label: &str, bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let lines = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| format!("{label}: {line}"))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        vec![format!("{label}: <empty>")]
    } else {
        lines
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::{execute_approved_command, ApprovedCommand};

    fn root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ahreumcode-command-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create root");
        root
    }

    #[test]
    fn executes_approved_argv_without_shell() {
        let root = root("pwd");

        let observation = execute_approved_command(
            &root,
            ApprovedCommand {
                arguments: json!({"argv":["cargo","--version"],"cwd":".","timeout_ms":30000}),
                max_timeout_ms: 30000,
            },
        );

        assert_eq!(observation.status.as_str(), "succeeded");
        assert!(observation.preview_text().contains("parsed_argv"));
    }

    #[test]
    fn rejects_invalid_command_execution_boundaries() {
        let cases = [
            (
                "manual",
                json!({"argv":["rm","-rf","target"],"cwd":".","timeout_ms":30000}),
                30000,
                "permission_error",
            ),
            (
                "cwd-parent",
                json!({"argv":["cargo","--version"],"cwd":"..","timeout_ms":30000}),
                30000,
                "path_outside_workspace",
            ),
            (
                "cwd-control",
                json!({"argv":["pwd"],"cwd":"bad\npath","timeout_ms":30000}),
                30000,
                "path_outside_workspace",
            ),
            (
                "zero-timeout",
                json!({"argv":["pwd"],"cwd":".","timeout_ms":30000}),
                0,
                "invalid_arguments",
            ),
        ];

        for (name, arguments, max_timeout_ms, expected_error) in cases {
            let root = root(name);

            let observation = execute_approved_command(
                &root,
                ApprovedCommand {
                    arguments,
                    max_timeout_ms,
                },
            );

            assert_eq!(observation.status.as_str(), "failed");
            assert_eq!(observation.error_kind.unwrap().as_str(), expected_error);
        }
    }
}
