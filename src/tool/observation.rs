use std::fs;
use std::io;
use std::path::Path;

pub const DEFAULT_PREVIEW_LINE_LIMIT: usize = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationStatus {
    Succeeded,
    Failed,
}

impl ObservationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolErrorKind {
    InvalidArguments,
    UnsupportedTool,
    UnsupportedArgument,
    PathOutsideWorkspace,
    PathNotFound,
    NotAFile,
    NotADirectory,
    IoError,
    GitError,
}

impl ToolErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArguments => "invalid_arguments",
            Self::UnsupportedTool => "unsupported_tool",
            Self::UnsupportedArgument => "unsupported_argument",
            Self::PathOutsideWorkspace => "path_outside_workspace",
            Self::PathNotFound => "path_not_found",
            Self::NotAFile => "not_a_file",
            Self::NotADirectory => "not_a_directory",
            Self::IoError => "io_error",
            Self::GitError => "git_error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolObservation {
    pub status: ObservationStatus,
    pub tool_name: String,
    pub target_raw: Option<String>,
    pub target_resolved: Option<String>,
    pub preview: Vec<String>,
    pub total_lines: usize,
    pub total_bytes: usize,
    pub truncated: bool,
    pub source_truncated: bool,
    pub preview_truncated: bool,
    pub artifact_path: Option<String>,
    pub next_range_hint: Option<String>,
    pub error_kind: Option<ToolErrorKind>,
    pub message: String,
}

impl ToolObservation {
    pub fn succeeded(
        tool_name: impl Into<String>,
        target_raw: Option<String>,
        target_resolved: Option<String>,
        preview: Vec<String>,
        source_truncated: bool,
        next_range_hint: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        let total_lines = preview.len();
        let total_bytes = joined_bytes(&preview);
        Self {
            status: ObservationStatus::Succeeded,
            tool_name: tool_name.into(),
            target_raw,
            target_resolved,
            preview,
            total_lines,
            total_bytes,
            truncated: source_truncated,
            source_truncated,
            preview_truncated: false,
            artifact_path: None,
            next_range_hint,
            error_kind: None,
            message: message.into(),
        }
    }

    pub fn failed(
        tool_name: impl Into<String>,
        target_raw: Option<String>,
        error_kind: ToolErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: ObservationStatus::Failed,
            tool_name: tool_name.into(),
            target_raw,
            target_resolved: None,
            preview: Vec::new(),
            total_lines: 0,
            total_bytes: 0,
            truncated: false,
            source_truncated: false,
            preview_truncated: false,
            artifact_path: None,
            next_range_hint: None,
            error_kind: Some(error_kind),
            message: message.into(),
        }
    }

    pub fn apply_output_policy(
        &mut self,
        preview_line_limit: usize,
        artifact_root: &Path,
        artifact_name: &str,
    ) -> io::Result<()> {
        if self.status != ObservationStatus::Succeeded {
            return Ok(());
        }

        let full_output = self.preview.clone();
        self.total_lines = full_output.len();
        self.total_bytes = joined_bytes(&full_output);

        if full_output.len() > preview_line_limit {
            fs::create_dir_all(artifact_root)?;
            let artifact_path = artifact_root.join(artifact_name);
            fs::write(&artifact_path, full_output.join("\n"))?;
            self.preview = full_output.into_iter().take(preview_line_limit).collect();
            self.preview_truncated = true;
            self.artifact_path = Some(artifact_path.display().to_string());
        }

        self.truncated = self.source_truncated || self.preview_truncated;
        Ok(())
    }

    pub fn summary(&self) -> String {
        match (self.status, self.error_kind) {
            (ObservationStatus::Succeeded, _) => {
                let target = self.target_raw.as_deref().unwrap_or("-");
                format!("{} ok: {} ({})", self.tool_name, target, self.message)
            }
            (ObservationStatus::Failed, Some(error_kind)) => {
                format!(
                    "{} failed: {} ({})",
                    self.tool_name,
                    error_kind.as_str(),
                    self.message
                )
            }
            (ObservationStatus::Failed, None) => {
                format!("{} failed: {}", self.tool_name, self.message)
            }
        }
    }

    pub fn preview_text(&self) -> String {
        if self.preview.is_empty() {
            return self.message.clone();
        }

        let mut lines = self.preview.clone();
        if let Some(path) = &self.artifact_path {
            lines.push(format!("artifact: {path}"));
        }
        if let Some(hint) = &self.next_range_hint {
            lines.push(format!("next: {hint}"));
        }
        lines.join("\n")
    }

    pub fn history_message(&self) -> String {
        let mut lines = vec![
            "<AHREUM_TOOL_OBSERVATION>".to_owned(),
            format!("tool_name: {}", self.tool_name),
            format!("status: {}", self.status.as_str()),
            format!("target_raw: {}", self.target_raw.as_deref().unwrap_or("-")),
            format!(
                "target_resolved: {}",
                self.target_resolved.as_deref().unwrap_or("-")
            ),
            format!("total_lines: {}", self.total_lines),
            format!("total_bytes: {}", self.total_bytes),
            format!("truncated: {}", self.truncated),
            format!("source_truncated: {}", self.source_truncated),
            format!("preview_truncated: {}", self.preview_truncated),
            format!(
                "artifact_path: {}",
                self.artifact_path.as_deref().unwrap_or("-")
            ),
            format!(
                "next_range_hint: {}",
                self.next_range_hint.as_deref().unwrap_or("-")
            ),
            format!("message: {}", self.message),
            "preview:".to_owned(),
        ];
        lines.extend(self.preview.iter().cloned());
        lines.push("</AHREUM_TOOL_OBSERVATION>".to_owned());
        lines.push(
            "Use this observation. Return answer if enough; otherwise return exactly one next tool candidate."
                .to_owned(),
        );
        lines.join("\n")
    }
}

fn joined_bytes(lines: &[String]) -> usize {
    lines.join("\n").len()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::ToolObservation;

    #[test]
    fn output_policy_writes_artifact_when_preview_is_truncated() {
        let root = test_artifact_root("preview");
        let mut observation = ToolObservation::succeeded(
            "read_file",
            Some("sample.txt".to_owned()),
            Some("/workspace/sample.txt".to_owned()),
            vec![
                "1: one".to_owned(),
                "2: two".to_owned(),
                "3: three".to_owned(),
            ],
            false,
            None,
            "3 lines",
        );

        observation
            .apply_output_policy(2, &root, "run-0001_turn-0001_read_file.txt")
            .expect("policy should apply");

        assert_eq!(observation.preview, vec!["1: one", "2: two"]);
        assert_eq!(observation.total_lines, 3);
        assert!(observation.preview_truncated);
        assert!(observation.truncated);
        let artifact_path = observation.artifact_path.expect("artifact path");
        assert_eq!(
            fs::read_to_string(artifact_path).expect("artifact"),
            "1: one\n2: two\n3: three"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn history_message_contains_structured_observation() {
        let observation = ToolObservation::succeeded(
            "search_text",
            Some(".".to_owned()),
            Some("/workspace".to_owned()),
            vec!["README.md:1: hello".to_owned()],
            false,
            None,
            "1 matches",
        );

        let message = observation.history_message();

        assert!(message.contains("<AHREUM_TOOL_OBSERVATION>"));
        assert!(message.contains("tool_name: search_text"));
        assert!(message.contains("preview:\nREADME.md:1: hello"));
        assert!(message.contains("</AHREUM_TOOL_OBSERVATION>"));
    }

    fn test_artifact_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ahreumcode-tool-artifact-{name}-{}-{unique}",
            std::process::id()
        ))
    }
}
