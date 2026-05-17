use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{
    CONFIG_RELATIVE_PATH, DEFAULT_BASE_URL, DEFAULT_COMMAND_TIMEOUT_MS, DEFAULT_CONTEXT_TOKENS,
    DEFAULT_MAX_MODEL_TURNS, DEFAULT_MAX_SAME_TOOL_REPEATS, DEFAULT_MAX_TOOL_CALLS, DEFAULT_MODE,
    DEFAULT_MODEL, DEFAULT_PERSONA, DEFAULT_PERSONA_MIN_TERMINAL_WIDTH, DEFAULT_PROVIDER,
    DEFAULT_PROVIDER_TYPE, DEFAULT_READ_MAX_LINES, DEFAULT_SEARCH_MAX_RESULTS, DEFAULT_WEB_ENABLED,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderType {
    OpenAiCompatible,
}

impl ProviderType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
        }
    }
}

impl<'de> Deserialize<'de> for ProviderType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "openai-compatible" => Ok(Self::OpenAiCompatible),
            _ => Err(serde::de::Error::custom(format!(
                "unsupported provider type: {value}"
            ))),
        }
    }
}

#[derive(Clone)]
pub struct RuntimeConfig {
    pub config_path: PathBuf,
    pub provider: ProviderConfig,
    pub workspace: WorkspaceConfig,
    pub mode: ModeConfig,
    pub persona: PersonaConfig,
    pub limits: RuntimeLimits,
    pub web: WebConfig,
}

#[derive(Clone)]
pub struct ProviderConfig {
    pub active: String,
    pub provider_type: ProviderType,
    pub base_url: String,
    pub model: String,
    pub context_tokens: u32,
    pub api_key_env: Option<String>,
}

#[derive(Clone)]
pub struct WorkspaceConfig {
    pub root: String,
}

#[derive(Clone)]
pub struct ModeConfig {
    pub default: String,
}

#[derive(Clone)]
pub struct PersonaConfig {
    pub default: String,
    pub min_terminal_width: u16,
}

#[derive(Clone)]
pub struct RuntimeLimits {
    pub max_model_turns: u16,
    pub max_tool_calls: u16,
    pub max_same_tool_repeats: u16,
    pub read_max_lines: u16,
    pub search_max_results: u16,
    pub command_timeout_ms: u32,
}

#[derive(Clone)]
pub struct WebConfig {
    pub enabled: bool,
}

pub struct ConfigLoadOutcome {
    pub config: RuntimeConfig,
    pub source: ConfigLoadSource,
    pub warning: Option<ConfigWarning>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ConfigLoadSource {
    ExistingFile,
    DefaultCreated,
    DefaultApplied,
}

impl ConfigLoadSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExistingFile => "existing_file",
            Self::DefaultCreated => "default_created",
            Self::DefaultApplied => "default_applied",
        }
    }
}

pub struct ConfigWarning {
    pub message: String,
}

#[derive(Debug)]
pub enum RuntimeConfigError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    Invalid {
        path: PathBuf,
        message: String,
    },
}

impl RuntimeConfigError {
    pub fn message(&self) -> String {
        match self {
            Self::Read { path, source } => {
                format!("failed to read {}: {source}", path.display())
            }
            Self::Parse { path, source } => {
                format!("failed to parse {}: {source}", path.display())
            }
            Self::Invalid { path, message } => {
                format!("invalid {}: {message}", path.display())
            }
        }
    }
}

impl fmt::Display for RuntimeConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl RuntimeConfig {
    pub fn load(project_root: &Path) -> Result<ConfigLoadOutcome, RuntimeConfigError> {
        let config_path = project_root.join(CONFIG_RELATIVE_PATH);
        if config_path.exists() {
            let raw =
                fs::read_to_string(&config_path).map_err(|source| RuntimeConfigError::Read {
                    path: config_path.clone(),
                    source,
                })?;
            let config = parse_config_toml(&raw, config_path.clone())?;
            return Ok(ConfigLoadOutcome {
                config,
                source: ConfigLoadSource::ExistingFile,
                warning: None,
            });
        }

        let config = Self::default_local(config_path.clone());
        let warning = write_default_config(&config_path)
            .err()
            .map(|source| ConfigWarning {
                message: format!(
                    "default config applied, but {} was not written: {source}",
                    config_path.display()
                ),
            });
        let source = if warning.is_some() {
            ConfigLoadSource::DefaultApplied
        } else {
            ConfigLoadSource::DefaultCreated
        };

        Ok(ConfigLoadOutcome {
            config,
            source,
            warning,
        })
    }

