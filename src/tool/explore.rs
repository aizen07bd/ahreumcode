use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use super::observation::{ToolErrorKind, ToolObservation};
use super::path::{resolve_existing_workspace_path, WorkspacePath};
use super::runtime::{string_arg, u64_arg};

const READ_FILE: &str = "read_file";
const LIST_FILES: &str = "list_files";
const SEARCH_TEXT: &str = "search_text";
const INSPECT_GIT: &str = "inspect_git";

pub struct ListFilesArgs {
    path: String,
    max_depth: usize,
    max_entries: usize,
}

impl ListFilesArgs {
    pub fn from_value(arguments: &Value) -> Result<Self, String> {
        Ok(Self {
            path: string_arg(arguments, "path")?.to_owned(),
            max_depth: usize::try_from(u64_arg(arguments, "max_depth")?)
                .map_err(|_| "max_depth is too large".to_owned())?,
            max_entries: usize::try_from(u64_arg(arguments, "max_entries")?)
                .map_err(|_| "max_entries is too large".to_owned())?,
        })
    }
}

pub struct SearchTextArgs {
    path: String,
    query: String,
    max_results: usize,
}

impl SearchTextArgs {
    pub fn from_value(arguments: &Value) -> Result<Self, String> {
        Ok(Self {
            path: string_arg(arguments, "path")?.to_owned(),
            query: string_arg(arguments, "query")?.to_owned(),
            max_results: usize::try_from(u64_arg(arguments, "max_results")?)
                .map_err(|_| "max_results is too large".to_owned())?,
        })
    }
}

pub struct ReadFileArgs {
    path: String,
    start_line: usize,
    max_lines: usize,
}

impl ReadFileArgs {
    pub fn from_value(arguments: &Value) -> Result<Self, String> {
        Ok(Self {
            path: string_arg(arguments, "path")?.to_owned(),
            start_line: usize::try_from(u64_arg(arguments, "start_line")?)
                .map_err(|_| "start_line is too large".to_owned())?,
            max_lines: usize::try_from(u64_arg(arguments, "max_lines")?)
                .map_err(|_| "max_lines is too large".to_owned())?,
        })
    }
}

pub fn list_files(root: &Path, args: ListFilesArgs) -> ToolObservation {
    let target = match resolve_existing_workspace_path(root, &args.path) {
        Ok(target) => target,
        Err(error) => return path_failure(LIST_FILES, Some(args.path), error.kind, error.message),
    };

    if !target.resolved.is_dir() {
        return path_failure(
            LIST_FILES,
            Some(target.raw),
            ToolErrorKind::NotADirectory,
            "target path is not a directory",
        );
    }

    let mut entries = Vec::new();
    match collect_entries(
        &target.resolved,
        &target,
        0,
        args.max_depth,
        args.max_entries,
        &mut entries,
    ) {
        Ok(truncated) => ToolObservation::succeeded(
            LIST_FILES,
            Some(target.raw),
            Some(target.resolved.display().to_string()),
            entries.clone(),
            truncated,
            None,
            format!("{} entries", entries.len()),
        ),
        Err(message) => ToolObservation::failed(
            LIST_FILES,
            Some(target.raw),
            ToolErrorKind::IoError,
            message,
        ),
    }
}

pub fn search_text(root: &Path, args: SearchTextArgs) -> ToolObservation {
    let target = match resolve_existing_workspace_path(root, &args.path) {
        Ok(target) => target,
        Err(error) => return path_failure(SEARCH_TEXT, Some(args.path), error.kind, error.message),
    };

    let mut files = Vec::new();
    if target.resolved.is_file() {
        files.push(target.resolved.clone());
    } else if target.resolved.is_dir() {
        if let Err(message) = collect_files(&target.resolved, &mut files) {
            return ToolObservation::failed(
                SEARCH_TEXT,
                Some(target.raw),
                ToolErrorKind::IoError,
                message,
            );
        }
    } else {
        return path_failure(
            SEARCH_TEXT,
            Some(target.raw),
            ToolErrorKind::PathNotFound,
            "target path is not searchable",
        );
    }

    files.sort();
    let mut preview = Vec::new();
    let mut scanned = 0usize;
    let display_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut skipped_unreadable = 0usize;
    let mut truncated = false;
    'files: for file in files {
        scanned += 1;
        let content = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(_) => {
                skipped_unreadable += 1;
                continue;
            }
        };
        for (index, line) in content.lines().enumerate() {
            if !line.contains(&args.query) {
                continue;
            }
            if preview.len() >= args.max_results {
                truncated = true;
                break 'files;
            }
            preview.push(format!(
                "{}:{}: {}",
                display_relative(&display_root, &file),
                index + 1,
                line
            ));
        }
    }

    let count = preview.len();
    let message = if skipped_unreadable == 0 {
        format!("{count} matches in {scanned} files")
    } else {
        format!("{count} matches in {scanned} files, {skipped_unreadable} unreadable files skipped")
    };
    ToolObservation::succeeded(
        SEARCH_TEXT,
        Some(target.raw),
        Some(target.resolved.display().to_string()),
        preview,
        truncated,
        None,
        message,
    )
}

