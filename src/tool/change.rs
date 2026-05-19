use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::llm::{ChangePreview, PatchOperation};

use super::observation::{ToolErrorKind, ToolObservation};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangePrecondition {
    target_raw: String,
    target_resolved: PathBuf,
    display: String,
    exists: bool,
    len: Option<u64>,
    modified: Option<SystemTime>,
    content_hash: Option<u64>,
}

pub struct ApprovedChange {
    pub preview: ChangePreview,
    pub precondition: ChangePrecondition,
}

pub fn capture_change_precondition(
    workspace_root: &Path,
    preview: &ChangePreview,
) -> Result<ChangePrecondition, ToolObservation> {
    let target = resolve_workspace_target(workspace_root, &preview.target_path)
        .map_err(|error| change_failure(preview, error.kind, error.message))?;

    let metadata = match fs::metadata(&target.resolved) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(change_failure(
                preview,
                ToolErrorKind::IoError,
                format!("target metadata could not be read: {error}"),
            ));
        }
    };

    match (preview.operation, metadata.as_ref()) {
        (PatchOperation::Add, Some(_)) => {
            return Err(change_failure(
                preview,
                ToolErrorKind::InvalidArguments,
                "add patch target already exists",
            ));
        }
        (PatchOperation::Update | PatchOperation::Delete, None) => {
            return Err(change_failure(
                preview,
                ToolErrorKind::PathNotFound,
                "patch target does not exist",
            ));
        }
        _ => {}
    }

    let mut len = None;
    let mut modified = None;
    let mut content_hash = None;
    if let Some(metadata) = metadata {
        if !metadata.is_file() {
            return Err(change_failure(
                preview,
                ToolErrorKind::NotAFile,
                "patch target is not a file",
            ));
        }
        let bytes = fs::read(&target.resolved).map_err(|error| {
            change_failure(
                preview,
                ToolErrorKind::IoError,
                format!("target could not be read for precondition: {error}"),
            )
        })?;
        len = Some(metadata.len());
        modified = metadata.modified().ok();
        content_hash = Some(fnv1a_hash(&bytes));
    }

    Ok(ChangePrecondition {
        target_raw: preview.target_path.clone(),
        target_resolved: target.resolved,
        display: target.display,
        exists: len.is_some(),
        len,
        modified,
        content_hash,
    })
}

pub fn apply_approved_change(workspace_root: &Path, change: ApprovedChange) -> ToolObservation {
    let preview = change.preview;
    if let Err(observation) = verify_precondition(&preview, &change.precondition) {
        return observation;
    }

    let result = match preview.operation {
        PatchOperation::Add => apply_add(&preview, &change.precondition),
        PatchOperation::Update => apply_update(&preview, &change.precondition),
        PatchOperation::Delete => apply_delete(&preview, &change.precondition),
    };

    match result {
        Ok(lines) => {
            if let Err(observation) =
                verify_postcondition(workspace_root, &preview, &change.precondition)
            {
                return observation;
            }
            ToolObservation::succeeded(
                "apply_patch",
                Some(preview.target_path),
                Some(change.precondition.display),
                lines,
                false,
                None,
                "approved patch applied",
            )
        }
        Err(observation) => observation,
    }
}

fn apply_add(
    preview: &ChangePreview,
    precondition: &ChangePrecondition,
) -> Result<Vec<String>, ToolObservation> {
    let content = parse_add_content(preview)?;
    fs::write(&precondition.target_resolved, content.as_bytes()).map_err(|error| {
        change_failure(
            preview,
            ToolErrorKind::IoError,
            format!("add patch could not write target: {error}"),
        )
    })?;
    Ok(vec![format!(
        "added {} (+{} -{})",
        precondition.display, preview.additions, preview.deletions
    )])
}

fn apply_update(
    preview: &ChangePreview,
    precondition: &ChangePrecondition,
) -> Result<Vec<String>, ToolObservation> {
    let original = fs::read_to_string(&precondition.target_resolved).map_err(|error| {
        change_failure(
            preview,
            ToolErrorKind::IoError,
            format!("update target could not be read: {error}"),
        )
    })?;
    let updated = apply_update_hunks(preview, &original)?;
    fs::write(&precondition.target_resolved, updated.as_bytes()).map_err(|error| {
        change_failure(
            preview,
            ToolErrorKind::IoError,
            format!("update patch could not write target: {error}"),
        )
    })?;
    Ok(vec![format!(
        "updated {} (+{} -{})",
        precondition.display, preview.additions, preview.deletions
    )])
}

