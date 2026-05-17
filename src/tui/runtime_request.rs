use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::config::RuntimeConfig;
use crate::llm::{
    LlmChatReport, LlmChatRequest, LlmMessage, LlmMessageRole, LlmProviderFactory, MessageHistory,
    RuntimeDecision,
};

pub(super) struct ActivePlainRequest {
    pub(super) run_id: String,
    pub(super) turn_id: String,
    pub(super) prompt: String,
    pub(super) history: MessageHistory,
    pub(super) receiver: Receiver<LlmChatReport>,
    pub(super) cancelled: bool,
    pub(super) repair_attempts: u16,
    pub(super) tool_call_count: u16,
    pub(super) last_tool_signature: Option<String>,
    pub(super) same_tool_repeat_count: u16,
}

impl ActivePlainRequest {
    pub(super) fn new(
        run_id: String,
        turn_id: String,
        prompt: String,
        history: MessageHistory,
        receiver: Receiver<LlmChatReport>,
    ) -> Self {
        Self {
            run_id,
            turn_id,
            prompt,
            history,
            receiver,
            cancelled: false,
            repair_attempts: 0,
            tool_call_count: 0,
            last_tool_signature: None,
            same_tool_repeat_count: 0,
        }
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
