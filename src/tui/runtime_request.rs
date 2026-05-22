use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::RuntimeConfig;
use crate::llm::{
    payload_ordering_contract_lines, response_boundary_contract_lines,
    tool_path_selection_contract_lines, Activity, LlmChatReport, LlmChatRequest, LlmMessage,
    LlmMessageRole, LlmProviderFactory, MessageHistory, PatchOperation, RuntimeDecision,
};
use crate::tool::ToolObservation;

const MIN_LOCAL_CHAT_TIMEOUT_MS: u64 = 180_000;
const RECENT_TOOL_OBSERVATION_LIMIT: usize = 4;

pub(super) struct ActivePlainRequest {
    pub(super) run_id: String,
    pub(super) turn_id: String,
    pub(super) prompt: String,
    pub(super) schema_message: LlmMessage,
    pub(super) user_message: LlmMessage,
    pub(super) history: MessageHistory,
    pub(super) receiver: Receiver<LlmChatReport>,
    pub(super) request_started_at: Instant,
    pub(super) persona_context_start: usize,
    pub(super) cancelled: bool,
    pub(super) repair_attempts: u16,
    pub(super) repair_source: Option<&'static str>,
    pub(super) tool_call_count: u16,
    pub(super) last_tool_signature: Option<String>,
    pub(super) executed_tool_signatures: Vec<String>,
    pub(super) executed_tool_records: Vec<ToolLoopExecutionRecord>,
    pub(super) same_tool_repeat_count: u16,
    pub(super) duplicate_redirect_count: u16,
    pub(super) last_tool_observation: Option<ToolLoopObservation>,
    pub(super) last_successful_tool_observation: Option<ToolLoopObservation>,
    pub(super) pending_change_after_read_target: Option<String>,
    pub(super) final_response_text: Option<String>,
}

impl ActivePlainRequest {
    pub(super) fn new(
        run_id: String,
        turn_id: String,
        prompt: String,
        schema_message: LlmMessage,
        user_message: LlmMessage,
        history: MessageHistory,
        receiver: Receiver<LlmChatReport>,
        persona_context_start: usize,
    ) -> Self {
        Self {
            run_id,
            turn_id,
            prompt,
            schema_message,
            user_message,
            history,
            receiver,
            request_started_at: Instant::now(),
            persona_context_start,
            cancelled: false,
            repair_attempts: 0,
            repair_source: None,
            tool_call_count: 0,
            last_tool_signature: None,
            executed_tool_signatures: Vec::new(),
            executed_tool_records: Vec::new(),
            same_tool_repeat_count: 0,
            duplicate_redirect_count: 0,
            last_tool_observation: None,
            last_successful_tool_observation: None,
            pending_change_after_read_target: None,
            final_response_text: None,
        }
    }

    pub(super) fn repair_attempts_for_source(&self, source: &'static str) -> u16 {
        if self.repair_source == Some(source) {
            self.repair_attempts
        } else {
            0
        }
    }

    pub(super) fn reset_repair_state(&mut self) {
        self.repair_attempts = 0;
        self.repair_source = None;
    }

    pub(super) fn reset_request_timer(&mut self) {
        self.request_started_at = Instant::now();
    }

    pub(super) fn request_timed_out(&self, timeout: Duration) -> bool {
        self.request_started_at.elapsed() >= timeout
    }

    pub(super) fn record_tool_execution(
        &mut self,
        signature: String,
        observation: &ToolObservation,
        observation_message: LlmMessage,
    ) {
        let loop_observation = ToolLoopObservation::from(observation);
        self.executed_tool_signatures.push(signature.clone());
        self.executed_tool_records.push(ToolLoopExecutionRecord {
            signature,
            observation: loop_observation.clone(),
            observation_message,
        });
        self.duplicate_redirect_count = 0;
        if loop_observation.status == "succeeded"
            && loop_observation.error_kind.is_none()
            && (self.last_successful_tool_observation.is_none()
                || !loop_observation.preview.is_empty())
        {
            self.last_successful_tool_observation = Some(loop_observation.clone());
        }
        if loop_observation.tool_name == "apply_patch"
            && loop_observation.status == "succeeded"
            && loop_observation.error_kind.is_none()
        {
            self.pending_change_after_read_target = None;
        }
        self.last_tool_observation = Some(loop_observation);
    }

    pub(super) fn record_final_decision(&mut self, decision: &RuntimeDecision) {
        match decision {
            RuntimeDecision::Answer {
                summary,
                payload_body,
            } => {
                let text = match payload_body {
                    Some(payload) if !payload.trim().is_empty() => {
                        format!("{summary}\n\n{payload}")
                    }
                    _ => summary.clone(),
                };
                self.final_response_text = Some(text);
            }
            RuntimeDecision::Clarify { message, .. } | RuntimeDecision::Blocked { message, .. } => {
                self.final_response_text = Some(message.clone());
            }
            RuntimeDecision::ToolCandidatePending { .. }
            | RuntimeDecision::ApprovalNeeded { .. } => {}
        }
    }

