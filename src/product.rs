pub const APP_NAME: &str = "AhreumCode";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const KOREAN_VERSION_LINE: &str = "아름코드 v1.0.0";

pub const DEFAULT_PROVIDER: &str = "lm-studio";
pub const DEFAULT_PROVIDER_DISPLAY: &str = "LM Studio";
pub const DEFAULT_MODEL: &str = "google/gemma-4-e4b";
pub const DEFAULT_MODE: &str = "Crew";
pub const DEFAULT_CONTEXT_STATUS: &str = "ctx 0%/100%";
pub const DEFAULT_TOKEN_STATUS: &str = "tokens 0";
pub const DEFAULT_WEB_STATUS: &str = "web on";
pub const DEFAULT_RUNTIME_STATE: &str = "Ready";

pub const INTRO_PROMPT_PLACEHOLDER: &str = "Ask anything... \"이 프로젝트 구조 분석해줘\"";
pub const INTRO_HEALTH_HINT: &str = "/health";
pub const INTRO_HEALTH_HINT_TEXT: &str = " check local model";
pub const INTRO_COMMAND_HINT: &str = "/";
pub const INTRO_COMMAND_HINT_TEXT: &str = " for commands";

pub const SESSION_SAVED_LABEL: &str = "saved";
pub const GOODBYE_LABEL: &str = "goodbye";
pub const EPILOGUE_TIP_PREFIX: &str = "tip: ";
pub const EPILOGUE_TIP_COMMAND: &str = "`ahreumcode`";
pub const EPILOGUE_TIP_TEXT: &str = "로 다시 시작하거나 ";
pub const EPILOGUE_TIP_SESSIONS_COMMAND: &str = "`ahreumcode sessions`";
pub const EPILOGUE_TIP_SUFFIX: &str = "로 이전 작업을 확인하세요";

pub fn version_label() -> String {
    format!("v{VERSION}")
}
