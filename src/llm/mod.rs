mod decision;
mod diagnostics;
mod history;
mod lm_studio;
mod provider;
mod repair;
mod response_parser;
mod schema_prompt;

pub use decision::{
    ChangePreview, ChangeTargetPreview, DecisionGate, PatchOperation, RuntimeDecision,
    RuntimeDecisionError,
};
pub use diagnostics::{
    LlmDiagnostics, LlmDiagnosticsRuntime, LlmDiagnosticsSnapshot, LlmDiagnosticsState,
};
pub use history::{LlmMessage, LlmMessageRole, LlmMessageVisibility, MessageHistory};
pub use provider::{
    ChatFailureKind, LlmChatFailure, LlmChatReport, LlmChatRequest, LlmChatStatus, LlmHealthReport,
    LlmHealthStatus, LlmProviderFactory,
};
pub use repair::{RepairLimitReached, RepairLoop, RepairRequest};
pub use response_parser::{
    parse_runtime_response, Activity, ParsedRuntimeResponse, PlanOperation, RuntimePlanItem,
    RuntimeResponseParseError, RuntimeResponseParseErrorKind,
};
pub use schema_prompt::{attach_schema_prompt, SchemaPrompt, SchemaPromptBuilder};
pub(crate) use schema_prompt::{
    payload_ordering_contract_lines, response_boundary_contract_lines,
    tool_path_selection_contract_lines,
};
