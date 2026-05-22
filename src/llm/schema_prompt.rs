use super::history::{LlmMessage, LlmMessageRole, LlmMessageVisibility, MessageHistory};
use super::response_parser::RESPONSE_ACTIVITY_PAIRS;
use crate::tool::tool_argument_schema_lines;

pub const TOOL_MANIFEST_ID: &str = "ahreumcode.local-llm.tool-manifest.v1";
pub const TOOL_MANIFEST_VERSION: &str = "1";

const REQUIRED_SCHEMA_RULES: &[&str] = &[
    "Return exactly one next action candidate.",
    "Workspace evidence rules:",
    "Explore tool selection rules:",
    "Tool path selection rules:",
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
    "Integer fields must be JSON numbers, not strings.",
    "Omitted optional Explore bounds use runtime defaults",
    "workspace-relative path",
    "Do not ask the runtime to normalize",
    "http:// or https://",
    "Output boundary rules:",
    "The contract envelope must be the first non-whitespace text in the assistant response.",
    "Natural language outside the contract envelope and payload blocks is invalid.",
    "Do not wrap JSON in markdown or code fences.",
    "Payload block rules:",
    "A response with AHREUM_PAYLOAD block(s) must use this exact order: one AHREUM_ACTION block first, then AHREUM_PAYLOAD block(s).",
    "No text may appear before, between, or after contract blocks.",
    "Use plain JSON without AHREUM_PAYLOAD for Explore tools and short answers.",
    "Do not put source, patch, or file body text inside JSON string fields.",
    "answer_payload_id references exactly one AHREUM_PAYLOAD block with format=\"markdown\".",
    "payload_id references exactly one AHREUM_PAYLOAD block for source, patch, or file body text.",
    "Payload blocks without a matching payload id reference are invalid.",
    "apply_patch JSON arguments must contain payload_id only.",
    "Encode apply_patch target path and operation only in the patch target header.",
    "<AHREUM_ACTION>",
    "</AHREUM_ACTION>",
];

const RESPONSE_BOUNDARY_CONTRACT_LINES: &[&str] = &[
    "Output boundary rules:",
    "- The assistant response has one contract envelope and optional payload blocks.",
    "- The contract envelope must be the first non-whitespace text in the assistant response.",
    "- The contract envelope is either plain JSON or one AHREUM_ACTION block.",
    "- If AHREUM_ACTION is used, the response must start with <AHREUM_ACTION>.",
    "- Natural language outside the contract envelope and payload blocks is invalid.",
    "- Markdown or code fences around the contract envelope are invalid.",
];

pub(crate) fn response_boundary_contract_lines() -> &'static [&'static str] {
    RESPONSE_BOUNDARY_CONTRACT_LINES
}

const PAYLOAD_ORDERING_CONTRACT_LINES: &[&str] = &[
    "Payload block rules:",
    "- A response with AHREUM_PAYLOAD block(s) must use this exact order: one AHREUM_ACTION block first, then AHREUM_PAYLOAD block(s).",
    "- No text may appear before, between, or after contract blocks.",
    "- Use plain JSON without AHREUM_PAYLOAD for Explore tools and short answers.",
    "- Use AHREUM_PAYLOAD only for markdown answer bodies, source text, patch text, or file body text that is referenced from the JSON envelope.",
    "- answer_payload_id references exactly one AHREUM_PAYLOAD block with format=\"markdown\".",
    "- payload_id references exactly one AHREUM_PAYLOAD block for source, patch, or file body text.",
    "- Payload blocks without a matching payload id reference are invalid.",
];

pub(crate) fn payload_ordering_contract_lines() -> &'static [&'static str] {
    PAYLOAD_ORDERING_CONTRACT_LINES
}

