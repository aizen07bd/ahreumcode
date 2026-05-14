use super::provider::{LlmChatReport, LlmChatStatus, LlmHealthReport, LlmHealthStatus};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LlmDiagnosticsState {
    pub last_health: Option<LlmHealthSnapshot>,
    pub last_request: Option<LlmRequestSnapshot>,
    pub last_parse: Option<LlmParseSnapshot>,
    pub last_repair: Option<LlmRepairSnapshot>,
    pub last_decision: Option<LlmDecisionSnapshot>,
    pub last_failure: Option<String>,
}

impl LlmDiagnosticsState {
    pub fn record_health(&mut self, report: &LlmHealthReport) {
        let status = match &report.status {
            LlmHealthStatus::Succeeded { .. } => "ok".to_owned(),
            LlmHealthStatus::Failed(failure) => {
                self.last_failure = Some(format!("health: {}", failure.kind.as_str()));
                format!("failed: {}", failure.kind.as_str())
            }
        };

        self.last_health = Some(LlmHealthSnapshot {
            status,
            latency_ms: report.latency_ms,
            endpoint: report.models_url.clone(),
        });
    }

    pub fn record_request_started(&mut self, run_id: &str, turn_id: &str) {
        self.last_request = Some(LlmRequestSnapshot {
            run_id: run_id.to_owned(),
            turn_id: turn_id.to_owned(),
            status: "started".to_owned(),
            latency_ms: None,
        });
    }

    pub fn record_request_report(&mut self, report: &LlmChatReport) {
        let Some(request) = self.last_request.as_mut() else {
            return;
        };

        request.latency_ms = Some(report.latency_ms);
        match &report.status {
            LlmChatStatus::Succeeded { .. } => {
                request.status = "succeeded".to_owned();
            }
            LlmChatStatus::Failed(failure) => {
                request.status = format!("failed: {}", failure.kind.as_str());
                self.last_failure = Some(format!("request: {}", failure.kind.as_str()));
            }
        }
    }

    pub fn record_parse_success(
        &mut self,
        response_type: &str,
        activity: &str,
        payload_count: usize,
    ) {
        self.last_parse = Some(LlmParseSnapshot {
            status: "succeeded".to_owned(),
            response_type: Some(response_type.to_owned()),
            activity: Some(activity.to_owned()),
            payload_count,
            error_kind: None,
        });
    }

    pub fn record_parse_failure(&mut self, error_kind: &str) {
        self.last_parse = Some(LlmParseSnapshot {
            status: "failed".to_owned(),
            response_type: None,
            activity: None,
            payload_count: 0,
            error_kind: Some(error_kind.to_owned()),
        });
        self.last_failure = Some(format!("parse: {error_kind}"));
    }

    pub fn record_repair_started(&mut self, attempt: u16, max_attempts: u16) {
        self.last_repair = Some(LlmRepairSnapshot {
            status: "started".to_owned(),
            attempt,
            max_attempts,
        });
    }

    pub fn record_repair_succeeded(&mut self, attempt: u16, max_attempts: u16) {
        self.last_repair = Some(LlmRepairSnapshot {
            status: "succeeded".to_owned(),
            attempt,
            max_attempts,
        });
    }

    pub fn record_repair_limited(&mut self, attempts: u16, max_attempts: u16) {
        self.last_repair = Some(LlmRepairSnapshot {
            status: "limit_reached".to_owned(),
            attempt: attempts,
            max_attempts,
        });
        self.last_failure = Some("repair: limit_reached".to_owned());
    }

    pub fn record_decision(
        &mut self,
        decision: &str,
        activity: Option<&str>,
        tool_name: Option<&str>,
    ) {
        self.last_decision = Some(LlmDecisionSnapshot {
            status: "recorded".to_owned(),
            decision: decision.to_owned(),
            activity: activity.map(ToOwned::to_owned),
            tool_name: tool_name.map(ToOwned::to_owned),
        });
    }

