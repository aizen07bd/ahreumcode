use std::path::{Path, PathBuf};
use std::process::Command;

use super::observation::{ToolErrorKind, ToolObservation};

pub struct PostEditDiagnosticRequest {
    pub run_id: String,
    pub turn_id: String,
    pub target_path: String,
}

pub fn run_post_edit_diagnostics(
    workspace_root: &Path,
    request: &PostEditDiagnosticRequest,
) -> Option<ToolObservation> {
    let command = diagnostic_command_for_target(workspace_root, &request.target_path)?;
    let output = match Command::new(&command.program)
        .args(&command.args)
        .current_dir(workspace_root)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return Some(ToolObservation::failed(
                "post_edit_diagnostics",
                Some(request.target_path.clone()),
                ToolErrorKind::IoError,
                format!("diagnostic command could not start: {error}"),
            ));
        }
    };

    let preview = diagnostic_preview(&command, &output.stdout, &output.stderr);
    if output.status.success() {
        Some(ToolObservation::succeeded(
            "post_edit_diagnostics",
            Some(request.target_path.clone()),
            Some(command.display()),
            preview,
            false,
            None,
            "post-edit diagnostics passed",
        ))
    } else {
        let mut observation = ToolObservation::failed(
            "post_edit_diagnostics",
            Some(request.target_path.clone()),
            ToolErrorKind::ExecutionError,
            format!(
                "post-edit diagnostics failed with exit status {}",
                output.status
            ),
        );
        observation.preview = preview;
        observation.total_lines = observation.preview.len();
        observation.total_bytes = observation.preview.join("\n").len();
        Some(observation)
    }
}

pub(super) fn diagnostic_command_for_target(
    workspace_root: &Path,
    target_path: &str,
) -> Option<DiagnosticCommand> {
    let cargo_toml = workspace_root.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return None;
    }
    if !is_rust_diagnostic_target(target_path) {
        return None;
    }
    Some(DiagnosticCommand {
        program: "cargo".to_owned(),
        args: vec!["check".to_owned()],
        workspace_root: workspace_root.to_path_buf(),
    })
}

fn is_rust_diagnostic_target(target_path: &str) -> bool {
    let path = Path::new(target_path);
    target_path == "Cargo.toml"
        || target_path == "Cargo.lock"
        || path.extension().and_then(|extension| extension.to_str()) == Some("rs")
}

pub(super) struct DiagnosticCommand {
    program: String,
    args: Vec<String>,
    workspace_root: PathBuf,
}

impl DiagnosticCommand {
    fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.args.clone());
        format!("{} @ {}", parts.join(" "), self.workspace_root.display())
    }
}

fn diagnostic_preview(command: &DiagnosticCommand, stdout: &[u8], stderr: &[u8]) -> Vec<String> {
    let mut lines = vec![format!("command: {}", command.display())];
    lines.extend(decode_lines("stdout", stdout));
    lines.extend(decode_lines("stderr", stderr));
    lines
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

    use super::diagnostic_command_for_target;

    fn root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ahreumcode-diagnostics-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create root");
        root
    }

    #[test]
    fn selects_cargo_check_for_rust_target_when_manifest_exists() {
        let root = root("rust");
        fs::write(root.join("Cargo.toml"), "[package]\nname=\"x\"\n").expect("manifest");

        let command = diagnostic_command_for_target(&root, "src/main.rs").expect("command");

        assert_eq!(command.program, "cargo");
        assert_eq!(command.args, vec!["check"]);
    }

    #[test]
    fn skips_non_rust_target_without_guessing_command() {
        let root = root("html");
        fs::write(root.join("Cargo.toml"), "[package]\nname=\"x\"\n").expect("manifest");

        assert!(diagnostic_command_for_target(&root, "index.html").is_none());
    }

    #[test]
    fn skips_when_project_manifest_is_absent() {
        let root = root("none");

        assert!(diagnostic_command_for_target(&root, "src/main.rs").is_none());
    }
}
