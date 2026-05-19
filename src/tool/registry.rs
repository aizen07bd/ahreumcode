use serde_json::Value;

use crate::llm::Activity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolName {
    ListFiles,
    SearchText,
    ReadFile,
    InspectGit,
    WebSearch,
    WebFetch,
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
            "web_search" => Some(Self::WebSearch),
            "web_fetch" => Some(Self::WebFetch),
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
            Self::WebSearch => &WEB_SEARCH,
            Self::WebFetch => &WEB_FETCH,
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
    WebNetwork,
    ApprovalOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolArgumentSpec {
    pub name: &'static str,
    pub kind: ToolArgumentKind,
    pub default: Option<ToolArgumentDefault>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolArgumentDefault {
    Integer(i64),
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

pub fn redacted_tool_arguments(tool_name: &str, arguments: &Value) -> Value {
    let Some(object) = arguments.as_object() else {
        return redacted_unknown_value(arguments);
    };
    let spec = tool_spec(tool_name);
    let mut fields = serde_json::Map::new();
    let mut unknown_fields = Vec::new();

    for (key, value) in object {
        let argument_spec =
            spec.and_then(|spec| spec.arguments.iter().find(|rule| rule.name == key.as_str()));
        if argument_spec.is_none() {
            unknown_fields.push(Value::String(key.clone()));
        }
        fields.insert(key.clone(), redacted_argument_value(argument_spec, value));
    }

    let mut redacted = serde_json::Map::new();
    redacted.insert("type".to_owned(), Value::String("object".to_owned()));
    redacted.insert("fields".to_owned(), Value::Object(fields));
    if !unknown_fields.is_empty() {
        redacted.insert("unknown_fields".to_owned(), Value::Array(unknown_fields));
    }
    Value::Object(redacted)
}

fn redacted_argument_value(rule: Option<&ToolArgumentSpec>, value: &Value) -> Value {
    match rule.map(|rule| rule.kind) {
        Some(
            ToolArgumentKind::WorkspacePath
            | ToolArgumentKind::ConfigKey
            | ToolArgumentKind::IntegerRange { .. }
            | ToolArgumentKind::StringEnum(_),
        ) => unredacted_contract_value(value),
        Some(ToolArgumentKind::HttpUrl | ToolArgumentKind::NonEmptyString) => {
            redacted_unknown_value(value)
        }
        Some(ToolArgumentKind::NonEmptyStringArray | ToolArgumentKind::Any) | None => {
            redacted_unknown_value(value)
        }
    }
}

fn unredacted_contract_value(value: &Value) -> Value {
    let mut object = redacted_shape(value);
    object.insert("redacted".to_owned(), Value::Bool(false));
    object.insert("value".to_owned(), value.clone());
    Value::Object(object)
}

fn redacted_unknown_value(value: &Value) -> Value {
    let mut object = redacted_shape(value);
    object.insert("redacted".to_owned(), Value::Bool(true));
    Value::Object(object)
}

fn redacted_shape(value: &Value) -> serde_json::Map<String, Value> {
    let mut object = serde_json::Map::new();
    object.insert(
        "type".to_owned(),
        Value::String(
            match value {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            }
            .to_owned(),
        ),
    );
    match value {
        Value::String(value) => {
            object.insert(
                "chars".to_owned(),
                Value::Number(serde_json::Number::from(value.chars().count())),
            );
        }
        Value::Array(values) => {
            object.insert(
                "items".to_owned(),
                Value::Number(serde_json::Number::from(values.len())),
            );
        }
        Value::Object(values) => {
            object.insert(
                "keys".to_owned(),
                Value::Array(values.keys().cloned().map(Value::String).collect()),
            );
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    object
}

pub fn normalize_tool_arguments(tool_name: ToolName, arguments: &Value) -> Result<Value, String> {
    let rules = tool_name.spec().arguments;
    let object = arguments
        .as_object()
        .ok_or_else(|| "arguments must be a JSON object".to_owned())?;

    for key in object.keys() {
        if !rules.iter().any(|rule| rule.name == key) {
            return Err(format!("unknown argument field: {key}"));
        }
    }

    let mut normalized = serde_json::Map::new();
    for rule in rules {
        let value = match object.get(rule.name) {
            Some(value) => normalize_argument_value(rule, value)?,
            None => default_argument_value(rule)
                .ok_or_else(|| format!("missing argument: {}", rule.name))?,
        };
        normalized.insert(rule.name.to_owned(), value);
    }

    Ok(Value::Object(normalized))
}

fn default_argument_value(rule: &ToolArgumentSpec) -> Option<Value> {
    match rule.default {
        Some(ToolArgumentDefault::Integer(value)) => Some(Value::Number(value.into())),
        None => None,
    }
}

fn normalize_argument_value(rule: &ToolArgumentSpec, value: &Value) -> Result<Value, String> {
    match rule.kind {
        ToolArgumentKind::NonEmptyString => {
            normalize_string(rule, value, validate_non_empty_plain_string)
        }
        ToolArgumentKind::WorkspacePath => {
            normalize_string(rule, value, validate_workspace_relative_path)
        }
        ToolArgumentKind::ConfigKey => normalize_string(rule, value, validate_config_key),
        ToolArgumentKind::HttpUrl => normalize_string(rule, value, validate_http_url),
        ToolArgumentKind::IntegerRange { min, max } => {
            let Some(actual) = value.as_i64() else {
                return Err(format!("invalid argument type or value: {}", rule.name));
            };
            if !(min..=max).contains(&actual) {
                return Err(format!("invalid argument type or value: {}", rule.name));
            }
            Ok(Value::Number(actual.into()))
        }
        ToolArgumentKind::NonEmptyStringArray => {
            let Some(items) = value.as_array() else {
                return Err(format!("invalid argument type or value: {}", rule.name));
            };
            if items.is_empty() {
                return Err(format!("invalid argument type or value: {}", rule.name));
            }
            let mut normalized = Vec::with_capacity(items.len());
            for item in items {
                let Some(value) = item.as_str() else {
                    return Err(format!("invalid argument type or value: {}", rule.name));
                };
                if !validate_non_empty_plain_string(value) {
                    return Err(format!("invalid argument type or value: {}", rule.name));
                }
                normalized.push(Value::String(value.to_owned()));
            }
            Ok(Value::Array(normalized))
        }
        ToolArgumentKind::StringEnum(values) => {
            let Some(actual) = value.as_str() else {
                return Err(format!("invalid argument type or value: {}", rule.name));
            };
            if !values.contains(&actual) {
                return Err(format!("invalid argument type or value: {}", rule.name));
            }
            Ok(Value::String(actual.to_owned()))
        }
        ToolArgumentKind::Any => Ok(value.clone()),
    }
}

fn normalize_string(
    rule: &ToolArgumentSpec,
    value: &Value,
    validate: fn(&str) -> bool,
) -> Result<Value, String> {
    let Some(actual) = value.as_str() else {
        return Err(format!("invalid argument type or value: {}", rule.name));
    };
    if !validate(actual) {
        return Err(format!("invalid argument type or value: {}", rule.name));
    }
    Ok(Value::String(actual.to_owned()))
}

fn validate_non_empty_plain_string(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !contains_control_char(value)
}

fn validate_workspace_relative_path(value: &str) -> bool {
    validate_non_empty_plain_string(value)
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
        default: None,
    },
    ToolArgumentSpec {
        name: "max_depth",
        kind: ToolArgumentKind::IntegerRange { min: 1, max: 5 },
        default: Some(ToolArgumentDefault::Integer(2)),
    },
    ToolArgumentSpec {
        name: "max_entries",
        kind: ToolArgumentKind::IntegerRange { min: 1, max: 500 },
        default: Some(ToolArgumentDefault::Integer(100)),
    },
];

const SEARCH_TEXT_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "path",
        kind: ToolArgumentKind::WorkspacePath,
        default: None,
    },
    ToolArgumentSpec {
        name: "query",
        kind: ToolArgumentKind::NonEmptyString,
        default: None,
    },
    ToolArgumentSpec {
        name: "max_results",
        kind: ToolArgumentKind::IntegerRange { min: 1, max: 200 },
        default: Some(ToolArgumentDefault::Integer(20)),
    },
];

const READ_FILE_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "path",
        kind: ToolArgumentKind::WorkspacePath,
        default: None,
    },
    ToolArgumentSpec {
        name: "start_line",
        kind: ToolArgumentKind::IntegerRange {
            min: 1,
            max: i64::MAX,
        },
        default: Some(ToolArgumentDefault::Integer(1)),
    },
    ToolArgumentSpec {
        name: "max_lines",
        kind: ToolArgumentKind::IntegerRange { min: 1, max: 300 },
        default: Some(ToolArgumentDefault::Integer(120)),
    },
];

