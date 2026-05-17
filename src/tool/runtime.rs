use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::llm::Activity;

use super::explore::{
    inspect_git_status, list_files, read_file, search_text, ListFilesArgs, ReadFileArgs,
    SearchTextArgs,
};
use super::observation::{ToolErrorKind, ToolObservation, DEFAULT_PREVIEW_LINE_LIMIT};

#[derive(Clone)]
pub struct ToolRuntime {
    workspace_root: PathBuf,
    artifact_root: PathBuf,
    limits: ToolRuntimeLimits,
}

impl ToolRuntime {
    pub fn new(workspace_root: impl Into<PathBuf>, artifact_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            artifact_root: artifact_root.into(),
            limits: ToolRuntimeLimits::default(),
        }
    }

    pub fn execute(&self, call: ToolCall) -> ToolObservation {
        if call.activity != Activity::Explore {
            return ToolObservation::failed(
                call.tool_name,
                None,
                ToolErrorKind::UnsupportedTool,
                "tool-01 only executes Explore tools",
            );
        }

        let mut observation = match call.tool_name.as_str() {
            "list_files" => match ListFilesArgs::from_value(&call.arguments) {
                Ok(args) => list_files(&self.workspace_root, args),
                Err(error) => invalid_arguments("list_files", error),
            },
            "search_text" => match SearchTextArgs::from_value(&call.arguments) {
                Ok(args) => search_text(&self.workspace_root, args),
                Err(error) => invalid_arguments("search_text", error),
            },
            "read_file" => match ReadFileArgs::from_value(&call.arguments) {
                Ok(args) => read_file(&self.workspace_root, args),
                Err(error) => invalid_arguments("read_file", error),
            },
            "inspect_git" => match string_arg(&call.arguments, "scope") {
                Ok("status") => inspect_git_status(&self.workspace_root),
                Ok(_) => ToolObservation::failed(
                    "inspect_git",
                    None,
                    ToolErrorKind::UnsupportedArgument,
                    "tool-01 only supports inspect_git scope=status",
                ),
                Err(error) => invalid_arguments("inspect_git", error),
            },
            _ => ToolObservation::failed(
                call.tool_name.clone(),
                None,
                ToolErrorKind::UnsupportedTool,
                "tool is not implemented in tool-01",
            ),
        };
        let artifact_name = call.artifact_name();
        if let Err(error) = observation.apply_output_policy(
            self.limits.preview_line_limit,
            &self.artifact_root,
            &artifact_name,
        ) {
            return ToolObservation::failed(
                observation.tool_name,
                observation.target_raw,
                ToolErrorKind::IoError,
                format!("tool artifact could not be written: {error}"),
            );
        }

        observation
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}

#[derive(Clone)]
struct ToolRuntimeLimits {
    preview_line_limit: usize,
}

impl Default for ToolRuntimeLimits {
    fn default() -> Self {
        Self {
            preview_line_limit: DEFAULT_PREVIEW_LINE_LIMIT,
        }
    }
}

pub struct ToolCall {
    pub run_id: String,
    pub turn_id: String,
    pub activity: Activity,
    pub tool_name: String,
    pub arguments: Value,
}

impl ToolCall {
    pub fn new(
        run_id: String,
        turn_id: String,
        activity: Activity,
        tool_name: String,
        arguments: Value,
    ) -> Self {
        Self {
            run_id,
            turn_id,
            activity,
            tool_name,
            arguments,
        }
    }

    fn artifact_name(&self) -> String {
        format!(
            "{}_{}_{}.txt",
            sanitize_artifact_component(&self.run_id),
            sanitize_artifact_component(&self.turn_id),
            sanitize_artifact_component(&self.tool_name)
        )
    }
}

pub(super) fn string_arg<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    arguments
        .as_object()
        .and_then(|object| object.get(name))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

pub(super) fn u64_arg(arguments: &Value, name: &str) -> Result<u64, String> {
    arguments
        .as_object()
        .and_then(|object| object.get(name))
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{name} must be an unsigned integer"))
}

pub(super) fn bool_arg(arguments: &Value, name: &str) -> Result<bool, String> {
    arguments
        .as_object()
        .and_then(|object| object.get(name))
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{name} must be a boolean"))
}

fn invalid_arguments(tool_name: &'static str, message: String) -> ToolObservation {
    ToolObservation::failed(tool_name, None, ToolErrorKind::InvalidArguments, message)
}

fn sanitize_artifact_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}
