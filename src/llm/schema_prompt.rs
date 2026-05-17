use super::history::{LlmMessage, LlmMessageRole, LlmMessageVisibility, MessageHistory};
use crate::tool::tool_argument_schema_lines;

pub const TOOL_MANIFEST_ID: &str = "ahreumcode.local-llm.tool-manifest.v1";
pub const TOOL_MANIFEST_VERSION: &str = "1";

const REQUIRED_SCHEMA_RULES: &[&str] = &[
    "Return exactly one next action candidate.",
    "response_type",
    "answer",
    "tool",
    "clarify",
    "blocked",
    "activity",
    "None",
    "Explore",
    "Change",
    "Execute",
    "Configure",
    "Ask",
    "tool_manifest_id",
    "tool_manifest_version",
    "Unknown fields are rejected.",
    "Tool argument schemas:",
    "workspace-relative path",
    "Do not ask the runtime to normalize",
    "http:// or https://",
    "Do not wrap JSON in markdown or code fences.",
    "Use answer_payload_id and raw markdown payload blocks for code or markdown answers.",
    "Do not put source, patch, or file body text inside JSON string fields.",
    "Use payload_id and raw payload blocks for source, patch, or file body text.",
    "If any AHREUM_PAYLOAD block is present, wrap the action JSON in AHREUM_ACTION tags.",
    "<AHREUM_ACTION>",
    "</AHREUM_ACTION>",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaPrompt {
    pub tool_manifest_id: &'static str,
    pub tool_manifest_version: &'static str,
    pub content: String,
}

pub struct SchemaPromptBuilder;

impl SchemaPromptBuilder {
    pub fn build() -> Result<SchemaPrompt, SchemaPromptBuildError> {
        let mut lines = [
            "You are AhreumCode local LLM runtime.",
            "",
            "Return exactly one next action candidate.",
            "Return only the response contract. Do not mix unrelated prose before or after it.",
            "",
            "Required manifest fields:",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        lines.push(format!("- tool_manifest_id: {TOOL_MANIFEST_ID}"));
        lines.push(format!("- tool_manifest_version: {TOOL_MANIFEST_VERSION}"));
        lines.extend(
            [
            "",
            "Allowed response_type values:",
            "- answer",
            "- tool",
            "- clarify",
            "- blocked",
            "",
            "Allowed activity values:",
            "- None",
            "- Explore",
            "- Change",
            "- Execute",
            "- Configure",
            "- Ask",
            "",
            "Response shape rules:",
            "- Unknown fields are rejected.",
            "- answer uses response_type, activity, message, optional answer_payload_id, tool_manifest_id, tool_manifest_version.",
            "- tool uses response_type, activity, message, tool_name, arguments, reason, tool_manifest_id, tool_manifest_version.",
            "- clarify uses response_type, activity, message, reason, tool_manifest_id, tool_manifest_version.",
            "- blocked uses response_type, activity, message, reason, tool_manifest_id, tool_manifest_version.",
            "- One response cannot contain multiple tool candidates.",
            "- Do not invent tool names or argument fields.",
            "- Tool arguments must match the provided typed argument schema.",
            "- Do not ask the runtime to normalize paths, URLs, command arguments, or payload ids.",
            "- Do not wrap JSON in markdown or code fences.",
            "",
            "Tool argument schemas:",
        ]
            .into_iter()
            .map(str::to_owned),
        );

        for schema_line in tool_argument_schema_lines() {
            lines.push(format!("- {schema_line}"));
        }

        lines.extend(
            [
            "- workspace-relative path means non-empty, not absolute, no '..', and no control characters.",
            "",
            "Raw payload rules:",
            "- Do not put code or markdown answer bodies inside message.",
            "- Use answer_payload_id and raw markdown payload blocks for code or markdown answers.",
            "- Do not put source, patch, or file body text inside JSON string fields.",
            "- Use payload_id and raw payload blocks for source, patch, or file body text.",
            "- If any AHREUM_PAYLOAD block is present, wrap the action JSON in AHREUM_ACTION tags.",
            "- A payload_id reference must have exactly one matching raw payload block.",
            "",
            "Example payload answer:",
            r#"<AHREUM_ACTION>"#,
            r#"{"response_type":"answer","activity":"None","message":"short summary","answer_payload_id":"answer_001","tool_manifest_id":"ahreumcode.local-llm.tool-manifest.v1","tool_manifest_version":"1"}"#,
            r#"</AHREUM_ACTION>"#,
            r#"<AHREUM_PAYLOAD id="answer_001" format="markdown">answer body</AHREUM_PAYLOAD>"#,
        ]
            .into_iter()
            .map(str::to_owned),
        );
        let content = lines.join("\n");

        validate_schema_prompt(&content)?;

        Ok(SchemaPrompt {
            tool_manifest_id: TOOL_MANIFEST_ID,
            tool_manifest_version: TOOL_MANIFEST_VERSION,
            content,
        })
    }
}

pub fn attach_schema_prompt(
    history: &mut MessageHistory,
    turn_id: impl Into<String>,
    prompt: &SchemaPrompt,
) -> LlmMessage {
    history.append(
        turn_id,
        LlmMessageRole::System,
        LlmMessageVisibility::Internal,
        prompt.content.clone(),
    )
}

pub fn validate_schema_prompt(prompt: &str) -> Result<(), SchemaPromptBuildError> {
    for rule in REQUIRED_SCHEMA_RULES {
        if !prompt.contains(rule) {
            return Err(SchemaPromptBuildError {
                missing_rule: (*rule).to_owned(),
            });
        }
    }

    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaPromptBuildError {
    pub missing_rule: String,
}

#[cfg(test)]
mod tests {
    use super::{
        attach_schema_prompt, validate_schema_prompt, SchemaPromptBuilder, TOOL_MANIFEST_ID,
        TOOL_MANIFEST_VERSION,
    };
    use crate::llm::{LlmMessageRole, LlmMessageVisibility, MessageHistory};

    #[test]
    fn builds_schema_prompt_with_manifest_and_contract_rules() {
        let prompt = SchemaPromptBuilder::build().expect("schema prompt should build");

        assert_eq!(prompt.tool_manifest_id, TOOL_MANIFEST_ID);
        assert_eq!(prompt.tool_manifest_version, TOOL_MANIFEST_VERSION);
        assert!(prompt
            .content
            .contains("Return exactly one next action candidate."));
        assert!(prompt.content.contains("Unknown fields are rejected."));
        assert!(prompt.content.contains("payload_id"));
        assert!(prompt.content.contains("read_file arguments"));
        assert!(!prompt.content.contains("find_files arguments"));
        assert!(!prompt.content.contains("web_search arguments"));
        assert!(!prompt.content.contains("use_regex"));
        assert!(prompt.content.contains("<AHREUM_ACTION>"));
        assert!(prompt.content.contains("</AHREUM_ACTION>"));
        assert!(prompt.content.contains(
            "If any AHREUM_PAYLOAD block is present, wrap the action JSON in AHREUM_ACTION tags."
        ));
    }

    #[test]
    fn payload_answer_example_uses_action_framing() {
        let prompt = SchemaPromptBuilder::build().expect("schema prompt should build");
        let action_open = prompt.content.find("<AHREUM_ACTION>").expect("action open");
        let action_close = prompt
            .content
            .find("</AHREUM_ACTION>")
            .expect("action close");
        let payload = prompt
            .content
            .find(r#"<AHREUM_PAYLOAD id="answer_001" format="markdown">"#)
            .expect("payload example");

        assert!(action_open < action_close);
        assert!(action_close < payload);
    }

    #[test]
    fn rejects_prompt_missing_required_rule() {
        let error = validate_schema_prompt("response_type")
            .expect_err("incomplete schema prompt should fail validation");

        assert_eq!(
            error.missing_rule,
            "Return exactly one next action candidate."
        );
    }

    #[test]
    fn attaches_schema_prompt_as_internal_system_message() {
        let prompt = SchemaPromptBuilder::build().expect("schema prompt should build");
        let mut history = MessageHistory::new("run-0001");

        let message = attach_schema_prompt(&mut history, "turn-1", &prompt);

        assert_eq!(message.role, LlmMessageRole::System);
        assert_eq!(message.visibility, LlmMessageVisibility::Internal);
        assert_eq!(message.content, prompt.content);
    }
}