const INSPECT_GIT_ARGS: &[ToolArgumentSpec] = &[ToolArgumentSpec {
    name: "scope",
    kind: ToolArgumentKind::StringEnum(STATUS_SCOPE),
    default: None,
}];

const WEB_SEARCH_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "query",
        kind: ToolArgumentKind::NonEmptyString,
        default: None,
    },
    ToolArgumentSpec {
        name: "max_results",
        kind: ToolArgumentKind::IntegerRange { min: 1, max: 20 },
        default: Some(ToolArgumentDefault::Integer(5)),
    },
];

const WEB_FETCH_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "url",
        kind: ToolArgumentKind::HttpUrl,
        default: None,
    },
    ToolArgumentSpec {
        name: "max_bytes",
        kind: ToolArgumentKind::IntegerRange {
            min: 1,
            max: 200_000,
        },
        default: Some(ToolArgumentDefault::Integer(50_000)),
    },
];

const APPLY_PATCH_ARGS: &[ToolArgumentSpec] = &[ToolArgumentSpec {
    name: "payload_id",
    kind: ToolArgumentKind::NonEmptyString,
    default: None,
}];

const RUN_COMMAND_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "argv",
        kind: ToolArgumentKind::NonEmptyStringArray,
        default: None,
    },
    ToolArgumentSpec {
        name: "cwd",
        kind: ToolArgumentKind::WorkspacePath,
        default: None,
    },
    ToolArgumentSpec {
        name: "timeout_ms",
        kind: ToolArgumentKind::IntegerRange {
            min: 1,
            max: i64::MAX,
        },
        default: None,
    },
];

