use crate::llm::{
    LlmChatReport, LlmChatStatus, LlmDiagnosticsSnapshot, LlmHealthReport, LlmHealthStatus,
    RuntimeDecision, RuntimeDecisionError, RuntimeResponseParseError,
};

use super::state::TuiState;
use super::workspace::WorkspaceEvents;

pub(super) fn record_health_report(
    state: &mut TuiState,
    report: &LlmHealthReport,
) -> WorkspaceEvents {
    match &report.status {
        LlmHealthStatus::Succeeded { available_models } => {
            let mut events = state.record_system_notice("health ok");
            events.extend(state.record_system_notice(format!(
                "provider {} | model {}",
                report.provider, report.model
            )));
            events.extend(state.record_system_notice(format!(
                "latency {} ms | models {} | endpoint {}",
                report.latency_ms, available_models, report.models_url
            )));
            events
        }
        LlmHealthStatus::Failed(failure) => {
            let mut events =
                state.record_system_notice(format!("health failed: {}", failure.kind.as_str()));
            events.extend(state.record_system_notice(format!(
                "provider {} | model {}",
                report.provider, report.model
            )));
            events.extend(state.record_system_notice(format!("endpoint {}", report.models_url)));
            events.extend(state.record_system_notice(format!("message {}", failure.message)));
            events
        }
    }
}

pub(super) fn record_llm_diagnostics(
    state: &mut TuiState,
    snapshot: &LlmDiagnosticsSnapshot,
) -> WorkspaceEvents {
    let mut events = WorkspaceEvents::none();
    for line in snapshot.lines() {
        events.extend(state.record_system_notice(line));
    }
    events
}

pub(super) fn record_plain_chat_failure(
    state: &mut TuiState,
    report: &LlmChatReport,
) -> WorkspaceEvents {
    let LlmChatStatus::Failed(failure) = &report.status else {
        return WorkspaceEvents::none();
    };

    let mut events =
        state.record_system_notice(format!("request failed: {}", failure.kind.as_str()));
    events.extend(state.record_system_notice(format!(
        "provider {} | model {}",
        report.provider, report.model
    )));
    events.extend(state.record_system_notice(format!("endpoint {}", report.chat_url)));
    events.extend(state.record_system_notice(format!("message {}", failure.message)));
    events
}

pub(super) fn record_runtime_decision(
    state: &mut TuiState,
    decision: &RuntimeDecision,
) -> WorkspaceEvents {
    match decision {
        RuntimeDecision::Answer {
            summary,
            payload_body,
        } => state.record_answer(answer_display_text(summary, payload_body.as_deref())),
        RuntimeDecision::Clarify { message, .. } => state.record_answer(message.clone()),
        RuntimeDecision::Blocked { message, .. } => {
            let mut events = state.record_system_notice("response blocked");
            events.extend(state.record_system_notice(message.clone()));
            events
        }
        RuntimeDecision::PlanCandidate { message, items, .. } => state.record_system_notice(
            format!("task plan accepted: {} ({} items)", message, items.len()),
        ),
        RuntimeDecision::ToolCandidatePending {
            activity,
            tool_name,
            ..
        } => state.record_system_notice(format!(
            "tool candidate pending: {} ({})",
            tool_name,
            activity.as_str()
        )),
        RuntimeDecision::ApprovalNeeded {
            activity,
            tool_name,
            ..
        } => state.record_system_notice(format!(
            "approval needed: {} ({})",
            tool_name,
            activity.as_str()
        )),
    }
}

pub(super) fn record_tool_loop_limit(
    state: &mut TuiState,
    reason: &str,
    diagnosis: &str,
) -> WorkspaceEvents {
    let mut events = state.record_system_notice(format!("tool loop stopped: {reason}"));
    events.extend(
        state.record_system_notice("도구 반복 제한에 도달해 추가 LLM 요청을 보내지 않습니다."),
    );
    events.extend(state.record_system_notice(format!("tool loop diagnosis: {diagnosis}")));
    events
}

pub(super) fn record_runtime_decision_error(
    state: &mut TuiState,
    error: &RuntimeDecisionError,
) -> WorkspaceEvents {
    let mut events =
        state.record_system_notice(format!("runtime decision failed: {}", error.kind.as_str()));
    events.extend(state.record_system_notice(error.message.clone()));
    events
}

pub(super) fn record_runtime_response_parse_error(
    state: &mut TuiState,
    error: &RuntimeResponseParseError,
) -> WorkspaceEvents {
    let mut events =
        state.record_system_notice(format!("response parse failed: {}", error.kind.as_str()));
    events.extend(state.record_system_notice(error.message.clone()));
    events
}

fn answer_display_text(summary: &str, payload_body: Option<&str>) -> String {
    match payload_body {
        Some(body) if summary.trim().is_empty() => body.to_owned(),
        Some(body) => format!("{summary}\n\n{body}"),
        None => summary.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::answer_display_text;

    #[test]
    fn answer_display_keeps_summary_and_payload_separate() {
        assert_eq!(
            answer_display_text("summary", Some("body")),
            "summary\n\nbody"
        );
        assert_eq!(answer_display_text("", Some("body")), "body");
        assert_eq!(answer_display_text("summary", None), "summary");
    }
}
