use std::error::Error as StdError;
use std::io;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::config::RuntimeConfig;

use super::provider::{HealthFailureKind, LlmHealthFailure, LlmHealthReport, LlmHealthStatus};

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
}

pub fn build_models_request(base_url: &str) -> String {
    format!("{}/models", base_url.trim_end_matches('/'))
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

#[cfg(test)]
mod tests {
    use super::{build_models_request, parse_models_response};

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
}