fn apply_delete(
    preview: &ChangePreview,
    precondition: &ChangePrecondition,
) -> Result<Vec<String>, ToolObservation> {
    fs::remove_file(&precondition.target_resolved).map_err(|error| {
        change_failure(
            preview,
            ToolErrorKind::IoError,
            format!("delete patch could not remove target: {error}"),
        )
    })?;
    Ok(vec![format!("deleted {}", precondition.display)])
}

fn verify_precondition(
    preview: &ChangePreview,
    precondition: &ChangePrecondition,
) -> Result<(), ToolObservation> {
    let metadata = match fs::metadata(&precondition.target_resolved) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(change_failure(
                preview,
                ToolErrorKind::IoError,
                format!("target metadata could not be rechecked: {error}"),
            ));
        }
    };

    if precondition.exists != metadata.is_some() {
        return Err(change_failure(
            preview,
            ToolErrorKind::InvalidArguments,
            "patch target precondition changed before approval was applied",
        ));
    }

    if let Some(metadata) = metadata {
        if !metadata.is_file() {
            return Err(change_failure(
                preview,
                ToolErrorKind::NotAFile,
                "patch target is no longer a file",
            ));
        }
        let bytes = fs::read(&precondition.target_resolved).map_err(|error| {
            change_failure(
                preview,
                ToolErrorKind::IoError,
                format!("target could not be reread for precondition: {error}"),
            )
        })?;
        if Some(metadata.len()) != precondition.len
            || metadata.modified().ok() != precondition.modified
            || Some(fnv1a_hash(&bytes)) != precondition.content_hash
        {
            return Err(change_failure(
                preview,
                ToolErrorKind::InvalidArguments,
                "patch target content changed before approval was applied",
            ));
        }
    }

    Ok(())
}

fn verify_postcondition(
    workspace_root: &Path,
    preview: &ChangePreview,
    precondition: &ChangePrecondition,
) -> Result<(), ToolObservation> {
    let target = resolve_workspace_target(workspace_root, &preview.target_path)
        .map_err(|error| change_failure(preview, error.kind, error.message))?;
    if target.resolved != precondition.target_resolved {
        return Err(change_failure(
            preview,
            ToolErrorKind::PathOutsideWorkspace,
            "patch target resolved differently after apply",
        ));
    }

    match preview.operation {
        PatchOperation::Add | PatchOperation::Update => {
            let metadata = fs::metadata(&precondition.target_resolved).map_err(|error| {
                change_failure(
                    preview,
                    ToolErrorKind::PathNotFound,
                    format!("postcondition target is missing: {error}"),
                )
            })?;
            if !metadata.is_file() {
                return Err(change_failure(
                    preview,
                    ToolErrorKind::NotAFile,
                    "postcondition target is not a file",
                ));
            }
        }
        PatchOperation::Delete => {
            if precondition.target_resolved.exists() {
                return Err(change_failure(
                    preview,
                    ToolErrorKind::IoError,
                    "postcondition target still exists after delete",
                ));
            }
        }
    }

    Ok(())
}

fn parse_add_content(preview: &ChangePreview) -> Result<String, ToolObservation> {
    let mut output = Vec::new();
    let mut in_add = false;
    for line in preview.payload_body.lines() {
        if line == "*** Begin Patch" || line == "*** End Patch" {
            continue;
        }
        if line.starts_with("*** Add File: ") {
            in_add = true;
            continue;
        }
        if !in_add {
            continue;
        }
        let Some(content) = line.strip_prefix('+') else {
            return Err(change_failure(
                preview,
                ToolErrorKind::InvalidArguments,
                "add patch body lines must start with +",
            ));
        };
        output.push(content.to_owned());
    }

    Ok(join_lines_with_trailing_newline(&output))
}

fn apply_update_hunks(preview: &ChangePreview, original: &str) -> Result<String, ToolObservation> {
    let mut current = split_lines(original);
    let mut cursor = 0usize;
    let hunks = parse_update_hunks(preview)?;
    for hunk in hunks {
        let (old_lines, new_lines) = hunk.old_new_lines();
        if old_lines.is_empty() {
            return Err(change_failure(
                preview,
                ToolErrorKind::InvalidArguments,
                "update hunk has no context or removal lines",
            ));
        }
        let Some(index) = find_unique_sequence(&current, &old_lines, cursor) else {
            return Err(change_failure(
                preview,
                ToolErrorKind::InvalidArguments,
                "update hunk did not match the target exactly once",
            ));
        };
        let end = index + old_lines.len();
        current.splice(index..end, new_lines);
        cursor = index.saturating_add(1);
    }

    Ok(join_lines_with_trailing_newline(&current))
}

