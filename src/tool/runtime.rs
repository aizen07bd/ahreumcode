use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::llm::Activity;

use super::explore::{
    inspect_git_status, list_files, read_file, search_text, ListFilesArgs, ReadFileArgs,
    SearchTextArgs,
};
use super::observation::{ToolErrorKind, ToolObservation, DEFAULT_PREVIEW_LINE_LIMIT};
use crate::tool::{
    execute_approved_command, execute_approved_web, normalize_tool_arguments,
    run_post_edit_diagnostics, ApprovedCommand, ApprovedWebRequest, PostEditDiagnosticRequest,
    ToolName, ToolRuntimeSupport,
};

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
                "tool runtime cannot execute this activity",
            );
        }

        let Some(tool_name) = ToolName::parse(&call.tool_name) else {
            return ToolObservation::failed(
                call.tool_name,
                None,
                ToolErrorKind::UnsupportedTool,
                "tool is not available in runtime",
            );
        };
        let spec = tool_name.spec();
        if spec.runtime != ToolRuntimeSupport::Explore {
            return ToolObservation::failed(
                spec.name,
                None,
                ToolErrorKind::UnsupportedTool,
                "tool is not executable by the current runtime",
            );
        }
        let arguments = match normalize_tool_arguments(tool_name, &call.arguments) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_arguments(spec.name, error),
        };

        let mut observation = match tool_name {
            ToolName::ListFiles => match ListFilesArgs::from_value(&arguments) {
                Ok(args) => list_files(&self.workspace_root, args),
                Err(error) => invalid_arguments("list_files", error),
            },
            ToolName::SearchText => match SearchTextArgs::from_value(&arguments) {
                Ok(args) => search_text(&self.workspace_root, args),
                Err(error) => invalid_arguments("search_text", error),
            },
            ToolName::ReadFile => match ReadFileArgs::from_value(&arguments) {
                Ok(args) => read_file(&self.workspace_root, args),
                Err(error) => invalid_arguments("read_file", error),
            },
            ToolName::InspectGit => match string_arg(&arguments, "scope") {
                Ok("status") => inspect_git_status(&self.workspace_root),
                Ok(_) => ToolObservation::failed(
                    ToolName::InspectGit.as_str(),
                    None,
                    ToolErrorKind::UnsupportedArgument,
                    "unsupported inspect_git scope",
                ),
                Err(error) => invalid_arguments("inspect_git", error),
            },
            ToolName::ApplyPatch
            | ToolName::WebSearch
            | ToolName::WebFetch
            | ToolName::RunCommand
            | ToolName::AddProvider
            | ToolName::UpdateConfig => ToolObservation::failed(
                spec.name,
                None,
                ToolErrorKind::UnsupportedTool,
                "tool is not executable by the current runtime",
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

    pub fn post_edit_diagnostics(
        &self,
        request: PostEditDiagnosticRequest,
    ) -> Option<ToolObservation> {
        let mut observation = run_post_edit_diagnostics(&self.workspace_root, &request)?;
        let artifact_name = format!(
            "{}_{}_post_edit_diagnostics.txt",
            sanitize_artifact_component(&request.run_id),
            sanitize_artifact_component(&request.turn_id)
        );
        if let Err(error) = observation.apply_output_policy(
            self.limits.preview_line_limit,
            &self.artifact_root,
            &artifact_name,
        ) {
            return Some(ToolObservation::failed(
                observation.tool_name,
                observation.target_raw,
                ToolErrorKind::IoError,
                format!("diagnostic artifact could not be written: {error}"),
            ));
        }
        Some(observation)
    }

    pub fn execute_approved_command(
        &self,
        run_id: &str,
        turn_id: &str,
        arguments: Value,
        max_timeout_ms: u32,
    ) -> ToolObservation {
        let mut observation = execute_approved_command(
            &self.workspace_root,
            ApprovedCommand {
                arguments,
                max_timeout_ms,
            },
        );
        let artifact_name = format!(
            "{}_{}_run_command.txt",
            sanitize_artifact_component(run_id),
            sanitize_artifact_component(turn_id)
        );
        if let Err(error) = observation.apply_output_policy(
            self.limits.preview_line_limit,
            &self.artifact_root,
            &artifact_name,
        ) {
            return ToolObservation::failed(
                observation.tool_name,
                observation.target_raw,
                ToolErrorKind::IoError,
                format!("command artifact could not be written: {error}"),
            );
        }
        observation
    }

    pub fn execute_approved_web(
        &self,
        run_id: &str,
        turn_id: &str,
        tool_name: String,
        arguments: Value,
        web_enabled: bool,
        timeout_ms: u32,
    ) -> ToolObservation {
        let mut observation = execute_approved_web(ApprovedWebRequest {
            tool_name,
            arguments,
            web_enabled,
            timeout_ms,
        });
        let artifact_name = format!(
            "{}_{}_web_network.txt",
            sanitize_artifact_component(run_id),
            sanitize_artifact_component(turn_id)
        );
        if let Err(error) = observation.apply_output_policy(
            self.limits.preview_line_limit,
            &self.artifact_root,
            &artifact_name,
        ) {
            return ToolObservation::failed(
                observation.tool_name,
                observation.target_raw,
                ToolErrorKind::IoError,
                format!("web artifact could not be written: {error}"),
            );
        }
        observation
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
