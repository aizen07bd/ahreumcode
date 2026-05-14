mod schema;

pub use schema::{ConfigLoadOutcome, ConfigLoadSource, ConfigWarning, ProviderType, RuntimeConfig};

pub const CONFIG_RELATIVE_PATH: &str = ".ahreumcode/config.toml";
pub const DEFAULT_PROVIDER: &str = "lm-studio";
pub const DEFAULT_PROVIDER_DISPLAY: &str = "LM Studio";
pub const DEFAULT_PROVIDER_TYPE: ProviderType = ProviderType::OpenAiCompatible;
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:1234/v1";
pub const DEFAULT_MODEL: &str = "google/gemma-4-e4b";
pub const DEFAULT_CONTEXT_TOKENS: u32 = 32000;
pub const DEFAULT_MODE: &str = "Crew";
pub const DEFAULT_PERSONA: &str = "off";
pub const DEFAULT_PERSONA_MIN_TERMINAL_WIDTH: u16 = 140;
pub const DEFAULT_MAX_MODEL_TURNS: u16 = 8;
pub const DEFAULT_MAX_TOOL_CALLS: u16 = 8;
pub const DEFAULT_MAX_SAME_TOOL_REPEATS: u16 = 2;
pub const DEFAULT_READ_MAX_LINES: u16 = 300;
pub const DEFAULT_SEARCH_MAX_RESULTS: u16 = 200;
pub const DEFAULT_COMMAND_TIMEOUT_MS: u32 = 30000;
pub const DEFAULT_WEB_ENABLED: bool = true;
