#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageHistory {
    run_id: String,
    next_turn_index: u64,
    messages: Vec<LlmMessage>,
}

impl MessageHistory {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            next_turn_index: 1,
            messages: Vec::new(),
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn next_turn_id(&mut self) -> String {
        let turn_id = format!(
            "{run_id}-turn-{number:04}",
            run_id = self.run_id,
            number = self.next_turn_index
        );
        self.next_turn_index += 1;
        turn_id
    }

    pub fn append(
        &mut self,
        turn_id: impl Into<String>,
        role: LlmMessageRole,
        visibility: LlmMessageVisibility,
        content: impl Into<String>,
    ) -> LlmMessage {
        let message = LlmMessage {
            turn_id: turn_id.into(),
            role,
            visibility,
            content: content.into(),
        };
        self.messages.push(message.clone());
        message
    }

    pub fn for_request(&self, limit: Option<usize>) -> Vec<LlmMessage> {
        let Some(limit) = limit else {
            return self.messages.clone();
        };

        if self.messages.len() <= limit {
            return self.messages.clone();
        }

        let system_messages = self
            .messages
            .iter()
            .filter(|message| message.role == LlmMessageRole::System);
        let non_system_messages = self
            .messages
            .iter()
            .filter(|message| message.role != LlmMessageRole::System)
            .collect::<Vec<_>>();

        let remaining = limit.saturating_sub(
            self.messages
                .iter()
                .filter(|message| message.role == LlmMessageRole::System)
                .count(),
        );
        let tail_start = non_system_messages.len().saturating_sub(remaining);

        system_messages
            .chain(non_system_messages[tail_start..].iter().copied())
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmMessage {
    pub turn_id: String,
    pub role: LlmMessageRole,
    pub visibility: LlmMessageVisibility,
    pub content: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LlmMessageRole {
    System,
    User,
    Assistant,
}

impl LlmMessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LlmMessageVisibility {
    Internal,
    UserVisible,
}

impl LlmMessageVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::UserVisible => "user_visible",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LlmMessageRole, LlmMessageVisibility, MessageHistory};

    #[test]
    fn assigns_turn_ids_for_run() {
        let mut history = MessageHistory::new("run-0001");

        assert_eq!(history.next_turn_id(), "run-0001-turn-0001");
        assert_eq!(history.next_turn_id(), "run-0001-turn-0002");
    }

    #[test]
    fn stores_messages_in_order() {
        let mut history = MessageHistory::new("run-0001");
        history.append(
            "turn-1",
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "system",
        );
        history.append(
            "turn-1",
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            "hello",
        );

        let messages = history.for_request(None);
        assert_eq!(messages[0].content, "system");
        assert_eq!(messages[1].content, "hello");
    }

    #[test]
    fn limits_request_messages_from_tail() {
        let mut history = MessageHistory::new("run-0001");
        for index in 0..3 {
            history.append(
                "turn-1",
                LlmMessageRole::User,
                LlmMessageVisibility::UserVisible,
                format!("message-{index}"),
            );
        }

        let messages = history.for_request(Some(2));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "message-1");
        assert_eq!(messages[1].content, "message-2");
    }

    #[test]
    fn preserves_system_messages_before_limited_tail() {
        let mut history = MessageHistory::new("run-0001");
        history.append(
            "turn-1",
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "schema",
        );
        for index in 0..3 {
            history.append(
                "turn-1",
                LlmMessageRole::User,
                LlmMessageVisibility::UserVisible,
                format!("message-{index}"),
            );
        }

        let messages = history.for_request(Some(2));

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, LlmMessageRole::System);
        assert_eq!(messages[0].content, "schema");
        assert_eq!(messages[1].content, "message-2");
    }

    #[test]
    fn preserves_all_system_messages_even_when_they_exceed_limit() {
        let mut history = MessageHistory::new("run-0001");
        history.append(
            "turn-1",
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "schema",
        );
        history.append(
            "turn-1",
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "guardrails",
        );
        history.append(
            "turn-1",
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            "message",
        );

        let messages = history.for_request(Some(1));

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "schema");
        assert_eq!(messages[1].content, "guardrails");
        assert!(messages
            .iter()
            .all(|message| message.role == LlmMessageRole::System));
    }
}
