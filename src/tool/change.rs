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
    pub preconditions: Vec<ChangePrecondition>,
}

pub fn validate_approved_change(change: &ApprovedChange) -> Result<(), ToolObservation> {
    let target_previews = split_change_preview(&change.preview)?;
    if target_previews.len() != change.preconditions.len() {
        return Err(change_failure(
            &change.preview,
            ToolErrorKind::InvalidArguments,
            "patch target precondition count does not match target count",
        ));
    }
    for (preview, precondition) in target_previews.iter().zip(&change.preconditions) {
        validate_single_approved_change(preview, precondition)?;
    }
    Ok(())
}

fn validate_single_approved_change(
    preview: &ChangePreview,
    precondition: &ChangePrecondition,
) -> Result<(), ToolObservation> {
    verify_precondition(preview, precondition)?;
    match preview.operation {
        PatchOperation::Add => {
            parse_add_content(preview)?;
        }
        PatchOperation::Update => {
            let original = fs::read_to_string(&precondition.target_resolved).map_err(|error| {
                change_failure(
                    preview,
                    ToolErrorKind::IoError,
                    format!("update target could not be read: {error}"),
                )
            })?;
            apply_update_hunks(preview, &original)?;
        }
        PatchOperation::Delete => {
            validate_delete_patch_body(preview)?;
        }
    }
    Ok(())
}

pub fn capture_change_preconditions(
    workspace_root: &Path,
    preview: &ChangePreview,
) -> Result<Vec<ChangePrecondition>, ToolObservation> {
    split_change_preview(preview)?
        .iter()
        .map(|target_preview| capture_single_change_precondition(workspace_root, target_preview))
        .collect()
}

#[cfg(test)]
pub fn capture_change_precondition(
    workspace_root: &Path,
    preview: &ChangePreview,
) -> Result<ChangePrecondition, ToolObservation> {
    let mut preconditions = capture_change_preconditions(workspace_root, preview)?;
    if preconditions.len() != 1 {
        return Err(change_failure(
            preview,
            ToolErrorKind::InvalidArguments,
            "single-target precondition requested for multi-target patch",
        ));
    }
    Ok(preconditions.remove(0))
}

