use std::path::{Component, Path, PathBuf};

use super::observation::ToolErrorKind;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspacePath {
    pub raw: String,
    pub resolved: PathBuf,
    pub display: String,
}

pub fn resolve_existing_workspace_path(
    root: &Path,
    raw: &str,
) -> Result<WorkspacePath, ToolPathError> {
    validate_workspace_relative_path(raw)?;
    let root = root.canonicalize().map_err(|source| ToolPathError {
        kind: ToolErrorKind::IoError,
        message: format!("workspace root cannot be resolved: {source}"),
    })?;
    let candidate = root.join(raw);
    let resolved = candidate.canonicalize().map_err(|source| ToolPathError {
        kind: ToolErrorKind::PathNotFound,
        message: format!("path cannot be resolved: {source}"),
    })?;

    if !resolved.starts_with(&root) {
        return Err(ToolPathError {
            kind: ToolErrorKind::PathOutsideWorkspace,
            message: "path resolved outside workspace".to_owned(),
        });
    }

    Ok(WorkspacePath {
        raw: raw.to_owned(),
        display: display_path(&root, &resolved),
        resolved,
    })
}

fn validate_workspace_relative_path(raw: &str) -> Result<(), ToolPathError> {
    if raw.is_empty() || raw.trim() != raw || raw.chars().any(char::is_control) {
        return Err(ToolPathError {
            kind: ToolErrorKind::InvalidArguments,
            message: "path must be a non-empty workspace-relative string".to_owned(),
        });
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(ToolPathError {
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
        return Err(ToolPathError {
            kind: ToolErrorKind::PathOutsideWorkspace,
            message: "parent or rooted path component is not allowed".to_owned(),
        });
    }

    Ok(())
}

fn display_path(root: &Path, resolved: &Path) -> String {
    resolved
        .strip_prefix(root)
        .ok()
        .and_then(|path| path.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(".")
        .to_owned()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolPathError {
    pub kind: ToolErrorKind,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::resolve_existing_workspace_path;

    #[test]
    fn rejects_parent_path_before_resolution() {
        let root = std::env::current_dir().expect("current dir");
        let error = resolve_existing_workspace_path(&root, "../outside").expect_err("reject");

        assert_eq!(error.kind.as_str(), "path_outside_workspace");
    }
}