fn parse_update_hunks(preview: &ChangePreview) -> Result<Vec<UpdateHunk>, ToolObservation> {
    let mut hunks = Vec::new();
    let mut current = UpdateHunk::default();
    let mut in_update = false;
    for line in preview.payload_body.lines() {
        if line == "*** Begin Patch" || line == "*** End Patch" {
            continue;
        }
        if line.starts_with("*** Update File: ") {
            in_update = true;
            continue;
        }
        if !in_update {
            continue;
        }
        if line.starts_with("@@") {
            if !current.lines.is_empty() {
                hunks.push(current);
                current = UpdateHunk::default();
            }
            continue;
        }
        let Some(kind) = UpdateLineKind::from_patch_line(line) else {
            return Err(change_failure(
                preview,
                ToolErrorKind::InvalidArguments,
                "update patch body lines must start with space, +, -, or @@",
            ));
        };
        current.lines.push(kind);
    }
    if !current.lines.is_empty() {
        hunks.push(current);
    }
    if hunks.is_empty() {
        return Err(change_failure(
            preview,
            ToolErrorKind::InvalidArguments,
            "update patch contains no hunks",
        ));
    }
    Ok(hunks)
}

#[derive(Default)]
struct UpdateHunk {
    lines: Vec<UpdateLineKind>,
}

impl UpdateHunk {
    fn old_new_lines(self) -> (Vec<String>, Vec<String>) {
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        for line in self.lines {
            match line {
                UpdateLineKind::Context(value) => {
                    old_lines.push(value.clone());
                    new_lines.push(value);
                }
                UpdateLineKind::Remove(value) => old_lines.push(value),
                UpdateLineKind::Add(value) => new_lines.push(value),
            }
        }
        (old_lines, new_lines)
    }
}

enum UpdateLineKind {
    Context(String),
    Remove(String),
    Add(String),
}

impl UpdateLineKind {
    fn from_patch_line(line: &str) -> Option<Self> {
        line.strip_prefix(' ')
            .map(|value| Self::Context(value.to_owned()))
            .or_else(|| {
                line.strip_prefix('-')
                    .map(|value| Self::Remove(value.to_owned()))
            })
            .or_else(|| {
                line.strip_prefix('+')
                    .map(|value| Self::Add(value.to_owned()))
            })
    }
}