fn capture_single_change_precondition(
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
                ToolErrorKind::TargetAlreadyExists,
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
    if let Err(observation) = validate_approved_change(&ApprovedChange {
        preview: preview.clone(),
        preconditions: change.preconditions.clone(),
    }) {
        return observation;
    }

    let target_previews = match split_change_preview(&preview) {
        Ok(target_previews) => target_previews,
        Err(observation) => return observation,
    };
    let snapshots = match capture_snapshots(&change.preconditions) {
        Ok(snapshots) => snapshots,
        Err(observation) => return observation,
    };

    let mut output = Vec::new();
    for (target_preview, precondition) in target_previews.iter().zip(&change.preconditions) {
        let result = match target_preview.operation {
            PatchOperation::Add => apply_add(target_preview, precondition),
            PatchOperation::Update => apply_update(target_preview, precondition),
            PatchOperation::Delete => apply_delete(target_preview, precondition),
        };
        match result {
            Ok(lines) => output.extend(lines),
            Err(observation) => {
                if let Err(rollback) = restore_snapshots(&snapshots) {
                    return change_failure_with_preview(
                        &preview,
                        ToolErrorKind::IoError,
                        format!("patch failed and rollback failed: {rollback}"),
                        observation.preview,
                    );
                }
                return observation;
            }
        }
    }

    for (target_preview, precondition) in target_previews.iter().zip(&change.preconditions) {
        if let Err(observation) = verify_postcondition(workspace_root, target_preview, precondition)
        {
            let _ = restore_snapshots(&snapshots);
            return observation;
        }
    }

    let resolved = change
        .preconditions
        .iter()
        .map(|precondition| precondition.display.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    ToolObservation::succeeded(
        "apply_patch",
        Some(preview.target_path),
        Some(resolved),
        output,
        false,
        None,
        "approved patch applied",
    )
}

#[derive(Clone)]
struct ChangeSnapshot {
    path: PathBuf,
    existed: bool,
    bytes: Option<Vec<u8>>,
}

fn capture_snapshots(
    preconditions: &[ChangePrecondition],
) -> Result<Vec<ChangeSnapshot>, ToolObservation> {
    preconditions
        .iter()
        .map(|precondition| {
            let bytes = if precondition.exists {
                Some(fs::read(&precondition.target_resolved).map_err(|error| {
                    ToolObservation::failed(
                        "apply_patch",
                        Some(precondition.target_raw.clone()),
                        ToolErrorKind::IoError,
                        format!("target snapshot could not be read: {error}"),
                    )
                })?)
            } else {
                None
            };
            Ok(ChangeSnapshot {
                path: precondition.target_resolved.clone(),
                existed: precondition.exists,
                bytes,
            })
        })
        .collect()
}

fn restore_snapshots(snapshots: &[ChangeSnapshot]) -> io::Result<()> {
    for snapshot in snapshots.iter().rev() {
        if snapshot.existed {
            if let Some(parent) = snapshot.path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(
                &snapshot.path,
                snapshot.bytes.as_deref().unwrap_or_default(),
            )?;
        } else if snapshot.path.exists() {
            fs::remove_file(&snapshot.path)?;
        }
    }
    Ok(())
}

fn split_change_preview(preview: &ChangePreview) -> Result<Vec<ChangePreview>, ToolObservation> {
    preview
        .split_by_target()
        .map_err(|message| change_failure(preview, ToolErrorKind::InvalidArguments, message))
}

fn apply_add(
    preview: &ChangePreview,
    precondition: &ChangePrecondition,
) -> Result<Vec<String>, ToolObservation> {
    let content = parse_add_content(preview)?;
    if let Some(parent) = precondition.target_resolved.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            change_failure(
                preview,
                ToolErrorKind::IoError,
                format!("add patch target parent could not be created: {error}"),
            )
        })?;
    }
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
    validate_delete_patch_body(preview)?;
    fs::remove_file(&precondition.target_resolved).map_err(|error| {
        change_failure(
            preview,
            ToolErrorKind::IoError,
            format!("delete patch could not remove target: {error}"),
        )
    })?;
    Ok(vec![format!("deleted {}", precondition.display)])
}

fn validate_delete_patch_body(preview: &ChangePreview) -> Result<(), ToolObservation> {
    for line in preview.payload_body.lines() {
        if line == "*** Begin Patch"
            || line == "*** End Patch"
            || line.starts_with("*** Delete File: ")
        {
            continue;
        }
        return Err(change_failure(
            preview,
            ToolErrorKind::InvalidArguments,
            "Delete File patch must not include patch body lines",
        ));
    }
    Ok(())
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
    let mut current = TextLines::from_text(original);
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
        let Some(index) = find_unique_sequence(&current.lines, &old_lines, cursor) else {
            return Err(change_failure_with_preview(
                preview,
                ToolErrorKind::InvalidArguments,
                "update hunk did not match the target exactly once",
                mismatch_preview_lines(&old_lines, &new_lines),
            ));
        };
        let end = index + old_lines.len();
        current.splice(index, end, new_lines);
        cursor = index.saturating_add(1);
    }

    Ok(current.into_text())
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

struct TextLines {
    lines: Vec<String>,
    endings: Vec<&'static str>,
}

impl TextLines {
    fn from_text(value: &str) -> Self {
        let mut lines = Vec::new();
        let mut endings = Vec::new();
        let bytes = value.as_bytes();
        let mut start = 0usize;
        let mut index = 0usize;

        while index < bytes.len() {
            if bytes[index] == b'\n' {
                let has_cr = index > start && bytes[index - 1] == b'\r';
                let content_end = if has_cr { index - 1 } else { index };
                lines.push(value[start..content_end].to_owned());
                endings.push(if has_cr { "\r\n" } else { "\n" });
                index += 1;
                start = index;
                continue;
            }
            index += 1;
        }

        if start < value.len() {
            lines.push(value[start..].to_owned());
            endings.push("");
        }

        Self { lines, endings }
    }

    fn splice(&mut self, start: usize, end: usize, new_lines: Vec<String>) {
        let old_endings = self.endings[start..end].to_vec();
        let replacement_endings =
            self.replacement_endings(start, end, &old_endings, new_lines.len());
        self.lines.splice(start..end, new_lines);
        self.endings.splice(start..end, replacement_endings);
    }