    pub(super) fn repeat_redirect_for_tool_candidate(
        &self,
        signature: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Option<(ToolLoopRepeatRedirect, &ToolLoopExecutionRecord)> {
        let exact_record = self
            .executed_tool_records
            .iter()
            .rev()
            .find(|record| record.signature == signature)
            .and_then(|record| {
                record
                    .observation
                    .repeat_redirect()
                    .map(|redirect| (redirect, record))
            });
        if exact_record.is_some() {
            return exact_record;
        }

        let target_raw = repeat_failure_candidate_target(arguments)?;
        self.executed_tool_records.iter().rev().find_map(|record| {
            if record
                .observation
                .matches_repeat_failure_target(tool_name, target_raw)
            {
                Some((ToolLoopRepeatRedirect::FailedDuplicate, record))
            } else {
                None
            }
        })
    }

    pub(super) fn duplicate_redirect_count_for_signature(&self, signature: &str) -> u16 {
        if self.last_tool_signature.as_deref() == Some(signature) {
            self.duplicate_redirect_count
        } else {
            0
        }
    }
}

pub(super) fn effective_chat_timeout(config: &RuntimeConfig) -> Duration {
    Duration::from_millis(
        u64::from(config.limits.command_timeout_ms).max(MIN_LOCAL_CHAT_TIMEOUT_MS),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ToolLoopExecutionRecord {
    pub(super) signature: String,
    pub(super) observation: ToolLoopObservation,
    pub(super) observation_message: LlmMessage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ToolLoopRepeatRedirect {
    SettledDuplicate,
    TruncatedContinuation,
    FailedDuplicate,
}

impl ToolLoopRepeatRedirect {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::SettledDuplicate => "settled_duplicate_tool_candidate",
            Self::TruncatedContinuation => "truncated_duplicate_tool_candidate",
            Self::FailedDuplicate => "failed_duplicate_tool_candidate",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ToolLoopObservation {
    pub(super) tool_name: String,
    pub(super) target_raw: Option<String>,
    pub(super) status: &'static str,
    pub(super) error_kind: Option<&'static str>,
    pub(super) truncated: bool,
    pub(super) source_truncated: bool,
    pub(super) preview_truncated: bool,
    pub(super) has_next_range_hint: bool,
    pub(super) preview: Vec<String>,
}

impl From<&ToolObservation> for ToolLoopObservation {
    fn from(observation: &ToolObservation) -> Self {
        Self {
            tool_name: observation.tool_name.clone(),
            target_raw: observation.target_raw.clone(),
            status: observation.status.as_str(),
            error_kind: observation.error_kind.map(|kind| kind.as_str()),
            truncated: observation.truncated,
            source_truncated: observation.source_truncated,
            preview_truncated: observation.preview_truncated,
            has_next_range_hint: observation.next_range_hint.is_some(),
            preview: observation.preview.clone(),
        }
    }
}

impl ToolLoopObservation {
    fn repeat_redirect(&self) -> Option<ToolLoopRepeatRedirect> {
        if self.is_settled_success() {
            Some(ToolLoopRepeatRedirect::SettledDuplicate)
        } else if self.is_truncated_continuation() {
            Some(ToolLoopRepeatRedirect::TruncatedContinuation)
        } else if self.is_failed() {
            Some(ToolLoopRepeatRedirect::FailedDuplicate)
        } else {
            None
        }
    }

    fn is_settled_success(&self) -> bool {
        self.status == "succeeded"
            && self.error_kind.is_none()
            && !self.truncated
            && !self.source_truncated
            && !self.preview_truncated
            && !self.has_next_range_hint
    }

    fn is_truncated_continuation(&self) -> bool {
        self.status == "succeeded"
            && self.error_kind.is_none()
            && (self.truncated
                || self.source_truncated
                || self.preview_truncated
                || self.has_next_range_hint)
    }

    fn is_failed(&self) -> bool {
        self.status == "failed" || self.error_kind.is_some()
    }

    fn matches_repeat_failure_target(&self, tool_name: &str, target_raw: &str) -> bool {
        self.tool_name == tool_name
            && self.target_raw.as_deref() == Some(target_raw)
            && self.is_repeat_failure_target_invariant()
    }

    fn is_repeat_failure_target_invariant(&self) -> bool {
        matches!(
            self.error_kind,
            Some("path_not_found" | "not_a_file" | "not_a_directory" | "path_outside_workspace")
        )
    }
}

fn repeat_failure_candidate_target(arguments: &serde_json::Value) -> Option<&str> {
    arguments.get("path").and_then(serde_json::Value::as_str)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ToolLoopLimitDiagnosis {
    SameToolSignatureRepeat,
    TruncatedObservationContinuation,
    PathFailureRecovery,
    FailedObservationRecovery,
    MaxToolCallsWithoutPendingObservationSignal,
}

impl ToolLoopLimitDiagnosis {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::SameToolSignatureRepeat => "same_tool_signature_repeat",
            Self::TruncatedObservationContinuation => "truncated_observation_continuation",
            Self::PathFailureRecovery => "path_failure_recovery",
            Self::FailedObservationRecovery => "failed_observation_recovery",
            Self::MaxToolCallsWithoutPendingObservationSignal => {
                "max_tool_calls_without_pending_observation_signal"
            }
        }
    }
}

pub(super) fn diagnose_tool_loop_limit(
    reason: &str,
    last_observation: Option<&ToolLoopObservation>,
) -> ToolLoopLimitDiagnosis {
    if reason == "max_same_tool_repeats" {
        return ToolLoopLimitDiagnosis::SameToolSignatureRepeat;
    }

    let Some(observation) = last_observation else {
        return ToolLoopLimitDiagnosis::MaxToolCallsWithoutPendingObservationSignal;
    };

    if observation.truncated || observation.has_next_range_hint {
        return ToolLoopLimitDiagnosis::TruncatedObservationContinuation;
    }

    match observation.error_kind {
        Some("path_not_found" | "not_a_file" | "not_a_directory") => {
            ToolLoopLimitDiagnosis::PathFailureRecovery
        }
        Some(_) => ToolLoopLimitDiagnosis::FailedObservationRecovery,
        None => ToolLoopLimitDiagnosis::MaxToolCallsWithoutPendingObservationSignal,
    }
}

pub(super) fn settled_duplicate_final_decision(
    active: &ActivePlainRequest,
    execution_record: &ToolLoopExecutionRecord,
) -> RuntimeDecision {
    let observation = best_settled_duplicate_observation(active, execution_record);
    let target = observation.target_raw.as_deref().unwrap_or("-");
    if read_recovery_needs_file_content(active) && observation.tool_name != "read_file" {
        return read_recovery_requires_content_decision(observation);
    }
    if observation.preview.is_empty() {
        return RuntimeDecision::Blocked {
            message: format!(
                "동일한 도구 후보가 반복됐지만 기존 관측에 답변 근거가 없습니다. tool: {}, target: {}",
                observation.tool_name, target
            ),
            reason: "settled_duplicate_without_evidence".to_owned(),
        };
    }

    RuntimeDecision::Answer {
        summary: "동일한 도구 후보가 이미 성공한 관측 이후 반복되어 추가 도구 호출 없이 기존 관측으로 마무리합니다."
            .to_owned(),
        payload_body: Some(observation_payload(observation)),
    }
}

pub(super) fn apply_final_answer_evidence_boundary(
    active: &ActivePlainRequest,
    decision: RuntimeDecision,
) -> RuntimeDecision {
    if let Some(recovery_decision) = read_recovery_next_tool_decision(active, &decision) {
        return recovery_decision;
    }

    if !matches!(decision, RuntimeDecision::Answer { .. })
        || active.executed_tool_records.is_empty()
    {
        return decision;
    }

    if read_recovery_needs_file_content(active) {
        let observation = active
            .last_tool_observation
            .as_ref()
            .or(active.last_successful_tool_observation.as_ref());
        return match observation {
            Some(observation) => read_recovery_requires_content_decision(observation),
            None => RuntimeDecision::Blocked {
                message: "파일 경로 복구가 끝나지 않아 파일 내용 근거 없이 답변할 수 없습니다."
                    .to_owned(),
                reason: "answer_without_file_content_after_read_recovery".to_owned(),
            },
        };
    }

    if let Some(observation) = unresolved_failed_change_observation(active) {
        return RuntimeDecision::Blocked {
            message: format!(
                "파일 변경 도구가 실패했고 이후 성공한 변경 관측이 없어 완료 답변을 할 수 없습니다. next_required_tool: apply_patch, failed_tool: {}, target: {}",
                observation.tool_name,
                observation.target_raw.as_deref().unwrap_or("-")
            ),
            reason: "answer_after_failed_change_without_success".to_owned(),
        };
    }

    if let Some(target) = active.pending_change_after_read_target.as_deref() {
        return RuntimeDecision::Blocked {
            message: format!(
                "변경 전 읽기는 성공했지만 아직 파일 변경 관측이 없어 완료 답변을 할 수 없습니다. next_required_tool: apply_patch, target: {target}"
            ),
            reason: "answer_after_prerequisite_read_without_change".to_owned(),
        };
    }

    if has_informative_successful_observation(active) {
        return decision;
    }

    let observation = active
        .last_tool_observation
        .as_ref()
        .map(|observation| {
            format!(
                "last_tool: {} target: {} status: {}",
                observation.tool_name,
                observation.target_raw.as_deref().unwrap_or("-"),
                observation.status,
            )
        })
        .unwrap_or_else(|| "last_tool: none".to_owned());

    RuntimeDecision::Blocked {
        message: format!("도구를 실행했지만 답변 근거가 되는 관측 내용이 없습니다. {observation}"),
        reason: "answer_without_observation_evidence".to_owned(),
    }
}

pub(super) fn apply_change_evidence_boundary(
    active: &ActivePlainRequest,
    decision: RuntimeDecision,
) -> RuntimeDecision {
    let Some(target_path) = change_target_requiring_read(active, &decision) else {
        return decision;
    };

    RuntimeDecision::ToolCandidatePending {
        activity: Activity::Explore,
        tool_name: "read_file".to_owned(),
        arguments: serde_json::json!({
            "path": target_path,
            "start_line": 1,
            "max_lines": 120
        }),
        summary: "Read the existing target before applying an update or delete patch.".to_owned(),
    }
}

pub(super) fn change_target_requiring_read(
    active: &ActivePlainRequest,
    decision: &RuntimeDecision,
) -> Option<String> {
    let RuntimeDecision::ApprovalNeeded {
        tool_name,
        change_preview: Some(change_preview),
        ..
    } = decision
    else {
        return None;
    };

    if tool_name != "apply_patch"
        || !matches!(
            change_preview.operation,
            PatchOperation::Update | PatchOperation::Delete
        )
    {
        return None;
    }

    let failed_change_index =
        latest_failed_change_index_for_target(active, &change_preview.target_path);
    if !has_successful_read_for_target_after(
        active,
        &change_preview.target_path,
        failed_change_index,
    ) {
        return Some(change_preview.target_path.clone());
    }

    None
}

fn latest_failed_change_index_for_target(
    active: &ActivePlainRequest,
    target_path: &str,
) -> Option<usize> {
    active
        .executed_tool_records
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, record)| {
            let observation = &record.observation;
            if observation.tool_name == "apply_patch"
                && observation.is_failed()
                && observation.target_raw.as_deref() == Some(target_path)
            {
                Some(index)
            } else {
                None
            }
        })
}

fn has_successful_read_for_target_after(
    active: &ActivePlainRequest,
    target_path: &str,
    after_index: Option<usize>,
) -> bool {
    let start = after_index.map_or(0, |index| index.saturating_add(1));
    active
        .executed_tool_records
        .iter()
        .skip(start)
        .any(|record| {
            let observation = &record.observation;
            observation.tool_name == "read_file"
                && observation.status == "succeeded"
                && observation.error_kind.is_none()
                && observation.target_raw.as_deref() == Some(target_path)
                && !observation.preview.is_empty()
        })
}

fn read_recovery_next_tool_decision(
    active: &ActivePlainRequest,
    decision: &RuntimeDecision,
) -> Option<RuntimeDecision> {
    if matches!(
        decision,
        RuntimeDecision::ToolCandidatePending { .. } | RuntimeDecision::ApprovalNeeded { .. }
    ) {
        return None;
    }

    let state = latest_failed_read_recovery_state(active)?;
    if state.read_succeeded_after_failure {
        return None;
    }

    if let Some(observation) = active.last_tool_observation.as_ref() {
        if observation.tool_name == "read_file" && observation.is_failed() {
            if let Some(decision) = read_file_candidate_decision(observation, &state.failed_target)
            {
                return Some(decision);
            }
        }
        if state.discovery_succeeded_after_failure
            && is_discovery_observation(observation)
            && observation.status == "succeeded"
            && observation.error_kind.is_none()
        {
            if let Some(decision) = read_file_candidate_decision(observation, &state.failed_target)
            {
                return Some(decision);
            }
        }
    }

    if !state.discovery_succeeded_after_failure {
        return Some(RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name: "list_files".to_owned(),
            arguments: serde_json::json!({
                "path": ".",
                "max_depth": 3,
                "max_entries": 160
            }),
            summary: "Discover workspace file candidates after the direct read path failed."
                .to_owned(),
        });
    }

    None
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FailedReadRecoveryState {
    failed_target: String,
    discovery_succeeded_after_failure: bool,
    read_succeeded_after_failure: bool,
}

fn latest_failed_read_recovery_state(
    active: &ActivePlainRequest,
) -> Option<FailedReadRecoveryState> {
    let failed_index = active
        .executed_tool_records
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, record)| {
            let observation = &record.observation;
            if observation.tool_name == "read_file"
                && observation.is_failed()
                && matches!(
                    observation.error_kind,
                    Some("path_not_found" | "not_a_file" | "not_a_directory")
                )
            {
                observation
                    .target_raw
                    .as_ref()
                    .map(|target| (index, target.clone()))
            } else {
                None
            }
        })?;

    let mut state = FailedReadRecoveryState {
        failed_target: failed_index.1,
        discovery_succeeded_after_failure: false,
        read_succeeded_after_failure: false,
    };

    for record in active.executed_tool_records.iter().skip(failed_index.0 + 1) {
        let observation = &record.observation;
        if observation.tool_name == "read_file"
            && observation.status == "succeeded"
            && observation.error_kind.is_none()
            && !observation.preview.is_empty()
        {
            state.read_succeeded_after_failure = true;
        } else if is_discovery_observation(observation)
            && observation.status == "succeeded"
            && observation.error_kind.is_none()
            && !observation.preview.is_empty()
        {
            state.discovery_succeeded_after_failure = true;
        }
    }

    Some(state)
}

