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
    TargetAlreadyExists,
    PathOutsideWorkspace,
    PathNotFound,
    NotAFile,
    NotADirectory,
    PermissionError,
    ExecutionError,
    Timeout,
    NetworkError,
    IoError,
    GitError,
}

impl ToolErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArguments => "invalid_arguments",
            Self::UnsupportedTool => "unsupported_tool",
            Self::UnsupportedArgument => "unsupported_argument",
            Self::TargetAlreadyExists => "target_already_exists",
            Self::PathOutsideWorkspace => "path_outside_workspace",
            Self::PathNotFound => "path_not_found",
            Self::NotAFile => "not_a_file",
            Self::NotADirectory => "not_a_directory",
            Self::PermissionError => "permission_error",
            Self::ExecutionError => "execution_error",
            Self::Timeout => "timeout",
            Self::NetworkError => "network_error",
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
        Self::failed_with_preview(tool_name, target_raw, error_kind, message, Vec::new())
    }

    pub fn failed_with_preview(
        tool_name: impl Into<String>,
        target_raw: Option<String>,
        error_kind: ToolErrorKind,
        message: impl Into<String>,
        preview: Vec<String>,
    ) -> Self {
        let total_lines = preview.len();
        let total_bytes = joined_bytes(&preview);
        Self {
            status: ObservationStatus::Failed,
            tool_name: tool_name.into(),
            target_raw,
            target_resolved: None,
            preview,
            total_lines,
            total_bytes,
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
            "workspace_path_rule: Tool arguments must use workspace-relative paths from target_raw, preview, or next_range_hint. Runtime absolute paths are omitted from this LLM observation."
                .to_owned(),
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
            format!(
                "error_kind: {}",
                self.error_kind.map(|kind| kind.as_str()).unwrap_or("-")
            ),
            format!("message: {}", self.message),
        ];
        if self.status == ObservationStatus::Succeeded && self.tool_name == "read_file" {
            let exact_lines = read_file_preview_content_lines(&self.preview);
            lines.push("read_file_patch_content:".to_owned());
            lines.push("```".to_owned());
            lines.extend(exact_lines);
            lines.push("```".to_owned());
            lines.push("read_file_line_number_prefixes_are_display_only: true".to_owned());
        }
        lines.push("preview:".to_owned());
        lines.extend(self.preview.iter().cloned());
        lines.push("</AHREUM_TOOL_OBSERVATION>".to_owned());
        lines.push(self.follow_up_instruction().to_owned());
        lines.join("\n")
    }

    fn follow_up_instruction(&self) -> &'static str {
        if self.status == ObservationStatus::Failed
            && self.tool_name == "apply_patch"
            && self.error_kind == Some(ToolErrorKind::InvalidArguments)
            && self.message == "update hunk has no context or removal lines"
        {
            return "Update File failed because the hunk had only added lines. Read the target if exact contents are not already observed, then return an Update File patch whose hunk includes matching existing context lines prefixed with space or removal lines prefixed with -.";
        }
        if self.status == ObservationStatus::Failed
            && self.tool_name == "apply_patch"
            && self.error_kind == Some(ToolErrorKind::InvalidArguments)
            && self.message == "update patch body lines must start with space, +, -, or @@"
        {
            return "Update File failed because at least one hunk line was bare text. Every Update File hunk line must start with exactly one patch marker: space for matching existing context, - for removed existing text, + for added replacement text, or @@ for the hunk boundary.";
        }
        if self.status == ObservationStatus::Failed
            && self.tool_name == "apply_patch"
            && self.error_kind == Some(ToolErrorKind::InvalidArguments)
            && self.message == "update hunk did not match the target exactly once"
        {
            return "Update File failed because its context/removal lines did not exactly match the current target contents. Use the latest read_file observation for that target, then rebuild the hunk with exact existing lines. Patch markers are first characters, not separators: use -old text, not - old text, unless the file line itself starts with a space.";
        }

        match (self.status, self.error_kind) {
            (
                ObservationStatus::Failed,
                Some(ToolErrorKind::TargetAlreadyExists),
            ) => {
                "Target creation failed because the target already exists. Do not retry Add File for the same target. Use read_file when exact current contents are needed, then request Update File or Delete File as appropriate."
            }
            (
                ObservationStatus::Failed,
                Some(
                    ToolErrorKind::PathNotFound
                    | ToolErrorKind::NotAFile
                    | ToolErrorKind::NotADirectory,
                ),
            ) => {
                "Path selection failed. Do not retry the same target_raw. If preview includes candidate_path entries, request read_file for the best candidate. Otherwise use list_files to discover filename/path candidates or structure; use search_text only for symbols, content, or configuration key locations before read_file."
            }
            _ => {
                "Use this observation. Return answer if enough; otherwise return exactly one next tool candidate. If more of the same file is needed, use next_range_hint instead of repeating the same arguments."
            }
        }
    }
}