    fn replacement_endings(
        &self,
        start: usize,
        end: usize,
        old_endings: &[&'static str],
        count: usize,
    ) -> Vec<&'static str> {
        let default_ending = self.default_line_ending(start, end, old_endings);
        let old_block_ended_line = old_endings.last().is_some_and(|ending| !ending.is_empty());
        (0..count)
            .map(|index| {
                old_endings.get(index).copied().unwrap_or_else(|| {
                    if index + 1 == count && !old_block_ended_line {
                        ""
                    } else {
                        default_ending
                    }
                })
            })
            .collect()
    }

    fn default_line_ending(
        &self,
        start: usize,
        end: usize,
        old_endings: &[&'static str],
    ) -> &'static str {
        old_endings
            .iter()
            .copied()
            .find(|ending| !ending.is_empty())
            .or_else(|| {
                start
                    .checked_sub(1)
                    .and_then(|index| self.endings.get(index).copied())
                    .filter(|ending| !ending.is_empty())
            })
            .or_else(|| {
                self.endings
                    .get(end..)
                    .and_then(|endings| endings.iter().copied().find(|ending| !ending.is_empty()))
            })
            .unwrap_or("\n")
    }

    fn into_text(self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }

        let mut text = String::new();
        for (line, ending) in self.lines.into_iter().zip(self.endings) {
            text.push_str(&line);
            text.push_str(ending);
        }
        text
    }
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
    let anchor = existing_path_anchor(candidate.parent().unwrap_or(root.as_path()));
    let resolved_anchor = anchor.canonicalize().map_err(|source| ChangePathError {
        kind: ToolErrorKind::PathNotFound,
        message: format!("target path anchor cannot be resolved: {source}"),
    })?;
    if !resolved_anchor.starts_with(&root) {
        return Err(ChangePathError {
            kind: ToolErrorKind::PathOutsideWorkspace,
            message: "target path anchor resolved outside workspace".to_owned(),
        });
    }
    if !resolved_anchor.is_dir() {
        return Err(ChangePathError {
            kind: ToolErrorKind::NotADirectory,
            message: "target path anchor is not a directory".to_owned(),
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

fn existing_path_anchor(path: &Path) -> &Path {
    let mut candidate = path;
    while !candidate.exists() {
        let Some(parent) = candidate.parent() else {
            return candidate;
        };
        candidate = parent;
    }
    candidate
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

fn change_failure_with_preview(
    preview: &ChangePreview,
    error_kind: ToolErrorKind,
    message: impl Into<String>,
    diagnostic_preview: Vec<String>,
) -> ToolObservation {
    ToolObservation::failed_with_preview(
        "apply_patch",
        Some(preview.target_path.clone()),
        error_kind,
        message,
        diagnostic_preview,
    )
}

fn mismatch_preview_lines(old_lines: &[String], new_lines: &[String]) -> Vec<String> {
    const LIMIT: usize = 12;
    let mut lines = vec![
        "attempted_old_lines:".to_owned(),
        "These lines were the exact context/removal text the patch tried to match.".to_owned(),
    ];
    lines.extend(
        old_lines
            .iter()
            .take(LIMIT)
            .enumerate()
            .map(|(index, line)| format!("old[{}]: {}", index + 1, line)),
    );
    if old_lines.len() > LIMIT {
        lines.push(format!("old_truncated_after: {LIMIT}"));
    }
    lines.push("attempted_new_lines:".to_owned());
    lines.extend(
        new_lines
            .iter()
            .take(LIMIT)
            .enumerate()
            .map(|(index, line)| format!("new[{}]: {}", index + 1, line)),
    );
    if new_lines.len() > LIMIT {
        lines.push(format!("new_truncated_after: {LIMIT}"));
    }
    lines
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

    use super::{
        apply_approved_change, capture_change_precondition, capture_change_preconditions,
        validate_approved_change, ApprovedChange,
    };

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
    fn applies_update_preserving_text_boundaries() {
        let cases = [
            ("update", "one\ntwo\n", "three", "one\nthree\n"),
            ("update-no-final-newline", "one\ntwo", "three", "one\nthree"),
            ("update-crlf", "one\r\ntwo\r\n", "three", "one\r\nthree\r\n"),
            (
                "update-mixed-line-endings",
                "one\r\ntwo\nthree",
                "changed",
                "one\r\nchanged\nthree",
            ),
        ];

        for (name, original, replacement, expected) in cases {
            let root = root(name);
            fs::write(root.join("sample.txt"), original).expect("write");
            let body = format!(
                "*** Begin Patch\n*** Update File: sample.txt\n@@\n one\n-two\n+{replacement}\n*** End Patch"
            );
            let preview = preview(&body, PatchOperation::Update, "sample.txt");
            let precondition = capture_change_precondition(&root, &preview).expect("precondition");

            let observation = apply_approved_change(
                &root,
                ApprovedChange {
                    preview,
                    preconditions: vec![precondition],
                },
            );

            assert_eq!(observation.status.as_str(), "succeeded");
            assert_eq!(
                fs::read_to_string(root.join("sample.txt")).unwrap(),
                expected
            );
        }
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
                preconditions: vec![precondition],
            },
        );

        assert_eq!(observation.status.as_str(), "failed");
        assert_eq!(
            observation.error_kind.unwrap().as_str(),
            "invalid_arguments"
        );
    }

    #[test]
    fn failed_update_mismatch_reports_attempted_lines() {
        let root = root("update-mismatch");
        fs::write(root.join("sample.txt"), "one\ntwo\n").expect("write");
        let body =
            "*** Begin Patch\n*** Update File: sample.txt\n@@\n one\n-missing\n+three\n*** End Patch";
        let preview = preview(body, PatchOperation::Update, "sample.txt");
        let precondition = capture_change_precondition(&root, &preview).expect("precondition");

        let observation = apply_approved_change(
            &root,
            ApprovedChange {
                preview,
                preconditions: vec![precondition],
            },
        );

        assert_eq!(observation.status.as_str(), "failed");
        assert!(observation.preview_text().contains("attempted_old_lines:"));
        assert!(observation.preview_text().contains("old[2]: missing"));
        assert!(observation
            .history_message()
            .contains("latest read_file observation"));
    }

    #[test]
    fn validates_update_hunk_before_approval_without_mutating_target() {
        let root = root("preapproval-update-mismatch");
        fs::write(root.join("sample.txt"), "alpha\n").expect("write");
        let body =
            "*** Begin Patch\n*** Update File: sample.txt\n@@\n- alpha\n+beta\n*** End Patch";
        let preview = preview(body, PatchOperation::Update, "sample.txt");
        let precondition = capture_change_precondition(&root, &preview).expect("precondition");

        let observation = validate_approved_change(&ApprovedChange {
            preview,
            preconditions: vec![precondition],
        })
        .expect_err("mismatched update should fail before approval");

        assert_eq!(observation.status.as_str(), "failed");
        assert!(observation.preview_text().contains("attempted_old_lines:"));
        assert_eq!(
            fs::read_to_string(root.join("sample.txt")).unwrap(),
            "alpha\n"
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
                preconditions: vec![precondition],
            },
        );

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(
            fs::read_to_string(root.join("created.txt")).unwrap(),
            "hello\nworld\n"
        );
    }

    #[test]
    fn applies_single_file_add_with_missing_parent_directories() {
        let root = root("add-parent");
        let body =
            "*** Begin Patch\n*** Add File: nested/pages/index.html\n+<main>Hello</main>\n*** End Patch";
        let preview = preview(body, PatchOperation::Add, "nested/pages/index.html");
        let precondition = capture_change_precondition(&root, &preview).expect("precondition");

        let observation = apply_approved_change(
            &root,
            ApprovedChange {
                preview,
                preconditions: vec![precondition],
            },
        );

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(
            fs::read_to_string(root.join("nested/pages/index.html")).unwrap(),
            "<main>Hello</main>\n"
        );
    }

    #[test]
    fn applies_multi_file_add_atomically() {
        let root = root("multi-add");
        let body = concat!(
            "*** Begin Patch\n",
            "*** Add File: web/index.html\n",
            "+<link rel=\"stylesheet\" href=\"styles.css\">\n",
            "+<script src=\"game.js\"></script>\n",
            "*** Add File: web/styles.css\n",
            "+body { font-family: sans-serif; }\n",
            "*** Add File: web/game.js\n",
            "+console.log('ready');\n",
            "*** End Patch"
        );
        let preview = preview(
            body,
            PatchOperation::Add,
            "3 targets: web/index.html, web/styles.css, web/game.js",
        );
        let preconditions =
            capture_change_preconditions(&root, &preview).expect("multi preconditions");

        let observation = apply_approved_change(
            &root,
            ApprovedChange {
                preview,
                preconditions,
            },
        );

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(
            fs::read_to_string(root.join("web/index.html")).unwrap(),
            "<link rel=\"stylesheet\" href=\"styles.css\">\n<script src=\"game.js\"></script>\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("web/styles.css")).unwrap(),
            "body { font-family: sans-serif; }\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("web/game.js")).unwrap(),
            "console.log('ready');\n"
        );
    }

    #[test]
    fn applies_multi_file_add_with_raw_add_body_lines() {
        let root = root("multi-add-raw-body");
        let body = concat!(
            "*** Begin Patch\n",
            "*** Add File: web/index.html\n",
            "<link rel=\"stylesheet\" href=\"styles.css\">\n",
            "<script src=\"game.js\"></script>\n",
            "*** Add File: web/styles.css\n",
            "body { font-family: sans-serif; }\n",
            "*** Add File: web/game.js\n",
            "console.log('ready');\n",
            "*** End Patch"
        );
        let preview = preview(
            body,
            PatchOperation::Add,
            "3 targets: web/index.html, web/styles.css, web/game.js",
        );
        let preconditions =
            capture_change_preconditions(&root, &preview).expect("multi preconditions");

        let observation = apply_approved_change(
            &root,
            ApprovedChange {
                preview,
                preconditions,
            },
        );

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(
            fs::read_to_string(root.join("web/index.html")).unwrap(),
            "<link rel=\"stylesheet\" href=\"styles.css\">\n<script src=\"game.js\"></script>\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("web/styles.css")).unwrap(),
            "body { font-family: sans-serif; }\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("web/game.js")).unwrap(),
            "console.log('ready');\n"
        );
    }

    #[test]
    fn rejects_add_when_existing_path_anchor_is_file() {
        let root = root("add-parent-file");
        fs::write(root.join("nested"), "not a directory\n").expect("write");
        let body =
            "*** Begin Patch\n*** Add File: nested/pages/index.html\n+<main>Hello</main>\n*** End Patch";
        let preview = preview(body, PatchOperation::Add, "nested/pages/index.html");

        let observation =
            capture_change_precondition(&root, &preview).expect_err("file anchor should fail");

        assert_eq!(observation.status.as_str(), "failed");
        assert_eq!(observation.error_kind.unwrap().as_str(), "not_a_directory");
    }

    #[test]
    fn rejects_add_when_target_exists_with_specific_error_kind() {
        let root = root("add-existing");
        fs::write(root.join("created.txt"), "old\n").expect("write");
        let body = "*** Begin Patch\n*** Add File: created.txt\n+hello\n*** End Patch";
        let preview = preview(body, PatchOperation::Add, "created.txt");

        let observation = capture_change_precondition(&root, &preview)
            .expect_err("existing add target should fail");

        assert_eq!(observation.status.as_str(), "failed");
        assert_eq!(
            observation.error_kind.unwrap().as_str(),
            "target_already_exists"
        );
        assert!(observation
            .history_message()
            .contains("Do not retry Add File for the same target"));
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
                preconditions: vec![precondition],
            },
        );

        assert_eq!(observation.status.as_str(), "succeeded");
        assert!(!root.join("old.txt").exists());
    }

    #[test]
    fn rejects_delete_patch_with_body_lines_at_execution_boundary() {
        let root = root("delete-with-body");
        fs::write(root.join("old.txt"), "remove me\n").expect("write");
        let body = "*** Begin Patch\n*** Delete File: old.txt\n-remove me\n*** End Patch";
        let preview = preview(body, PatchOperation::Delete, "old.txt");
        let observation =
            capture_change_precondition(&root, &preview).expect_err("delete body should fail");

        assert_eq!(observation.status.as_str(), "failed");
        assert!(root.join("old.txt").exists());
        assert_eq!(
            observation.error_kind.unwrap().as_str(),
            "invalid_arguments"
        );
    }
}