fn read_file_candidate_decision(
    observation: &ToolLoopObservation,
    failed_target: &str,
) -> Option<RuntimeDecision> {
    let candidates = read_file_candidates_from_observation(observation, failed_target);
    match candidates.as_slice() {
        [] => None,
        [path] => Some(RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name: "read_file".to_owned(),
            arguments: serde_json::json!({
                "path": path,
                "start_line": 1,
                "max_lines": 120
            }),
            summary: "Read the discovered file candidate before answering from file contents."
                .to_owned(),
        }),
        _ => Some(RuntimeDecision::Clarify {
            message: format!(
                "같은 요청에 맞는 파일 후보가 여러 개라 임의로 선택하지 않습니다. 읽을 파일을 하나로 지정해 주세요.\n{}",
                candidates
                    .iter()
                    .map(|path| format!("- {path}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            reason: "multiple_read_file_candidates".to_owned(),
        }),
    }
}

fn read_file_candidates_from_observation(
    observation: &ToolLoopObservation,
    failed_target: &str,
) -> Vec<String> {
    let mut matches = Vec::new();
    for line in &observation.preview {
        let candidate = match observation.tool_name.as_str() {
            "list_files" => list_files_candidate_path(line),
            "search_text" => search_text_candidate_path(line),
            "read_file" => read_file_failure_candidate_path(line),
            _ => None,
        };
        let Some(candidate) = candidate else {
            continue;
        };
        if candidate_matches_failed_read_target(&candidate, failed_target)
            && !matches.iter().any(|existing| existing == &candidate)
        {
            matches.push(candidate);
        }
    }

    matches
}

fn candidate_matches_failed_read_target(candidate: &str, failed_target: &str) -> bool {
    if candidate == failed_target {
        return true;
    }

    let failed_name = failed_target
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(failed_target);
    candidate
        .rsplit('/')
        .next()
        .map(|name| name == failed_name)
        .unwrap_or(false)
}

pub(super) fn duplicate_approval_final_decision(
    active: &ActivePlainRequest,
    approval_signature: &str,
) -> Option<RuntimeDecision> {
    let record = active
        .executed_tool_records
        .iter()
        .rev()
        .find(|record| record.signature == approval_signature)?;

    if record.observation.is_failed() {
        return Some(RuntimeDecision::Blocked {
            message: format!(
                "승인이 필요한 동일한 도구 후보가 이미 실패했습니다. 같은 후보를 다시 승인하지 않습니다. next_required_tool: different_tool_candidate, tool: {}, target: {}, error_kind: {}",
                record.observation.tool_name,
                record.observation.target_raw.as_deref().unwrap_or("-"),
                record.observation.error_kind.unwrap_or("-")
            ),
            reason: "duplicate_approval_failed_observation".to_owned(),
        });
    }

    if record.observation.preview.is_empty() {
        return Some(RuntimeDecision::Blocked {
            message: format!(
                "승인이 필요한 동일한 도구 후보가 이미 실행됐지만 기존 관측에 답변 근거가 없습니다. tool: {}, target: {}",
                record.observation.tool_name,
                record.observation.target_raw.as_deref().unwrap_or("-")
            ),
            reason: "duplicate_approval_without_evidence".to_owned(),
        });
    }

    Some(RuntimeDecision::Answer {
        summary: "승인이 필요한 동일한 도구 후보가 이미 실행되어 추가 승인 없이 기존 관측으로 마무리합니다."
            .to_owned(),
        payload_body: Some(observation_payload(&record.observation)),
    })
}

pub(super) fn duplicate_approval_repeat_redirect(
    active: &ActivePlainRequest,
    approval_signature: &str,
) -> Option<(ToolLoopRepeatRedirect, ToolLoopExecutionRecord)> {
    active
        .executed_tool_records
        .iter()
        .rev()
        .find(|record| record.signature == approval_signature)
        .and_then(|record| {
            record
                .observation
                .repeat_redirect()
                .map(|redirect| (redirect, record.clone()))
        })
}

pub(super) fn repeated_completed_add_change_final_decision(
    active: &ActivePlainRequest,
    decision: &RuntimeDecision,
) -> Option<RuntimeDecision> {
    let RuntimeDecision::ApprovalNeeded {
        activity: Activity::Change,
        tool_name,
        change_preview: Some(change_preview),
        ..
    } = decision
    else {
        return None;
    };
    if tool_name != "apply_patch" || change_preview.operation != PatchOperation::Add {
        return None;
    }

    let record = active.executed_tool_records.iter().rev().find(|record| {
        record.observation.tool_name == "apply_patch"
            && record.observation.target_raw.as_deref() == Some(change_preview.target_path.as_str())
            && record.observation.status == "succeeded"
            && record.observation.error_kind.is_none()
            && !record.observation.preview.is_empty()
    })?;

    Some(RuntimeDecision::Answer {
        summary: format!("파일 생성이 완료되었습니다: {}", change_preview.target_path),
        payload_body: Some(observation_payload(&record.observation)),
    })
}

fn has_informative_successful_observation(active: &ActivePlainRequest) -> bool {
    active.executed_tool_records.iter().any(|record| {
        record.observation.status == "succeeded"
            && record.observation.error_kind.is_none()
            && !record.observation.preview.is_empty()
    })
}

fn unresolved_failed_change_observation(
    active: &ActivePlainRequest,
) -> Option<&ToolLoopObservation> {
    let mut unresolved = Vec::new();
    for record in &active.executed_tool_records {
        let observation = &record.observation;
        if observation.tool_name != "apply_patch" {
            continue;
        }
        if observation.status == "succeeded" && observation.error_kind.is_none() {
            if let Some(target) = observation.target_raw.as_deref() {
                unresolved.retain(|failed: &&ToolLoopObservation| {
                    failed.target_raw.as_deref() != Some(target)
                });
            }
        } else if observation.is_failed() {
            unresolved.push(observation);
        }
    }
    unresolved.pop()
}

fn best_settled_duplicate_observation<'a>(
    active: &'a ActivePlainRequest,
    execution_record: &'a ToolLoopExecutionRecord,
) -> &'a ToolLoopObservation {
    if !execution_record.observation.preview.is_empty() {
        return &execution_record.observation;
    }

    active
        .executed_tool_records
        .iter()
        .rev()
        .map(|record| &record.observation)
        .find(|observation| {
            observation.status == "succeeded"
                && observation.error_kind.is_none()
                && !observation.preview.is_empty()
        })
        .unwrap_or(&execution_record.observation)
}

fn read_recovery_needs_file_content(active: &ActivePlainRequest) -> bool {
    let mut failed_read_needs_recovery = false;
    let mut discovery_after_failed_read = false;
    let mut read_success_after_failed_read = false;

    for record in &active.executed_tool_records {
        let observation = &record.observation;
        if observation.tool_name == "read_file"
            && observation.is_failed()
            && matches!(
                observation.error_kind,
                Some("path_not_found" | "not_a_file" | "not_a_directory")
            )
        {
            failed_read_needs_recovery = true;
            discovery_after_failed_read = false;
            read_success_after_failed_read = false;
            continue;
        }

        if !failed_read_needs_recovery {
            continue;
        }

        if observation.tool_name == "read_file"
            && observation.status == "succeeded"
            && observation.error_kind.is_none()
            && !observation.preview.is_empty()
        {
            read_success_after_failed_read = true;
        } else if is_discovery_observation(observation)
            && observation.status == "succeeded"
            && observation.error_kind.is_none()
            && !observation.preview.is_empty()
        {
            discovery_after_failed_read = true;
        }
    }

    failed_read_needs_recovery && discovery_after_failed_read && !read_success_after_failed_read
}

fn is_discovery_observation(observation: &ToolLoopObservation) -> bool {
    matches!(observation.tool_name.as_str(), "list_files" | "search_text")
}

fn read_recovery_requires_content_decision(observation: &ToolLoopObservation) -> RuntimeDecision {
    RuntimeDecision::Blocked {
        message: format!(
            "파일 경로 후보는 발견했지만 파일 내용을 아직 읽지 않았습니다. 발견 관측만으로 파일 내용 기반 답변을 할 수 없습니다. next_required_tool: read_file, last_tool: {}, target: {}",
            observation.tool_name,
            observation.target_raw.as_deref().unwrap_or("-")
        ),
        reason: "answer_without_file_content_after_read_recovery".to_owned(),
    }
}

fn observation_payload(observation: &ToolLoopObservation) -> String {
    let target = observation.target_raw.as_deref().unwrap_or("-");
    let mut payload_lines = vec![
        format!("tool: {}", observation.tool_name),
        format!("target: {target}"),
        format!("status: {}", observation.status),
        "preview:".to_owned(),
    ];
    payload_lines.extend(observation.preview.iter().take(80).cloned());
    if observation.preview.len() > 80 {
        payload_lines.push("...".to_owned());
    }
    payload_lines.join("\n")
}

pub(super) fn completed_tool_fallback_final_decision(
    active: &ActivePlainRequest,
    fallback_reason: &str,
) -> Option<RuntimeDecision> {
    if active.executed_tool_records.is_empty()
        || read_recovery_needs_file_content(active)
        || unresolved_failed_change_observation(active).is_some()
        || active.pending_change_after_read_target.is_some()
        || fallback_reason == "payload_validation_failed"
    {
        return None;
    }

    let observation = active
        .last_successful_tool_observation
        .as_ref()
        .filter(|observation| {
            observation.status == "succeeded"
                && observation.error_kind.is_none()
                && !observation.preview.is_empty()
        })?;

    Some(RuntimeDecision::Answer {
        summary: "도구 실행은 성공했지만 모델 후속 응답이 유효한 계약으로 마무리되지 않아 성공한 관측으로 종료합니다."
            .to_owned(),
        payload_body: Some(observation_payload(observation)),
    })
}

pub(super) fn conversation_task_context_summary(active: &ActivePlainRequest) -> String {
    let mut lines = vec![
        format!("previous_run_id: {}", active.run_id),
        "previous_task_context: use this only when the current user message depends on the previous task.".to_owned(),
        "path_grounding_rule: when the current message refers to prior evidence, a prior file, or a prior result, use exact paths from previous_final_response or successful_observation_preview; do not invent conventional path candidates.".to_owned(),
    ];

    if let Some(text) = active.final_response_text.as_deref() {
        lines.push("previous_final_response:".to_owned());
        lines.extend(truncated_context_lines(text, 40));
    } else {
        lines.push("previous_final_response: none".to_owned());
    }

    if let Some(observation) = active.last_successful_tool_observation.as_ref() {
        lines.push(format!(
            "successful_observation: tool_name={} target_raw={}",
            observation.tool_name,
            observation.target_raw.as_deref().unwrap_or("-"),
        ));
        if observation.preview.is_empty() {
            lines.push("successful_observation_preview: none".to_owned());
        } else {
            lines.push("successful_observation_preview:".to_owned());
            lines.extend(observation.preview.iter().take(80).cloned());
            if observation.preview.len() > 80 {
                lines.push("... preview omitted".to_owned());
            }
        }
    } else {
        lines.push("successful_observation: none".to_owned());
    }

    lines.join("\n")
}

fn truncated_context_lines(text: &str, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().take(max_lines).map(str::to_owned).collect();
    if text.lines().count() > max_lines {
        lines.push("... response omitted".to_owned());
    }
    lines
}

pub(super) fn next_run_id(next_run_index: &mut u64) -> String {
    let run_id = format!("run-{number:04}", number = *next_run_index);
    *next_run_index += 1;
    run_id
}

pub(super) fn spawn_chat_request(
    config: &RuntimeConfig,
    messages: Vec<LlmMessage>,
) -> Receiver<LlmChatReport> {
    let (sender, receiver) = mpsc::channel();
    let config = config.clone();
    thread::spawn(move || {
        let provider = LlmProviderFactory::from_config(&config);
        let report = provider.send_chat(LlmChatRequest { messages });
        let _ = sender.send(report);
    });
    receiver
}

pub(super) fn repair_request_messages(history: &MessageHistory) -> Vec<LlmMessage> {
    history
        .for_request(None)
        .into_iter()
        .filter(|message| message.role != LlmMessageRole::Assistant)
        .collect()
}

