use super::response_parser::RuntimeResponseParseError;

pub const MAX_REPAIR_ATTEMPTS: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairRequest {
    pub attempt: u16,
    pub max_attempts: u16,
    pub failure_signature: String,
    pub prompt: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairLimitReached {
    pub attempts: u16,
    pub max_attempts: u16,
    pub failure_signature: String,
    pub reason: RepairLimitReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepairLimitReason {
    MaxAttemptsReached,
}

impl RepairLimitReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MaxAttemptsReached => "max_attempts_reached",
        }
    }
}

pub struct RepairLoop {
    max_attempts: u16,
}

impl RepairLoop {
    pub fn new(max_attempts: u16) -> Self {
        Self { max_attempts }
    }

    pub fn default_local() -> Self {
        Self::new(MAX_REPAIR_ATTEMPTS)
    }

    pub fn max_attempts(&self) -> u16 {
        self.max_attempts
    }

    pub fn next_request(
        &self,
        attempts: u16,
        error: &RuntimeResponseParseError,
    ) -> Result<RepairRequest, RepairLimitReached> {
        let failure_signature = failure_signature(error);
        if attempts >= self.max_attempts {
            return Err(RepairLimitReached {
                attempts,
                max_attempts: self.max_attempts,
                failure_signature,
                reason: RepairLimitReason::MaxAttemptsReached,
            });
        }

        let attempt = attempts + 1;
        Ok(RepairRequest {
            attempt,
            max_attempts: self.max_attempts,
            failure_signature,
            prompt: build_repair_prompt(error, attempt, self.max_attempts),
        })
    }
}

pub fn build_repair_prompt(
    error: &RuntimeResponseParseError,
    attempt: u16,
    max_attempts: u16,
) -> String {
    [
        "The previous assistant response did not satisfy the AhreumCode response contract.",
        "Regenerate the response for the same user intent.",
        "",
        "Repair constraints:",
        "- Return exactly one response contract.",
        "- Return only valid JSON, or one AHREUM_ACTION block plus required AHREUM_PAYLOAD blocks.",
        "- Do not include natural language before or after the contract.",
        "- Do not add unknown fields.",
        "- Do not put source, patch, or file body text inside JSON string fields.",
        "- Use payload_id and AHREUM_PAYLOAD blocks for source, patch, or file body text.",
        "",
        &format!("Repair attempt: {attempt}/{max_attempts}"),
        &format!("Failure kind: {}", error.kind.as_str()),
        &format!("Failure message: {}", error.message),
    ]
    .join("\n")
}

fn failure_signature(error: &RuntimeResponseParseError) -> String {
    format!("{}:{}", error.kind.as_str(), error.message)
}

#[cfg(test)]
mod tests {
    use super::{build_repair_prompt, RepairLimitReason, RepairLoop};
    use crate::llm::{RuntimeResponseParseError, RuntimeResponseParseErrorKind};

    fn parse_error() -> RuntimeResponseParseError {
        RuntimeResponseParseError {
            kind: RuntimeResponseParseErrorKind::JsonParseFailed,
            message: "expected value".to_owned(),
        }
    }

    #[test]
    fn builds_repair_prompt_with_failure_reason() {
        let prompt = build_repair_prompt(&parse_error(), 1, 1);

        assert!(prompt.contains("same user intent"));
        assert!(prompt.contains("Failure kind: json_parse_failed"));
        assert!(prompt.contains("Failure message: expected value"));
        assert!(prompt.contains("Repair attempt: 1/1"));
    }

    #[test]
    fn allows_first_repair_attempt() {
        let loop_state = RepairLoop::default_local();

        let request = loop_state
            .next_request(0, &parse_error())
            .expect("first repair should be allowed");

        assert_eq!(request.attempt, 1);
        assert_eq!(request.max_attempts, 1);
    }

    #[test]
    fn blocks_after_repair_limit() {
        let loop_state = RepairLoop::default_local();

        let limit = loop_state
            .next_request(1, &parse_error())
            .expect_err("second repair should be blocked");

        assert_eq!(limit.attempts, 1);
        assert_eq!(limit.reason, RepairLimitReason::MaxAttemptsReached);
    }
}
