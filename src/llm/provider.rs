use crate::config::{ProviderType, RuntimeConfig};

use super::history::LlmMessage;
use super::lm_studio::LmStudioProvider;

pub struct LlmProviderFactory;

impl LlmProviderFactory {
    pub fn from_config(config: &RuntimeConfig) -> LlmProviderClient {
        match config.provider.provider_type {
            ProviderType::OpenAiCompatible => {
                LlmProviderClient::LmStudio(LmStudioProvider::from_config(config))
            }
        }
    }
}

pub enum LlmProviderClient {
    LmStudio(LmStudioProvider),
}

impl LlmProviderClient {
    pub fn health_check(&self) -> LlmHealthReport {
        match self {
            Self::LmStudio(provider) => provider.health_check(),
        }
    }

    pub fn send_chat(&self, request: LlmChatRequest) -> LlmChatReport {
        match self {
            Self::LmStudio(provider) => provider.send_chat(request),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmChatRequest {
    pub messages: Vec<LlmMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmChatReport {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub chat_url: String,
    pub latency_ms: u128,
    pub status: LlmChatStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LlmChatStatus {
    Succeeded { answer: String },
    Failed(LlmChatFailure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmChatFailure {
    pub kind: ChatFailureKind,
    pub message: String,
    pub http_status: Option<u16>,
}

impl LlmChatFailure {
    pub fn new(kind: ChatFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status: None,
        }
    }

    pub fn with_http_status(
        kind: ChatFailureKind,
        status: u16,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status: Some(status),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatFailureKind {
    ConnectionFailed,
    Timeout,
    EndpointFailure,
    InvalidEndpoint,
    InvalidResponse,
}

impl ChatFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConnectionFailed => "connection_failed",
            Self::Timeout => "timeout",
            Self::EndpointFailure => "endpoint_failure",
            Self::InvalidEndpoint => "invalid_endpoint",
            Self::InvalidResponse => "invalid_response",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmHealthReport {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub models_url: String,
    pub latency_ms: u128,
    pub status: LlmHealthStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LlmHealthStatus {
    Succeeded { available_models: usize },
    Failed(LlmHealthFailure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmHealthFailure {
    pub kind: HealthFailureKind,
    pub message: String,
    pub http_status: Option<u16>,
}

impl LlmHealthFailure {
    pub fn new(kind: HealthFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status: None,
        }
    }

    pub fn with_http_status(
        kind: HealthFailureKind,
        status: u16,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status: Some(status),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthFailureKind {
    ConnectionFailed,
    Timeout,
    ModelMissing,
    EndpointFailure,
    InvalidEndpoint,
    InvalidResponse,
}

impl HealthFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConnectionFailed => "connection_failed",
            Self::Timeout => "timeout",
            Self::ModelMissing => "model_missing",
            Self::EndpointFailure => "endpoint_failure",
            Self::InvalidEndpoint => "invalid_endpoint",
            Self::InvalidResponse => "invalid_response",
        }
    }
}