    pub fn default_local(config_path: PathBuf) -> Self {
        Self {
            config_path,
            provider: ProviderConfig {
                active: DEFAULT_PROVIDER.to_owned(),
                provider_type: DEFAULT_PROVIDER_TYPE,
                base_url: DEFAULT_BASE_URL.to_owned(),
                model: DEFAULT_MODEL.to_owned(),
                context_tokens: DEFAULT_CONTEXT_TOKENS,
                api_key_env: None,
            },
            workspace: WorkspaceConfig {
                root: ".".to_owned(),
            },
            mode: ModeConfig {
                default: DEFAULT_MODE.to_owned(),
            },
            persona: PersonaConfig {
                default: DEFAULT_PERSONA.to_owned(),
                min_terminal_width: DEFAULT_PERSONA_MIN_TERMINAL_WIDTH,
            },
            limits: RuntimeLimits {
                max_model_turns: DEFAULT_MAX_MODEL_TURNS,
                max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
                max_same_tool_repeats: DEFAULT_MAX_SAME_TOOL_REPEATS,
                read_max_lines: DEFAULT_READ_MAX_LINES,
                search_max_results: DEFAULT_SEARCH_MAX_RESULTS,
                command_timeout_ms: DEFAULT_COMMAND_TIMEOUT_MS,
            },
            web: WebConfig {
                enabled: DEFAULT_WEB_ENABLED,
            },
        }
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    provider: ProviderSelectionFile,
    providers: BTreeMap<String, ProviderFile>,
    workspace: WorkspaceFile,
    mode: ModeFile,
    persona: PersonaFile,
    limits: LimitsFile,
    web: WebFile,
}

#[derive(Deserialize)]
struct ProviderSelectionFile {
    active: String,
}

#[derive(Deserialize)]
struct ProviderFile {
    #[serde(rename = "type")]
    provider_type: ProviderType,
    base_url: String,
    model: String,
    context_tokens: u32,
    api_key_env: String,
}

#[derive(Deserialize)]
struct WorkspaceFile {
    root: String,
}

#[derive(Deserialize)]
struct ModeFile {
    default: String,
}

#[derive(Deserialize)]
struct PersonaFile {
    default: String,
    min_terminal_width: u16,
}

#[derive(Deserialize)]
struct LimitsFile {
    max_model_turns: u16,
    max_tool_calls: u16,
    max_same_tool_repeats: u16,
    read_max_lines: u16,
    search_max_results: u16,
    command_timeout_ms: u32,
}

#[derive(Deserialize)]
struct WebFile {
    enabled: bool,
}

fn parse_config_toml(raw: &str, config_path: PathBuf) -> Result<RuntimeConfig, RuntimeConfigError> {
    let file = toml::from_str::<ConfigFile>(raw).map_err(|source| RuntimeConfigError::Parse {
        path: config_path.clone(),
        source,
    })?;
    validate_config_file(file, config_path)
}

fn validate_config_file(
    file: ConfigFile,
    config_path: PathBuf,
) -> Result<RuntimeConfig, RuntimeConfigError> {
    let active = trim_required(&file.provider.active, "provider.active", &config_path)?;
    let provider = file
        .providers
        .get(active)
        .ok_or_else(|| RuntimeConfigError::Invalid {
            path: config_path.clone(),
            message: format!("provider.active references missing provider '{active}'"),
        })?;
    let base_url = trim_required(
        &provider.base_url,
        "providers.<active>.base_url",
        &config_path,
    )?;
    let model = trim_required(&provider.model, "providers.<active>.model", &config_path)?;
    let workspace_root = trim_required(&file.workspace.root, "workspace.root", &config_path)?;
    let mode = trim_required(&file.mode.default, "mode.default", &config_path)?;
    validate_mode(mode, &config_path)?;
    let persona_default = trim_required(&file.persona.default, "persona.default", &config_path)?;
    validate_persona_default(persona_default, &config_path)?;
    if provider.context_tokens == 0 {
        return Err(RuntimeConfigError::Invalid {
            path: config_path,
            message: "providers.<active>.context_tokens must be greater than 0".to_owned(),
        });
    }

    Ok(RuntimeConfig {
        config_path,
        provider: ProviderConfig {
            active: active.to_owned(),
            provider_type: provider.provider_type,
            base_url: base_url.to_owned(),
            model: model.to_owned(),
            context_tokens: provider.context_tokens,
            api_key_env: optional_trimmed(&provider.api_key_env),
        },
        workspace: WorkspaceConfig {
            root: workspace_root.to_owned(),
        },
        mode: ModeConfig {
            default: mode.to_owned(),
        },
        persona: PersonaConfig {
            default: persona_default.to_owned(),
            min_terminal_width: file.persona.min_terminal_width,
        },
        limits: RuntimeLimits {
            max_model_turns: file.limits.max_model_turns,
            max_tool_calls: file.limits.max_tool_calls,
            max_same_tool_repeats: file.limits.max_same_tool_repeats,
            read_max_lines: file.limits.read_max_lines,
            search_max_results: file.limits.search_max_results,
            command_timeout_ms: file.limits.command_timeout_ms,
        },
        web: WebConfig {
            enabled: file.web.enabled,
        },
    })
}

fn trim_required<'a>(
    value: &'a str,
    field: &str,
    config_path: &Path,
) -> Result<&'a str, RuntimeConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(RuntimeConfigError::Invalid {
            path: config_path.to_path_buf(),
            message: format!("{field} must not be empty"),
        });
    }
    Ok(trimmed)
}