const ADD_PROVIDER_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "provider_id",
        kind: ToolArgumentKind::ConfigKey,
        default: None,
    },
    ToolArgumentSpec {
        name: "base_url",
        kind: ToolArgumentKind::HttpUrl,
        default: None,
    },
    ToolArgumentSpec {
        name: "model",
        kind: ToolArgumentKind::NonEmptyString,
        default: None,
    },
    ToolArgumentSpec {
        name: "context_tokens",
        kind: ToolArgumentKind::IntegerRange {
            min: 1,
            max: i64::MAX,
        },
        default: None,
    },
];

const UPDATE_CONFIG_ARGS: &[ToolArgumentSpec] = &[
    ToolArgumentSpec {
        name: "key_path",
        kind: ToolArgumentKind::ConfigKey,
        default: None,
    },
    ToolArgumentSpec {
        name: "value",
        kind: ToolArgumentKind::Any,
        default: None,
    },
];

const LIST_FILES: ToolSpec = ToolSpec {
    name: "list_files",
    activity: Activity::Explore,
    arguments: LIST_FILES_ARGS,
    schema_line: r#"list_files arguments: {"path":"src","max_depth":2,"max_entries":100}"#,
    permission: ToolPermission::Allow,
    runtime: ToolRuntimeSupport::Explore,
};

const SEARCH_TEXT: ToolSpec = ToolSpec {
    name: "search_text",
    activity: Activity::Explore,
    arguments: SEARCH_TEXT_ARGS,
    schema_line: r#"search_text arguments: {"path":"src","query":"RuntimeDecision","max_results":20}"#,
    permission: ToolPermission::Allow,
    runtime: ToolRuntimeSupport::Explore,
};

const READ_FILE: ToolSpec = ToolSpec {
    name: "read_file",
    activity: Activity::Explore,
    arguments: READ_FILE_ARGS,
    schema_line: r#"read_file arguments: {"path":"src/main.rs","start_line":1,"max_lines":120}"#,
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

const WEB_SEARCH: ToolSpec = ToolSpec {
    name: "web_search",
    activity: Activity::Explore,
    arguments: WEB_SEARCH_ARGS,
    schema_line: r#"web_search arguments: {"query":"current release notes","max_results":5}"#,
    permission: ToolPermission::Ask,
    runtime: ToolRuntimeSupport::WebNetwork,
};

const WEB_FETCH: ToolSpec = ToolSpec {
    name: "web_fetch",
    activity: Activity::Explore,
    arguments: WEB_FETCH_ARGS,
    schema_line: r#"web_fetch arguments: {"url":"https://example.com/page","max_bytes":50000}"#,
    permission: ToolPermission::Ask,
    runtime: ToolRuntimeSupport::WebNetwork,
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
    schema_line: r#"run_command arguments: {"argv":["cargo","check"],"cwd":".","timeout_ms":30000}"#,
    permission: ToolPermission::Ask,
    runtime: ToolRuntimeSupport::ApprovalOnly,
};

const ADD_PROVIDER: ToolSpec = ToolSpec {
    name: "add_provider",
    activity: Activity::Configure,
    arguments: ADD_PROVIDER_ARGS,
    schema_line: r#"add_provider arguments: {"provider_id":"local","base_url":"http://127.0.0.1:1234/v1","model":"model-name","context_tokens":32000}"#,
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
    WEB_SEARCH,
    WEB_FETCH,
    APPLY_PATCH,
    RUN_COMMAND,
    ADD_PROVIDER,
    UPDATE_CONFIG,
];

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::redacted_tool_arguments;

    #[test]
    fn redacted_arguments_keep_workspace_path_and_bounds() {
        let redacted = redacted_tool_arguments(
            "read_file",
            &json!({"path":"src/main.rs","start_line":1,"max_lines":120}),
        );

        assert_eq!(
            redacted["fields"]["path"],
            json!({"type":"string","chars":11,"redacted":false,"value":"src/main.rs"})
        );
        assert_eq!(
            redacted["fields"]["start_line"],
            json!({"type":"number","redacted":false,"value":1})
        );
    }

    #[test]
    fn redacted_arguments_hide_free_text_query() {
        let redacted = redacted_tool_arguments(
            "search_text",
            &json!({"path":"src","query":"secret token value","max_results":20}),
        );

        assert_eq!(
            redacted["fields"]["query"],
            json!({"type":"string","chars":18,"redacted":true})
        );
        assert_eq!(redacted["fields"]["query"].get("value"), None);
    }

    #[test]
    fn redacted_arguments_mark_unknown_fields_without_raw_value() {
        let redacted =
            redacted_tool_arguments("read_file", &json!({"path":"src/main.rs","extra":"secret"}));

        assert_eq!(redacted["unknown_fields"], json!(["extra"]));
        assert_eq!(
            redacted["fields"]["extra"],
            json!({"type":"string","chars":6,"redacted":true})
        );
    }
}
