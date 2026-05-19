use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::config::RuntimeConfig;
use crate::llm::{
    payload_ordering_contract_lines, response_boundary_contract_lines,
    tool_path_selection_contract_lines, LlmChatReport, LlmChatRequest, LlmMessage, LlmMessageRole,
    LlmProviderFactory, MessageHistory, RuntimeDecision,
};
use crate::tool::ToolObservation;

pub(super) struct ActivePlainRequest {
    pub(super) run_id: String,
    pub(super) turn_id: String,
    pub(super) prompt: String,
    pub(super) schema_message: LlmMessage,
    pub(super) user_message: LlmMessage,
    pub(super) history: MessageHistory,
    pub(super) receiver: Receiver<LlmChatReport>,
    pub(super) persona_context_start: usize,
    pub(super) cancelled: bool,
    pub(super) repair_attempts: u16,
    pub(super) tool_call_count: u16,
    pub(super) last_tool_signature: Option<String>,
    pub(super) executed_tool_signatures: Vec<String>,
    pub(super) executed_tool_records: Vec<ToolLoopExecutionRecord>,
    pub(super) same_tool_repeat_count: u16,
    pub(super) last_tool_observation: Option<ToolLoopObservation>,
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
            persona_context_start,
            cancelled: false,
            repair_attempts: 0,
            tool_call_count: 0,
            last_tool_signature: None,
            executed_tool_signatures: Vec::new(),
            executed_tool_records: Vec::new(),
            same_tool_repeat_count: 0,
            last_tool_observation: None,
        }
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
        self.last_tool_observation = Some(loop_observation);
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
            "Request exactly one next tool only when more workspace evidence is required.",
            "Use search_text when the next missing evidence is a symbol, implementation, registry entry, tool mapping, or configuration key location.",
            "Use read_file when the next missing evidence is content from a known workspace file.",
            "Use list_files when the next missing evidence is current workspace, directory structure, or a filename/path candidate after a direct path failed.",
            "If the latest observation failed, do not treat its target as read or analyzed evidence.",
            "If a path failure cannot be resolved by a bounded next tool, return an answer or blocked response that reports the unresolved path instead of inventing file contents.",
            "If the latest observation has next_range_hint and more content is needed, follow that hint.",
            "Do not repeat an already executed tool candidate with the same tool_name and arguments.",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    instruction_lines.extend(tool_loop_state_lines(executed_tool_signatures));

    vec![
        schema_message.clone(),
        user_message.clone(),
        observation_message.clone(),
        LlmMessage {
            turn_id: next_turn_id.to_owned(),
            role: LlmMessageRole::System,
            visibility: observation_message.visibility,
            content: instruction_lines.join("\n"),
        },
    ]
}

pub(super) fn tool_repeat_answer_request_messages(
    schema_message: &LlmMessage,
    user_message: &LlmMessage,
    execution_record: &ToolLoopExecutionRecord,
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
            "Request a tool only if a different tool candidate is required for missing evidence.",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    instruction_lines.extend(tool_loop_state_lines(executed_tool_signatures));

    vec![
        schema_message.clone(),
        user_message.clone(),
        execution_record.observation_message.clone(),
        LlmMessage {
            turn_id: next_turn_id.to_owned(),
            role: LlmMessageRole::System,
            visibility: execution_record.observation_message.visibility,
            content: instruction_lines.join("\n"),
        },
    ]
}

pub(super) fn tool_repeat_continuation_request_messages(
    schema_message: &LlmMessage,
    user_message: &LlmMessage,
    execution_record: &ToolLoopExecutionRecord,
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
            "If different evidence is required, request exactly one different tool candidate.",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    instruction_lines.extend(tool_loop_state_lines(executed_tool_signatures));

    vec![
        schema_message.clone(),
        user_message.clone(),
        execution_record.observation_message.clone(),
        LlmMessage {
            turn_id: next_turn_id.to_owned(),
            role: LlmMessageRole::System,
            visibility: execution_record.observation_message.visibility,
            content: instruction_lines.join("\n"),
        },
    ]
}

pub(super) fn tool_repeat_failure_request_messages(
    schema_message: &LlmMessage,
    user_message: &LlmMessage,
    execution_record: &ToolLoopExecutionRecord,
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

    vec![
        schema_message.clone(),
        user_message.clone(),
        execution_record.observation_message.clone(),
        LlmMessage {
            turn_id: next_turn_id.to_owned(),
            role: LlmMessageRole::System,
            visibility: execution_record.observation_message.visibility,
            content: instruction_lines.join("\n"),
        },
    ]
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
        diagnose_tool_loop_limit, tool_loop_request_messages, tool_repeat_answer_request_messages,
        tool_repeat_continuation_request_messages, tool_repeat_failure_request_messages,
        ActivePlainRequest, ToolLoopExecutionRecord, ToolLoopLimitDiagnosis, ToolLoopObservation,
        ToolLoopRepeatRedirect,
    };
    use crate::llm::{
        payload_ordering_contract_lines, response_boundary_contract_lines,
        tool_path_selection_contract_lines, LlmMessage, LlmMessageRole, LlmMessageVisibility,
        MessageHistory,
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
            tool_loop_request_messages(&schema, &user, &observation, &executed, "turn-2");

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
            .contains("do not treat its target as read or analyzed evidence"));
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

        let messages =
            tool_repeat_answer_request_messages(&schema, &user, &record, &executed, "turn-2");

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
        assert!(messages[3].content.contains("<AHREUM_TOOL_LOOP_STATE>"));
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

        let messages =
            tool_repeat_continuation_request_messages(&schema, &user, &record, &executed, "turn-2");

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

        let messages =
            tool_repeat_failure_request_messages(&schema, &user, &record, &executed, "turn-2");

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