fn joined_bytes(lines: &[String]) -> usize {
    lines.join("\n").len()
}

fn read_file_preview_content_lines(preview: &[String]) -> Vec<String> {
    preview
        .iter()
        .map(|line| {
            let Some((prefix, content)) = line.split_once(": ") else {
                return line.clone();
            };
            if prefix.chars().all(|ch| ch.is_ascii_digit()) {
                content.to_owned()
            } else {
                line.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{ToolErrorKind, ToolObservation};

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
        assert!(message.contains("workspace_path_rule:"));
        assert!(!message.contains("target_resolved: /workspace"));
        assert!(message.contains("preview:\nREADME.md:1: hello"));
        assert!(message.contains("</AHREUM_TOOL_OBSERVATION>"));
    }

    #[test]
    fn read_file_history_message_includes_patch_content_without_line_numbers() {
        let observation = ToolObservation::succeeded(
            "read_file",
            Some("sample.txt".to_owned()),
            Some("/workspace/sample.txt".to_owned()),
            vec![
                "1: title: live validation".to_owned(),
                "2: status: created".to_owned(),
                "3: ".to_owned(),
            ],
            false,
            None,
            "3 lines",
        );

        let message = observation.history_message();

        assert!(message.contains(
            "read_file_patch_content:\n```\ntitle: live validation\nstatus: created\n\n```"
        ));
        assert!(message.contains("read_file_line_number_prefixes_are_display_only: true"));
        assert!(message.contains("preview:\n1: title: live validation"));
    }

    #[test]
    fn path_failure_history_message_directs_search_or_list_recovery() {
        let observation = ToolObservation::failed(
            "read_file",
            Some("src/missing.rs".to_owned()),
            ToolErrorKind::PathNotFound,
            "path cannot be resolved",
        );

        let message = observation.history_message();

        assert!(message.contains("error_kind: path_not_found"));
        assert!(message.contains("Do not retry the same target_raw"));
        assert!(message.contains("list_files"));
        assert!(message.contains("search_text"));
    }

    #[test]
    fn target_exists_history_message_directs_read_before_update() {
        let observation = ToolObservation::failed(
            "apply_patch",
            Some("sample.txt".to_owned()),
            ToolErrorKind::TargetAlreadyExists,
            "add patch target already exists",
        );

        let message = observation.history_message();

        assert!(message.contains("error_kind: target_already_exists"));
        assert!(message.contains("Do not retry Add File"));
        assert!(message.contains("Use read_file"));
    }

    #[test]
    fn failed_update_hunk_without_context_has_specific_follow_up() {
        let observation = ToolObservation::failed(
            "apply_patch",
            Some("sample.txt".to_owned()),
            ToolErrorKind::InvalidArguments,
            "update hunk has no context or removal lines",
        );

        let message = observation.history_message();

        assert!(message.contains("Update File failed"));
        assert!(message.contains("matching existing context lines"));
        assert!(message.contains("removal lines prefixed with -"));
    }

    #[test]
    fn failed_update_hunk_with_bare_lines_has_specific_follow_up() {
        let observation = ToolObservation::failed(
            "apply_patch",
            Some("sample.txt".to_owned()),
            ToolErrorKind::InvalidArguments,
            "update patch body lines must start with space, +, -, or @@",
        );

        let message = observation.history_message();

        assert!(message.contains("bare text"));
        assert!(message.contains("space for matching existing context"));
        assert!(message.contains("+ for added replacement text"));
    }

    #[test]
    fn failed_update_hunk_mismatch_has_specific_follow_up() {
        let observation = ToolObservation::failed(
            "apply_patch",
            Some("sample.txt".to_owned()),
            ToolErrorKind::InvalidArguments,
            "update hunk did not match the target exactly once",
        );

        let message = observation.history_message();

        assert!(message.contains("did not exactly match"));
        assert!(message.contains("latest read_file observation"));
        assert!(message.contains("Patch markers are first characters"));
    }

    #[test]
    fn failed_observation_can_include_diagnostic_preview() {
        let observation = ToolObservation::failed_with_preview(
            "apply_patch",
            Some("sample.txt".to_owned()),
            ToolErrorKind::InvalidArguments,
            "update hunk did not match the target exactly once",
            vec!["attempted_old_lines:".to_owned(), "old[1]: one".to_owned()],
        );

        let message = observation.history_message();

        assert!(message.contains("total_lines: 2"));
        assert!(message.contains("preview:\nattempted_old_lines:\nold[1]: one"));
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
