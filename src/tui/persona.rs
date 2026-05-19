#[cfg(test)]
use serde::Deserialize;
use time::OffsetDateTime;

pub const MIN_PERSONA_TERMINAL_WIDTH: u16 = 140;
pub const MIN_PERSONA_PANEL_WIDTH: u16 = 36;
pub const MAX_PERSONA_MESSAGES: usize = 80;
pub const MAX_PERSONA_MESSAGE_CHARS: usize = 180;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaSpeaker {
    Lead,
    Planning,
    Implementation,
    Verification,
    Documentation,
}

impl PersonaSpeaker {
    pub fn name(self) -> &'static str {
        match self {
            Self::Lead => "팀장",
            Self::Planning => "지윤",
            Self::Implementation => "민호",
            Self::Verification => "서연",
            Self::Documentation => "하준",
        }
    }

    pub fn role_label(self) -> Option<&'static str> {
        match self {
            Self::Lead => None,
            Self::Planning => Some("기획/설계"),
            Self::Implementation => Some("구현"),
            Self::Verification => Some("검증"),
            Self::Documentation => Some("문서"),
        }
    }

    pub fn speaker_role(self) -> PersonaSpeakerRole {
        match self {
            Self::Lead => PersonaSpeakerRole::Lead,
            Self::Planning | Self::Implementation | Self::Verification | Self::Documentation => {
                PersonaSpeakerRole::Member
            }
        }
    }

    pub fn from_id(value: &str) -> Option<Self> {
        match value {
            "lead" => Some(Self::Lead),
            "planning" => Some(Self::Planning),
            "implementation" => Some(Self::Implementation),
            "verification" => Some(Self::Verification),
            "documentation" => Some(Self::Documentation),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaSpeakerRole {
    Lead,
    Member,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaMessage {
    pub speaker: PersonaSpeaker,
    pub time_label: String,
    pub body: String,
    pub repeat_count: u16,
}

impl PersonaMessage {
    pub fn from_speaker(speaker: PersonaSpeaker, body: impl Into<String>) -> Self {
        Self {
            speaker,
            time_label: current_time_label(),
            body: body.into(),
            repeat_count: 1,
        }
    }

    pub fn role(&self) -> PersonaSpeakerRole {
        self.speaker.speaker_role()
    }

    fn can_coalesce_with(&self, other: &Self) -> bool {
        self.speaker == other.speaker && self.body == other.body
    }

    fn merge_repeat(&mut self, other: Self) {
        self.time_label = other.time_label;
        self.repeat_count = self.repeat_count.saturating_add(1);
    }
}

#[derive(Default)]
pub struct PersonaBuffer {
    messages: Vec<PersonaMessage>,
    scroll: usize,
    render_pending: bool,
}

impl PersonaBuffer {
    pub fn push_message(&mut self, message: PersonaMessage) {
        if let Some(last) = self.messages.last_mut() {
            if last.can_coalesce_with(&message) {
                last.merge_repeat(message);
                self.scroll = 0;
                self.render_pending = true;
                return;
            }
        }

        self.messages.push(message);
        self.trim_to_retention_limit();
        self.scroll = 0;
        self.render_pending = true;
    }

    pub fn messages(&self) -> &[PersonaMessage] {
        &self.messages
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub fn scroll(&mut self, delta: isize) {
        let previous = self.scroll;
        let max_scroll = self.total_visible_lines().saturating_sub(1);
        let next = (self.scroll as isize + delta).clamp(0, max_scroll as isize) as usize;
        self.scroll = next;
        if self.scroll != previous {
            self.render_pending = true;
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll
    }

    pub fn take_render_event(&mut self) -> Option<PersonaRendered> {
        if !self.render_pending {
            return None;
        }

        self.render_pending = false;
        Some(PersonaRendered {
            message_count: self.messages.len(),
        })
    }

    fn trim_to_retention_limit(&mut self) {
        if self.messages.len() <= MAX_PERSONA_MESSAGES {
            return;
        }

        let overflow = self.messages.len() - MAX_PERSONA_MESSAGES;
        self.messages.drain(0..overflow);
    }

    fn total_visible_lines(&self) -> usize {
        self.messages.len().saturating_mul(3)
    }
}

pub enum PersonaEvent {
    PanelOpened,
    PanelClosed,
    WidthRejected { width: u16, min_width: u16 },
}

#[derive(Default)]
pub struct PersonaEvents {
    pub events: Vec<PersonaEvent>,
}

impl PersonaEvents {
    pub fn none() -> Self {
        Self { events: Vec::new() }
    }

    pub fn single(event: PersonaEvent) -> Self {
        Self {
            events: vec![event],
        }
    }
}

pub struct PersonaRendered {
    pub message_count: usize,
}

#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
pub enum PersonaTurnError {
    InvalidJson,
    UnknownSpeaker(String),
    UnexpectedSpeaker {
        expected: PersonaSpeaker,
        actual: PersonaSpeaker,
    },
    EmptyBody,
    BodyTooLong,
}

#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonaTurnPayload {
    speaker: String,
    body: String,
}

#[cfg(test)]
pub fn parse_persona_turn(
    raw: &str,
    expected_speaker: PersonaSpeaker,
) -> Result<PersonaMessage, PersonaTurnError> {
    let payload: PersonaTurnPayload =
        serde_json::from_str(raw).map_err(|_| PersonaTurnError::InvalidJson)?;
    let speaker = PersonaSpeaker::from_id(payload.speaker.trim())
        .ok_or_else(|| PersonaTurnError::UnknownSpeaker(payload.speaker.clone()))?;
    if speaker != expected_speaker {
        return Err(PersonaTurnError::UnexpectedSpeaker {
            expected: expected_speaker,
            actual: speaker,
        });
    }

    let body = payload.body.trim();
    if body.is_empty() {
        return Err(PersonaTurnError::EmptyBody);
    }
    if body.chars().count() > MAX_PERSONA_MESSAGE_CHARS {
        return Err(PersonaTurnError::BodyTooLong);
    }

    Ok(PersonaMessage::from_speaker(speaker, body.to_owned()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaRuntimeEvent {
    LlmRequestStarted,
    LlmResponseReceived,
    LlmRequestFailed,
    RuntimeResponseParsed,
    RuntimeResponseParseFailed,
    SchemaValidationFailed,
    RuntimeDecisionRecorded,
    RuntimeDecisionFailed,
    RepairRequestStarted,
    RepairSucceeded,
    RepairLimitReached,
    ToolCandidateClassified,
    ToolPermissionAllowed,
    ToolPermissionApprovalNeeded,
    ToolPermissionDenied,
    ToolExecutionStarted,
    ToolExecutionSucceeded,
    ToolExecutionFailed,
    ToolLoopDuplicateRedirected,
    ToolLoopLimitReached,
    RawResponseReceived,
    MessageRecorded,
    ToolObservationAttached,
    ToolWorkspaceSummaryRendered,
    FinalAnswerRecorded,
    WorkspaceSystemNotice,
}

pub const PERSONA_RUNTIME_EVENTS: [PersonaRuntimeEvent; 26] = [
    PersonaRuntimeEvent::LlmRequestStarted,
    PersonaRuntimeEvent::LlmResponseReceived,
    PersonaRuntimeEvent::LlmRequestFailed,
    PersonaRuntimeEvent::RuntimeResponseParsed,
    PersonaRuntimeEvent::RuntimeResponseParseFailed,
    PersonaRuntimeEvent::SchemaValidationFailed,
    PersonaRuntimeEvent::RuntimeDecisionRecorded,
    PersonaRuntimeEvent::RuntimeDecisionFailed,
    PersonaRuntimeEvent::RepairRequestStarted,
    PersonaRuntimeEvent::RepairSucceeded,
    PersonaRuntimeEvent::RepairLimitReached,
    PersonaRuntimeEvent::ToolCandidateClassified,
    PersonaRuntimeEvent::ToolPermissionAllowed,
    PersonaRuntimeEvent::ToolPermissionApprovalNeeded,
    PersonaRuntimeEvent::ToolPermissionDenied,
    PersonaRuntimeEvent::ToolExecutionStarted,
    PersonaRuntimeEvent::ToolExecutionSucceeded,
    PersonaRuntimeEvent::ToolExecutionFailed,
    PersonaRuntimeEvent::ToolLoopDuplicateRedirected,
    PersonaRuntimeEvent::ToolLoopLimitReached,
    PersonaRuntimeEvent::RawResponseReceived,
    PersonaRuntimeEvent::MessageRecorded,
    PersonaRuntimeEvent::ToolObservationAttached,
    PersonaRuntimeEvent::ToolWorkspaceSummaryRendered,
    PersonaRuntimeEvent::FinalAnswerRecorded,
    PersonaRuntimeEvent::WorkspaceSystemNotice,
];

impl PersonaRuntimeEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LlmRequestStarted => "llm_request_started",
            Self::LlmResponseReceived => "llm_response_received",
            Self::LlmRequestFailed => "llm_request_failed",
            Self::RuntimeResponseParsed => "runtime_response_parsed",
            Self::RuntimeResponseParseFailed => "runtime_response_parse_failed",
            Self::SchemaValidationFailed => "schema_validation_failed",
            Self::RuntimeDecisionRecorded => "runtime_decision_recorded",
            Self::RuntimeDecisionFailed => "runtime_decision_failed",
            Self::RepairRequestStarted => "repair_request_started",
            Self::RepairSucceeded => "repair_succeeded",
            Self::RepairLimitReached => "repair_limit_reached",
            Self::ToolCandidateClassified => "tool_candidate_classified",
            Self::ToolPermissionAllowed => "tool_permission_allowed",
            Self::ToolPermissionApprovalNeeded => "tool_permission_approval_needed",
            Self::ToolPermissionDenied => "tool_permission_denied",
            Self::ToolExecutionStarted => "tool_execution_started",
            Self::ToolExecutionSucceeded => "tool_execution_succeeded",
            Self::ToolExecutionFailed => "tool_execution_failed",
            Self::ToolLoopDuplicateRedirected => "tool_loop_duplicate_redirected",
            Self::ToolLoopLimitReached => "tool_loop_limit_reached",
            Self::RawResponseReceived => "raw_response_received",
            Self::MessageRecorded => "message_recorded",
            Self::ToolObservationAttached => "tool_observation_attached",
            Self::ToolWorkspaceSummaryRendered => "tool_workspace_summary_rendered",
            Self::FinalAnswerRecorded => "final_answer_recorded",
            Self::WorkspaceSystemNotice => "workspace_system_notice",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaRuntimeRoute {
    LeftLogOnly(PersonaRuntimeExclusion),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaRuntimeExclusion {
    RuntimeEvent,
    RawModelText,
    InternalHistory,
    ToolOutputBody,
    FinalAnswer,
    SystemNotice,
}

pub fn runtime_event_route(event: PersonaRuntimeEvent) -> PersonaRuntimeRoute {
    use PersonaRuntimeEvent as Event;

    match event {
        Event::LlmRequestFailed
        | Event::LlmRequestStarted
        | Event::LlmResponseReceived
        | Event::RuntimeResponseParsed
        | Event::RuntimeResponseParseFailed
        | Event::SchemaValidationFailed
        | Event::RuntimeDecisionRecorded
        | Event::RuntimeDecisionFailed
        | Event::RepairRequestStarted
        | Event::RepairSucceeded
        | Event::RepairLimitReached
        | Event::ToolCandidateClassified
        | Event::ToolPermissionAllowed
        | Event::ToolPermissionApprovalNeeded
        | Event::ToolPermissionDenied
        | Event::ToolExecutionStarted
        | Event::ToolExecutionSucceeded
        | Event::ToolExecutionFailed
        | Event::ToolLoopDuplicateRedirected
        | Event::ToolLoopLimitReached => {
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::RuntimeEvent)
        }
        Event::RawResponseReceived => {
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::RawModelText)
        }
        Event::MessageRecorded => {
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::InternalHistory)
        }
        Event::ToolObservationAttached | Event::ToolWorkspaceSummaryRendered => {
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::ToolOutputBody)
        }
        Event::FinalAnswerRecorded => {
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::FinalAnswer)
        }
        Event::WorkspaceSystemNotice => {
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::SystemNotice)
        }
    }
}

pub fn runtime_event_catalog_is_complete() -> bool {
    PERSONA_RUNTIME_EVENTS
        .iter()
        .all(|event| !event.as_str().is_empty() && runtime_event_route(*event).is_defined())
}

impl PersonaRuntimeRoute {
    fn is_defined(self) -> bool {
        matches!(self, Self::LeftLogOnly(_))
    }
}

fn current_time_label() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    format!("{:02}:{:02}", now.hour(), now.minute())
}

#[cfg(test)]
mod tests {
    use super::{
        parse_persona_turn, runtime_event_route, PersonaBuffer, PersonaMessage,
        PersonaRuntimeEvent, PersonaRuntimeExclusion, PersonaRuntimeRoute, PersonaSpeaker,
        PersonaTurnError, MAX_PERSONA_MESSAGES, PERSONA_RUNTIME_EVENTS,
    };

    #[test]
    fn catalog_keeps_runtime_events_left_log_only() {
        for event in [
            PersonaRuntimeEvent::LlmRequestStarted,
            PersonaRuntimeEvent::RuntimeResponseParsed,
            PersonaRuntimeEvent::RepairRequestStarted,
            PersonaRuntimeEvent::ToolExecutionSucceeded,
            PersonaRuntimeEvent::ToolPermissionApprovalNeeded,
            PersonaRuntimeEvent::SchemaValidationFailed,
        ] {
            assert_eq!(
                runtime_event_route(event),
                PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::RuntimeEvent)
            );
        }
    }

    #[test]
    fn catalog_keeps_raw_or_user_facing_bodies_out_of_persona() {
        assert_eq!(
            runtime_event_route(PersonaRuntimeEvent::RawResponseReceived),
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::RawModelText)
        );
        assert_eq!(
            runtime_event_route(PersonaRuntimeEvent::MessageRecorded),
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::InternalHistory)
        );
        assert_eq!(
            runtime_event_route(PersonaRuntimeEvent::ToolObservationAttached),
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::ToolOutputBody)
        );
        assert_eq!(
            runtime_event_route(PersonaRuntimeEvent::ToolWorkspaceSummaryRendered),
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::ToolOutputBody)
        );
        assert_eq!(
            runtime_event_route(PersonaRuntimeEvent::FinalAnswerRecorded),
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::FinalAnswer)
        );
        assert_eq!(
            runtime_event_route(PersonaRuntimeEvent::WorkspaceSystemNotice),
            PersonaRuntimeRoute::LeftLogOnly(PersonaRuntimeExclusion::SystemNotice)
        );
    }

    #[test]
    fn catalog_event_names_are_stable_for_later_log_mapping() {
        assert_eq!(
            PersonaRuntimeEvent::ToolLoopDuplicateRedirected.as_str(),
            "tool_loop_duplicate_redirected"
        );
        assert_eq!(
            PersonaRuntimeEvent::RepairLimitReached.as_str(),
            "repair_limit_reached"
        );
        assert_eq!(
            PersonaRuntimeEvent::FinalAnswerRecorded.as_str(),
            "final_answer_recorded"
        );
    }

    #[test]
    fn catalog_has_a_route_for_every_declared_runtime_event() {
        for event in PERSONA_RUNTIME_EVENTS {
            let route = runtime_event_route(event);
            assert_eq!(route, runtime_event_route(event));
        }
    }

    #[test]
    fn buffer_coalesces_consecutive_duplicate_messages() {
        let mut buffer = PersonaBuffer::default();

        buffer.push_message(PersonaMessage::from_speaker(
            PersonaSpeaker::Implementation,
            "body-a",
        ));
        buffer.push_message(PersonaMessage::from_speaker(
            PersonaSpeaker::Implementation,
            "body-a",
        ));

        assert_eq!(buffer.messages().len(), 1);
        assert_eq!(buffer.messages()[0].repeat_count, 2);
        assert!(buffer.take_render_event().is_some());
    }

    #[test]
    fn buffer_keeps_latest_messages_with_fixed_retention_limit() {
        let mut buffer = PersonaBuffer::default();

        for index in 0..(MAX_PERSONA_MESSAGES + 5) {
            buffer.push_message(PersonaMessage::from_speaker(
                PersonaSpeaker::Verification,
                format!("message {index}"),
            ));
        }

        assert_eq!(buffer.messages().len(), MAX_PERSONA_MESSAGES);
        assert_eq!(buffer.messages()[0].body, "message 5");
    }

    #[test]
    fn persona_turn_accepts_only_one_expected_speaker_message() {
        let message = parse_persona_turn(
            r#"{"speaker": "implementation", "body": "body-a"}"#,
            PersonaSpeaker::Implementation,
        )
        .expect("single speaker turn should parse");

        assert_eq!(message.speaker, PersonaSpeaker::Implementation);
        assert_eq!(message.body, "body-a");
    }

    #[test]
    fn persona_turn_rejects_batch_script_shape() {
        let error = parse_persona_turn(
            r#"{"messages": [{"speaker": "lead", "body": "body-a"}]}"#,
            PersonaSpeaker::Lead,
        )
        .expect_err("turn parser must reject multi-speaker script payloads");

        assert_eq!(error, PersonaTurnError::InvalidJson);
    }

    #[test]
    fn persona_turn_rejects_unexpected_speaker() {
        let error = parse_persona_turn(
            r#"{"speaker": "planning", "body": "body-a"}"#,
            PersonaSpeaker::Lead,
        )
        .expect_err("speaker must match the requested turn");

        assert_eq!(
            error,
            PersonaTurnError::UnexpectedSpeaker {
                expected: PersonaSpeaker::Lead,
                actual: PersonaSpeaker::Planning,
            }
        );
    }
}