fn find_unique_sequence(lines: &[String], needle: &[String], start: usize) -> Option<usize> {
    let mut matches = lines
        .windows(needle.len())
        .enumerate()
        .skip(start)
        .filter_map(|(index, window)| (window == needle).then_some(index));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

fn split_lines(value: &str) -> Vec<String> {
    value.lines().map(str::to_owned).collect()
}

fn join_lines_with_trailing_newline(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

struct WorkspaceTarget {
    resolved: PathBuf,
    display: String,
}

fn resolve_workspace_target(root: &Path, raw: &str) -> Result<WorkspaceTarget, ChangePathError> {
    validate_workspace_relative_path(raw)?;
    let root = root.canonicalize().map_err(|source| ChangePathError {
        kind: ToolErrorKind::IoError,
        message: format!("workspace root cannot be resolved: {source}"),
    })?;
    let candidate = root.join(raw);
    let parent = candidate.parent().unwrap_or(root.as_path());
    let resolved_parent = parent.canonicalize().map_err(|source| ChangePathError {
        kind: ToolErrorKind::PathNotFound,
        message: format!("target parent cannot be resolved: {source}"),
    })?;
    if !resolved_parent.starts_with(&root) {
        return Err(ChangePathError {
            kind: ToolErrorKind::PathOutsideWorkspace,
            message: "target parent resolved outside workspace".to_owned(),
        });
    }
    let resolved = if candidate.exists() {
        candidate.canonicalize().map_err(|source| ChangePathError {
            kind: ToolErrorKind::IoError,
            message: format!("target cannot be resolved: {source}"),
        })?
    } else {
        candidate
    };
    if !resolved.starts_with(&root) {
        return Err(ChangePathError {
            kind: ToolErrorKind::PathOutsideWorkspace,
            message: "target resolved outside workspace".to_owned(),
        });
    }

    Ok(WorkspaceTarget {
        display: raw.to_owned(),
        resolved,
    })
}

fn validate_workspace_relative_path(raw: &str) -> Result<(), ChangePathError> {
    if raw.is_empty() || raw.trim() != raw || raw.chars().any(char::is_control) {
        return Err(ChangePathError {
            kind: ToolErrorKind::InvalidArguments,
            message: "path must be a non-empty workspace-relative string".to_owned(),
        });
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(ChangePathError {
            kind: ToolErrorKind::PathOutsideWorkspace,
            message: "absolute path is not allowed".to_owned(),
        });
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return Err(ChangePathError {
            kind: ToolErrorKind::PathOutsideWorkspace,
            message: "parent or rooted path component is not allowed".to_owned(),
        });
    }
    Ok(())
}

struct ChangePathError {
    kind: ToolErrorKind,
    message: String,
}

fn change_failure(
    preview: &ChangePreview,
    error_kind: ToolErrorKind,
    message: impl Into<String>,
) -> ToolObservation {
    ToolObservation::failed(
        "apply_patch",
        Some(preview.target_path.clone()),
        error_kind,
        message,
    )
}

fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::llm::{ChangePreview, PatchOperation};

    use super::{apply_approved_change, capture_change_precondition, ApprovedChange};

    fn root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ahreumcode-change-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create root");
        root
    }

    fn preview(body: &str, operation: PatchOperation, target_path: &str) -> ChangePreview {
        ChangePreview {
            payload_id: "patch_001".to_owned(),
            target_path: target_path.to_owned(),
            operation,
            additions: body.lines().filter(|line| line.starts_with('+')).count() as u16,
            deletions: body.lines().filter(|line| line.starts_with('-')).count() as u16,
            payload_body: body.to_owned(),
        }
    }

    #[test]
    fn applies_single_file_update_after_precondition_match() {
        let root = root("update");
        fs::write(root.join("sample.txt"), "one\ntwo\n").expect("write");
        let body =
            "*** Begin Patch\n*** Update File: sample.txt\n@@\n one\n-two\n+three\n*** End Patch";
        let preview = preview(body, PatchOperation::Update, "sample.txt");
        let precondition = capture_change_precondition(&root, &preview).expect("precondition");

        let observation = apply_approved_change(
            &root,
            ApprovedChange {
                preview,
                precondition,
            },
        );

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(
            fs::read_to_string(root.join("sample.txt")).unwrap(),
            "one\nthree\n"
        );
    }

    #[test]
    fn rejects_changed_precondition_before_apply() {
        let root = root("precondition");
        fs::write(root.join("sample.txt"), "one\ntwo\n").expect("write");
        let body =
            "*** Begin Patch\n*** Update File: sample.txt\n@@\n one\n-two\n+three\n*** End Patch";
        let preview = preview(body, PatchOperation::Update, "sample.txt");
        let precondition = capture_change_precondition(&root, &preview).expect("precondition");
        fs::write(root.join("sample.txt"), "changed\n").expect("change");

        let observation = apply_approved_change(
            &root,
            ApprovedChange {
                preview,
                precondition,
            },
        );

        assert_eq!(observation.status.as_str(), "failed");
        assert_eq!(
            observation.error_kind.unwrap().as_str(),
            "invalid_arguments"
        );
    }

    #[test]
    fn applies_single_file_add() {
        let root = root("add");
        let body = "*** Begin Patch\n*** Add File: created.txt\n+hello\n+world\n*** End Patch";
        let preview = preview(body, PatchOperation::Add, "created.txt");
        let precondition = capture_change_precondition(&root, &preview).expect("precondition");

        let observation = apply_approved_change(
            &root,
            ApprovedChange {
                preview,
                precondition,
            },
        );

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(
            fs::read_to_string(root.join("created.txt")).unwrap(),
            "hello\nworld\n"
        );
    }

    #[test]
    fn applies_single_file_delete() {
        let root = root("delete");
        fs::write(root.join("old.txt"), "remove me\n").expect("write");
        let body = "*** Begin Patch\n*** Delete File: old.txt\n*** End Patch";
        let preview = preview(body, PatchOperation::Delete, "old.txt");
        let precondition = capture_change_precondition(&root, &preview).expect("precondition");

        let observation = apply_approved_change(
            &root,
            ApprovedChange {
                preview,
                precondition,
            },
        );

        assert_eq!(observation.status.as_str(), "succeeded");
        assert!(!root.join("old.txt").exists());
    }
}