    pub fn record_decision_failure(&mut self, error_kind: &str) {
        self.last_decision = Some(LlmDecisionSnapshot {
            status: "failed".to_owned(),
            decision: "failed".to_owned(),
            activity: None,
            tool_name: None,
        });
        self.last_failure = Some(format!("decision: {error_kind}"));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmHealthSnapshot {
    pub status: String,
    pub latency_ms: u128,
    pub endpoint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmRequestSnapshot {
    pub run_id: String,
    pub turn_id: String,
    pub status: String,
    pub latency_ms: Option<u128>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmParseSnapshot {
    pub status: String,
    pub response_type: Option<String>,
    pub activity: Option<String>,
    pub payload_count: usize,
    pub error_kind: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmRepairSnapshot {
    pub status: String,
    pub attempt: u16,
    pub max_attempts: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmDecisionSnapshot {
    pub status: String,
    pub decision: String,
    pub activity: Option<String>,
    pub tool_name: Option<String>,
}

pub struct LlmDiagnosticsRuntime<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub base_url: &'a str,
    pub context_tokens: u32,
    pub mode: &'a str,
    pub web: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmDiagnosticsSnapshot {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub context_tokens: u32,
    pub mode: String,
    pub web: String,
    pub last_health: String,
    pub last_request: String,
    pub last_parse: String,
    pub last_repair: String,
    pub last_decision: String,
    pub last_failure: String,
    pub tool_stage_ready: bool,
    pub tool_stage_reason: String,
}

impl LlmDiagnosticsSnapshot {
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("llm provider {} | model {}", self.provider, self.model),
            format!(
                "llm endpoint {} | context {} | mode {} | {}",
                self.base_url, self.context_tokens, self.mode, self.web
            ),
            format!("llm health {}", self.last_health),
            format!("llm request {}", self.last_request),
            format!("llm parse {}", self.last_parse),
            format!("llm repair {}", self.last_repair),
            format!("llm decision {}", self.last_decision),
            format!("llm failure {}", self.last_failure),
            format!(
                "tool stage {} | {}",
                if self.tool_stage_ready {
                    "structural_ready"
                } else {
                    "not_ready"
                },
                self.tool_stage_reason
            ),
        ]
    }
}

pub struct LlmDiagnostics;

impl LlmDiagnostics {
    pub fn snapshot(
        runtime: LlmDiagnosticsRuntime<'_>,
        state: &LlmDiagnosticsState,
    ) -> LlmDiagnosticsSnapshot {
        LlmDiagnosticsSnapshot {
            provider: runtime.provider.to_owned(),
            model: runtime.model.to_owned(),
            base_url: runtime.base_url.to_owned(),
            context_tokens: runtime.context_tokens,
            mode: runtime.mode.to_owned(),
            web: runtime.web.to_owned(),
            last_health: format_health(state.last_health.as_ref()),
            last_request: format_request(state.last_request.as_ref()),
            last_parse: format_parse(state.last_parse.as_ref()),
            last_repair: format_repair(state.last_repair.as_ref()),
            last_decision: format_decision(state.last_decision.as_ref()),
            last_failure: state
                .last_failure
                .clone()
                .unwrap_or_else(|| "none".to_owned()),
            tool_stage_ready: true,
            tool_stage_reason: "llm-04..10 runtime path is structurally wired; e2e pending"
                .to_owned(),
        }
    }
}

fn format_health(value: Option<&LlmHealthSnapshot>) -> String {
    value
        .map(|health| format!("{} | {} ms", health.status, health.latency_ms))
        .unwrap_or_else(|| "not_checked".to_owned())
}

fn format_request(value: Option<&LlmRequestSnapshot>) -> String {
    value
        .map(|request| {
            let latency = request
                .latency_ms
                .map(|value| format!("{value} ms"))
                .unwrap_or_else(|| "pending".to_owned());
            format!(
                "{} | {} | {} | {}",
                request.status, request.run_id, request.turn_id, latency
            )
        })
        .unwrap_or_else(|| "none".to_owned())
}

fn format_parse(value: Option<&LlmParseSnapshot>) -> String {
    value
        .map(|parse| {
            if parse.status == "succeeded" {
                format!(
                    "succeeded | {} | {} | payloads {}",
                    parse.response_type.as_deref().unwrap_or("unknown"),
                    parse.activity.as_deref().unwrap_or("unknown"),
                    parse.payload_count
                )
            } else {
                format!(
                    "failed | {}",
                    parse.error_kind.as_deref().unwrap_or("unknown")
                )
            }
        })
        .unwrap_or_else(|| "none".to_owned())
}

fn format_repair(value: Option<&LlmRepairSnapshot>) -> String {
    value
        .map(|repair| {
            format!(
                "{} | attempt {}/{}",
                repair.status, repair.attempt, repair.max_attempts
            )
        })
        .unwrap_or_else(|| "none".to_owned())
}

fn format_decision(value: Option<&LlmDecisionSnapshot>) -> String {
    value
        .map(|decision| {
            let activity = decision.activity.as_deref().unwrap_or("none");
            let tool_name = decision.tool_name.as_deref().unwrap_or("none");
            format!(
                "{} | {} | {} | {}",
                decision.status, decision.decision, activity, tool_name
            )
        })
        .unwrap_or_else(|| "none".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{LlmDiagnostics, LlmDiagnosticsRuntime, LlmDiagnosticsState};

    #[test]
    fn snapshot_contains_default_runtime_state() {
        let state = LlmDiagnosticsState::default();
        let snapshot = LlmDiagnostics::snapshot(
            LlmDiagnosticsRuntime {
                provider: "lm-studio",
                model: "google/gemma-4-e4b",
                base_url: "http://127.0.0.1:1234/v1",
                context_tokens: 32000,
                mode: "Crew",
                web: "web on",
            },
            &state,
        );

        assert_eq!(snapshot.last_health, "not_checked");
        assert_eq!(snapshot.last_request, "none");
        assert!(snapshot.tool_stage_ready);
    }

    #[test]
    fn snapshot_renders_recent_parse_failure() {
        let mut state = LlmDiagnosticsState::default();
        state.record_parse_failure("json_parse_failed");

        let snapshot = LlmDiagnostics::snapshot(
            LlmDiagnosticsRuntime {
                provider: "lm-studio",
                model: "google/gemma-4-e4b",
                base_url: "http://127.0.0.1:1234/v1",
                context_tokens: 32000,
                mode: "Crew",
                web: "web on",
            },
            &state,
        );

        assert_eq!(snapshot.last_parse, "failed | json_parse_failed");
        assert_eq!(snapshot.last_failure, "parse: json_parse_failed");
    }
}
