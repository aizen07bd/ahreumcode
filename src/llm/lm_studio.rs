use std::error::Error as StdError;
use std::io;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use crate::config::RuntimeConfig;

use super::history::{LlmMessage, LlmMessageRole};
use super::provider::{
    ChatFailureKind, HealthFailureKind, LlmChatFailure, LlmChatReport, LlmChatRequest,
    LlmChatStatus, LlmHealthFailure, LlmHealthReport, LlmHealthStatus,
};

pub struct LmStudioProvider {
    provider: String,
    base_url: String,
    model: String,
    timeout: Duration,
}

impl LmStudioProvider {
    pub fn from_config(config: &RuntimeConfig) -> Self {
        Self {
            provider: config.provider.active.clone(),
            base_url: config.provider.base_url.clone(),
            model: config.provider.model.clone(),
            timeout: Duration::from_millis(u64::from(config.limits.command_timeout_ms)),
        }
    }

    pub fn health_check(&self) -> LlmHealthReport {
        let models_url = build_models_request(&self.base_url);
        let started_at = Instant::now();
        let status = match self.request_models(&models_url) {
            Ok(models) => {
                if models.iter().any(|model| model == &self.model) {
                    LlmHealthStatus::Succeeded {
                        available_models: models.len(),
                    }
                } else {
                    LlmHealthStatus::Failed(LlmHealthFailure::new(
                        HealthFailureKind::ModelMissing,
                        format!(
                            "configured model '{}' is not listed by provider",
                            self.model
                        ),
                    ))
                }
            }
            Err(failure) => LlmHealthStatus::Failed(failure),
        };

        LlmHealthReport {
            provider: self.provider.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            models_url,
            latency_ms: started_at.elapsed().as_millis(),
            status,
        }
    }

    pub fn send_chat(&self, request: LlmChatRequest) -> LlmChatReport {
        let chat_url = build_chat_request(&self.base_url);
        let started_at = Instant::now();
        let status = self
            .request_chat(&chat_url, &request.messages)
            .map(|answer| LlmChatStatus::Succeeded { answer })
            .unwrap_or_else(LlmChatStatus::Failed);

        LlmChatReport {
            provider: self.provider.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            chat_url,
            latency_ms: started_at.elapsed().as_millis(),
            status,
        }
    }

    fn request_models(&self, models_url: &str) -> Result<Vec<String>, LlmHealthFailure> {
        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();
        let response = agent.get(models_url).call().map_err(map_provider_error)?;
        let raw = response.into_string().map_err(|source| {
            LlmHealthFailure::new(
                HealthFailureKind::InvalidResponse,
                format!("failed to read models response: {source}"),
            )
        })?;

        parse_models_response(&raw)
    }

    fn request_chat(
        &self,
        chat_url: &str,
        messages: &[LlmMessage],
    ) -> Result<String, LlmChatFailure> {
        let body = build_chat_request_body(&self.model, messages)?;
        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();
        let response = agent
            .post(chat_url)
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(map_chat_provider_error)?;
        let raw = response.into_string().map_err(|source| {
            LlmChatFailure::new(
                ChatFailureKind::InvalidResponse,
                format!("failed to read chat response: {source}"),
            )
        })?;

        parse_chat_response(&raw)
    }
}

pub fn build_models_request(base_url: &str) -> String {
    format!("{}/models", base_url.trim_end_matches('/'))
}

pub fn build_chat_request(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim_end_matches('/'))
}

fn build_chat_request_body(model: &str, messages: &[LlmMessage]) -> Result<String, LlmChatFailure> {
    serde_json::to_string(&json!({
        "model": model,
        "messages": messages
            .iter()
            .map(|message| json!({
                "role": chat_role(message.role),
                "content": message.content
            }))
            .collect::<Vec<_>>()
    }))
    .map_err(|source| {
        LlmChatFailure::new(
            ChatFailureKind::InvalidResponse,
            format!("failed to build chat request: {source}"),
        )
    })
}

fn chat_role(role: LlmMessageRole) -> &'static str {
    match role {
        LlmMessageRole::System => "system",
        LlmMessageRole::User => "user",
        LlmMessageRole::Assistant => "assistant",
    }
}

fn parse_models_response(raw: &str) -> Result<Vec<String>, LlmHealthFailure> {
    let response: ModelsResponse = serde_json::from_str(raw).map_err(|source| {
        LlmHealthFailure::new(
            HealthFailureKind::InvalidResponse,
            format!("failed to parse models response: {source}"),
        )
    })?;

    Ok(response.data.into_iter().map(|model| model.id).collect())
}

fn parse_chat_response(raw: &str) -> Result<String, LlmChatFailure> {
    let response: ChatResponse = serde_json::from_str(raw).map_err(|source| {
        LlmChatFailure::new(
            ChatFailureKind::InvalidResponse,
            format!("failed to parse chat response: {source}"),
        )
    })?;

    let Some(choice) = response.choices.into_iter().next() else {
        return Err(LlmChatFailure::new(
            ChatFailureKind::InvalidResponse,
            "chat response did not include choices",
        ));
    };

    if choice.message.content.trim().is_empty() {
        return Err(LlmChatFailure::new(
            ChatFailureKind::ModelEmptyResponse,
            "chat response content was empty",
        ));
    }

    Ok(choice.message.content)
}