pub fn read_file(root: &Path, args: ReadFileArgs) -> ToolObservation {
    let target = match resolve_existing_workspace_path(root, &args.path) {
        Ok(target) => target,
        Err(error) => {
            if error.kind == ToolErrorKind::PathNotFound {
                let candidates = workspace_path_candidates(root, &args.path, CandidateKind::File);
                if !candidates.is_empty() {
                    return ToolObservation::failed_with_preview(
                        READ_FILE,
                        Some(args.path),
                        error.kind,
                        error.message,
                        candidates
                            .into_iter()
                            .map(|path| format!("candidate_path: {path}"))
                            .collect(),
                    );
                }
            }
            return path_failure(READ_FILE, Some(args.path), error.kind, error.message);
        }
    };

    if !target.resolved.is_file() {
        return path_failure(
            READ_FILE,
            Some(target.raw),
            ToolErrorKind::NotAFile,
            "target path is not a file",
        );
    }

    let content = match fs::read_to_string(&target.resolved) {
        Ok(content) => content,
        Err(error) => {
            return ToolObservation::failed(
                READ_FILE,
                Some(target.raw),
                ToolErrorKind::IoError,
                format!("file cannot be read as UTF-8 text: {error}"),
            );
        }
    };

    let lines = content.lines().collect::<Vec<_>>();
    let start_index = args.start_line.saturating_sub(1);
    let end_index = start_index.saturating_add(args.max_lines).min(lines.len());
    let preview = lines
        .iter()
        .enumerate()
        .skip(start_index)
        .take(args.max_lines)
        .map(|(index, line)| format!("{}: {}", index + 1, line))
        .collect::<Vec<_>>();
    let truncated = end_index < lines.len();
    let next_range_hint = truncated.then(|| {
        format!(
            "read_file path={} start_line={} max_lines={}",
            target.raw,
            end_index + 1,
            args.max_lines
        )
    });
    let count = preview.len();

    ToolObservation::succeeded(
        READ_FILE,
        Some(target.raw),
        Some(target.resolved.display().to_string()),
        preview,
        truncated,
        next_range_hint,
        format!("{count} lines"),
    )
}