const TOOL_PATH_SELECTION_CONTRACT_LINES: &[&str] = &[
    "Tool path selection rules:",
    "- Use path \".\" for the workspace root. Empty path strings are invalid.",
    "- read_file path must be an exact workspace-relative path from the user request or a previous observation.",
    "- If the target is a symbol, type, function, registry entry, tool mapping, or configuration key location, use search_text before read_file.",
    "- If the target is current structure, a directory, an unknown filename location, or a filename that failed as a direct path, use list_files before read_file.",
    "- If a previous observation reports path_not_found, not_a_file, or not_a_directory, do not retry the same path.",
    "- The runtime does not fuzzy-match, rename, or correct guessed paths.",
];

pub(crate) fn tool_path_selection_contract_lines() -> &'static [&'static str] {
    TOOL_PATH_SELECTION_CONTRACT_LINES
}

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
            "Return only the response contract.",
            "",
            "Runtime manifest:",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        lines.push(format!(
            "- tool_manifest_id is runtime-owned and resolves to: {TOOL_MANIFEST_ID}"
        ));
        lines.push(format!(
            "- tool_manifest_version is runtime-owned and resolves to: {TOOL_MANIFEST_VERSION}"
        ));
        lines.push(
            "- Every response must include tool_manifest_id and tool_manifest_version with those exact values."
                .to_owned(),
        );
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
                "Allowed response_type/activity pairs:",
            ]
            .into_iter()
            .map(str::to_owned),
        );

        for pair in RESPONSE_ACTIVITY_PAIRS {
            lines.push(format!("- {}", pair.rule_text()));
        }

        lines.extend(
            [
            "",
            "Response shape rules:",
            "- Unknown fields are rejected.",
            "- answer uses response_type, activity, message, tool_manifest_id, tool_manifest_version, and optional answer_payload_id.",
            "- tool uses response_type, activity, message, tool_manifest_id, tool_manifest_version, tool_name, arguments, and optional reason.",
            "- clarify uses response_type, activity, message, tool_manifest_id, tool_manifest_version, and optional reason.",
            "- blocked uses response_type, activity, message, tool_manifest_id, tool_manifest_version, and optional reason.",
            "- One response cannot contain multiple tool candidates.",
            "- Do not invent tool names or argument fields.",
            "- Tool arguments must match the provided typed argument schema.",
            "- Integer fields must be JSON numbers, not strings.",
            "- Omitted optional Explore bounds use runtime defaults: read_file start_line=1 max_lines=120, search_text max_results=20, list_files max_depth=2 max_entries=100.",
            "- HttpUrl fields must use http:// or https:// URLs.",
            "- Do not ask the runtime to normalize paths, URLs, command arguments, or payload ids.",
            "- Do not wrap JSON in markdown or code fences.",
        ]
            .into_iter()
            .map(str::to_owned),
        );
        lines.extend(
            response_boundary_contract_lines()
                .iter()
                .map(|line| line.to_string()),
        );
        lines.extend(
            [
            "",
            "Workspace evidence rules:",
            "- If the user asks about current workspace files, directories, code locations, implementations, dependencies, git state, configuration, or registered tools, do not answer from memory.",
            "- If no relevant AHREUM_TOOL_OBSERVATION is available for a workspace fact request, return exactly one Explore tool candidate.",
            "- Do not return blocked for a workspace request before trying a registered bounded tool that can make progress.",
            "- If the user asks to create new self-contained content and does not ask to inspect existing workspace state, do not Explore first. Return exactly one Change tool candidate with an apply_patch payload for the requested file.",
            "- If the user asks to modify, update, replace, or delete content in a named existing workspace file and no successful read_file observation for that target is available in this request, request read_file first; do not build an apply_patch payload from assumed file contents.",
            "- read_file observations include read_file_patch_content for exact file lines. Use that block for Update File hunks; preview line number prefixes are display-only.",
            "- Use answer with activity None only when no workspace evidence is needed, project context is sufficient, or the latest observation is enough evidence.",
            "",
            "Explore tool selection rules:",
            "- Use read_file when the user names an exact workspace file path or when a previous observation identifies the file to read.",
            "- If the needed evidence is the contents or values of a known file, use read_file directly; do not search source code for definitions of that file first.",
            "- Use list_files when the user asks for the current directory, project structure, or when a named file was not found at the direct path and its location must be discovered.",
            "- Use search_text when the user asks where a symbol, type, function, implementation, registry entry, tool mapping, or configuration key is defined.",
            "- After search_text returns candidate files or lines, use read_file if file content is needed before answering.",
            "- Do not use clarify when safe bounded read/search can reduce uncertainty.",
            "- Do not use list_files or search_text before a self-contained new-file Change unless existing workspace evidence is required to choose the target or content.",
            "",
            "Execute tool selection rules:",
            "- If the user asks to run, execute, check with a shell command, print the current working directory, or invoke a named command, use run_command with activity=Execute.",
            "- Do not replace a command execution request with list_files, read_file, search_text, or an answer unless a prior run_command observation already satisfies the request.",
        ]
            .into_iter()
            .map(str::to_owned),
        );
        lines.extend(
            tool_path_selection_contract_lines()
                .iter()
                .map(|line| line.to_string()),
        );
        lines.extend(
            ["", "Tool argument schemas:"]
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
            "- Plain JSON is preferred for Explore tools, clarify, blocked, and short answers.",
            "- Explore tools must not include AHREUM_PAYLOAD blocks because read/search/list arguments fit in JSON.",
            "- Do not put code or markdown answer bodies inside message.",
            "- Do not put source, patch, or file body text inside JSON string fields.",
        ]
            .into_iter()
            .map(str::to_owned),
        );
        lines.extend(
            payload_ordering_contract_lines()
                .iter()
                .map(|line| line.to_string()),
        );
        lines.extend(
            [
            "",
            "Explore tool contract:",
            "- Explore responses use response_type=tool and activity=Explore.",
            "- Select read_file, list_files, search_text, web_search, or web_fetch from the available tool registry according to the needed evidence.",
            "- Arguments must follow the matching tool argument schema below; do not copy sample paths or invent path corrections.",
            "",
            "apply_patch payload contract:",
            "- A Change tool that uses apply_patch must return one AHREUM_ACTION block followed by exactly one matching AHREUM_PAYLOAD block.",
            "- apply_patch JSON arguments must contain payload_id only.",
            "- Do not put target path, patch operation, patch text, or file body text in apply_patch JSON arguments.",
            "- arguments.payload_id must match the AHREUM_PAYLOAD id, and the payload format must be \"apply_patch\".",
            "- The payload body must be one complete patch document beginning with *** Begin Patch and ending with *** End Patch.",
            "- The payload body is the patch wrapper, not the final file body by itself.",
            "- For a new file, use *** Add File: <requested workspace path> and prefix every created content line with +.",
            "- Do not use Add File for a path whose current contents are being modified; read the file first, then use Update File with exact context lines.",
            "- For Update File, each hunk must include at least one matching existing context line prefixed with space or one removal line prefixed with -; a hunk with only + lines is invalid.",
            "- For Update File, bare content lines are invalid. Use exactly one line marker per hunk line: space for existing context, - for removed existing text, + for added replacement text, or @@ for the hunk boundary.",
            "- For Update File after read_file, copy exact existing lines from read_file_patch_content, not from display preview prefixes such as \"12: \".",
            "- The space, -, and + markers are the first character of the patch line, not separators. Do not add an extra space after - or + unless the file line itself starts with a space.",
            "- Plain JSON alone is invalid for apply_patch because the patch body must be carried in AHREUM_PAYLOAD.",
            "- Select the patch operation from the requested change and observed file state; do not copy a fixed operation or path from this contract.",
            "- Valid patch target headers are Add File for a new path, Update File for an existing path, Delete File for an existing path, and Move to only as part of a valid patch move.",
            "- Encode apply_patch target path and operation only in the patch target header.",
            "- Do not wrap the patch body in markdown fences.",
            "- apply_patch response shape template; replace payload id, target path, and patch lines with the actual requested change:",
            r#"<AHREUM_ACTION>"#,
            r#"{"response_type":"tool","activity":"Change","message":"prepared patch","tool_name":"apply_patch","arguments":{"payload_id":"patch_id"},"reason":"requested file change","tool_manifest_id":"ahreumcode.local-llm.tool-manifest.v1","tool_manifest_version":"1"}"#,
            r#"</AHREUM_ACTION>"#,
            r#"<AHREUM_PAYLOAD id="patch_id" format="apply_patch">"#,
            "*** Begin Patch",
            "*** Update File: workspace/relative/path",
            "@@",
            " existing context line",
            "-old line",
            "+new line",
            "*** End Patch",
            r#"</AHREUM_PAYLOAD>"#,
            "",
            "Example short answer:",
            r#"{"response_type":"answer","activity":"None","message":"Done.","tool_manifest_id":"ahreumcode.local-llm.tool-manifest.v1","tool_manifest_version":"1"}"#,
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
        attach_schema_prompt, payload_ordering_contract_lines, response_boundary_contract_lines,
        tool_path_selection_contract_lines, validate_schema_prompt, SchemaPromptBuilder,
        TOOL_MANIFEST_ID, TOOL_MANIFEST_VERSION,
    };
    use crate::llm::{LlmMessageRole, LlmMessageVisibility, MessageHistory};

    fn expected_pair_lines() -> Vec<String> {
        super::RESPONSE_ACTIVITY_PAIRS
            .iter()
            .map(|pair| format!("- {}", pair.rule_text()))
            .collect()
    }

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
        assert!(prompt.content.contains("web_search arguments"));
        assert!(prompt.content.contains("web_fetch arguments"));
        assert!(!prompt.content.contains("use_regex"));
        assert!(prompt
            .content
            .contains("Allowed response_type/activity pairs:"));
        for pair_line in expected_pair_lines() {
            assert!(prompt.content.contains(&pair_line));
        }
        assert!(prompt.content.contains("Workspace evidence rules:"));
        assert!(prompt.content.contains("Explore tool selection rules:"));
        assert!(prompt.content.contains("do not answer from memory"));
        assert!(prompt.content.contains("create new self-contained content"));
        assert!(prompt.content.contains("do not Explore first"));
        assert!(prompt
            .content
            .contains("no successful read_file observation for that target"));
        assert!(prompt
            .content
            .contains("Use search_text when the user asks where a symbol"));
        assert!(prompt
            .content
            .contains("Use read_file when the user names an exact workspace file path"));
        assert!(prompt
            .content
            .contains("Use list_files when the user asks for the current directory"));
        assert!(prompt
            .content
            .contains("named file was not found at the direct path"));
        assert!(prompt
            .content
            .contains("Use plain JSON without AHREUM_PAYLOAD for Explore tools"));
        assert!(prompt
            .content
            .contains("Explore tools must not include AHREUM_PAYLOAD blocks"));
        assert!(prompt.content.contains("Explore tool contract:"));
        assert!(prompt.content.contains("apply_patch payload contract:"));
        assert!(prompt
            .content
            .contains("apply_patch response shape template"));
        assert!(prompt.content.contains(r#""tool_name":"apply_patch""#));
        assert!(prompt
            .content
            .contains(r#"<AHREUM_PAYLOAD id="patch_id" format="apply_patch">"#));
        assert!(prompt.content.contains("payload body is the patch wrapper"));
        assert!(prompt
            .content
            .contains("prefix every created content line with +"));
        assert!(prompt
            .content
            .contains("Do not use Add File for a path whose current contents are being modified"));
        assert!(prompt.content.contains("Example short answer:"));
        assert!(prompt.content.contains("<AHREUM_ACTION>"));
        assert!(prompt.content.contains("</AHREUM_ACTION>"));
        for boundary_line in response_boundary_contract_lines() {
            assert!(prompt.content.contains(boundary_line));
        }
        for payload_line in payload_ordering_contract_lines() {
            assert!(prompt.content.contains(payload_line));
        }
        for path_line in tool_path_selection_contract_lines() {
            assert!(prompt.content.contains(path_line));
        }
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