fn map_provider_error(error: ureq::Error) -> LlmHealthFailure {
    match error {
        ureq::Error::Status(status, _) => LlmHealthFailure::with_http_status(
            HealthFailureKind::EndpointFailure,
            status,
            format!("models endpoint returned HTTP {status}"),
        ),
        ureq::Error::Transport(transport) => match transport.kind() {
            ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Dns => {
                LlmHealthFailure::new(HealthFailureKind::ConnectionFailed, transport.to_string())
            }
            ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme => {
                LlmHealthFailure::new(HealthFailureKind::InvalidEndpoint, transport.to_string())
            }
            ureq::ErrorKind::Io if is_timeout(&transport) => {
                LlmHealthFailure::new(HealthFailureKind::Timeout, transport.to_string())
            }
            _ => LlmHealthFailure::new(HealthFailureKind::EndpointFailure, transport.to_string()),
        },
    }
}

fn map_chat_provider_error(error: ureq::Error) -> LlmChatFailure {
    match error {
        ureq::Error::Status(status, _) => LlmChatFailure::with_http_status(
            ChatFailureKind::EndpointFailure,
            status,
            format!("chat endpoint returned HTTP {status}"),
        ),
        ureq::Error::Transport(transport) => match transport.kind() {
            ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Dns => {
                LlmChatFailure::new(ChatFailureKind::ConnectionFailed, transport.to_string())
            }
            ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme => {
                LlmChatFailure::new(ChatFailureKind::InvalidEndpoint, transport.to_string())
            }
            ureq::ErrorKind::Io if is_timeout(&transport) => {
                LlmChatFailure::new(ChatFailureKind::Timeout, transport.to_string())
            }
            _ => LlmChatFailure::new(ChatFailureKind::EndpointFailure, transport.to_string()),
        },
    }
}

fn is_timeout(transport: &ureq::Transport) -> bool {
    let Some(source) = transport.source() else {
        return false;
    };
    let Some(io_error) = source.downcast_ref::<io::Error>() else {
        return false;
    };
    io_error.kind() == io::ErrorKind::TimedOut
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelItem>,
}

#[derive(Deserialize)]
struct ModelItem {
    id: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use crate::llm::{LlmMessage, LlmMessageRole, LlmMessageVisibility};

    use super::{
        build_chat_request, build_chat_request_body, build_models_request, parse_chat_response,
        parse_models_response,
    };

    #[test]
    fn builds_models_endpoint_from_openai_compatible_base_url() {
        assert_eq!(
            build_models_request("http://127.0.0.1:1234/v1/"),
            "http://127.0.0.1:1234/v1/models"
        );
    }

    #[test]
    fn parses_openai_compatible_models_response() {
        let models = parse_models_response(
            r#"{"object":"list","data":[{"id":"google/gemma-4-e4b"},{"id":"other"}]}"#,
        )
        .expect("models response should parse");

        assert_eq!(models, vec!["google/gemma-4-e4b", "other"]);
    }

    #[test]
    fn builds_chat_endpoint_from_openai_compatible_base_url() {
        assert_eq!(
            build_chat_request("http://127.0.0.1:1234/v1/"),
            "http://127.0.0.1:1234/v1/chat/completions"
        );
    }

    #[test]
    fn builds_plain_chat_request_with_user_message() {
        let messages = vec![
            LlmMessage {
                turn_id: "turn-1".to_owned(),
                role: LlmMessageRole::System,
                visibility: LlmMessageVisibility::Internal,
                content: "system instruction".to_owned(),
            },
            LlmMessage {
                turn_id: "turn-1".to_owned(),
                role: LlmMessageRole::User,
                visibility: LlmMessageVisibility::UserVisible,
                content: "hello".to_owned(),
            },
        ];
        let raw = build_chat_request_body("google/gemma-4-e4b", &messages)
            .expect("plain chat request should serialize");
        let value: Value = serde_json::from_str(&raw).expect("request should be json");

        assert_eq!(value["model"], "google/gemma-4-e4b");
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][0]["content"], "system instruction");
        assert_eq!(value["messages"][1]["role"], "user");
        assert_eq!(value["messages"][1]["content"], "hello");
    }

    #[test]
    fn parses_openai_compatible_chat_response() {
        let answer = parse_chat_response(
            r#"{"choices":[{"message":{"role":"assistant","content":"plain answer"}}]}"#,
        )
        .expect("chat response should parse");

        assert_eq!(answer, "plain answer");
    }

    #[test]
    fn classifies_empty_chat_content_as_model_empty_response() {
        let failure =
            parse_chat_response(r#"{"choices":[{"message":{"role":"assistant","content":""}}]}"#)
                .expect_err("empty assistant content should fail");

        assert_eq!(failure.kind.as_str(), "model_empty_response");
    }
}