fn validate_mode(mode: &str, config_path: &Path) -> Result<(), RuntimeConfigError> {
    match mode {
        "Guide" | "Crew" | "Pilot" => Ok(()),
        _ => Err(RuntimeConfigError::Invalid {
            path: config_path.to_path_buf(),
            message: format!("mode.default must be Guide, Crew, or Pilot, got '{mode}'"),
        }),
    }
}

fn validate_persona_default(value: &str, config_path: &Path) -> Result<(), RuntimeConfigError> {
    match value {
        "off" => Ok(()),
        _ => Err(RuntimeConfigError::Invalid {
            path: config_path.to_path_buf(),
            message: format!("persona.default must be off, got '{value}'"),
        }),
    }
}

fn optional_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn write_default_config(config_path: &Path) -> io::Result<()> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(config_path, default_config_toml())
}

fn default_config_toml() -> String {
    format!(
        r#"[provider]
active = "{provider}"

[providers.{provider}]
type = "{provider_type}"
base_url = "{base_url}"
model = "{model}"
context_tokens = {context_tokens}
api_key_env = ""

[workspace]
root = "."

[mode]
default = "{mode}"

[persona]
default = "{persona}"
min_terminal_width = {persona_width}

[limits]
max_model_turns = {max_model_turns}
max_tool_calls = {max_tool_calls}
max_same_tool_repeats = {max_same_tool_repeats}
read_max_lines = {read_max_lines}
search_max_results = {search_max_results}
command_timeout_ms = {command_timeout_ms}

[web]
enabled = {web_enabled}
"#,
        provider = DEFAULT_PROVIDER,
        provider_type = DEFAULT_PROVIDER_TYPE.as_str(),
        base_url = DEFAULT_BASE_URL,
        model = DEFAULT_MODEL,
        context_tokens = DEFAULT_CONTEXT_TOKENS,
        mode = DEFAULT_MODE,
        persona = DEFAULT_PERSONA,
        persona_width = DEFAULT_PERSONA_MIN_TERMINAL_WIDTH,
        max_model_turns = DEFAULT_MAX_MODEL_TURNS,
        max_tool_calls = DEFAULT_MAX_TOOL_CALLS,
        max_same_tool_repeats = DEFAULT_MAX_SAME_TOOL_REPEATS,
        read_max_lines = DEFAULT_READ_MAX_LINES,
        search_max_results = DEFAULT_SEARCH_MAX_RESULTS,
        command_timeout_ms = DEFAULT_COMMAND_TIMEOUT_MS,
        web_enabled = DEFAULT_WEB_ENABLED,
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        parse_config_toml, RuntimeConfig, DEFAULT_BASE_URL, DEFAULT_CONTEXT_TOKENS, DEFAULT_MODE,
        DEFAULT_MODEL, DEFAULT_PROVIDER,
    };

    #[test]
    fn default_local_uses_lm_studio_values() {
        let config = RuntimeConfig::default_local(PathBuf::from(".ahreumcode/config.toml"));

        assert_eq!(config.provider.active, DEFAULT_PROVIDER);
        assert_eq!(config.provider.base_url, DEFAULT_BASE_URL);
        assert_eq!(config.provider.model, DEFAULT_MODEL);
        assert_eq!(config.provider.context_tokens, DEFAULT_CONTEXT_TOKENS);
        assert_eq!(config.mode.default, DEFAULT_MODE);
    }

    #[test]
    fn parser_rejects_missing_active_provider() {
        let result = parse_config_toml(
            &format!(
                r#"
[provider]
active = "missing"

[providers.{DEFAULT_PROVIDER}]
type = "openai-compatible"
base_url = "{DEFAULT_BASE_URL}"
model = "{DEFAULT_MODEL}"
context_tokens = {DEFAULT_CONTEXT_TOKENS}
api_key_env = ""

[workspace]
root = "."

[mode]
default = "{DEFAULT_MODE}"

[persona]
default = "off"
min_terminal_width = 140

[limits]
max_model_turns = 8
max_tool_calls = 8
max_same_tool_repeats = 2
read_max_lines = 300
search_max_results = 200
command_timeout_ms = 30000

[web]
enabled = true
"#,
            ),
            PathBuf::from("config.toml"),
        );
        let Err(error) = result else {
            panic!("missing active provider must be invalid");
        };

        assert!(error.message().contains("missing provider"));
    }
}
