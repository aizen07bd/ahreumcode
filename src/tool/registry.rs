use std::path::{Component, Path};

use serde_json::Value;

use crate::llm::Activity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolName {
    ListFiles,
    SearchText,
    ReadFile,
    InspectGit,
    ApplyPatch,
    RunCommand,
    AddProvider,
    UpdateConfig,
}

impl ToolName {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "list_files" => Some(Self::ListFiles),
            "search_text" => Some(Self::SearchText),
            "read_file" => Some(Self::ReadFile),
            "inspect_git" => Some(Self::InspectGit),
            "apply_patch" => Some(Self::ApplyPatch),
            "run_command" => Some(Self::RunCommand),
            "add_provider" => Some(Self::AddProvider),
            "update_config" => Some(Self::UpdateConfig),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        self.spec().name
    }

    pub fn spec(self) -> &'static ToolSpec {
        match self {
            Self::ListFiles => &LIST_FILES,
            Self::SearchText => &SEARCH_TEXT,
            Self::ReadFile => &READ_FILE,
            Self::InspectGit => &INSPECT_GIT,
            Self::ApplyPatch => &APPLY_PATCH,
            Self::RunCommand => &RUN_COMMAND,
            Self::AddProvider => &ADD_PROVIDER,
            Self::UpdateConfig => &UPDATE_CONFIG,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub activity: Activity,
    pub arguments: &'static [ToolArgumentSpec],
    pub schema_line: &'static str,
    pub permission: ToolPermission,
    pub runtime: ToolRuntimeSupport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolPermission {
    Allow,
    Ask,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolRuntimeSupport {
    Explore,
    ApprovalOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolArgumentSpec {
    pub name: &'static str,
    pub kind: ToolArgumentKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolArgumentKind {
    NonEmptyString,
    WorkspacePath,
    ConfigKey,
    HttpUrl,
    IntegerRange { min: i64, max: i64 },
    NonEmptyStringArray,
    StringEnum(&'static [&'static str]),
    Any,
}

pub fn tool_spec(name: &str) -> Option<&'static ToolSpec> {
    ToolName::parse(name).map(ToolName::spec)
}

pub fn tool_argument_schema_lines() -> impl Iterator<Item = &'static str> {
    TOOL_SPECS.iter().map(|spec| spec.schema_line)
}

pub fn validate_tool_arguments(tool_name: ToolName, arguments: &Value) -> Result<(), String> {
    let rules = tool_name.spec().arguments;
    let object = arguments
        .as_object()
        .ok_or_else(|| "arguments must be a JSON object".to_owned())?;

    for key in object.keys() {
        if !rules.iter().any(|rule| rule.name == key) {
            return Err(format!("unknown argument field: {key}"));
        }
    }

    for rule in rules {
        let value = object
            .get(rule.name)
            .ok_or_else(|| format!("missing argument: {}", rule.name))?;
        validate_argument_value(rule, value)?;
    }

    Ok(())
}

fn validate_argument_value(rule: &ToolArgumentSpec, value: &Value) -> Result<(), String> {
    let valid = match rule.kind {
        ToolArgumentKind::NonEmptyString => value
            .as_str()
            .map(validate_non_empty_plain_string)
            .unwrap_or(false),
        ToolArgumentKind::WorkspacePath => value
            .as_str()
            .map(validate_workspace_relative_path)
            .unwrap_or(false),
        ToolArgumentKind::ConfigKey => value.as_str().map(validate_config_key).unwrap_or(false),
        ToolArgumentKind::HttpUrl => value.as_str().map(validate_http_url).unwrap_or(false),
        ToolArgumentKind::IntegerRange { min, max } => value
            .as_i64()
            .map(|actual| (min..=max).contains(&actual))
            .unwrap_or(false),
        ToolArgumentKind::NonEmptyStringArray => value
            .as_array()
            .map(|items| {
                !items.is_empty()
                    && items.iter().all(|item| {
                        item.as_str()
                            .map(validate_non_empty_plain_string)
                            .unwrap_or(false)
                    })
            })
            .unwrap_or(false),
        ToolArgumentKind::StringEnum(values) => value
            .as_str()
            .map(|actual| values.contains(&actual))
            .unwrap_or(false),
        ToolArgumentKind::Any => true,
    };

    if valid {
        Ok(())
    } else {
        Err(format!("invalid argument type or value: {}", rule.name))
    }
}

fn validate_non_empty_plain_string(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !contains_control_char(value)
}

fn validate_workspace_relative_path(value: &str) -> bool {
    if !validate_non_empty_plain_string(value) {
        return false;
    }

    let path = Path::new(value);
    if path.is_absolute() {
        return false;
    }

    path.components().all(|component| {
        !matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    })
}

fn validate_config_key(value: &str) -> bool {
    validate_non_empty_plain_string(value)
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
}

fn validate_http_url(value: &str) -> bool {
    validate_non_empty_plain_string(value)
        && (value.starts_with("http://") || value.starts_with("https://"))
}

fn contains_control_char(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
}

const STATUS_SCOPE: &[&str] = &["status"];

const LIST_FILES_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "path",
        kind: ToolArgumentKind::WorkspacePath,
    },
    ToolArgumentSpec {
        name: "max_depth",
        kind: ToolArgumentKind::IntegerRange { min: 1, max: 5 },
    },
    ToolArgumentSpec {
        name: "max_entries",
        kind: ToolArgumentKind::IntegerRange { min: 1, max: 500 },
    },
];

const SEARCH_TEXT_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "path",
        kind: ToolArgumentKind::WorkspacePath,
    },
    ToolArgumentSpec {
        name: "query",
        kind: ToolArgumentKind::NonEmptyString,
    },
    ToolArgumentSpec {
        name: "max_results",
        kind: ToolArgumentKind::IntegerRange { min: 1, max: 200 },
    },
];

