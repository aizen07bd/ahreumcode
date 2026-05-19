use std::collections::VecDeque;

use serde::Deserialize;

use super::persona::{PersonaMessage, PersonaSpeaker, MAX_PERSONA_MESSAGE_CHARS};
pub use super::persona_prompt::PersonaTurnPrompt;
use super::persona_prompt::{
    build_persona_turn_prompt, speaker_from_output_value, speaker_from_peer_recipient,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaRuntimeStage {
    Kickoff,
    Progress,
    FollowUp,
    Completion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaTurnKind {
    LeadSummon,
    MemberResponse,
    LeadSummary,
    ProgressLead,
    FollowUpLead,
    Completion,
}

impl PersonaTurnKind {
    pub(crate) fn requires_visible_message(self) -> bool {
        !matches!(self, Self::MemberResponse)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaSessionStatus {
    Idle,
    WaitingForModel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaRuntimeMode {
    Off,
    Full,
}

impl PersonaRuntimeMode {
    fn allows_requests(self) -> bool {
        self == Self::Full
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaTaskRun {
    pub task_id: String,
    pub user_prompt: String,
    pub task_state_summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaPeerMessage {
    pub id: String,
    pub from: PersonaSpeaker,
    pub to: PersonaSpeaker,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaPeerMessageDraft {
    pub to: PersonaSpeaker,
    pub body: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaTeamTaskStatus {
    Open,
    Claimed,
    Reported,
    Passed,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaTeamTask {
    pub id: String,
    pub owner: PersonaSpeaker,
    pub title: String,
    pub status: PersonaTeamTaskStatus,
    pub report: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaTurn {
    pub id: String,
    pub task_id: String,
    pub speaker: PersonaSpeaker,
    pub stage: PersonaRuntimeStage,
    pub kind: PersonaTurnKind,
    pub user_prompt: String,
    pub task_state_summary: String,
    pub peer_messages: Vec<PersonaPeerMessage>,
    pub team_tasks: Vec<PersonaTeamTask>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersonaTurnOutcome {
    Spoken(PersonaMessage),
    Passed { speaker: PersonaSpeaker },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaTurnResult {
    pub outcome: PersonaTurnOutcome,
    pub peer_messages: Vec<PersonaPeerMessageDraft>,
}

impl PersonaTurnResult {
    fn new(outcome: PersonaTurnOutcome, peer_messages: Vec<PersonaPeerMessageDraft>) -> Self {
        Self {
            outcome,
            peer_messages,
        }
    }
}

impl From<PersonaTurnOutcome> for PersonaTurnResult {
    fn from(outcome: PersonaTurnOutcome) -> Self {
        Self::new(outcome, Vec::new())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum PersonaTurnOutcomeError {
    InvalidJson,
    UnknownSpeaker(String),
    UnexpectedSpeaker {
        expected: PersonaSpeaker,
        actual: PersonaSpeaker,
    },
    UnknownDecision(String),
    EmptyBody,
    BodyTooLong,
    UnknownPeerRecipient(String),
    PeerMessageToSelf,
    PeerMessageOnPass,
    EmptyPeerMessageBody,
    PeerMessageBodyTooLong,
    RequiredSpeakerPassed,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonaTurnOutcomePayload {
    speaker: String,
    decision: String,
    body: String,
    peer_messages: Vec<PersonaPeerMessagePayload>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonaPeerMessagePayload {
    to: String,
    body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaSession {
    pub speaker: PersonaSpeaker,
    pub status: PersonaSessionStatus,
    pub history: Vec<PersonaMessage>,
}

impl PersonaSession {
    fn new(speaker: PersonaSpeaker) -> Self {
        Self {
            speaker,
            status: PersonaSessionStatus::Idle,
            history: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct PersonaRuntime {
    current_task: Option<PersonaTaskRun>,
    sessions: Vec<PersonaSession>,
    pending_turns: VecDeque<PersonaTurn>,
    peer_messages: Vec<PersonaPeerMessage>,
    team_tasks: Vec<PersonaTeamTask>,
    next_task_index: u64,
    next_turn_index: u64,
    next_message_index: u64,
    next_team_task_index: u64,
    follow_up_summary_scheduled: bool,
    task_completed: bool,
}

impl PersonaRuntime {
    pub fn new() -> Self {
        Self {
            sessions: fixed_team_sessions(),
            next_task_index: 1,
            next_turn_index: 1,
            next_message_index: 1,
            next_team_task_index: 1,
            ..Self::default()
        }
    }

    pub fn start_kickoff(&mut self, user_prompt: impl Into<String>) -> PersonaTurn {
        self.reset_for_new_task(
            user_prompt.into(),
            "main_runtime_status: not_started".to_owned(),
        );
        let turn = self.build_turn(
            PersonaSpeaker::Lead,
            PersonaRuntimeStage::Kickoff,
            PersonaTurnKind::LeadSummon,
        );
        self.mark_session_waiting(PersonaSpeaker::Lead);
        self.pending_turns.push_back(turn.clone());
        turn
    }

    pub fn start_kickoff_for_mode(
        &mut self,
        mode: PersonaRuntimeMode,
        user_prompt: impl Into<String>,
    ) -> Option<PersonaTurn> {
        if !mode.allows_requests() {
            self.clear_active_task();
            return None;
        }

        Some(self.start_kickoff(user_prompt))
    }

    pub fn enqueue_completion_for_mode(
        &mut self,
        mode: PersonaRuntimeMode,
        task_state_summary: impl Into<String>,
    ) -> Option<PersonaTurn> {
        if !mode.allows_requests() || self.current_task.is_none() {
            return None;
        }

        let summary = task_state_summary.into();
        if let Some(task) = self.current_task.as_mut() {
            task.task_state_summary = summary.clone();
        }

        self.task_completed = true;
        self.remove_pending_lifecycle_turns();
        if let Some(existing) = self.pending_turns.iter_mut().find(|turn| {
            turn.stage == PersonaRuntimeStage::Completion
                && turn.kind == PersonaTurnKind::Completion
        }) {
            existing.task_state_summary = summary;
            return Some(existing.clone());
        }

        let turn = self.build_turn(
            PersonaSpeaker::Lead,
            PersonaRuntimeStage::Completion,
            PersonaTurnKind::Completion,
        );
        self.mark_session_waiting(PersonaSpeaker::Lead);
        self.pending_turns.push_back(turn.clone());
        Some(turn)
    }

    pub fn absorb_completion_into_active_closure(
        &mut self,
        active_turn: Option<&PersonaTurn>,
        task_state_summary: impl Into<String>,
    ) -> bool {
        let Some(active_turn) = active_turn else {
            return false;
        };
        if !matches!(
            active_turn.kind,
            PersonaTurnKind::LeadSummary | PersonaTurnKind::Completion
        ) {
            return false;
        }

        let summary = task_state_summary.into();
        let Some(task) = self.current_task.as_mut() else {
            return false;
        };
        if task.task_id != active_turn.task_id {
            return false;
        }

        task.task_state_summary = summary;
        true
    }

    pub fn enqueue_progress_for_mode(
        &mut self,
        mode: PersonaRuntimeMode,
        task_state_summary: impl Into<String>,
    ) -> Option<PersonaTurn> {
        if !mode.allows_requests() || self.current_task.is_none() || self.task_completed {
            return None;
        }

        let summary = task_state_summary.into();
        if let Some(task) = self.current_task.as_mut() {
            task.task_state_summary = summary;
        }

        if let Some(existing) = self.pending_turns.iter_mut().find(|turn| {
            turn.speaker == PersonaSpeaker::Lead && turn.kind == PersonaTurnKind::ProgressLead
        }) {
            if let Some(task) = self.current_task.as_ref() {
                existing.task_state_summary = task.task_state_summary.clone();
            }
            return Some(existing.clone());
        }

        let turn = self.build_turn(
            PersonaSpeaker::Lead,
            PersonaRuntimeStage::Progress,
            PersonaTurnKind::ProgressLead,
        );
        self.mark_session_waiting(PersonaSpeaker::Lead);
        self.pending_turns.push_back(turn.clone());
        Some(turn)
    }

    fn remove_pending_lifecycle_turns(&mut self) {
        self.pending_turns.retain(|turn| {
            turn.stage == PersonaRuntimeStage::Completion
                && turn.kind == PersonaTurnKind::Completion
        });
    }

    pub fn start_follow_up_for_mode(
        &mut self,
        mode: PersonaRuntimeMode,
        user_prompt: impl Into<String>,
        task_state_summary: impl Into<String>,
    ) -> Option<PersonaTurn> {
        if !mode.allows_requests() {
            self.clear_active_task();
            return None;
        }

        if self.current_task.is_none() {
            return Some(self.start_kickoff(user_prompt));
        }

        let user_prompt = user_prompt.into();
        let task_state_summary = task_state_summary.into();
        if let Some(task) = self.current_task.as_mut() {
            task.user_prompt = user_prompt;
            task.task_state_summary = task_state_summary;
        }
        self.pending_turns.clear();
        self.peer_messages.clear();
        self.follow_up_summary_scheduled = false;
        self.task_completed = false;
        for session in &mut self.sessions {
            session.status = PersonaSessionStatus::Idle;
        }

        let turn = self.build_turn(
            PersonaSpeaker::Lead,
            PersonaRuntimeStage::FollowUp,
            PersonaTurnKind::FollowUpLead,
        );
        self.mark_session_waiting(PersonaSpeaker::Lead);
        self.pending_turns.push_back(turn.clone());
        Some(turn)
    }

    pub fn pop_next_turn(&mut self) -> Option<PersonaTurn> {
        self.pending_turns.pop_front()
    }

    pub fn record_turn_result(&mut self, turn: &PersonaTurn, result: impl Into<PersonaTurnResult>) {
        let result = result.into();
        let peer_messages = result.peer_messages.clone();
        let speaker = match &result.outcome {
            PersonaTurnOutcome::Spoken(message) => message.speaker,
            PersonaTurnOutcome::Passed { speaker } => *speaker,
        };

        if turn.kind.requires_visible_message()
            && matches!(result.outcome, PersonaTurnOutcome::Passed { .. })
        {
            self.mark_session_idle(speaker);
            return;
        }

        if self.task_completed && turn.stage != PersonaRuntimeStage::Completion {
            self.mark_session_idle(speaker);
            return;
        }

        if turn.kind == PersonaTurnKind::MemberResponse {
            match &result.outcome {
                PersonaTurnOutcome::Spoken(message) => {
                    self.report_claimed_team_tasks(speaker, message.body.clone());
                }
                PersonaTurnOutcome::Passed { .. } => {
                    self.pass_claimed_team_tasks(speaker);
                }
            }
        }
        if matches!(
            turn.kind,
            PersonaTurnKind::LeadSummary | PersonaTurnKind::Completion
        ) {
            self.close_reported_team_tasks();
        }

        match result.outcome {
            PersonaTurnOutcome::Spoken(message) => self.record_session_message(message),
            PersonaTurnOutcome::Passed { speaker } => self.mark_session_idle(speaker),
        }
        for peer_message in &peer_messages {
            self.register_peer_message(speaker, peer_message.clone());
        }

        match turn.stage {
            PersonaRuntimeStage::Kickoff => {
                self.schedule_next_kickoff_turn(turn, &peer_messages);
            }
            PersonaRuntimeStage::Progress => {
                self.schedule_next_progress_turns(turn, &peer_messages);
            }
            PersonaRuntimeStage::FollowUp => {
                self.schedule_next_follow_up_turns(turn, &peer_messages);
            }
            PersonaRuntimeStage::Completion => {}
        }
    }

    fn register_peer_message(
        &mut self,
        from: PersonaSpeaker,
        draft: PersonaPeerMessageDraft,
    ) -> PersonaPeerMessage {
        let message = PersonaPeerMessage {
            id: format!("persona-peer-{:04}", self.next_message_index),
            from,
            to: draft.to,
            body: draft.body,
        };
        self.next_message_index = self.next_message_index.saturating_add(1);
        self.peer_messages.push(message.clone());
        if from == PersonaSpeaker::Lead && message.to != PersonaSpeaker::Lead {
            self.create_team_task(message.to, message.body.clone());
        }
        message
    }

    fn create_team_task(&mut self, owner: PersonaSpeaker, title: String) {
        let task = PersonaTeamTask {
            id: format!("persona-team-task-{:04}", self.next_team_task_index),
            owner,
            title,
            status: PersonaTeamTaskStatus::Open,
            report: None,
        };
        self.next_team_task_index = self.next_team_task_index.saturating_add(1);
        self.team_tasks.push(task);
    }

    fn claim_open_team_tasks(&mut self, owner: PersonaSpeaker) {
        for task in self
            .team_tasks
            .iter_mut()
            .filter(|task| task.owner == owner && task.status == PersonaTeamTaskStatus::Open)
        {
            task.status = PersonaTeamTaskStatus::Claimed;
        }
    }

    fn report_claimed_team_tasks(&mut self, owner: PersonaSpeaker, report: String) {
        for task in self.team_tasks.iter_mut().filter(|task| {
            task.owner == owner
                && matches!(
                    task.status,
                    PersonaTeamTaskStatus::Open | PersonaTeamTaskStatus::Claimed
                )
        }) {
            task.status = PersonaTeamTaskStatus::Reported;
            task.report = Some(report.clone());
        }
    }

    fn pass_claimed_team_tasks(&mut self, owner: PersonaSpeaker) {
        for task in self.team_tasks.iter_mut().filter(|task| {
            task.owner == owner
                && matches!(
                    task.status,
                    PersonaTeamTaskStatus::Open | PersonaTeamTaskStatus::Claimed
                )
        }) {
            task.status = PersonaTeamTaskStatus::Passed;
            task.report = None;
        }
    }

    fn close_reported_team_tasks(&mut self) {
        for task in self
            .team_tasks
            .iter_mut()
            .filter(|task| task.status == PersonaTeamTaskStatus::Reported)
        {
            task.status = PersonaTeamTaskStatus::Closed;
        }
    }

    #[cfg(test)]
    pub fn sessions(&self) -> &[PersonaSession] {
        &self.sessions
    }

    pub fn session_history(&self, speaker: PersonaSpeaker) -> Option<&[PersonaMessage]> {
        self.sessions
            .iter()
            .find(|session| session.speaker == speaker)
            .map(|session| session.history.as_slice())
    }

    pub fn current_task(&self) -> Option<&PersonaTaskRun> {
        self.current_task.as_ref()
    }

    pub fn is_task_completed(&self) -> bool {
        self.task_completed
    }

    #[cfg(test)]
    pub fn peer_messages(&self) -> &[PersonaPeerMessage] {
        &self.peer_messages
    }

    #[cfg(test)]
    pub fn team_tasks(&self) -> &[PersonaTeamTask] {
        &self.team_tasks
    }

    pub fn has_pending_turns(&self) -> bool {
        !self.pending_turns.is_empty()
    }

    pub fn build_turn_prompt(
        &self,
        turn: &PersonaTurn,
        speaker_history: &[PersonaMessage],
    ) -> PersonaTurnPrompt {
        build_persona_turn_prompt(turn, speaker_history)
    }

    pub fn clear(&mut self) {
        self.clear_active_task();
    }

    fn reset_for_new_task(&mut self, user_prompt: String, task_state_summary: String) {
        let task_id = format!("persona-task-{:04}", self.next_task_index);
        self.next_task_index = self.next_task_index.saturating_add(1);
        self.current_task = Some(PersonaTaskRun {
            task_id,
            user_prompt,
            task_state_summary,
        });
        self.pending_turns.clear();
        self.peer_messages.clear();
        self.team_tasks.clear();
        self.follow_up_summary_scheduled = false;
        self.task_completed = false;
        for session in &mut self.sessions {
            session.status = PersonaSessionStatus::Idle;
            session.history.clear();
        }
    }

    fn clear_active_task(&mut self) {
        self.current_task = None;
        self.pending_turns.clear();
        self.peer_messages.clear();
        self.team_tasks.clear();
        self.follow_up_summary_scheduled = false;
        self.task_completed = false;
        for session in &mut self.sessions {
            session.status = PersonaSessionStatus::Idle;
            session.history.clear();
        }
    }

    fn build_turn(
        &mut self,
        speaker: PersonaSpeaker,
        stage: PersonaRuntimeStage,
        kind: PersonaTurnKind,
    ) -> PersonaTurn {
        let task = self
            .current_task
            .as_ref()
            .expect("persona task must be started before building a turn");
        let turn = PersonaTurn {
            id: format!("persona-turn-{:04}", self.next_turn_index),
            task_id: task.task_id.clone(),
            speaker,
            stage,
            kind,
            user_prompt: task.user_prompt.clone(),
            task_state_summary: task.task_state_summary.clone(),
            peer_messages: self.peer_messages.clone(),
            team_tasks: self.team_tasks.clone(),
        };
        self.next_turn_index = self.next_turn_index.saturating_add(1);
        turn
    }

    fn record_session_message(&mut self, message: PersonaMessage) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.speaker == message.speaker)
        {
            session.status = PersonaSessionStatus::Idle;
            session.history.push(message);
        }
    }

    fn mark_session_idle(&mut self, speaker: PersonaSpeaker) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.speaker == speaker)
        {
            session.status = PersonaSessionStatus::Idle;
        }
    }

    fn schedule_next_kickoff_turn(
        &mut self,
        turn: &PersonaTurn,
        peer_messages: &[PersonaPeerMessageDraft],
    ) {
        match turn.kind {
            PersonaTurnKind::LeadSummon => {
                self.schedule_kickoff_members(peer_messages);
                if !self.pending_turns.iter().any(|pending| {
                    pending.stage == PersonaRuntimeStage::Kickoff
                        && pending.kind == PersonaTurnKind::MemberResponse
                }) {
                    self.schedule_kickoff_summary();
                }
            }
            PersonaTurnKind::MemberResponse => {
                if !self.pending_turns.iter().any(|pending| {
                    pending.stage == PersonaRuntimeStage::Kickoff
                        && pending.kind == PersonaTurnKind::MemberResponse
                }) {
                    self.schedule_kickoff_summary();
                }
            }
            _ => {}
        }
    }

    fn schedule_kickoff_members(&mut self, peer_messages: &[PersonaPeerMessageDraft]) {
        let mut scheduled = Vec::new();
        for peer_message in peer_messages {
            if peer_message.to == PersonaSpeaker::Lead || scheduled.contains(&peer_message.to) {
                continue;
            }
            scheduled.push(peer_message.to);
            if scheduled.len() >= 4 {
                break;
            }
        }

        for speaker in scheduled {
            self.claim_open_team_tasks(speaker);
            let turn = self.build_turn(
                speaker,
                PersonaRuntimeStage::Kickoff,
                PersonaTurnKind::MemberResponse,
            );
            self.mark_session_waiting(speaker);
            self.push_kickoff_turn_before_completion(turn);
        }
    }

    fn push_kickoff_turn_before_completion(&mut self, turn: PersonaTurn) {
        if let Some(index) = self
            .pending_turns
            .iter()
            .position(|pending| pending.stage != PersonaRuntimeStage::Kickoff)
        {
            self.pending_turns.insert(index, turn);
            return;
        }

        self.pending_turns.push_back(turn);
    }

    fn schedule_kickoff_summary(&mut self) {
        if self.task_completed {
            return;
        }
        if self.pending_turns.iter().any(|turn| {
            turn.stage == PersonaRuntimeStage::Kickoff && turn.kind == PersonaTurnKind::LeadSummary
        }) {
            return;
        }

        let turn = self.build_turn(
            PersonaSpeaker::Lead,
            PersonaRuntimeStage::Kickoff,
            PersonaTurnKind::LeadSummary,
        );
        self.mark_session_waiting(PersonaSpeaker::Lead);
        self.push_kickoff_turn_before_completion(turn);
    }

    fn schedule_next_progress_turns(
        &mut self,
        turn: &PersonaTurn,
        peer_messages: &[PersonaPeerMessageDraft],
    ) {
        match turn.kind {
            PersonaTurnKind::ProgressLead => {
                self.schedule_progress_members(peer_messages);
                if !self.pending_turns.iter().any(|pending| {
                    pending.stage == PersonaRuntimeStage::Progress
                        && pending.kind == PersonaTurnKind::MemberResponse
                }) {
                    self.schedule_progress_summary();
                }
            }
            PersonaTurnKind::MemberResponse => {
                if !self.pending_turns.iter().any(|pending| {
                    pending.stage == PersonaRuntimeStage::Progress
                        && pending.kind == PersonaTurnKind::MemberResponse
                }) {
                    self.schedule_progress_summary();
                }
            }
            _ => {}
        }
    }

    fn schedule_progress_members(&mut self, peer_messages: &[PersonaPeerMessageDraft]) {
        let mut scheduled = Vec::new();
        for peer_message in peer_messages {
            if peer_message.to == PersonaSpeaker::Lead || scheduled.contains(&peer_message.to) {
                continue;
            }
            scheduled.push(peer_message.to);
            if scheduled.len() >= 3 {
                break;
            }
        }

        for speaker in scheduled {
            self.claim_open_team_tasks(speaker);
            let turn = self.build_turn(
                speaker,
                PersonaRuntimeStage::Progress,
                PersonaTurnKind::MemberResponse,
            );
            self.mark_session_waiting(speaker);
            self.pending_turns.push_back(turn);
        }
    }

    fn schedule_progress_summary(&mut self) {
        if self.task_completed {
            return;
        }
        if self.pending_turns.iter().any(|turn| {
            turn.stage == PersonaRuntimeStage::Progress && turn.kind == PersonaTurnKind::LeadSummary
        }) {
            return;
        }

        let turn = self.build_turn(
            PersonaSpeaker::Lead,
            PersonaRuntimeStage::Progress,
            PersonaTurnKind::LeadSummary,
        );
        self.mark_session_waiting(PersonaSpeaker::Lead);
        self.pending_turns.push_back(turn);
    }

    fn schedule_next_follow_up_turns(
        &mut self,
        turn: &PersonaTurn,
        peer_messages: &[PersonaPeerMessageDraft],
    ) {
        match turn.kind {
            PersonaTurnKind::FollowUpLead => {
                self.schedule_follow_up_members(peer_messages);
                if self.pending_turns.is_empty() {
                    self.schedule_follow_up_summary();
                }
            }
            PersonaTurnKind::MemberResponse => {
                if !self.pending_turns.iter().any(|pending| {
                    pending.stage == PersonaRuntimeStage::FollowUp
                        && pending.kind == PersonaTurnKind::MemberResponse
                }) {
                    self.schedule_follow_up_summary();
                }
            }
            _ => {}
        }
    }

    fn schedule_follow_up_members(&mut self, peer_messages: &[PersonaPeerMessageDraft]) {
        let mut scheduled = Vec::new();
        for peer_message in peer_messages {
            if peer_message.to == PersonaSpeaker::Lead || scheduled.contains(&peer_message.to) {
                continue;
            }
            scheduled.push(peer_message.to);
            if scheduled.len() >= 3 {
                break;
            }
        }

        for speaker in scheduled {
            self.claim_open_team_tasks(speaker);
            let turn = self.build_turn(
                speaker,
                PersonaRuntimeStage::FollowUp,
                PersonaTurnKind::MemberResponse,
            );
            self.mark_session_waiting(speaker);
            self.pending_turns.push_back(turn);
        }
    }

    fn schedule_follow_up_summary(&mut self) {
        if self.follow_up_summary_scheduled {
            return;
        }
        self.follow_up_summary_scheduled = true;
        let turn = self.build_turn(
            PersonaSpeaker::Lead,
            PersonaRuntimeStage::FollowUp,
            PersonaTurnKind::LeadSummary,
        );
        self.remove_pending_completion_turn();
        self.mark_session_waiting(PersonaSpeaker::Lead);
        self.pending_turns.push_back(turn);
    }

    fn remove_pending_completion_turn(&mut self) -> Option<PersonaTurn> {
        let index = self.pending_turns.iter().position(|turn| {
            turn.stage == PersonaRuntimeStage::Completion
                && turn.kind == PersonaTurnKind::Completion
        })?;
        self.pending_turns.remove(index)
    }

    fn mark_session_waiting(&mut self, speaker: PersonaSpeaker) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.speaker == speaker)
        {
            session.status = PersonaSessionStatus::WaitingForModel;
        }
    }
}

fn fixed_team_sessions() -> Vec<PersonaSession> {
    [
        PersonaSpeaker::Lead,
        PersonaSpeaker::Planning,
        PersonaSpeaker::Implementation,
        PersonaSpeaker::Verification,
        PersonaSpeaker::Documentation,
    ]
    .into_iter()
    .map(PersonaSession::new)
    .collect()
}

pub fn parse_persona_turn_outcome(
    raw: &str,
    expected_speaker: PersonaSpeaker,
) -> Result<PersonaTurnResult, PersonaTurnOutcomeError> {
    let payload: PersonaTurnOutcomePayload =
        serde_json::from_str(raw).map_err(|_| PersonaTurnOutcomeError::InvalidJson)?;
    let speaker = speaker_from_output_value(payload.speaker.trim())
        .ok_or_else(|| PersonaTurnOutcomeError::UnknownSpeaker(payload.speaker.clone()))?;
    if speaker != expected_speaker {
        return Err(PersonaTurnOutcomeError::UnexpectedSpeaker {
            expected: expected_speaker,
            actual: speaker,
        });
    }
    let peer_messages = parse_peer_message_drafts(speaker, payload.peer_messages)?;

    match payload.decision.trim() {
        "speak" => {
            let body = payload.body.trim();
            if body.is_empty() {
                return Err(PersonaTurnOutcomeError::EmptyBody);
            }
            if body.chars().count() > MAX_PERSONA_MESSAGE_CHARS {
                return Err(PersonaTurnOutcomeError::BodyTooLong);
            }
            Ok(PersonaTurnResult::new(
                PersonaTurnOutcome::Spoken(PersonaMessage::from_speaker(speaker, body.to_owned())),
                peer_messages,
            ))
        }
        "pass" => {
            if !payload.body.trim().is_empty() {
                return Err(PersonaTurnOutcomeError::EmptyBody);
            }
            if !peer_messages.is_empty() {
                return Err(PersonaTurnOutcomeError::PeerMessageOnPass);
            }
            Ok(PersonaTurnResult::new(
                PersonaTurnOutcome::Passed { speaker },
                Vec::new(),
            ))
        }
        other => Err(PersonaTurnOutcomeError::UnknownDecision(other.to_owned())),
    }
}

pub fn parse_persona_turn_result_for_turn(
    raw: &str,
    turn: &PersonaTurn,
) -> Result<PersonaTurnResult, PersonaTurnOutcomeError> {
    let result = parse_persona_turn_outcome(raw, turn.speaker)?;
    if turn.kind.requires_visible_message()
        && matches!(result.outcome, PersonaTurnOutcome::Passed { .. })
    {
        return Err(PersonaTurnOutcomeError::RequiredSpeakerPassed);
    }
    Ok(result)
}

fn parse_peer_message_drafts(
    from: PersonaSpeaker,
    payloads: Vec<PersonaPeerMessagePayload>,
) -> Result<Vec<PersonaPeerMessageDraft>, PersonaTurnOutcomeError> {
    payloads
        .into_iter()
        .map(|payload| {
            let to = speaker_from_peer_recipient(payload.to.trim())
                .ok_or_else(|| PersonaTurnOutcomeError::UnknownPeerRecipient(payload.to.clone()))?;
            if to == from {
                return Err(PersonaTurnOutcomeError::PeerMessageToSelf);
            }
            let body = payload.body.trim();
            if body.is_empty() {
                return Err(PersonaTurnOutcomeError::EmptyPeerMessageBody);
            }
            if body.chars().count() > MAX_PERSONA_MESSAGE_CHARS {
                return Err(PersonaTurnOutcomeError::PeerMessageBodyTooLong);
            }

            Ok(PersonaPeerMessageDraft {
                to,
                body: body.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::persona_prompt::fixed_persona_role_profiles;
    use super::*;

    fn message(speaker: PersonaSpeaker, body: &str) -> PersonaMessage {
        PersonaMessage::from_speaker(speaker, body)
    }

    fn outcome_with_peers(
        speaker: PersonaSpeaker,
        body: &str,
        peers: &[(PersonaSpeaker, &str)],
    ) -> PersonaTurnResult {
        PersonaTurnResult::new(
            PersonaTurnOutcome::Spoken(message(speaker, body)),
            peers
                .iter()
                .map(|(to, body)| PersonaPeerMessageDraft {
                    to: *to,
                    body: (*body).to_owned(),
                })
                .collect(),
        )
    }

    #[test]
    fn runtime_starts_with_fixed_independent_sessions() {
        let runtime = PersonaRuntime::new();

        let speakers = runtime
            .sessions()
            .iter()
            .map(|session| session.speaker)
            .collect::<Vec<_>>();

        assert_eq!(
            speakers,
            vec![
                PersonaSpeaker::Lead,
                PersonaSpeaker::Planning,
                PersonaSpeaker::Implementation,
                PersonaSpeaker::Verification,
                PersonaSpeaker::Documentation,
            ]
        );
        assert!(runtime
            .sessions()
            .iter()
            .all(|session| session.status == PersonaSessionStatus::Idle));
    }

    #[test]
    fn role_profiles_cover_fixed_team_with_distinct_missions() {
        let profiles = fixed_persona_role_profiles();
        let speakers = profiles
            .iter()
            .map(|profile| profile.speaker)
            .collect::<Vec<_>>();
        let speaker_ids = profiles
            .iter()
            .map(|profile| profile.speaker_id)
            .collect::<Vec<_>>();
        let missions = profiles
            .iter()
            .map(|profile| profile.mission)
            .collect::<Vec<_>>();

        assert_eq!(
            speakers,
            vec![
                PersonaSpeaker::Lead,
                PersonaSpeaker::Planning,
                PersonaSpeaker::Implementation,
                PersonaSpeaker::Verification,
                PersonaSpeaker::Documentation,
            ]
        );
        assert_eq!(
            speaker_ids,
            vec![
                "lead",
                "planning",
                "implementation",
                "verification",
                "documentation",
            ]
        );
        assert_eq!(missions.len(), 5);
        for (index, mission) in missions.iter().enumerate() {
            assert!(!mission.is_empty());
            assert!(!missions
                .iter()
                .skip(index + 1)
                .any(|other| other == mission));
        }
        assert!(profiles
            .iter()
            .all(|profile| !profile.speak_when.is_empty()));
        assert!(profiles.iter().all(|profile| !profile.pass_when.is_empty()));
    }

    #[test]
    fn lead_turn_prompt_requires_visible_single_speaker_message() {
        let mut runtime = PersonaRuntime::new();
        let turn = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("kickoff turn");
        let prompt = runtime.build_turn_prompt(&turn, &[]);

        assert_eq!(prompt.turn_id, turn.id);
        assert_eq!(prompt.speaker, PersonaSpeaker::Lead);
        assert_eq!(prompt.output_contract.speaker, PersonaSpeaker::Lead);
        assert!(!prompt.output_contract.allow_pass);
        assert_eq!(prompt.output_contract.speaker_id(), "lead");
        assert!(prompt.system_prompt.contains(r#""decision":"speak""#));
        assert!(prompt.system_prompt.contains("not pass"));
        assert!(prompt
            .system_prompt
            .contains("do not return decision=\"pass\""));
        assert!(prompt.system_prompt.contains(r#""speaker":"lead""#));
        assert!(prompt.system_prompt.contains(r#""to":"지윤""#));
        assert!(prompt.system_prompt.contains("peer_messages"));
        assert!(prompt
            .system_prompt
            .contains("Allowed peer message recipients for peer_messages.to: 팀장, 지윤(기획/설계), 민호(구현), 서연(검증), 하준(문서)"));
        assert!(!prompt
            .system_prompt
            .contains("Allowed peer message recipients: lead"));
        assert!(prompt.system_prompt.contains("under 140 Korean characters"));
        assert!(prompt
            .system_prompt
            .contains("The main runtime owns tools, evidence"));
        assert!(prompt
            .system_prompt
            .contains("The team task list is the persona collaboration state"));
        assert!(prompt.system_prompt.contains("advisory team tasks"));
        assert!(prompt.system_prompt.contains("Do not mention tool names"));
        assert!(prompt
            .system_prompt
            .contains("Do not expose internal speaker ids"));
        assert!(prompt
            .system_prompt
            .contains("Do not return a messages array"));
    }

    #[test]
    fn member_turn_prompt_allows_pass_without_visible_message() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("kickoff turn");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));
        runtime.record_turn_result(
            &lead,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Planning, "peer-a")],
            ),
        );
        let member = runtime.pop_next_turn().expect("member turn");
        let prompt = runtime.build_turn_prompt(&member, &[]);

        assert_eq!(member.kind, PersonaTurnKind::MemberResponse);
        assert!(prompt.output_contract.allow_pass);
        assert!(prompt.system_prompt.contains(r#""decision":"speak""#));
        assert!(prompt.system_prompt.contains(r#""decision":"pass""#));
        assert!(prompt.system_prompt.contains("return pass"));
    }

    #[test]
    fn required_lead_pass_does_not_schedule_member_turns() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("lead turn");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));

        runtime.record_turn_result(
            &lead,
            PersonaTurnOutcome::Passed {
                speaker: PersonaSpeaker::Lead,
            },
        );

        assert!(runtime.pop_next_turn().is_none());
    }

    #[test]
    fn turn_parser_rejects_pass_for_required_visible_turn() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("lead turn");

        let result = parse_persona_turn_result_for_turn(
            r#"{"speaker":"lead","decision":"pass","body":"","peer_messages":[]}"#,
            &lead,
        );

        assert_eq!(result, Err(PersonaTurnOutcomeError::RequiredSpeakerPassed));
    }

    #[test]
    fn turn_outcome_parser_accepts_speak_and_pass_for_expected_speaker() {
        let spoken = parse_persona_turn_outcome(
            r#"{"speaker":"planning","decision":"speak","body":"body-a","peer_messages":[]}"#,
            PersonaSpeaker::Planning,
        )
        .expect("spoken outcome");
        let passed = parse_persona_turn_outcome(
            r#"{"speaker":"verification","decision":"pass","body":"","peer_messages":[]}"#,
            PersonaSpeaker::Verification,
        )
        .expect("pass outcome");

        assert!(
            matches!(spoken.outcome, PersonaTurnOutcome::Spoken(message) if message.speaker == PersonaSpeaker::Planning)
        );
        assert_eq!(
            passed.outcome,
            PersonaTurnOutcome::Passed {
                speaker: PersonaSpeaker::Verification
            }
        );
        assert!(spoken.peer_messages.is_empty());
        assert!(passed.peer_messages.is_empty());
    }

    #[test]
    fn turn_outcome_parser_accepts_display_label_speaker_values() {
        let lead = parse_persona_turn_outcome(
            r#"{"speaker":"팀장","decision":"speak","body":"body-a","peer_messages":[{"to":"지윤(기획/설계)","body":"peer-a"}]}"#,
            PersonaSpeaker::Lead,
        )
        .expect("display label speaker");
        let member = parse_persona_turn_outcome(
            r#"{"speaker":"지윤(기획/설계)","decision":"speak","body":"body-b","peer_messages":[]}"#,
            PersonaSpeaker::Planning,
        )
        .expect("display label member");

        assert!(
            matches!(lead.outcome, PersonaTurnOutcome::Spoken(message) if message.speaker == PersonaSpeaker::Lead)
        );
        assert_eq!(
            lead.peer_messages,
            vec![PersonaPeerMessageDraft {
                to: PersonaSpeaker::Planning,
                body: "peer-a".to_owned(),
            }]
        );
        assert!(
            matches!(member.outcome, PersonaTurnOutcome::Spoken(message) if message.speaker == PersonaSpeaker::Planning)
        );
    }

    #[test]
    fn turn_outcome_parser_rejects_batch_or_wrong_speaker_shapes() {
        let batch = parse_persona_turn_outcome(
            r#"{"messages":[{"speaker":"lead","decision":"speak","body":"body-a"}]}"#,
            PersonaSpeaker::Lead,
        );
        let wrong_speaker = parse_persona_turn_outcome(
            r#"{"speaker":"implementation","decision":"speak","body":"body-a","peer_messages":[]}"#,
            PersonaSpeaker::Planning,
        );
        let pass_with_body = parse_persona_turn_outcome(
            r#"{"speaker":"planning","decision":"pass","body":"body-a","peer_messages":[]}"#,
            PersonaSpeaker::Planning,
        );
        let missing_peer_messages = parse_persona_turn_outcome(
            r#"{"speaker":"planning","decision":"speak","body":"body-a"}"#,
            PersonaSpeaker::Planning,
        );

        assert_eq!(batch, Err(PersonaTurnOutcomeError::InvalidJson));
        assert!(matches!(
            wrong_speaker,
            Err(PersonaTurnOutcomeError::UnexpectedSpeaker {
                expected: PersonaSpeaker::Planning,
                actual: PersonaSpeaker::Implementation,
            })
        ));
        assert_eq!(pass_with_body, Err(PersonaTurnOutcomeError::EmptyBody));
        assert_eq!(
            missing_peer_messages,
            Err(PersonaTurnOutcomeError::InvalidJson)
        );
    }

    #[test]
    fn turn_outcome_parser_accepts_structured_peer_messages() {
        let result = parse_persona_turn_outcome(
            r#"{"speaker":"lead","decision":"speak","body":"body-a","peer_messages":[{"to":"지윤","body":"peer-a"},{"to":"민호(구현)","body":"peer-b"}]}"#,
            PersonaSpeaker::Lead,
        )
        .expect("peer message outcome");

        assert!(
            matches!(result.outcome, PersonaTurnOutcome::Spoken(message) if message.speaker == PersonaSpeaker::Lead)
        );
        assert_eq!(
            result.peer_messages,
            vec![
                PersonaPeerMessageDraft {
                    to: PersonaSpeaker::Planning,
                    body: "peer-a".to_owned(),
                },
                PersonaPeerMessageDraft {
                    to: PersonaSpeaker::Implementation,
                    body: "peer-b".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn turn_outcome_parser_rejects_invalid_peer_messages() {
        let unknown = parse_persona_turn_outcome(
            r#"{"speaker":"lead","decision":"speak","body":"body-a","peer_messages":[{"to":"backend","body":"peer-a"}]}"#,
            PersonaSpeaker::Lead,
        );
        let self_message = parse_persona_turn_outcome(
            r#"{"speaker":"planning","decision":"speak","body":"body-a","peer_messages":[{"to":"planning","body":"peer-a"}]}"#,
            PersonaSpeaker::Planning,
        );
        let pass_message = parse_persona_turn_outcome(
            r#"{"speaker":"verification","decision":"pass","body":"","peer_messages":[{"to":"lead","body":"peer-a"}]}"#,
            PersonaSpeaker::Verification,
        );

        assert_eq!(
            unknown,
            Err(PersonaTurnOutcomeError::UnknownPeerRecipient(
                "backend".to_owned()
            ))
        );
        assert_eq!(
            self_message,
            Err(PersonaTurnOutcomeError::PeerMessageToSelf)
        );
        assert_eq!(
            pass_message,
            Err(PersonaTurnOutcomeError::PeerMessageOnPass)
        );
    }

    #[test]
    fn turn_prompt_uses_only_requested_speaker_history_and_addressed_peer_messages() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime.start_kickoff("task-a");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));
        runtime.record_turn_result(
            &lead,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[
                    (PersonaSpeaker::Planning, "peer-a"),
                    (PersonaSpeaker::Implementation, "peer-b"),
                ],
            ),
        );
        let planning_turn = runtime.pop_next_turn().expect("planning turn");
        let planning_history = vec![message(PersonaSpeaker::Planning, "history-a")];
        let prompt = runtime.build_turn_prompt(&planning_turn, &planning_history);

        assert_eq!(planning_turn.speaker, PersonaSpeaker::Planning);
        assert!(prompt.user_prompt.contains("peer-a"));
        assert!(!prompt.user_prompt.contains("peer-b"));
        assert!(prompt.user_prompt.contains("history-a"));
        assert!(prompt.user_prompt.contains("speaker: 지윤(기획/설계)"));
        assert!(!prompt.user_prompt.contains("speaker: planning"));
    }

    #[test]
    fn parsed_peer_messages_are_delivered_to_later_speaker_turns() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("lead turn");
        runtime.pop_next_turn();
        let result = parse_persona_turn_outcome(
            r#"{"speaker":"lead","decision":"speak","body":"body-a","peer_messages":[{"to":"planning","body":"peer-a"},{"to":"implementation","body":"peer-b"}]}"#,
            PersonaSpeaker::Lead,
        )
        .expect("lead result with peer messages");

        runtime.record_turn_result(&lead, result);
        let planning = runtime.pop_next_turn().expect("planning turn");
        let planning_prompt = runtime.build_turn_prompt(&planning, &[]);

        assert_eq!(planning.speaker, PersonaSpeaker::Planning);
        assert!(planning_prompt.user_prompt.contains("peer-a"));
        assert!(!planning_prompt.user_prompt.contains("peer-b"));
    }

    #[test]
    fn lead_peer_messages_create_claimed_team_tasks_for_addressed_members() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime.start_kickoff("task-a");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));

        runtime.record_turn_result(
            &lead,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[
                    (PersonaSpeaker::Planning, "peer-a"),
                    (PersonaSpeaker::Verification, "peer-b"),
                ],
            ),
        );

        assert_eq!(runtime.team_tasks().len(), 2);
        assert_eq!(runtime.team_tasks()[0].owner, PersonaSpeaker::Planning);
        assert_eq!(runtime.team_tasks()[0].title, "peer-a");
        assert_eq!(
            runtime.team_tasks()[0].status,
            PersonaTeamTaskStatus::Claimed
        );
        assert_eq!(runtime.team_tasks()[1].owner, PersonaSpeaker::Verification);
        assert_eq!(
            runtime.team_tasks()[1].status,
            PersonaTeamTaskStatus::Claimed
        );

        let planning = runtime.pop_next_turn().expect("planning turn");
        assert_eq!(planning.speaker, PersonaSpeaker::Planning);
        assert_eq!(planning.team_tasks.len(), 2);
        let prompt = runtime.build_turn_prompt(&planning, &[]);
        assert!(prompt.user_prompt.contains("persona-team-task-0001"));
        assert!(prompt.user_prompt.contains("claimed"));
        assert!(prompt.user_prompt.contains("peer-a"));
    }

    #[test]
    fn member_reports_team_task_and_lead_summary_closes_it() {
        let mut runtime = PersonaRuntime::new();
        runtime.start_kickoff("task-a");
        runtime.pop_next_turn();
        let progress = runtime
            .enqueue_progress_for_mode(PersonaRuntimeMode::Full, "main_runtime_status: progress")
            .expect("progress turn");
        assert_eq!(runtime.pop_next_turn(), Some(progress.clone()));

        runtime.record_turn_result(
            &progress,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Documentation, "peer-a")],
            ),
        );
        assert_eq!(
            runtime.team_tasks()[0].status,
            PersonaTeamTaskStatus::Claimed
        );

        let documentation = runtime.pop_next_turn().expect("documentation turn");
        assert_eq!(documentation.speaker, PersonaSpeaker::Documentation);
        runtime.record_turn_result(
            &documentation,
            PersonaTurnOutcome::Spoken(message(PersonaSpeaker::Documentation, "body-b")),
        );
        assert_eq!(
            runtime.team_tasks()[0].status,
            PersonaTeamTaskStatus::Reported
        );
        assert_eq!(runtime.team_tasks()[0].report.as_deref(), Some("body-b"));

        let summary = runtime.pop_next_turn().expect("lead summary");
        assert_eq!(summary.kind, PersonaTurnKind::LeadSummary);
        runtime.record_turn_result(
            &summary,
            PersonaTurnOutcome::Spoken(message(PersonaSpeaker::Lead, "body-c")),
        );
        assert_eq!(
            runtime.team_tasks()[0].status,
            PersonaTeamTaskStatus::Closed
        );
    }

    #[test]
    fn member_pass_marks_team_task_as_passed_not_reported() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime.start_kickoff("task-a");
        runtime.pop_next_turn();
        runtime.record_turn_result(
            &lead,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Planning, "peer-a")],
            ),
        );

        let planning = runtime.pop_next_turn().expect("planning turn");
        runtime.record_turn_result(
            &planning,
            PersonaTurnOutcome::Passed {
                speaker: PersonaSpeaker::Planning,
            },
        );

        assert_eq!(
            runtime.team_tasks()[0].status,
            PersonaTeamTaskStatus::Passed
        );
        assert_eq!(runtime.team_tasks()[0].report, None);
        let summary = runtime.pop_next_turn().expect("kickoff summary");
        assert_eq!(summary.speaker, PersonaSpeaker::Lead);
        assert_eq!(summary.kind, PersonaTurnKind::LeadSummary);
        assert!(runtime.pop_next_turn().is_none());
    }

    #[test]
    fn completion_turn_keeps_task_state_summary_without_creating_new_task() {
        let mut runtime = PersonaRuntime::new();
        let kickoff = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("kickoff");
        runtime.pop_next_turn();

        let completion = runtime.enqueue_completion_for_mode(
            PersonaRuntimeMode::Full,
            "main_runtime_status: completed\nlast_observation: succeeded",
        );

        let completion = completion.expect("completion turn");
        assert_eq!(completion.stage, PersonaRuntimeStage::Completion);
        assert_eq!(completion.kind, PersonaTurnKind::Completion);
        assert_eq!(completion.speaker, PersonaSpeaker::Lead);
        assert_eq!(
            runtime.current_task().map(|task| task.task_id.as_str()),
            Some(kickoff.task_id.as_str())
        );
        assert!(runtime
            .current_task()
            .expect("current task")
            .task_state_summary
            .contains("main_runtime_status: completed"));
    }

    #[test]
    fn completion_replaces_stale_remaining_kickoff_turns_with_final_closure() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("lead turn");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));

        let completion = runtime
            .enqueue_completion_for_mode(PersonaRuntimeMode::Full, "main_runtime_status: completed")
            .expect("completion turn");
        runtime.record_turn_result(
            &lead,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Planning, "peer-a")],
            ),
        );

        let next = runtime
            .pop_next_turn()
            .expect("completion before stale kickoff");
        assert_eq!(next, completion);
        assert_eq!(next.stage, PersonaRuntimeStage::Completion);
        assert!(runtime.pending_turns.is_empty());
    }

    #[test]
    fn completion_removes_pending_lead_summary_and_adds_final_closure() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("lead turn");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));
        let pending_summary = runtime.build_turn(
            PersonaSpeaker::Lead,
            PersonaRuntimeStage::FollowUp,
            PersonaTurnKind::LeadSummary,
        );
        runtime.pending_turns.push_back(pending_summary.clone());

        let completion = runtime
            .enqueue_completion_for_mode(PersonaRuntimeMode::Full, "main_runtime_status: completed")
            .expect("completion turn");
        assert_eq!(runtime.pop_next_turn(), Some(completion));
        assert!(runtime.pop_next_turn().is_none());
        assert!(runtime
            .current_task()
            .expect("current task")
            .task_state_summary
            .contains("completed"));
    }

    #[test]
    fn later_member_result_does_not_create_lead_summary_after_completion_closure() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("lead turn");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));
        runtime.record_turn_result(
            &lead,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Planning, "peer-a")],
            ),
        );

        let completion = runtime
            .enqueue_completion_for_mode(PersonaRuntimeMode::Full, "main_runtime_status: completed")
            .expect("completion turn");
        assert_eq!(runtime.pop_next_turn(), Some(completion.clone()));

        let speaker = PersonaSpeaker::Planning;
        let stale_member = PersonaTurn {
            id: "stale-member".to_owned(),
            task_id: completion.task_id.clone(),
            speaker,
            stage: PersonaRuntimeStage::Kickoff,
            kind: PersonaTurnKind::MemberResponse,
            user_prompt: "task-a".to_owned(),
            task_state_summary: "main_runtime_status: completed".to_owned(),
            peer_messages: Vec::new(),
            team_tasks: Vec::new(),
        };
        runtime.record_turn_result(&stale_member, PersonaTurnOutcome::Passed { speaker });

        assert!(runtime.pop_next_turn().is_none());
        assert!(runtime
            .current_task()
            .expect("current task")
            .task_state_summary
            .contains("completed"));
        assert!(!runtime
            .pending_turns
            .iter()
            .any(|turn| turn.kind == PersonaTurnKind::LeadSummary));
    }

    #[test]
    fn active_lead_summary_absorbs_completion_without_extra_turn() {
        let mut runtime = PersonaRuntime::new();
        runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("lead turn");
        runtime.pop_next_turn();
        let active_summary = runtime.build_turn(
            PersonaSpeaker::Lead,
            PersonaRuntimeStage::FollowUp,
            PersonaTurnKind::LeadSummary,
        );
        assert_eq!(active_summary.kind, PersonaTurnKind::LeadSummary);

        assert!(runtime.absorb_completion_into_active_closure(
            Some(&active_summary),
            "main_runtime_status: completed"
        ));
        assert!(runtime.pop_next_turn().is_none());
        assert!(runtime
            .current_task()
            .expect("current task")
            .task_state_summary
            .contains("completed"));
    }

    #[test]
    fn active_member_turn_does_not_absorb_completion() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("lead turn");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));
        runtime.record_turn_result(
            &lead,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Planning, "peer-a")],
            ),
        );

        let active_member = runtime.pop_next_turn().expect("active member turn");
        assert_eq!(active_member.kind, PersonaTurnKind::MemberResponse);

        assert!(!runtime.absorb_completion_into_active_closure(
            Some(&active_member),
            "main_runtime_status: completed"
        ));
        let completion = runtime
            .enqueue_completion_for_mode(PersonaRuntimeMode::Full, "main_runtime_status: completed")
            .expect("completion turn");
        assert!(runtime
            .pending_turns
            .iter()
            .all(|turn| turn.stage == PersonaRuntimeStage::Completion));
        assert_eq!(runtime.pop_next_turn(), Some(completion));
    }

    #[test]
    fn progress_update_enqueues_lead_lifecycle_turn_inside_current_task() {
        let mut runtime = PersonaRuntime::new();
        let kickoff = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("kickoff");
        runtime.pop_next_turn();

        let progress = runtime
            .enqueue_progress_for_mode(
                PersonaRuntimeMode::Full,
                "main_runtime_status: tool_observation_recorded",
            )
            .expect("progress turn");

        assert_eq!(progress.task_id, kickoff.task_id);
        assert_eq!(progress.stage, PersonaRuntimeStage::Progress);
        assert_eq!(progress.kind, PersonaTurnKind::ProgressLead);
        assert_eq!(progress.speaker, PersonaSpeaker::Lead);
        assert!(progress
            .task_state_summary
            .contains("tool_observation_recorded"));
    }

    #[test]
    fn progress_lead_can_wake_requested_members_and_close_with_summary() {
        let mut runtime = PersonaRuntime::new();
        runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("kickoff");
        runtime.pop_next_turn();
        let progress = runtime
            .enqueue_progress_for_mode(PersonaRuntimeMode::Full, "main_runtime_status: repair")
            .expect("progress turn");
        assert_eq!(runtime.pop_next_turn(), Some(progress.clone()));

        runtime.record_turn_result(
            &progress,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Verification, "peer-a")],
            ),
        );
        let member = runtime.pop_next_turn().expect("member");
        assert_eq!(member.stage, PersonaRuntimeStage::Progress);
        assert_eq!(member.kind, PersonaTurnKind::MemberResponse);
        assert_eq!(member.speaker, PersonaSpeaker::Verification);

        runtime.record_turn_result(
            &member,
            outcome_with_peers(PersonaSpeaker::Verification, "body-b", &[]),
        );
        let summary = runtime.pop_next_turn().expect("summary");
        assert_eq!(summary.stage, PersonaRuntimeStage::Progress);
        assert_eq!(summary.kind, PersonaTurnKind::LeadSummary);
        assert_eq!(summary.speaker, PersonaSpeaker::Lead);
    }

    #[test]
    fn completion_clears_pending_progress_turns_and_runs_final_closure() {
        let mut runtime = PersonaRuntime::new();
        runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("kickoff");
        runtime.pop_next_turn();
        runtime
            .enqueue_progress_for_mode(PersonaRuntimeMode::Full, "main_runtime_status: repair")
            .expect("progress turn");

        let completion = runtime
            .enqueue_completion_for_mode(PersonaRuntimeMode::Full, "main_runtime_status: completed")
            .expect("completion turn");

        assert_eq!(runtime.pop_next_turn(), Some(completion));
        assert!(runtime.pop_next_turn().is_none());
    }

    #[test]
    fn kickoff_uses_separate_turn_queue_instead_of_batch_script() {
        let mut runtime = PersonaRuntime::new();

        let first = runtime.start_kickoff("task-a");

        assert_eq!(first.speaker, PersonaSpeaker::Lead);
        assert_eq!(first.kind, PersonaTurnKind::LeadSummon);
        assert_eq!(runtime.pop_next_turn(), Some(first.clone()));

        let expected = [
            (PersonaSpeaker::Planning, PersonaTurnKind::MemberResponse),
            (
                PersonaSpeaker::Implementation,
                PersonaTurnKind::MemberResponse,
            ),
        ];

        runtime.record_turn_result(
            &first,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[
                    (PersonaSpeaker::Planning, "peer-a"),
                    (PersonaSpeaker::Implementation, "peer-b"),
                ],
            ),
        );
        for (speaker, kind) in expected {
            let next = runtime.pop_next_turn().expect("next kickoff turn");
            assert_eq!(next.speaker, speaker);
            assert_eq!(next.kind, kind);
            runtime.record_turn_result(&next, PersonaTurnOutcome::Passed { speaker });
        }

        let summary = runtime.pop_next_turn().expect("kickoff summary");
        assert_eq!(summary.speaker, PersonaSpeaker::Lead);
        assert_eq!(summary.kind, PersonaTurnKind::LeadSummary);
        assert_eq!(summary.stage, PersonaRuntimeStage::Kickoff);

        assert!(runtime.pop_next_turn().is_none());
    }

    #[test]
    fn kickoff_without_member_requests_still_closes_with_lead_summary() {
        let mut runtime = PersonaRuntime::new();

        let lead = runtime.start_kickoff("task-a");
        runtime.pop_next_turn();
        runtime.record_turn_result(
            &lead,
            PersonaTurnOutcome::Spoken(message(PersonaSpeaker::Lead, "body-a")),
        );

        let summary = runtime.pop_next_turn().expect("kickoff summary");
        assert_eq!(summary.speaker, PersonaSpeaker::Lead);
        assert_eq!(summary.kind, PersonaTurnKind::LeadSummary);
        assert_eq!(summary.stage, PersonaRuntimeStage::Kickoff);
        assert!(!runtime.has_pending_turns());
    }

    #[test]
    fn turn_response_is_stored_in_speaker_history_only() {
        let mut runtime = PersonaRuntime::new();
        let lead_turn = runtime.start_kickoff("task-a");
        assert_eq!(runtime.pop_next_turn(), Some(lead_turn.clone()));

        runtime.record_turn_result(
            &lead_turn,
            PersonaTurnOutcome::Spoken(message(PersonaSpeaker::Lead, "body-a")),
        );

        let lead = runtime
            .sessions()
            .iter()
            .find(|session| session.speaker == PersonaSpeaker::Lead)
            .expect("lead session");
        let planning = runtime
            .sessions()
            .iter()
            .find(|session| session.speaker == PersonaSpeaker::Planning)
            .expect("planning session");

        assert_eq!(lead.history.len(), 1);
        assert!(planning.history.is_empty());
    }

    #[test]
    fn peer_messages_are_carried_into_later_turns() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime.start_kickoff("task-a");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));

        runtime.record_turn_result(
            &lead,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Planning, "peer-a")],
            ),
        );
        let peer_message = runtime.peer_messages()[0].clone();
        let next = runtime.pop_next_turn().expect("planning turn");

        assert_eq!(runtime.peer_messages(), &[peer_message.clone()]);
        assert_eq!(next.peer_messages, vec![peer_message]);
        assert_eq!(next.speaker, PersonaSpeaker::Planning);
    }

    #[test]
    fn mixed_spoken_and_passed_turns_store_only_visible_speaker_messages() {
        let mut runtime = PersonaRuntime::new();
        let lead = runtime.start_kickoff("task-a");
        assert_eq!(runtime.pop_next_turn(), Some(lead.clone()));

        runtime.record_turn_result(
            &lead,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[
                    (PersonaSpeaker::Planning, "peer-a"),
                    (PersonaSpeaker::Implementation, "peer-b"),
                ],
            ),
        );
        let planning = runtime.pop_next_turn().expect("planning turn");
        assert_eq!(planning.speaker, PersonaSpeaker::Planning);

        runtime.record_turn_result(
            &planning,
            PersonaTurnOutcome::Passed {
                speaker: PersonaSpeaker::Planning,
            },
        );
        let implementation = runtime.pop_next_turn().expect("implementation turn");
        assert_eq!(implementation.speaker, PersonaSpeaker::Implementation);

        runtime.record_turn_result(
            &implementation,
            PersonaTurnOutcome::Spoken(message(PersonaSpeaker::Implementation, "body-b")),
        );

        let lead = runtime
            .session_history(PersonaSpeaker::Lead)
            .expect("lead history");
        let planning = runtime
            .session_history(PersonaSpeaker::Planning)
            .expect("planning history");
        let implementation = runtime
            .session_history(PersonaSpeaker::Implementation)
            .expect("implementation history");

        assert_eq!(lead.len(), 1);
        assert!(planning.is_empty());
        assert_eq!(implementation.len(), 1);
        assert!(runtime.sessions().iter().all(|session| {
            session.speaker != PersonaSpeaker::Planning
                || session.status == PersonaSessionStatus::Idle
        }));
    }

    #[test]
    fn runtime_mode_off_clears_inflight_task_without_synthesizing_turns() {
        let mut runtime = PersonaRuntime::new();
        let first = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("full mode kickoff");
        assert_eq!(runtime.pop_next_turn(), Some(first.clone()));
        runtime.record_turn_result(
            &first,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Planning, "peer-a")],
            ),
        );
        let peer_message = runtime.peer_messages()[0].clone();
        assert_eq!(runtime.peer_messages(), &[peer_message]);

        let turn = runtime.start_kickoff_for_mode(PersonaRuntimeMode::Off, "task-b");

        assert!(turn.is_none());
        assert!(runtime.current_task().is_none());
        assert!(runtime.pop_next_turn().is_none());
        assert!(runtime.peer_messages().is_empty());
        assert!(runtime.sessions().iter().all(|session| {
            session.status == PersonaSessionStatus::Idle && session.history.is_empty()
        }));
    }

    #[test]
    fn starting_new_task_resets_old_session_history_peer_messages_and_pending_turns() {
        let mut runtime = PersonaRuntime::new();
        let first = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("first kickoff");
        runtime.pop_next_turn();
        runtime.record_turn_result(
            &first,
            outcome_with_peers(
                PersonaSpeaker::Lead,
                "body-a",
                &[(PersonaSpeaker::Planning, "peer-a")],
            ),
        );
        assert!(runtime.pop_next_turn().is_some());

        assert_eq!(first.task_id, "persona-task-0001");
        assert_eq!(
            runtime
                .session_history(PersonaSpeaker::Lead)
                .expect("lead history")
                .len(),
            1
        );

        let second = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-b")
            .expect("second kickoff");

        assert_eq!(second.task_id, "persona-task-0002");
        assert_eq!(
            runtime.current_task().map(|task| task.user_prompt.as_str()),
            Some("task-b")
        );
        assert!(runtime.peer_messages().is_empty());
        assert!(runtime
            .sessions()
            .iter()
            .all(|session| session.history.is_empty()));
        assert_eq!(runtime.pop_next_turn(), Some(second));
        assert!(runtime.pop_next_turn().is_none());
    }

    #[test]
    fn follow_up_keeps_task_history_and_starts_with_lead_only() {
        let mut runtime = PersonaRuntime::new();
        let kickoff = runtime
            .start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a")
            .expect("kickoff");
        runtime.pop_next_turn();
        runtime.record_turn_result(
            &kickoff,
            PersonaTurnOutcome::Spoken(message(PersonaSpeaker::Lead, "body-a")),
        );

        let follow_up = runtime
            .start_follow_up_for_mode(PersonaRuntimeMode::Full, "task-b", "status-a")
            .expect("follow-up lead");

        assert_eq!(follow_up.stage, PersonaRuntimeStage::FollowUp);
        assert_eq!(follow_up.kind, PersonaTurnKind::FollowUpLead);
        assert_eq!(follow_up.speaker, PersonaSpeaker::Lead);
        assert_eq!(follow_up.task_id, kickoff.task_id);
        assert_eq!(runtime.pop_next_turn(), Some(follow_up));
        assert_eq!(
            runtime
                .session_history(PersonaSpeaker::Lead)
                .expect("lead history")
                .len(),
            1
        );
    }

    #[test]
    fn follow_up_schedules_only_teammates_requested_by_peer_messages() {
        let mut runtime = PersonaRuntime::new();
        runtime.start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a");
        runtime.pop_next_turn();
        let follow_up = runtime
            .start_follow_up_for_mode(PersonaRuntimeMode::Full, "task-b", "status-a")
            .expect("follow-up lead");
        runtime.pop_next_turn();
        let result = parse_persona_turn_outcome(
            r#"{"speaker":"lead","decision":"speak","body":"body-a","peer_messages":[{"to":"verification","body":"peer-a"},{"to":"documentation","body":"peer-b"}]}"#,
            PersonaSpeaker::Lead,
        )
        .expect("lead follow-up");

        runtime.record_turn_result(&follow_up, result);

        let verification = runtime.pop_next_turn().expect("verification turn");
        let documentation = runtime.pop_next_turn().expect("documentation turn");
        assert_eq!(verification.speaker, PersonaSpeaker::Verification);
        assert_eq!(verification.stage, PersonaRuntimeStage::FollowUp);
        assert_eq!(documentation.speaker, PersonaSpeaker::Documentation);
        assert_eq!(documentation.stage, PersonaRuntimeStage::FollowUp);
        assert!(runtime.pop_next_turn().is_none());
    }

    #[test]
    fn follow_up_member_responses_end_with_lead_summary() {
        let mut runtime = PersonaRuntime::new();
        runtime.start_kickoff_for_mode(PersonaRuntimeMode::Full, "task-a");
        runtime.pop_next_turn();
        let follow_up = runtime
            .start_follow_up_for_mode(PersonaRuntimeMode::Full, "task-b", "status-a")
            .expect("follow-up lead");
        runtime.pop_next_turn();
        let result = parse_persona_turn_outcome(
            r#"{"speaker":"lead","decision":"speak","body":"body-a","peer_messages":[{"to":"documentation","body":"peer-a"}]}"#,
            PersonaSpeaker::Lead,
        )
        .expect("lead follow-up");
        runtime.record_turn_result(&follow_up, result);
        let documentation = runtime.pop_next_turn().expect("documentation turn");

        runtime.record_turn_result(
            &documentation,
            PersonaTurnOutcome::Spoken(message(PersonaSpeaker::Documentation, "body-b")),
        );
        let summary = runtime.pop_next_turn().expect("lead summary");

        assert_eq!(summary.speaker, PersonaSpeaker::Lead);
        assert_eq!(summary.kind, PersonaTurnKind::LeadSummary);
        assert_eq!(summary.stage, PersonaRuntimeStage::FollowUp);
    }

    #[test]
    fn runtime_labels_reserve_follow_up_and_completion_turns() {
        assert_eq!(PersonaRuntimeStage::FollowUp, PersonaRuntimeStage::FollowUp);
        assert_eq!(PersonaTurnKind::FollowUpLead, PersonaTurnKind::FollowUpLead);
        assert_eq!(
            PersonaRuntimeStage::Completion,
            PersonaRuntimeStage::Completion
        );
        assert_eq!(PersonaTurnKind::Completion, PersonaTurnKind::Completion);
    }
}