pub fn inspect_git_status(root: &Path) -> ToolObservation {
    let output = match Command::new("git")
        .args(["status", "--short"])
        .current_dir(root)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return ToolObservation::failed(
                INSPECT_GIT,
                None,
                ToolErrorKind::GitError,
                format!("git status could not start: {error}"),
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return ToolObservation::failed(
            INSPECT_GIT,
            None,
            ToolErrorKind::GitError,
            if stderr.is_empty() {
                "git status failed".to_owned()
            } else {
                stderr
            },
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let preview = if stdout.trim().is_empty() {
        vec!["working tree clean".to_owned()]
    } else {
        stdout.lines().map(ToOwned::to_owned).collect::<Vec<_>>()
    };
    let count = preview.len();
    ToolObservation::succeeded(
        INSPECT_GIT,
        None,
        None,
        preview,
        false,
        None,
        format!("{count} status lines"),
    )
}

fn collect_entries(
    current: &Path,
    target: &WorkspacePath,
    depth: usize,
    max_depth: usize,
    max_entries: usize,
    entries: &mut Vec<String>,
) -> Result<bool, String> {
    if entries.len() >= max_entries {
        return Ok(true);
    }

    let mut children = fs::read_dir(current)
        .map_err(|error| format!("directory cannot be read: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("directory entry cannot be read: {error}"))?;
    children.sort_by_key(|entry| entry.path());

    let mut truncated = false;
    for entry in children {
        if entries.len() >= max_entries {
            truncated = true;
            break;
        }

        let path = entry.path();
        let display = display_relative(&target.resolved, &path);
        let file_type = entry
            .file_type()
            .map_err(|error| format!("file type cannot be read: {error}"))?;
        let suffix = if file_type.is_dir() { "/" } else { "" };
        entries.push(format!("{display}{suffix}"));

        if file_type.is_dir() && depth + 1 < max_depth {
            truncated |=
                collect_entries(&path, target, depth + 1, max_depth, max_entries, entries)?;
        }
    }

    Ok(truncated)
}

fn collect_files(current: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let mut children = fs::read_dir(current)
        .map_err(|error| format!("directory cannot be read: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("directory entry cannot be read: {error}"))?;
    children.sort_by_key(|entry| entry.path());

    for entry in children {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("file type cannot be read: {error}"))?;
        if file_type.is_dir() {
            collect_files(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }

    Ok(())
}

fn path_failure(
    tool_name: &'static str,
    target_raw: Option<String>,
    kind: ToolErrorKind,
    message: impl Into<String>,
) -> ToolObservation {
    ToolObservation::failed(tool_name, target_raw, kind, message)
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|path| path.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(".")
        .to_owned()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateKind {
    File,
}

fn workspace_path_candidates(root: &Path, raw: &str, kind: CandidateKind) -> Vec<String> {
    let Ok(root) = root.canonicalize() else {
        return Vec::new();
    };
    let requested = normalized_components(raw);
    if requested.is_empty() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    collect_path_candidates(&root, &root, &requested, kind, &mut candidates);
    candidates.sort();
    candidates.dedup();
    candidates.truncate(20);
    candidates
}

fn collect_path_candidates(
    root: &Path,
    current: &Path,
    requested: &[String],
    kind: CandidateKind,
    candidates: &mut Vec<String>,
) {
    if candidates.len() >= 20 {
        return;
    }

    let Ok(children) = fs::read_dir(current) else {
        return;
    };
    let mut children = children.filter_map(Result::ok).collect::<Vec<_>>();
    children.sort_by_key(|entry| entry.path());

    for entry in children {
        if candidates.len() >= 20 {
            break;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            collect_path_candidates(root, &path, requested, kind, candidates);
            continue;
        }

        if kind == CandidateKind::File && file_type.is_file() {
            let display = display_relative(root, &path);
            if candidate_path_matches_request(&display, requested) {
                candidates.push(display);
            }
        }
    }
}

fn normalized_components(raw: &str) -> Vec<String> {
    raw.split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .map(str::to_owned)
        .collect()
}

fn candidate_path_matches_request(candidate: &str, requested: &[String]) -> bool {
    let candidate_components = normalized_components(candidate);
    if candidate_components.len() < requested.len() {
        return false;
    }
    candidate_components
        .iter()
        .rev()
        .zip(requested.iter().rev())
        .all(|(candidate, requested)| candidate == requested)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{
        inspect_git_status, list_files, read_file, search_text, ListFilesArgs, ReadFileArgs,
        SearchTextArgs,
    };

    fn test_workspace(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ahreumcode-tool-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create test workspace");
        root
    }

    #[test]
    fn read_file_returns_requested_line_range() {
        let root = test_workspace("read");
        fs::write(root.join("sample.txt"), "one\ntwo\nthree\n").expect("write");
        let args = ReadFileArgs::from_value(&json!({
            "path": "sample.txt",
            "start_line": 2,
            "max_lines": 1
        }))
        .expect("args");

        let observation = read_file(&root, args);

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(observation.preview, vec!["2: two"]);
        assert!(observation.truncated);
        assert_eq!(
            observation.next_range_hint.as_deref(),
            Some("read_file path=sample.txt start_line=3 max_lines=1")
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn list_files_returns_workspace_entries() {
        let root = test_workspace("list");
        fs::create_dir(root.join("src")).expect("mkdir");
        fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("write");
        fs::write(root.join("a.txt"), "a\n").expect("write");
        fs::write(root.join("b.txt"), "b\n").expect("write");
        let args = ListFilesArgs::from_value(&json!({
            "path": ".",
            "max_depth": 2,
            "max_entries": 10
        }))
        .expect("args");

        let observation = list_files(&root, args);

        assert_eq!(observation.status.as_str(), "succeeded");
        assert!(observation.preview.iter().any(|line| line == "src/"));
        assert!(observation.preview.iter().any(|line| line == "src/main.rs"));
        let truncated_args = ListFilesArgs::from_value(&json!({
            "path": ".",
            "max_depth": 1,
            "max_entries": 1
        }))
        .expect("args");

        let observation = list_files(&root, truncated_args);

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(observation.preview.len(), 1);
        assert!(observation.truncated);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn search_text_returns_literal_matches() {
        let root = test_workspace("search");
        fs::write(
            root.join("README.md"),
            "alpha\nbeta\nneedle one\nneedle two\n",
        )
        .expect("write");
        let args = SearchTextArgs::from_value(&json!({
            "path": ".",
            "query": "beta",
            "max_results": 10
        }))
        .expect("args");

        let observation = search_text(&root, args);

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(observation.preview, vec!["README.md:2: beta"]);
        let truncated_args = SearchTextArgs::from_value(&json!({
            "path": ".",
            "query": "needle",
            "max_results": 1
        }))
        .expect("args");

        let observation = search_text(&root, truncated_args);

        assert_eq!(observation.status.as_str(), "succeeded");
        assert_eq!(observation.preview.len(), 1);
        assert!(observation.truncated);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn read_file_path_not_found_returns_matching_file_candidates() {
        let root = test_workspace("read-candidates");
        fs::create_dir(root.join("nested")).expect("mkdir");
        fs::write(root.join("nested/settings.local"), "mode = \"local\"\n").expect("write");
        let args = ReadFileArgs::from_value(&json!({
            "path": "settings.local",
            "start_line": 1,
            "max_lines": 120
        }))
        .expect("args");

        let observation = read_file(&root, args);

        assert_eq!(observation.status.as_str(), "failed");
        assert_eq!(
            observation.error_kind.expect("kind").as_str(),
            "path_not_found"
        );
        assert_eq!(
            observation.preview,
            vec!["candidate_path: nested/settings.local"]
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn inspect_git_status_reports_git_failure_without_panicking() {
        let root = test_workspace("git-status");

        let observation = inspect_git_status(&root);

        assert_eq!(observation.status.as_str(), "failed");
        assert_eq!(observation.error_kind.expect("kind").as_str(), "git_error");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