const READ_FILE_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "path",
        kind: ToolArgumentKind::WorkspacePath,
    },
    ToolArgumentSpec {
        name: "start_line",
        kind: ToolArgumentKind::IntegerRange {
            min: 1,
            max: i64::MAX,
        },
    },
    ToolArgumentSpec {
        name: "max_lines",
        kind: ToolArgumentKind::IntegerRange { min: 1, max: 300 },
    },
];

const INSPECT_GIT_ARGS: &[ToolArgumentSpec] = &[ToolArgumentSpec {
    name: "scope",
    kind: ToolArgumentKind::StringEnum(STATUS_SCOPE),
}];

const APPLY_PATCH_ARGS: &[ToolArgumentSpec] = &[ToolArgumentSpec {
    name: "payload_id",
    kind: ToolArgumentKind::NonEmptyString,
}];

const RUN_COMMAND_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "argv",
        kind: ToolArgumentKind::NonEmptyStringArray,
    },
    ToolArgumentSpec {
        name: "cwd",
        kind: ToolArgumentKind::WorkspacePath,
    },
    ToolArgumentSpec {
        name: "timeout_ms",
        kind: ToolArgumentKind::IntegerRange {
            min: 1,
            max: i64::MAX,
        },
    },
];

const ADD_PROVIDER_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "provider_id",
        kind: ToolArgumentKind::ConfigKey,
    },
    ToolArgumentSpec {
        name: "base_url",
        kind: ToolArgumentKind::HttpUrl,
    },
    ToolArgumentSpec {
        name: "model",
        kind: ToolArgumentKind::NonEmptyString,
    },
    ToolArgumentSpec {
        name: "context_tokens",
        kind: ToolArgumentKind::IntegerRange {
            min: 1,
            max: i64::MAX,
        },
    },
];

const UPDATE_CONFIG_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "key_path",
        kind: ToolArgumentKind::ConfigKey,
    },
    ToolArgumentSpec {
        name: "value",
        kind: ToolArgumentKind::Any,
    },
];

const LIST_FILES: ToolSpec = ToolSpec {
    name: "list_files",
    activity: Activity::Explore,
    arguments: LIST_FILES_ARGS,
    schema_line: r#"list_files arguments: {"path":"workspace-relative path","max_depth":"integer 1..5","max_entries":"integer 1..500"}"#,
    permission: ToolPermission::Allow,
    runtime: ToolRuntimeSupport::Explore,
};

const SEARCH_TEXT: ToolSpec = ToolSpec {
    name: "search_text",
    activity: Activity::Explore,
    arguments: SEARCH_TEXT_ARGS,
    schema_line: r#"search_text arguments: {"path":"workspace-relative path","query":"non-empty string","max_results":"integer 1..200"}"#,
    permission: ToolPermission::Allow,
    runtime: ToolRuntimeSupport::Explore,
};

const READ_FILE: ToolSpec = ToolSpec {
    name: "read_file",
    activity: Activity::Explore,
    arguments: READ_FILE_ARGS,
    schema_line: r#"read_file arguments: {"path":"workspace-relative path","start_line":"integer >=1","max_lines":"integer 1..300"}"#,
    permission: ToolPermission::Allow,
    runtime: ToolRuntimeSupport::Explore,
};

const INSPECT_GIT: ToolSpec = ToolSpec {
    name: "inspect_git",
    activity: Activity::Explore,
    arguments: INSPECT_GIT_ARGS,
    schema_line: r#"inspect_git arguments: {"scope":"status"}"#,
    permission: ToolPermission::Allow,
    runtime: ToolRuntimeSupport::Explore,
};

const APPLY_PATCH: ToolSpec = ToolSpec {
    name: "apply_patch",
    activity: Activity::Change,
    arguments: APPLY_PATCH_ARGS,
    schema_line: r#"apply_patch arguments: {"payload_id":"non-empty string"}"#,
    permission: ToolPermission::Ask,
    runtime: ToolRuntimeSupport::ApprovalOnly,
};

const RUN_COMMAND: ToolSpec = ToolSpec {
    name: "run_command",
    activity: Activity::Execute,
    arguments: RUN_COMMAND_ARGS,
    schema_line: r#"run_command arguments: {"argv":"non-empty string array","cwd":"workspace-relative path","timeout_ms":"integer >=1"}"#,
    permission: ToolPermission::Ask,
    runtime: ToolRuntimeSupport::ApprovalOnly,
};

const ADD_PROVIDER: ToolSpec = ToolSpec {
    name: "add_provider",
    activity: Activity::Configure,
    arguments: ADD_PROVIDER_ARGS,
    schema_line: r#"add_provider arguments: {"provider_id":"config key","base_url":"http:// or https:// URL","model":"non-empty string","context_tokens":"integer >=1"}"#,
    permission: ToolPermission::Ask,
    runtime: ToolRuntimeSupport::ApprovalOnly,
};

const UPDATE_CONFIG: ToolSpec = ToolSpec {
    name: "update_config",
    activity: Activity::Configure,
    arguments: UPDATE_CONFIG_ARGS,
    schema_line: r#"update_config arguments: {"key_path":"dot-separated config key","value":"JSON value"}"#,
    permission: ToolPermission::Ask,
    runtime: ToolRuntimeSupport::ApprovalOnly,
};

const TOOL_SPECS: &[ToolSpec] = &[
    LIST_FILES,
    SEARCH_TEXT,
    READ_FILE,
    INSPECT_GIT,
    APPLY_PATCH,
    RUN_COMMAND,
    ADD_PROVIDER,
    UPDATE_CONFIG,
];