pub(super) fn tool_loop_request_messages(
    schema_message: &LlmMessage,
    user_message: &LlmMessage,
    observation_message: &LlmMessage,
    executed_tool_records: &[ToolLoopExecutionRecord],
    executed_tool_signatures: &[String],
    next_turn_id: &str,
) -> Vec<LlmMessage> {
    let mut instruction_lines =
        vec!["Continue from the latest AHREUM_TOOL_OBSERVATION.".to_owned()];
    instruction_lines.extend(
        response_boundary_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        payload_ordering_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        tool_path_selection_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        [
            "If the observation is enough evidence for the user goal, return exactly one answer response with activity None.",
            "If the latest observation is a succeeded Change or Execute tool and the user did not ask for another distinct action, return exactly one answer response with activity None.",
            "After a succeeded apply_patch Add File observation, do not request another Add File for the same target; summarize the completed change.",
            "After a succeeded run_command observation, do not request the same command again unless the observation is truncated or the user asked for a separate command.",
            "After a succeeded read_file observation, do not request the same read_file again. If the user goal still requires a file change or command execution, use the read evidence to request the next Change or Execute tool candidate.",
            "Request exactly one next tool only when more workspace evidence is required.",
            "Use path \".\" for the workspace root. Empty path strings are invalid.",
            "Use search_text when the next missing evidence is a symbol, implementation, registry entry, tool mapping, or configuration key location.",
            "Use read_file when the next missing evidence is content from a known workspace file.",
            "Use list_files when the next missing evidence is current workspace, directory structure, or a filename/path candidate after a direct path failed.",
            "When file contents or values are still required and list_files or search_text found candidate paths, do not answer from discovery alone; request read_file for the best candidate path from the observation.",
            "If the latest observation failed, do not treat its target as read or analyzed evidence.",
            "If the latest apply_patch observation failed and the user goal still requires a file change, request a repaired Change candidate instead of answering as if the file changed.",
            "If apply_patch failed with error_kind target_already_exists and no successful read_file observation in this request provides the target contents, request read_file for target_raw before another Change candidate.",
            "After target_already_exists is followed by a successful read_file and the user still requires a change, request an Update File patch for that existing target.",
            "If read_file failed with path_not_found and the user still asks for file-backed information, resolve the exact path with prior successful evidence, list_files, or search_text before blocking.",
            "If a failed read_file is followed by list_files or search_text that identifies a candidate file, request read_file for that candidate before answering file contents, settings, dependencies, fields, or other file-backed details.",
            "If a path failure cannot be resolved by a bounded next tool, return an answer or blocked response that reports the unresolved path instead of inventing file contents.",
            "If the latest observation has next_range_hint and more content is needed, follow that hint.",
            "Do not repeat an already executed tool candidate with the same tool_name and arguments.",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    append_read_candidate_guidance(&mut instruction_lines, &observation_message.content);
    instruction_lines.extend(tool_loop_state_lines(executed_tool_signatures));

    let mut messages = vec![schema_message.clone(), user_message.clone()];
    messages.extend(recent_tool_observation_messages_with_latest(
        executed_tool_records,
        observation_message,
    ));
    messages.push(LlmMessage {
        turn_id: next_turn_id.to_owned(),
        role: LlmMessageRole::System,
        visibility: observation_message.visibility,
        content: instruction_lines.join("\n"),
    });
    messages
}

pub(super) fn tool_repeat_answer_request_messages(
    schema_message: &LlmMessage,
    user_message: &LlmMessage,
    execution_record: &ToolLoopExecutionRecord,
    executed_tool_records: &[ToolLoopExecutionRecord],
    executed_tool_signatures: &[String],
    next_turn_id: &str,
) -> Vec<LlmMessage> {
    let mut instruction_lines = vec![
        "The latest assistant response proposed a tool candidate that has already been executed successfully.".to_owned(),
        format!("duplicate_tool_candidate: {}", execution_record.signature),
        "That previous observation succeeded, was not truncated, and provided no next_range_hint.".to_owned(),
    ];
    instruction_lines.extend(
        response_boundary_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        payload_ordering_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        [
            "Do not call the duplicate tool candidate again.",
            "Return exactly one answer response with activity None from the existing observation when it is enough evidence for the user goal.",
            "If the duplicate successful observation is read_file and the user goal still requires a file change or command execution, request the next non-duplicate Change or Execute tool candidate instead of answering or re-reading.",
            "When file contents or values are still required and the duplicate successful observation is list_files or search_text, do not answer from discovery alone; request read_file for a candidate path from that observation.",
            "If the existing successful observation is only path discovery after a failed read_file, request read_file for the discovered candidate before answering file-backed details.",
            "Request a tool only if a different tool candidate is required for missing evidence.",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    append_read_candidate_guidance(
        &mut instruction_lines,
        &execution_record.observation_message.content,
    );
    instruction_lines.extend(tool_loop_state_lines(executed_tool_signatures));

    let mut messages = vec![schema_message.clone(), user_message.clone()];
    messages.extend(recent_tool_observation_messages(
        executed_tool_records,
        execution_record,
    ));
    messages.push(LlmMessage {
        turn_id: next_turn_id.to_owned(),
        role: LlmMessageRole::System,
        visibility: execution_record.observation_message.visibility,
        content: instruction_lines.join("\n"),
    });
    messages
}

pub(super) fn tool_repeat_continuation_request_messages(
    schema_message: &LlmMessage,
    user_message: &LlmMessage,
    execution_record: &ToolLoopExecutionRecord,
    executed_tool_records: &[ToolLoopExecutionRecord],
    executed_tool_signatures: &[String],
    next_turn_id: &str,
) -> Vec<LlmMessage> {
    let mut instruction_lines = vec![
        "The latest assistant response repeated a tool candidate whose previous observation was truncated or had next_range_hint.".to_owned(),
        format!("duplicate_tool_candidate: {}", execution_record.signature),
        "Repeating the same tool_name and arguments cannot provide new workspace evidence.".to_owned(),
    ];
    instruction_lines.extend(
        response_boundary_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        payload_ordering_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        tool_path_selection_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        [
            "Do not call the duplicate tool candidate again.",
            "If the existing observation is enough evidence for the user goal, return exactly one answer response with activity None.",
            "If more content from the same file is required and the observation includes next_range_hint, request read_file with that next range instead of repeating the same range.",
            "If a directory listing is truncated and there is no next_range_hint, request a narrower path or search_text for the missing evidence instead of repeating the same directory.",
            "If different evidence is required, request exactly one different tool candidate.",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    instruction_lines.extend(tool_loop_state_lines(executed_tool_signatures));

    let mut messages = vec![schema_message.clone(), user_message.clone()];
    messages.extend(recent_tool_observation_messages(
        executed_tool_records,
        execution_record,
    ));
    messages.push(LlmMessage {
        turn_id: next_turn_id.to_owned(),
        role: LlmMessageRole::System,
        visibility: execution_record.observation_message.visibility,
        content: instruction_lines.join("\n"),
    });
    messages
}

pub(super) fn tool_repeat_failure_request_messages(
    schema_message: &LlmMessage,
    user_message: &LlmMessage,
    execution_record: &ToolLoopExecutionRecord,
    executed_tool_records: &[ToolLoopExecutionRecord],
    executed_tool_signatures: &[String],
    next_turn_id: &str,
) -> Vec<LlmMessage> {
    let mut instruction_lines = vec![
        "The latest assistant response repeated a tool candidate whose previous observation failed."
            .to_owned(),
        format!("duplicate_tool_candidate: {}", execution_record.signature),
        format!(
            "previous_failure_kind: {}",
            execution_record
                .observation
                .error_kind
                .unwrap_or("execution_error")
        ),
        "Repeating the same tool_name and arguments cannot provide new workspace evidence."
            .to_owned(),
    ];
    instruction_lines.extend(
        response_boundary_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        payload_ordering_contract_lines()
            .iter()
            .map(|line| line.to_string()),
    );
    instruction_lines.extend(
        [
            "Do not call the duplicate failed tool candidate again.",
            "Return exactly one blocked response if the failure prevents the user goal.",
            "Return exactly one answer response if the failure itself is enough to answer the user goal.",
            "Request a tool only if a different tool candidate is required for missing evidence.",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    instruction_lines.extend(tool_loop_state_lines(executed_tool_signatures));

    let mut messages = vec![schema_message.clone(), user_message.clone()];
    messages.extend(recent_tool_observation_messages(
        executed_tool_records,
        execution_record,
    ));
    messages.push(LlmMessage {
        turn_id: next_turn_id.to_owned(),
        role: LlmMessageRole::System,
        visibility: execution_record.observation_message.visibility,
        content: instruction_lines.join("\n"),
    });
    messages
}

fn recent_tool_observation_messages(
    executed_tool_records: &[ToolLoopExecutionRecord],
    execution_record: &ToolLoopExecutionRecord,
) -> Vec<LlmMessage> {
    recent_tool_observation_messages_with_latest(
        executed_tool_records,
        &execution_record.observation_message,
    )
}

fn recent_tool_observation_messages_with_latest(
    executed_tool_records: &[ToolLoopExecutionRecord],
    latest_observation_message: &LlmMessage,
) -> Vec<LlmMessage> {
    let mut messages = executed_tool_records
        .iter()
        .rev()
        .take(RECENT_TOOL_OBSERVATION_LIMIT)
        .collect::<Vec<_>>();
    messages.reverse();

    let mut result = Vec::new();
    for record in messages {
        push_unique_observation_message(&mut result, record.observation_message.clone());
    }
    push_unique_observation_message(&mut result, latest_observation_message.clone());
    result
}

fn push_unique_observation_message(messages: &mut Vec<LlmMessage>, message: LlmMessage) {
    if messages
        .iter()
        .any(|existing| existing.content == message.content)
    {
        return;
    }
    messages.push(message);
}

fn tool_loop_state_lines(executed_tool_signatures: &[String]) -> Vec<String> {
    let mut lines = vec!["<AHREUM_TOOL_LOOP_STATE>".to_owned()];
    lines.push(format!(
        "executed_tool_candidate_count: {}",
        executed_tool_signatures.len()
    ));
    lines.push("executed_tool_candidates:".to_owned());
    for signature in executed_tool_signatures.iter().rev().take(8).rev() {
        lines.push(format!("- {signature}"));
    }
    lines.push("</AHREUM_TOOL_LOOP_STATE>".to_owned());
    lines.push(
        "A tool candidate listed in AHREUM_TOOL_LOOP_STATE is already spent. Return an answer if the observations are enough; otherwise choose a different next tool candidate."
            .to_owned(),
    );
    lines
}

fn append_read_candidate_guidance(instruction_lines: &mut Vec<String>, observation_message: &str) {
    let candidates = read_file_candidate_paths_from_observation_message(observation_message);
    if candidates.is_empty() {
        return;
    }

    instruction_lines.push("<AHREUM_READ_FILE_CANDIDATES>".to_owned());
    instruction_lines.push("candidate_paths:".to_owned());
    for path in candidates {
        instruction_lines.push(format!("- {path}"));
    }
    instruction_lines.push("</AHREUM_READ_FILE_CANDIDATES>".to_owned());
    instruction_lines.push(
        "The latest observation exposed candidate file paths. If the user goal requires file contents or values, request read_file with one candidate path before answering; do not repeat list_files or search_text for the same candidates."
            .to_owned(),
    );
}

fn read_file_candidate_paths_from_observation_message(observation_message: &str) -> Vec<String> {
    let Some(tool_name) = observation_field(observation_message, "tool_name") else {
        return Vec::new();
    };

    let preview = observation_preview_lines(observation_message);
    let mut candidates = Vec::new();
    for line in preview {
        let candidate = match tool_name {
            "list_files" => list_files_candidate_path(line),
            "search_text" => search_text_candidate_path(line),
            "read_file" => read_file_failure_candidate_path(line),
            _ => None,
        };
        if let Some(candidate) = candidate {
            if !candidates.iter().any(|existing| existing == &candidate) {
                candidates.push(candidate);
            }
        }
        if candidates.len() >= 12 {
            break;
        }
    }
    candidates
}

fn observation_field<'a>(observation_message: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{field}: ");
    observation_message
        .lines()
        .find_map(|line| line.strip_prefix(&prefix).map(str::trim))
}

fn observation_preview_lines(observation_message: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut in_preview = false;
    for line in observation_message.lines() {
        if line == "preview:" {
            in_preview = true;
            continue;
        }
        if line == "</AHREUM_TOOL_OBSERVATION>" {
            break;
        }
        if in_preview {
            lines.push(line);
        }
    }
    lines
}

fn list_files_candidate_path(line: &str) -> Option<String> {
    let path = line.trim();
    if path.is_empty() || path == "." || path.ends_with('/') {
        return None;
    }
    Some(path.to_owned())
}

fn search_text_candidate_path(line: &str) -> Option<String> {
    let (path, rest) = line.split_once(':')?;
    if path.trim().is_empty() || rest.split_once(':').is_none() {
        return None;
    }
    Some(path.trim().to_owned())
}

fn read_file_failure_candidate_path(line: &str) -> Option<String> {
    line.trim()
        .strip_prefix("candidate_path:")
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
}

pub(super) fn runtime_execute_detail(decision: &RuntimeDecision) -> &'static str {
    match decision {
        RuntimeDecision::Answer { .. }
        | RuntimeDecision::Clarify { .. }
        | RuntimeDecision::Blocked { .. } => "실행할 도구가 없어 실행 단계를 통과합니다.",
        RuntimeDecision::ToolCandidatePending { .. } => "Explore 도구 후보를 실행합니다.",
        RuntimeDecision::ApprovalNeeded { .. } => {
            "승인이 필요한 후보이므로 직접 실행하지 않습니다."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_change_evidence_boundary, apply_final_answer_evidence_boundary,
        completed_tool_fallback_final_decision, diagnose_tool_loop_limit,
        duplicate_approval_final_decision, duplicate_approval_repeat_redirect,
        read_file_candidate_paths_from_observation_message,
        repeated_completed_add_change_final_decision, settled_duplicate_final_decision,
        tool_loop_request_messages, tool_repeat_answer_request_messages,
        tool_repeat_continuation_request_messages, tool_repeat_failure_request_messages,
        ActivePlainRequest, ToolLoopExecutionRecord, ToolLoopLimitDiagnosis, ToolLoopObservation,
        ToolLoopRepeatRedirect,
    };
    use crate::llm::{
        payload_ordering_contract_lines, response_boundary_contract_lines,
        tool_path_selection_contract_lines, Activity, ChangePreview, LlmMessage, LlmMessageRole,
        LlmMessageVisibility, MessageHistory, PatchOperation, RuntimeDecision,
    };
    use crate::tool::ToolObservation;
    use std::sync::mpsc;

    fn message(
        role: LlmMessageRole,
        visibility: LlmMessageVisibility,
        content: &str,
    ) -> LlmMessage {
        LlmMessage {
            turn_id: "turn-1".to_owned(),
            role,
            visibility,
            content: content.to_owned(),
        }
    }

    #[test]
    fn tool_loop_request_uses_schema_user_observation_and_final_instruction() {
        let schema = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "schema",
        );
        let user = message(
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            "user goal",
        );
        let observation = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "<AHREUM_TOOL_OBSERVATION>latest</AHREUM_TOOL_OBSERVATION>",
        );

        let executed = vec![
            r#"read_file:{"max_lines":120,"path":"Cargo.toml","start_line":1}"#.to_owned(),
            r#"search_text:{"max_results":20,"path":"src","query":"RuntimeDecision"}"#.to_owned(),
        ];
        let messages =
            tool_loop_request_messages(&schema, &user, &observation, &[], &executed, "turn-2");

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].content, "schema");
        assert_eq!(messages[1].content, "user goal");
        assert_eq!(
            messages[2].content,
            "<AHREUM_TOOL_OBSERVATION>latest</AHREUM_TOOL_OBSERVATION>"
        );
        assert_eq!(messages[3].turn_id, "turn-2");
        assert_eq!(messages[3].role, LlmMessageRole::System);
        assert!(messages[3].content.contains("return exactly one answer"));
        assert!(messages[3]
            .content
            .contains("latest observation is a succeeded Change or Execute"));
        assert!(messages[3]
            .content
            .contains("do not request another Add File for the same target"));
        assert!(messages[3]
            .content
            .contains("After a succeeded read_file observation"));
        for boundary_line in response_boundary_contract_lines() {
            assert!(messages[3].content.contains(boundary_line));
        }
        for payload_line in payload_ordering_contract_lines() {
            assert!(messages[3].content.contains(payload_line));
        }
        for path_line in tool_path_selection_contract_lines() {
            assert!(messages[3].content.contains(path_line));
        }
        assert!(messages[3].content.contains("Use search_text"));
        assert!(messages[3].content.contains("Use read_file"));
        assert!(messages[3].content.contains("Use list_files"));
        assert!(messages[3]
            .content
            .contains("filename/path candidate after a direct path failed"));
        assert!(messages[3]
            .content
            .contains("Empty path strings are invalid"));
        assert!(messages[3]
            .content
            .contains("do not treat its target as read or analyzed evidence"));
        assert!(messages[3]
            .content
            .contains("error_kind target_already_exists"));
        assert!(messages[3]
            .content
            .contains("request read_file for target_raw"));
        assert!(messages[3].content.contains("<AHREUM_TOOL_LOOP_STATE>"));
        assert!(messages[3]
            .content
            .contains("executed_tool_candidate_count: 2"));
        assert!(messages[3].content.contains(&executed[0]));
        assert!(messages[3].content.contains(&executed[1]));
        assert!(messages[3].content.contains("already spent"));
        assert!(messages
            .iter()
            .all(|message| !message.content.contains("assistant tool candidate")));
        assert!(messages
            .iter()
            .all(|message| !message.content.contains("repair request")));
    }

    #[test]
    fn tool_loop_request_lists_read_candidates_from_discovery_observation() {
        let schema = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "schema",
        );
        let user = message(
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            "프로젝트 설정 후보를 확인해줘.",
        );
        let observation = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            concat!(
                "<AHREUM_TOOL_OBSERVATION>\n",
                "tool_name: list_files\n",
                "status: succeeded\n",
                "preview:\n",
                "docs/\n",
                ".ahreumcode/config.toml\n",
                "Cargo.toml\n",
                "</AHREUM_TOOL_OBSERVATION>"
            ),
        );

        let messages = tool_loop_request_messages(&schema, &user, &observation, &[], &[], "turn-2");

        assert!(messages[3]
            .content
            .contains("<AHREUM_READ_FILE_CANDIDATES>"));
        assert!(messages[3].content.contains("- .ahreumcode/config.toml"));
        assert!(messages[3].content.contains("- Cargo.toml"));
        assert!(messages[3]
            .content
            .contains("If the user goal requires file contents or values"));
        assert!(!messages[3].content.contains("- docs/"));
    }

    #[test]
    fn tool_loop_request_keeps_recent_observations_for_repair_context() {
        let schema = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "schema",
        );
        let user = message(
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            "user goal",
        );
        let read_record = ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"file.md"}"#.to_owned(),
            observation: loop_observation(Some("file.md"), None, false, false),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>current file contents</AHREUM_TOOL_OBSERVATION>",
            ),
        };
        let failed_patch = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "<AHREUM_TOOL_OBSERVATION>failed patch diagnostic</AHREUM_TOOL_OBSERVATION>",
        );

        let messages = tool_loop_request_messages(
            &schema,
            &user,
            &failed_patch,
            &[read_record],
            &["apply_patch:target=file.md:operation=update".to_owned()],
            "turn-2",
        );

        assert!(messages
            .iter()
            .any(|message| message.content.contains("current file contents")));
        assert!(messages
            .iter()
            .any(|message| message.content.contains("failed patch diagnostic")));
        assert!(messages
            .last()
            .expect("instruction message")
            .content
            .contains("request a repaired Change candidate"));
    }

    #[test]
    fn read_candidates_extract_unique_paths_from_search_matches() {
        let observation = concat!(
            "<AHREUM_TOOL_OBSERVATION>\n",
            "tool_name: search_text\n",
            "status: succeeded\n",
            "preview:\n",
            "src/config/mod.rs:5: pub const CONFIG_RELATIVE_PATH\n",
            "src/config/mod.rs:9: more config\n",
            "src/main.rs:2: mod config;\n",
            "</AHREUM_TOOL_OBSERVATION>"
        );

        let candidates = read_file_candidate_paths_from_observation_message(observation);

        assert_eq!(
            candidates,
            vec!["src/config/mod.rs".to_owned(), "src/main.rs".to_owned()]
        );
    }

    #[test]
    fn read_candidates_extract_paths_from_failed_read_candidates() {
        let observation = concat!(
            "<AHREUM_TOOL_OBSERVATION>\n",
            "tool_name: read_file\n",
            "status: failed\n",
            "error_kind: path_not_found\n",
            "preview:\n",
            "candidate_path: nested/settings.local\n",
            "</AHREUM_TOOL_OBSERVATION>"
        );

        let candidates = read_file_candidate_paths_from_observation_message(observation);

        assert_eq!(candidates, vec!["nested/settings.local".to_owned()]);
    }

    #[test]
    fn active_request_detects_only_settled_duplicate_tool_candidates() {
        let mut active = active_request();
        let signature = r#"read_file:{"path":"Cargo.toml"}"#.to_owned();
        let observation = ToolObservation::succeeded(
            "read_file",
            Some("Cargo.toml".to_owned()),
            Some("/workspace/Cargo.toml".to_owned()),
            vec!["[package]".to_owned()],
            false,
            None,
            "read Cargo.toml",
        );

        active.record_tool_execution(
            signature.clone(),
            &observation,
            message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>read Cargo.toml</AHREUM_TOOL_OBSERVATION>",
            ),
        );

        let arguments = serde_json::json!({"path":"Cargo.toml"});
        let (redirect, _) = active
            .repeat_redirect_for_tool_candidate(&signature, "read_file", &arguments)
            .expect("settled duplicate should redirect");
        assert_eq!(redirect, ToolLoopRepeatRedirect::SettledDuplicate);
        assert!(active
            .repeat_redirect_for_tool_candidate(
                r#"read_file:{"path":"src/main.rs"}"#,
                "read_file",
                &serde_json::json!({"path":"src/main.rs"}),
            )
            .is_none());
    }

    #[test]
    fn duplicate_redirect_count_is_scoped_to_last_signature() {
        let mut active = active_request();
        active.last_tool_signature = Some("apply_patch:old".to_owned());
        active.duplicate_redirect_count = 1;

        assert_eq!(
            active.duplicate_redirect_count_for_signature("apply_patch:old"),
            1
        );
        assert_eq!(
            active.duplicate_redirect_count_for_signature("read_file:new"),
            0
        );
    }

    #[test]
    fn settled_duplicate_final_decision_uses_existing_observation() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"search_text:{"path":"src","query":"needle"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "search_text".to_owned(),
                target_raw: Some("src".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: true,
                source_truncated: true,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["src/config/mod.rs:5 needle".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "observation",
            ),
        });

        let record = ToolLoopExecutionRecord {
            signature: r#"search_text:{"path":"src","query":"missing needle"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "search_text".to_owned(),
                target_raw: Some("src".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: Vec::new(),
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "observation",
            ),
        };

        let decision = settled_duplicate_final_decision(&active, &record);

        let RuntimeDecision::Answer {
            summary,
            payload_body,
        } = decision
        else {
            panic!("settled duplicate should finalize as answer");
        };
        let payload = payload_body.expect("payload");
        assert!(summary.contains("추가 도구 호출 없이"));
        assert!(payload.contains("tool: search_text"));
        assert!(payload.contains("target: src"));
        assert!(payload.contains("src/config/mod.rs:5 needle"));
    }

    #[test]
    fn settled_duplicate_final_decision_blocks_without_evidence() {
        let active = active_request();
        let record = ToolLoopExecutionRecord {
            signature: r#"search_text:{"path":"src","query":"missing"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "search_text".to_owned(),
                target_raw: Some("src".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: Vec::new(),
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "observation",
            ),
        };

        let decision = settled_duplicate_final_decision(&active, &record);

        let RuntimeDecision::Blocked { message, reason } = decision else {
            panic!("empty settled duplicate should not become an answer");
        };
        assert_eq!(reason, "settled_duplicate_without_evidence");
        assert!(message.contains("답변 근거가 없습니다"));
        assert!(message.contains("tool: search_text"));
    }

    #[test]
    fn settled_duplicate_final_decision_blocks_discovery_only_after_failed_read_recovery() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"settings.local"}"#.to_owned(),
            observation: loop_observation(
                Some("settings.local"),
                Some("path_not_found"),
                false,
                false,
            ),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed read",
            ),
        });
        let record = ToolLoopExecutionRecord {
            signature: r#"list_files:{"path":"."}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "list_files".to_owned(),
                target_raw: Some(".".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["nested/settings.local".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "discovery",
            ),
        };
        active.executed_tool_records.push(record.clone());

        let decision = settled_duplicate_final_decision(&active, &record);

        let RuntimeDecision::Blocked { message, reason } = decision else {
            panic!("discovery-only read recovery must not finalize as answer");
        };
        assert_eq!(reason, "answer_without_file_content_after_read_recovery");
        assert!(message.contains("next_required_tool: read_file"));
        assert!(message.contains("last_tool: list_files"));
    }

    #[test]
    fn final_answer_evidence_boundary_blocks_empty_tool_observations() {
        let mut active = active_request();
        active.tool_call_count = 1;
        active.last_tool_observation = Some(ToolLoopObservation {
            tool_name: "search_text".to_owned(),
            target_raw: Some("src".to_owned()),
            status: "succeeded",
            error_kind: None,
            truncated: false,
            source_truncated: false,
            preview_truncated: false,
            has_next_range_hint: false,
            preview: Vec::new(),
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"search_text:{"path":"src","query":"missing"}"#.to_owned(),
            observation: active.last_tool_observation.clone().expect("observation"),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "observation",
            ),
        });

        let decision = apply_final_answer_evidence_boundary(
            &active,
            RuntimeDecision::Answer {
                summary: "not found".to_owned(),
                payload_body: None,
            },
        );

        let RuntimeDecision::Blocked { message, reason } = decision else {
            panic!("answer without observation evidence should be blocked");
        };
        assert_eq!(reason, "answer_without_observation_evidence");
        assert!(message.contains("답변 근거"));
        assert!(message.contains("last_tool: search_text"));
    }

    #[test]
    fn final_answer_evidence_boundary_blocks_discovery_only_after_failed_read_recovery() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"settings.local"}"#.to_owned(),
            observation: loop_observation(
                Some("settings.local"),
                Some("path_not_found"),
                false,
                false,
            ),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed read",
            ),
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"search_text:{"path":".","query":"settings.local"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "search_text".to_owned(),
                target_raw: Some(".".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["nested/settings.local".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "discovery",
            ),
        });
        active.last_tool_observation = Some(
            active
                .executed_tool_records
                .last()
                .expect("record")
                .observation
                .clone(),
        );

        let decision = apply_final_answer_evidence_boundary(
            &active,
            RuntimeDecision::Answer {
                summary: "setting is enabled".to_owned(),
                payload_body: None,
            },
        );

        let RuntimeDecision::Blocked { message, reason } = decision else {
            panic!("answer from discovery-only evidence should be blocked");
        };
        assert_eq!(reason, "answer_without_file_content_after_read_recovery");
        assert!(message.contains("파일 내용을 아직 읽지 않았습니다"));
    }

    #[test]
    fn final_answer_evidence_boundary_redirects_failed_read_candidate_to_read_file() {
        let mut active = active_request();
        let observation = ToolLoopObservation {
            tool_name: "read_file".to_owned(),
            target_raw: Some("settings.local".to_owned()),
            status: "failed",
            error_kind: Some("path_not_found"),
            truncated: false,
            source_truncated: false,
            preview_truncated: false,
            has_next_range_hint: false,
            preview: vec!["candidate_path: nested/settings.local".to_owned()],
        };
        active.last_tool_observation = Some(observation.clone());
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"settings.local"}"#.to_owned(),
            observation,
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed read with candidate",
            ),
        });

        let decision = apply_final_answer_evidence_boundary(
            &active,
            RuntimeDecision::Answer {
                summary: "candidate exists".to_owned(),
                payload_body: None,
            },
        );

        let RuntimeDecision::ToolCandidatePending {
            tool_name,
            arguments,
            ..
        } = decision
        else {
            panic!("failed read candidate should force a read_file candidate");
        };
        assert_eq!(tool_name, "read_file");
        assert_eq!(arguments["path"], "nested/settings.local");
    }

    #[test]
    fn final_answer_evidence_boundary_clarifies_multiple_failed_read_candidates() {
        let mut active = active_request();
        let observation = ToolLoopObservation {
            tool_name: "read_file".to_owned(),
            target_raw: Some("settings.local".to_owned()),
            status: "failed",
            error_kind: Some("path_not_found"),
            truncated: false,
            source_truncated: false,
            preview_truncated: false,
            has_next_range_hint: false,
            preview: vec![
                "candidate_path: alpha/settings.local".to_owned(),
                "candidate_path: beta/settings.local".to_owned(),
            ],
        };
        active.last_tool_observation = Some(observation.clone());
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"settings.local"}"#.to_owned(),
            observation,
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed read with multiple candidates",
            ),
        });

        let decision = apply_final_answer_evidence_boundary(
            &active,
            RuntimeDecision::Answer {
                summary: "candidate exists".to_owned(),
                payload_body: None,
            },
        );

        let RuntimeDecision::Clarify { message, reason } = decision else {
            panic!("multiple read candidates should ask for clarification");
        };
        assert_eq!(reason, "multiple_read_file_candidates");
        assert!(message.contains("alpha/settings.local"));
        assert!(message.contains("beta/settings.local"));
    }

    #[test]
    fn final_answer_evidence_boundary_allows_successful_read_after_recovery() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"settings.local"}"#.to_owned(),
            observation: loop_observation(
                Some("settings.local"),
                Some("path_not_found"),
                false,
                false,
            ),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed read",
            ),
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"list_files:{"path":"."}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "list_files".to_owned(),
                target_raw: Some(".".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["nested/settings.local".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "discovery",
            ),
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"nested/settings.local"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "read_file".to_owned(),
                target_raw: Some("nested/settings.local".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["setting = \"enabled\"".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "read",
            ),
        });

        let decision = apply_final_answer_evidence_boundary(
            &active,
            RuntimeDecision::Answer {
                summary: "setting is enabled".to_owned(),
                payload_body: None,
            },
        );

        let RuntimeDecision::Answer { summary, .. } = decision else {
            panic!("successful read after recovery should allow answer");
        };
        assert_eq!(summary, "setting is enabled");
    }

    #[test]
    fn final_answer_evidence_boundary_blocks_answer_after_failed_change() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"fixture-target.txt"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "read_file".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["old value".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "read",
            ),
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"apply_patch:{"patch":"*** Begin Patch..."}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "failed",
                error_kind: Some("invalid_arguments"),
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["add patch target already exists".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed change",
            ),
        });

        let decision = apply_final_answer_evidence_boundary(
            &active,
            RuntimeDecision::Answer {
                summary: "updated".to_owned(),
                payload_body: None,
            },
        );

        let RuntimeDecision::Blocked { message, reason } = decision else {
            panic!("answer after failed change should be blocked");
        };
        assert_eq!(reason, "answer_after_failed_change_without_success");
        assert!(message.contains("next_required_tool: apply_patch"));
        assert!(message.contains("target: fixture-target.txt"));
    }

    #[test]
    fn final_answer_evidence_boundary_blocks_answer_after_prerequisite_read_without_change() {
        let mut active = active_request();
        active.pending_change_after_read_target = Some("fixture-target.txt".to_owned());
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"fixture-target.txt"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "read_file".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["old value".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "read",
            ),
        });

        let decision = apply_final_answer_evidence_boundary(
            &active,
            RuntimeDecision::Answer {
                summary: "updated".to_owned(),
                payload_body: None,
            },
        );

        let RuntimeDecision::Blocked { message, reason } = decision else {
            panic!("answer after prerequisite read should be blocked until change succeeds");
        };
        assert_eq!(reason, "answer_after_prerequisite_read_without_change");
        assert!(message.contains("next_required_tool: apply_patch"));
        assert!(message.contains("target: fixture-target.txt"));
    }

    #[test]
    fn completed_tool_fallback_does_not_finish_prerequisite_read_without_change() {
        let mut active = active_request();
        active.pending_change_after_read_target = Some("fixture-target.txt".to_owned());
        active.last_successful_tool_observation = Some(ToolLoopObservation {
            tool_name: "read_file".to_owned(),
            target_raw: Some("fixture-target.txt".to_owned()),
            status: "succeeded",
            error_kind: None,
            truncated: false,
            source_truncated: false,
            preview_truncated: false,
            has_next_range_hint: false,
            preview: vec!["old value".to_owned()],
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"fixture-target.txt"}"#.to_owned(),
            observation: active
                .last_successful_tool_observation
                .clone()
                .expect("observation"),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "read",
            ),
        });

        let decision = completed_tool_fallback_final_decision(&active, "schema_validation_failed");

        assert!(decision.is_none());
    }

    #[test]
    fn completed_tool_fallback_does_not_finish_payload_validation_failure() {
        let mut active = active_request();
        active.last_successful_tool_observation = Some(ToolLoopObservation {
            tool_name: "read_file".to_owned(),
            target_raw: Some("fixture-target.txt".to_owned()),
            status: "succeeded",
            error_kind: None,
            truncated: false,
            source_truncated: false,
            preview_truncated: false,
            has_next_range_hint: false,
            preview: vec!["old value".to_owned()],
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"fixture-target.txt"}"#.to_owned(),
            observation: active
                .last_successful_tool_observation
                .clone()
                .expect("observation"),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "read",
            ),
        });

        let decision = completed_tool_fallback_final_decision(&active, "payload_validation_failed");

        assert!(decision.is_none());
    }

    #[test]
    fn final_answer_evidence_boundary_allows_answer_after_failed_change_is_resolved() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"apply_patch:{"patch":"*** Begin Patch...Add File"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "failed",
                error_kind: Some("invalid_arguments"),
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["add patch target already exists".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed change",
            ),
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"apply_patch:{"patch":"*** Begin Patch...Update File"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["updated fixture-target.txt".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "successful change",
            ),
        });

        let decision = apply_final_answer_evidence_boundary(
            &active,
            RuntimeDecision::Answer {
                summary: "updated".to_owned(),
                payload_body: None,
            },
        );

        let RuntimeDecision::Answer { summary, .. } = decision else {
            panic!("resolved failed change should allow answer");
        };
        assert_eq!(summary, "updated");
    }

    #[test]
    fn final_answer_evidence_boundary_keeps_failed_change_when_other_target_succeeds() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: "apply_patch:target=fixture-a.txt:operation=update:payload_hash=bad"
                .to_owned(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture-a.txt".to_owned()),
                status: "failed",
                error_kind: Some("invalid_arguments"),
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["update hunk did not match".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed change",
            ),
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: "apply_patch:target=fixture-b.txt:operation=add:payload_hash=ok".to_owned(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture-b.txt".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["added fixture-b.txt".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "successful other-target change",
            ),
        });

        let decision = apply_final_answer_evidence_boundary(
            &active,
            RuntimeDecision::Answer {
                summary: "done".to_owned(),
                payload_body: None,
            },
        );

        let RuntimeDecision::Blocked { message, reason } = decision else {
            panic!("other target success must not resolve failed change");
        };
        assert_eq!(reason, "answer_after_failed_change_without_success");
        assert!(message.contains("target: fixture-a.txt"));
    }

    #[test]
    fn change_evidence_boundary_reads_before_update_patch() {
        let active = active_request();
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Change,
            tool_name: "apply_patch".to_owned(),
            arguments: serde_json::json!({"payload_id":"patch_001"}),
            change_preview: Some(ChangePreview {
                payload_id: "patch_001".to_owned(),
                target_path: ".ahreumcode/check.md".to_owned(),
                operation: PatchOperation::Update,
                additions: 1,
                deletions: 1,
                payload_body: "*** Begin Patch\n*** Update File: .ahreumcode/check.md\n@@\n-old\n+new\n*** End Patch".to_owned(),
            }),
            reason: "change existing file".to_owned(),
        };

        let decision = apply_change_evidence_boundary(&active, decision);

        let RuntimeDecision::ToolCandidatePending {
            activity,
            tool_name,
            arguments,
            ..
        } = decision
        else {
            panic!("update without read should be redirected to read_file");
        };
        assert_eq!(activity, Activity::Explore);
        assert_eq!(tool_name, "read_file");
        assert_eq!(arguments["path"], ".ahreumcode/check.md");
    }

    #[test]
    fn change_evidence_boundary_allows_update_after_successful_read() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":".ahreumcode/check.md"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "read_file".to_owned(),
                target_raw: Some(".ahreumcode/check.md".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["old".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>read</AHREUM_TOOL_OBSERVATION>",
            ),
        });
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Change,
            tool_name: "apply_patch".to_owned(),
            arguments: serde_json::json!({"payload_id":"patch_001"}),
            change_preview: Some(ChangePreview {
                payload_id: "patch_001".to_owned(),
                target_path: ".ahreumcode/check.md".to_owned(),
                operation: PatchOperation::Update,
                additions: 1,
                deletions: 1,
                payload_body: "*** Begin Patch\n*** Update File: .ahreumcode/check.md\n@@\n-old\n+new\n*** End Patch".to_owned(),
            }),
            reason: "change existing file".to_owned(),
        };

        let decision = apply_change_evidence_boundary(&active, decision);

        assert!(matches!(decision, RuntimeDecision::ApprovalNeeded { .. }));
    }

    #[test]
    fn change_evidence_boundary_requires_fresh_read_after_failed_change() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"fixture-target.txt"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "read_file".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["alpha".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>read</AHREUM_TOOL_OBSERVATION>",
            ),
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: "apply_patch:target=fixture-target.txt:operation=update:payload_hash=bad"
                .to_owned(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "failed",
                error_kind: Some("invalid_arguments"),
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["update hunk did not match".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>failed change</AHREUM_TOOL_OBSERVATION>",
            ),
        });
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Change,
            tool_name: "apply_patch".to_owned(),
            arguments: serde_json::json!({"payload_id":"patch_001"}),
            change_preview: Some(ChangePreview {
                payload_id: "patch_001".to_owned(),
                target_path: "fixture-target.txt".to_owned(),
                operation: PatchOperation::Update,
                additions: 1,
                deletions: 1,
                payload_body:
                    "*** Begin Patch\n*** Update File: fixture-target.txt\n@@\n-alpha\n+beta\n*** End Patch"
                        .to_owned(),
            }),
            reason: "retry update".to_owned(),
        };

        let decision = apply_change_evidence_boundary(&active, decision);

        let RuntimeDecision::ToolCandidatePending {
            tool_name,
            arguments,
            ..
        } = decision
        else {
            panic!("failed change should invalidate older read evidence");
        };
        assert_eq!(tool_name, "read_file");
        assert_eq!(arguments["path"], "fixture-target.txt");
    }

    #[test]
    fn change_evidence_boundary_allows_update_after_failed_change_and_fresh_read() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: "apply_patch:target=fixture-target.txt:operation=update:payload_hash=bad"
                .to_owned(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "failed",
                error_kind: Some("invalid_arguments"),
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["update hunk did not match".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>failed change</AHREUM_TOOL_OBSERVATION>",
            ),
        });
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"path":"fixture-target.txt"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "read_file".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["alpha".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>fresh read</AHREUM_TOOL_OBSERVATION>",
            ),
        });
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Change,
            tool_name: "apply_patch".to_owned(),
            arguments: serde_json::json!({"payload_id":"patch_001"}),
            change_preview: Some(ChangePreview {
                payload_id: "patch_001".to_owned(),
                target_path: "fixture-target.txt".to_owned(),
                operation: PatchOperation::Update,
                additions: 1,
                deletions: 1,
                payload_body:
                    "*** Begin Patch\n*** Update File: fixture-target.txt\n@@\n-alpha\n+beta\n*** End Patch"
                        .to_owned(),
            }),
            reason: "retry update".to_owned(),
        };

        let decision = apply_change_evidence_boundary(&active, decision);

        assert!(matches!(decision, RuntimeDecision::ApprovalNeeded { .. }));
    }

    #[test]
    fn duplicate_approval_final_decision_uses_existing_command_observation() {
        let mut active = active_request();
        let signature = r#"run_command:{"argv":["tool","verify"]}"#.to_owned();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: signature.clone(),
            observation: ToolLoopObservation {
                tool_name: "run_command".to_owned(),
                target_raw: Some("tool verify".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["verification succeeded".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "command",
            ),
        });

        let decision = duplicate_approval_final_decision(&active, &signature)
            .expect("duplicate approval should finalize");

        let RuntimeDecision::Answer {
            summary,
            payload_body,
        } = decision
        else {
            panic!("duplicate approval should become answer");
        };
        assert!(summary.contains("추가 승인 없이"));
        assert!(payload_body
            .expect("payload")
            .contains("verification succeeded"));
    }

    #[test]
    fn duplicate_approval_final_decision_blocks_failed_observation() {
        let mut active = active_request();
        let signature = r#"apply_patch:{"patch":"*** Begin Patch...Add File"}"#.to_owned();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: signature.clone(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture-target.txt".to_owned()),
                status: "failed",
                error_kind: Some("target_already_exists"),
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["add patch target already exists".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed change",
            ),
        });

        let decision = duplicate_approval_final_decision(&active, &signature)
            .expect("duplicate failed approval should be blocked");

        let RuntimeDecision::Blocked { message, reason } = decision else {
            panic!("duplicate failed approval should not become answer");
        };
        assert_eq!(reason, "duplicate_approval_failed_observation");
        assert!(message.contains("different_tool_candidate"));
        assert!(message.contains("target_already_exists"));
    }

    #[test]
    fn duplicate_approval_repeat_redirect_reports_failed_duplicate() {
        let mut active = active_request();
        let signature = "apply_patch:target=fixture.txt:operation=update:payload_hash=1".to_owned();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: signature.clone(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture.txt".to_owned()),
                status: "failed",
                error_kind: Some("invalid_arguments"),
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["attempted_old_lines:".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed change",
            ),
        });

        let (redirect, record) = duplicate_approval_repeat_redirect(&active, &signature)
            .expect("duplicate failed approval should redirect");

        assert_eq!(redirect, ToolLoopRepeatRedirect::FailedDuplicate);
        assert_eq!(record.observation.error_kind, Some("invalid_arguments"));
    }

    #[test]
    fn duplicate_approval_repeat_redirect_reports_settled_duplicate() {
        let mut active = active_request();
        let signature = "apply_patch:target=fixture.txt:operation=update:payload_hash=2".to_owned();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: signature.clone(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some("fixture.txt".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["updated fixture.txt".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "successful change",
            ),
        });

        let (redirect, record) = duplicate_approval_repeat_redirect(&active, &signature)
            .expect("duplicate successful approval should redirect before finalizing");

        assert_eq!(redirect, ToolLoopRepeatRedirect::SettledDuplicate);
        assert_eq!(record.observation.status, "succeeded");
    }

    #[test]
    fn repeated_completed_add_change_final_decision_uses_existing_observation() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"apply_patch:{"payload_id":"patch_001"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some(".ahreumcode/check.md".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["apply_patch ok: .ahreumcode/check.md".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "change",
            ),
        });
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Change,
            tool_name: "apply_patch".to_owned(),
            arguments: serde_json::json!({"payload_id":"patch_002"}),
            change_preview: Some(ChangePreview {
                payload_id: "patch_002".to_owned(),
                target_path: ".ahreumcode/check.md".to_owned(),
                operation: PatchOperation::Add,
                additions: 1,
                deletions: 0,
                payload_body:
                    "*** Begin Patch\n*** Add File: .ahreumcode/check.md\n+again\n*** End Patch\n"
                        .to_owned(),
            }),
            reason: "repeat add".to_owned(),
        };

        let final_decision = repeated_completed_add_change_final_decision(&active, &decision)
            .expect("same target Add File after success should finalize");

        let RuntimeDecision::Answer {
            summary,
            payload_body,
        } = final_decision
        else {
            panic!("repeated add should become answer");
        };
        assert!(summary.contains("파일 생성이 완료되었습니다"));
        assert!(summary.contains(".ahreumcode/check.md"));
        assert!(payload_body
            .expect("payload")
            .contains("apply_patch ok: .ahreumcode/check.md"));
    }

    #[test]
    fn repeated_completed_add_change_final_decision_allows_update_on_same_target() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"apply_patch:{"payload_id":"patch_001"}"#.to_owned(),
            observation: ToolLoopObservation {
                tool_name: "apply_patch".to_owned(),
                target_raw: Some(".ahreumcode/check.md".to_owned()),
                status: "succeeded",
                error_kind: None,
                truncated: false,
                source_truncated: false,
                preview_truncated: false,
                has_next_range_hint: false,
                preview: vec!["apply_patch ok: .ahreumcode/check.md".to_owned()],
            },
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "change",
            ),
        });
        let decision = RuntimeDecision::ApprovalNeeded {
            activity: Activity::Change,
            tool_name: "apply_patch".to_owned(),
            arguments: serde_json::json!({"payload_id":"patch_002"}),
            change_preview: Some(ChangePreview {
                payload_id: "patch_002".to_owned(),
                target_path: ".ahreumcode/check.md".to_owned(),
                operation: PatchOperation::Update,
                additions: 1,
                deletions: 1,
                payload_body: "*** Begin Patch\n*** Update File: .ahreumcode/check.md\n@@\n-old\n+new\n*** End Patch\n"
                    .to_owned(),
            }),
            reason: "follow-up update".to_owned(),
        };

        assert!(repeated_completed_add_change_final_decision(&active, &decision).is_none());
    }

    #[test]
    fn active_request_classifies_failed_and_truncated_duplicates_separately() {
        let mut active = active_request();
        let failed_signature = r#"read_file:{"path":"missing.rs"}"#.to_owned();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: failed_signature.clone(),
            observation: loop_observation(Some("missing.rs"), Some("path_not_found"), false, false),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed observation",
            ),
        });

        let truncated_signature = r#"read_file:{"path":"large.rs"}"#.to_owned();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: truncated_signature.clone(),
            observation: loop_observation(Some("large.rs"), None, true, true),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "truncated observation",
            ),
        });

        let failed_arguments = serde_json::json!({"path":"missing.rs"});
        let (redirect, _) = active
            .repeat_redirect_for_tool_candidate(&failed_signature, "read_file", &failed_arguments)
            .expect("failed duplicate should redirect without execution");
        assert_eq!(redirect, ToolLoopRepeatRedirect::FailedDuplicate);
        let truncated_arguments = serde_json::json!({"path":"large.rs"});
        let (redirect, _) = active
            .repeat_redirect_for_tool_candidate(
                &truncated_signature,
                "read_file",
                &truncated_arguments,
            )
            .expect("truncated duplicate should redirect");
        assert_eq!(redirect, ToolLoopRepeatRedirect::TruncatedContinuation);
    }

    #[test]
    fn active_request_redirects_truncated_duplicate_without_marking_it_settled() {
        let mut active = active_request();
        let signature =
            r#"read_file:{"max_lines":120,"path":"src/tool/path.rs","start_line":1}"#.to_owned();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: signature.clone(),
            observation: loop_observation(Some("src/tool/path.rs"), None, true, true),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "truncated observation with next_range_hint",
            ),
        });

        let arguments =
            serde_json::json!({"max_lines":120,"path":"src/tool/path.rs","start_line":1});
        let (redirect, _) = active
            .repeat_redirect_for_tool_candidate(&signature, "read_file", &arguments)
            .expect("truncated duplicate should redirect");
        assert_eq!(redirect, ToolLoopRepeatRedirect::TruncatedContinuation);
        assert_eq!(redirect.as_str(), "truncated_duplicate_tool_candidate");
    }

    #[test]
    fn active_request_redirects_failed_duplicate_without_executing_again() {
        let mut active = active_request();
        let signature = r#"read_file:{"path":"missing.rs"}"#.to_owned();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: signature.clone(),
            observation: loop_observation(Some("missing.rs"), Some("path_not_found"), false, false),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed observation",
            ),
        });

        let arguments = serde_json::json!({"path":"missing.rs"});
        let (redirect, _) = active
            .repeat_redirect_for_tool_candidate(&signature, "read_file", &arguments)
            .expect("failed duplicate should redirect");
        assert_eq!(redirect, ToolLoopRepeatRedirect::FailedDuplicate);
        assert_eq!(redirect.as_str(), "failed_duplicate_tool_candidate");
    }

    #[test]
    fn active_request_redirects_same_failed_target_even_when_bounds_change() {
        let mut active = active_request();
        active.executed_tool_records.push(ToolLoopExecutionRecord {
            signature: r#"read_file:{"max_lines":120,"path":"missing.rs","start_line":1}"#
                .to_owned(),
            observation: loop_observation(Some("missing.rs"), Some("path_not_found"), false, false),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "failed observation",
            ),
        });

        let (redirect, _) = active
            .repeat_redirect_for_tool_candidate(
                r#"read_file:{"max_lines":20,"path":"missing.rs","start_line":1}"#,
                "read_file",
                &serde_json::json!({"max_lines":20,"path":"missing.rs","start_line":1}),
            )
            .expect("same target and target-invariant failure should redirect");

        assert_eq!(redirect, ToolLoopRepeatRedirect::FailedDuplicate);
    }

    #[test]
    fn tool_repeat_answer_request_uses_existing_observation_and_blocks_duplicate() {
        let schema = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "schema",
        );
        let user = message(
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            "user goal",
        );
        let observation = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "<AHREUM_TOOL_OBSERVATION>existing evidence</AHREUM_TOOL_OBSERVATION>",
        );
        let signature = r#"search_text:{"path":"src","query":"RuntimeDecision"}"#.to_owned();
        let record = ToolLoopExecutionRecord {
            signature: signature.clone(),
            observation: loop_observation(Some("src"), None, false, false),
            observation_message: observation,
        };
        let executed = vec![signature.clone()];
        let executed_records = vec![record.clone()];

        let messages = tool_repeat_answer_request_messages(
            &schema,
            &user,
            &record,
            &executed_records,
            &executed,
            "turn-2",
        );

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].content, "schema");
        assert_eq!(messages[1].content, "user goal");
        assert!(messages[2].content.contains("existing evidence"));
        assert_eq!(messages[3].turn_id, "turn-2");
        assert!(messages[3]
            .content
            .contains("already been executed successfully"));
        assert!(messages[3].content.contains(&signature));
        assert!(messages[3]
            .content
            .contains("Do not call the duplicate tool candidate again"));
        assert!(messages[3]
            .content
            .contains("Return exactly one answer response"));
        assert!(messages[3]
            .content
            .contains("duplicate successful observation is read_file"));
        assert!(messages[3].content.contains("<AHREUM_TOOL_LOOP_STATE>"));
    }

    #[test]
    fn tool_repeat_answer_request_keeps_recent_failed_observation_context() {
        let schema = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "schema",
        );
        let user = message(
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            "user goal",
        );
        let failed_record = ToolLoopExecutionRecord {
            signature: "apply_patch:target=file.md:operation=update:payload_hash=bad".to_owned(),
            observation: loop_observation(Some("file.md"), Some("invalid_arguments"), false, false),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>failed change diagnostic</AHREUM_TOOL_OBSERVATION>",
            ),
        };
        let read_signature =
            r#"read_file:{"max_lines":120,"path":"file.md","start_line":1}"#.to_owned();
        let read_record = ToolLoopExecutionRecord {
            signature: read_signature.clone(),
            observation: loop_observation(Some("file.md"), None, false, false),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>current file contents</AHREUM_TOOL_OBSERVATION>",
            ),
        };
        let executed_records = vec![failed_record, read_record.clone()];
        let executed = executed_records
            .iter()
            .map(|record| record.signature.clone())
            .collect::<Vec<_>>();

        let messages = tool_repeat_answer_request_messages(
            &schema,
            &user,
            &read_record,
            &executed_records,
            &executed,
            "turn-2",
        );

        assert!(messages
            .iter()
            .any(|message| message.content.contains("failed change diagnostic")));
        assert!(messages
            .iter()
            .any(|message| message.content.contains("current file contents")));
    }

    #[test]
    fn tool_repeat_continuation_request_requires_next_range_or_different_candidate() {
        let schema = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "schema",
        );
        let user = message(
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            "user goal",
        );
        let signature =
            r#"read_file:{"max_lines":120,"path":"src/tool/path.rs","start_line":1}"#.to_owned();
        let record = ToolLoopExecutionRecord {
            signature: signature.clone(),
            observation: loop_observation(Some("src/tool/path.rs"), None, true, true),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>next_range_hint: start_line=121</AHREUM_TOOL_OBSERVATION>",
            ),
        };
        let executed = vec![signature.clone()];
        let executed_records = vec![record.clone()];

        let messages = tool_repeat_continuation_request_messages(
            &schema,
            &user,
            &record,
            &executed_records,
            &executed,
            "turn-2",
        );

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].content, "schema");
        assert_eq!(messages[1].content, "user goal");
        assert!(messages[2].content.contains("next_range_hint"));
        assert!(messages[3]
            .content
            .contains("previous observation was truncated"));
        assert!(messages[3].content.contains(&signature));
        assert!(messages[3].content.contains(
            "Repeating the same tool_name and arguments cannot provide new workspace evidence"
        ));
        assert!(messages[3]
            .content
            .contains("request read_file with that next range"));
        assert!(messages[3].content.contains("narrower path or search_text"));
        assert!(messages[3].content.contains("different tool candidate"));
    }

    #[test]
    fn tool_repeat_failure_request_blocks_failed_duplicate() {
        let schema = message(
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "schema",
        );
        let user = message(
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            "user goal",
        );
        let signature = r#"read_file:{"path":"missing.rs"}"#.to_owned();
        let record = ToolLoopExecutionRecord {
            signature: signature.clone(),
            observation: loop_observation(Some("missing.rs"), Some("path_not_found"), false, false),
            observation_message: message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "<AHREUM_TOOL_OBSERVATION>path_not_found</AHREUM_TOOL_OBSERVATION>",
            ),
        };
        let executed = vec![signature.clone()];
        let executed_records = vec![record.clone()];

        let messages = tool_repeat_failure_request_messages(
            &schema,
            &user,
            &record,
            &executed_records,
            &executed,
            "turn-2",
        );

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].content, "schema");
        assert_eq!(messages[1].content, "user goal");
        assert!(messages[2].content.contains("path_not_found"));
        assert!(messages[3].content.contains("previous observation failed"));
        assert!(messages[3]
            .content
            .contains("previous_failure_kind: path_not_found"));
        assert!(messages[3].content.contains(&signature));
        assert!(messages[3]
            .content
            .contains("Do not call the duplicate failed tool candidate again"));
        assert!(messages[3]
            .content
            .contains("Return exactly one blocked response"));
    }

    #[test]
    fn diagnoses_same_tool_signature_repeat_limit() {
        let diagnosis = diagnose_tool_loop_limit("max_same_tool_repeats", None);

        assert_eq!(diagnosis, ToolLoopLimitDiagnosis::SameToolSignatureRepeat);
        assert_eq!(diagnosis.as_str(), "same_tool_signature_repeat");
    }

    #[test]
    fn diagnoses_truncated_observation_continuation_limit() {
        let observation = loop_observation(Some("src/main.rs"), None, true, true);

        let diagnosis = diagnose_tool_loop_limit("max_tool_calls", Some(&observation));

        assert_eq!(
            diagnosis,
            ToolLoopLimitDiagnosis::TruncatedObservationContinuation
        );
    }

    #[test]
    fn diagnoses_path_failure_recovery_limit() {
        let observation =
            loop_observation(Some("src/missing.rs"), Some("path_not_found"), false, false);

        let diagnosis = diagnose_tool_loop_limit("max_tool_calls", Some(&observation));

        assert_eq!(diagnosis, ToolLoopLimitDiagnosis::PathFailureRecovery);
    }

    fn loop_observation(
        target_raw: Option<&str>,
        error_kind: Option<&'static str>,
        truncated: bool,
        has_next_range_hint: bool,
    ) -> ToolLoopObservation {
        ToolLoopObservation {
            tool_name: "read_file".to_owned(),
            target_raw: target_raw.map(str::to_owned),
            status: if error_kind.is_some() {
                "failed"
            } else {
                "succeeded"
            },
            error_kind,
            truncated,
            source_truncated: truncated,
            preview_truncated: false,
            has_next_range_hint,
            preview: Vec::new(),
        }
    }

    fn active_request() -> ActivePlainRequest {
        let (_sender, receiver) = mpsc::channel();
        ActivePlainRequest::new(
            "run-0001".to_owned(),
            "turn-1".to_owned(),
            "prompt".to_owned(),
            message(
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "schema",
            ),
            message(
                LlmMessageRole::User,
                LlmMessageVisibility::UserVisible,
                "user",
            ),
            MessageHistory::new("run-0001".to_owned()),
            receiver,
            0,
        )
    }
}
