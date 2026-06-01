use std::collections::{BTreeSet, HashSet};

use serde_json::{Map, Value};

use super::schema_prompt::{TOOL_MANIFEST_ID, TOOL_MANIFEST_VERSION};

const ACTION_OPEN: &str = "<AHREUM_ACTION>";
const ACTION_CLOSE: &str = "</AHREUM_ACTION>";
const PAYLOAD_OPEN_PREFIX: &str = "<AHREUM_PAYLOAD ";
const PAYLOAD_CLOSE: &str = "</AHREUM_PAYLOAD>";

const COMMON_FIELDS: &[&str] = &[
    "response_type",
    "activity",
    "message",
    "tool_manifest_id",
    "tool_manifest_version",
];
const ANSWER_FIELDS: &[&str] = &["answer_payload_id"];
const TOOL_FIELDS: &[&str] = &["tool_name", "arguments", "reason"];
const PLAN_FIELDS: &[&str] = &["plan_items", "reason"];
const REASON_FIELD: &[&str] = &["reason"];
const FORBIDDEN_RAW_ARGUMENT_FIELDS: &[&str] = &[
    "content",
    "patch",
    "file_body",
    "file_content",
    "source",
    "source_code",
    "code",
    "command_body",
];

pub const RESPONSE_ACTIVITY_PAIRS: &[ResponseActivityPair] = &[
    ResponseActivityPair {
        response_type: "answer",
        activities: &[Activity::None],
    },
    ResponseActivityPair {
        response_type: "tool",
        activities: &[
            Activity::Explore,
            Activity::Change,
            Activity::Execute,
            Activity::Configure,
        ],
    },
    ResponseActivityPair {
        response_type: "plan",
        activities: &[Activity::None],
    },
    ResponseActivityPair {
        response_type: "clarify",
        activities: &[Activity::Ask],
    },
    ResponseActivityPair {
        response_type: "blocked",
        activities: &[Activity::None, Activity::Ask],
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseActivityPair {
    pub response_type: &'static str,
    pub activities: &'static [Activity],
}

impl ResponseActivityPair {
    pub fn rule_text(self) -> String {
        let activities = self
            .activities
            .iter()
            .map(|activity| activity.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{} -> activity {}", self.response_type, activities)
    }

    fn allows(self, response_type: &str, activity: Activity) -> bool {
        self.response_type == response_type && self.activities.contains(&activity)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedRuntimeResponse {
    pub response: RuntimeResponse,
    pub payloads: Vec<RuntimePayload>,
}

impl ParsedRuntimeResponse {
    pub fn tool_candidate_for_logging(&self) -> Option<RuntimeToolCandidateLog<'_>> {
        let RuntimeResponse::Tool(candidate) = &self.response else {
            return None;
        };
        Some(RuntimeToolCandidateLog {
            tool_name: &candidate.tool_name,
            arguments: &candidate.arguments,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeToolCandidateLog<'a> {
    pub tool_name: &'a str,
    pub arguments: &'a Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeResponseEnvelopeDiagnostic {
    pub(crate) response_type: Option<String>,
    pub(crate) activity: Option<String>,
    pub(crate) tool_name: Option<String>,
}

pub(crate) fn parse_runtime_response_envelope_diagnostic(
    raw: &str,
) -> Option<RuntimeResponseEnvelopeDiagnostic> {
    let unwrapped = unwrap_whole_markdown_fence(raw.trim());
    let contract_raw = isolate_contract_segment(unwrapped).ok()?;
    let json_raw = split_action_and_payloads(contract_raw)
        .map(|(json_raw, _)| json_raw)
        .or_else(|_| leading_unframed_json_before_payload(contract_raw))
        .ok()?;
    let value = serde_json::from_str::<Value>(json_raw.trim()).ok()?;
    let envelope = value.as_object()?;

    Some(RuntimeResponseEnvelopeDiagnostic {
        response_type: envelope
            .get("response_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        activity: envelope
            .get("activity")
            .and_then(Value::as_str)
            .map(str::to_owned),
        tool_name: envelope
            .get("tool_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn leading_unframed_json_before_payload(raw: &str) -> Result<&str, RuntimeResponseParseError> {
    let Some(payload_start) = raw.find(PAYLOAD_OPEN_PREFIX) else {
        return Err(RuntimeResponseParseError::schema(
            "no unframed payload block to diagnose",
        ));
    };
    let json_candidate = raw[..payload_start].trim();
    if json_candidate.is_empty() {
        return Err(RuntimeResponseParseError::schema(
            "missing JSON envelope before payload block",
        ));
    }
    Ok(json_candidate)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeResponse {
    Answer(RuntimeAnswer),
    Tool(RuntimeToolCandidate),
    Plan(RuntimePlan),
    Clarify(RuntimeClarification),
    Blocked(RuntimeBlocked),
}

impl RuntimeResponse {
    pub fn response_type(&self) -> &'static str {
        match self {
            Self::Answer(_) => "answer",
            Self::Tool(_) => "tool",
            Self::Plan(_) => "plan",
            Self::Clarify(_) => "clarify",
            Self::Blocked(_) => "blocked",
        }
    }

    pub fn activity(&self) -> Activity {
        match self {
            Self::Answer(response) => response.activity,
            Self::Tool(response) => response.activity,
            Self::Plan(response) => response.activity,
            Self::Clarify(response) => response.activity,
            Self::Blocked(response) => response.activity,
        }
    }

    pub fn manifest(&self) -> &RuntimeManifest {
        match self {
            Self::Answer(response) => &response.manifest,
            Self::Tool(response) => &response.manifest,
            Self::Plan(response) => &response.manifest,
            Self::Clarify(response) => &response.manifest,
            Self::Blocked(response) => &response.manifest,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeAnswer {
    pub activity: Activity,
    pub message: String,
    pub answer_payload_id: Option<String>,
    pub manifest: RuntimeManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeToolCandidate {
    pub activity: Activity,
    pub message: String,
    pub tool_name: String,
    pub arguments: Value,
    pub reason: String,
    pub manifest: RuntimeManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePlan {
    pub activity: Activity,
    pub message: String,
    pub plan_items: Vec<RuntimePlanItem>,
    pub reason: String,
    pub manifest: RuntimeManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePlanItem {
    pub operation: PlanOperation,
    pub target: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanOperation {
    Read,
    Create,
    Update,
    Delete,
    Execute,
    Verify,
    Answer,
}

impl PlanOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Execute => "execute",
            Self::Verify => "verify",
            Self::Answer => "answer",
        }
    }

    fn parse(raw: &str) -> Result<Self, RuntimeResponseParseError> {
        match raw {
            "read" => Ok(Self::Read),
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "delete" => Ok(Self::Delete),
            "execute" => Ok(Self::Execute),
            "verify" => Ok(Self::Verify),
            "answer" => Ok(Self::Answer),
            value => Err(RuntimeResponseParseError::schema(format!(
                "unknown plan operation: {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeClarification {
    pub activity: Activity,
    pub message: String,
    pub reason: String,
    pub manifest: RuntimeManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeBlocked {
    pub activity: Activity,
    pub message: String,
    pub reason: String,
    pub manifest: RuntimeManifest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activity {
    None,
    Explore,
    Change,
    Execute,
    Configure,
    Ask,
}

impl Activity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Explore => "Explore",
            Self::Change => "Change",
            Self::Execute => "Execute",
            Self::Configure => "Configure",
            Self::Ask => "Ask",
        }
    }

    fn parse(raw: &str) -> Result<Self, RuntimeResponseParseError> {
        match raw {
            "None" => Ok(Self::None),
            "Explore" => Ok(Self::Explore),
            "Change" => Ok(Self::Change),
            "Execute" => Ok(Self::Execute),
            "Configure" => Ok(Self::Configure),
            "Ask" => Ok(Self::Ask),
            value => Err(RuntimeResponseParseError::schema(format!(
                "unknown activity: {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeManifest {
    pub tool_manifest_id: String,
    pub tool_manifest_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePayload {
    pub id: String,
    pub format: String,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeResponseParseError {
    pub kind: RuntimeResponseParseErrorKind,
    pub message: String,
}

impl RuntimeResponseParseError {
    fn json(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeResponseParseErrorKind::JsonParseFailed,
            message: message.into(),
        }
    }

    fn schema(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeResponseParseErrorKind::SchemaValidationFailed,
            message: message.into(),
        }
    }

    fn payload(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeResponseParseErrorKind::PayloadValidationFailed,
            message: message.into(),
        }
    }

    fn partial(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeResponseParseErrorKind::PartialResponse,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeResponseParseErrorKind {
    JsonParseFailed,
    SchemaValidationFailed,
    PayloadValidationFailed,
    PartialResponse,
}

impl RuntimeResponseParseErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JsonParseFailed => "json_parse_failed",
            Self::SchemaValidationFailed => "schema_validation_failed",
            Self::PayloadValidationFailed => "payload_validation_failed",
            Self::PartialResponse => "partial_response",
        }
    }
}

pub fn parse_runtime_response(
    raw: &str,
) -> Result<ParsedRuntimeResponse, RuntimeResponseParseError> {
    let unwrapped = unwrap_whole_markdown_fence(raw.trim());
    let contract_raw = isolate_contract_segment(unwrapped)?;
    let (json_raw, payload_raw) = split_action_and_payloads(contract_raw)?;
    let json_raw = isolate_json_envelope(json_raw)?;
    let value = serde_json::from_str::<Value>(json_raw.trim())
        .map_err(|source| RuntimeResponseParseError::json(source.to_string()))?;
    let envelope = value.as_object().ok_or_else(|| {
        RuntimeResponseParseError::schema("response envelope must be a JSON object")
    })?;
    let payloads = parse_runtime_payloads(payload_raw)?;
    let response = to_runtime_response(envelope, &payloads)?;
    validate_payload_reference(&response, &payloads)?;

    Ok(ParsedRuntimeResponse { response, payloads })
}

pub(crate) fn unwrap_whole_markdown_fence(raw: &str) -> &str {
    let Some(body) = raw.strip_prefix("```") else {
        return raw;
    };
    let Some(last_line_start) = body.rfind("\n```") else {
        return raw;
    };
    if !body[last_line_start + 1..].trim().eq("```") {
        return raw;
    }

    let Some(first_line_end) = raw.find('\n') else {
        return raw;
    };
    raw[first_line_end + 1..raw.len() - 3].trim()
}

fn isolate_contract_segment(raw: &str) -> Result<&str, RuntimeResponseParseError> {
    let trimmed = raw.trim();
    let Some(start) = trimmed.find(ACTION_OPEN) else {
        return Ok(trimmed);
    };
    if trimmed[start + ACTION_OPEN.len()..].contains(ACTION_OPEN) {
        return Err(RuntimeResponseParseError::schema(
            "response must contain exactly one AHREUM_ACTION block",
        ));
    }

    let action_close_end = trimmed
        .find(ACTION_CLOSE)
        .map(|index| index + ACTION_CLOSE.len())
        .ok_or_else(|| RuntimeResponseParseError::partial("missing AHREUM_ACTION close tag"))?;
    let end = trimmed
        .rfind(PAYLOAD_CLOSE)
        .map(|index| index + PAYLOAD_CLOSE.len())
        .unwrap_or(action_close_end);

    Ok(trimmed[start..end].trim())
}

fn split_action_and_payloads(raw: &str) -> Result<(&str, &str), RuntimeResponseParseError> {
    let trimmed = raw.trim();
    if trimmed.starts_with(ACTION_OPEN) {
        return split_leading_action(trimmed);
    }

    if trimmed.contains(ACTION_OPEN)
        || trimmed.contains(ACTION_CLOSE)
        || trimmed.contains(PAYLOAD_OPEN_PREFIX)
        || trimmed.contains(PAYLOAD_CLOSE)
    {
        return Err(RuntimeResponseParseError::schema(
            "framed response must start with AHREUM_ACTION",
        ));
    }

    Ok((trimmed, ""))
}

fn isolate_json_envelope(raw: &str) -> Result<&str, RuntimeResponseParseError> {
    let trimmed = raw.trim();
    if trimmed.starts_with(ACTION_OPEN) {
        return Ok(trimmed);
    }
    let Some(start) = trimmed.find('{') else {
        return Ok(trimmed);
    };
    if start > 0 {
        return Err(RuntimeResponseParseError::schema(
            "plain JSON response must start with a JSON object",
        ));
    }

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in trimmed.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = index + ch.len_utf8();
                    let trailing = trimmed[end..].trim();
                    if trailing.is_empty() {
                        return Ok(&trimmed[..end]);
                    }
                    if trailing.starts_with(PAYLOAD_OPEN_PREFIX) {
                        return Err(RuntimeResponseParseError::schema(
                            "payload blocks require AHREUM_ACTION framing",
                        ));
                    }
                    return Ok(&trimmed[..end]);
                }
            }
            _ => {}
        }
    }

    Ok(trimmed)
}

fn split_leading_action(raw: &str) -> Result<(&str, &str), RuntimeResponseParseError> {
    let close_index = raw
        .find(ACTION_CLOSE)
        .ok_or_else(|| RuntimeResponseParseError::partial("missing AHREUM_ACTION close tag"))?;
    let json_raw = &raw[ACTION_OPEN.len()..close_index];
    let payload_raw = &raw[close_index + ACTION_CLOSE.len()..];
    Ok((json_raw, payload_raw))
}

fn parse_runtime_payloads(raw: &str) -> Result<Vec<RuntimePayload>, RuntimeResponseParseError> {
    let mut rest = raw.trim();
    let mut payloads = Vec::new();
    let mut ids = HashSet::new();

    while !rest.is_empty() {
        let Some(payload_start) = rest.find(PAYLOAD_OPEN_PREFIX) else {
            break;
        };
        rest = &rest[payload_start..];

        let header_end = rest.find('>').ok_or_else(|| {
            RuntimeResponseParseError::partial("missing AHREUM_PAYLOAD header end")
        })?;
        let header = &rest[1..header_end];
        let attributes = parse_payload_attributes(header)?;
        let body_start = header_end + 1;
        let body_end = rest[body_start..]
            .find(PAYLOAD_CLOSE)
            .map(|index| body_start + index)
            .ok_or_else(|| {
                RuntimeResponseParseError::partial("missing AHREUM_PAYLOAD close tag")
            })?;
        let body = rest[body_start..body_end].trim_matches('\n').to_owned();

        if !ids.insert(attributes.id.clone()) {
            return Err(RuntimeResponseParseError::payload(format!(
                "duplicate payload id: {}",
                attributes.id
            )));
        }

        payloads.push(RuntimePayload {
            id: attributes.id,
            format: attributes.format,
            body,
        });

        rest = rest[body_end + PAYLOAD_CLOSE.len()..].trim();
    }

    Ok(payloads)
}

struct PayloadAttributes {
    id: String,
    format: String,
}

fn parse_payload_attributes(header: &str) -> Result<PayloadAttributes, RuntimeResponseParseError> {
    let mut name = None;
    let mut id = None;
    let mut format = None;

    for part in header.split_whitespace() {
        if name.is_none() {
            name = Some(part);
            continue;
        }

        let Some((key, raw_value)) = part.split_once('=') else {
            return Err(RuntimeResponseParseError::payload(format!(
                "invalid payload attribute: {part}"
            )));
        };
        let value = raw_value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .ok_or_else(|| {
                RuntimeResponseParseError::payload(format!(
                    "payload attribute must use double quotes: {key}"
                ))
            })?;

        match key {
            "id" => id = Some(value.to_owned()),
            "format" => format = Some(value.to_owned()),
            value => {
                return Err(RuntimeResponseParseError::payload(format!(
                    "unknown payload attribute: {value}"
                )));
            }
        }
    }

    if name != Some("AHREUM_PAYLOAD") {
        return Err(RuntimeResponseParseError::payload(
            "payload block must start with AHREUM_PAYLOAD",
        ));
    }

    let id = id.ok_or_else(|| RuntimeResponseParseError::payload("payload id is required"))?;
    let format =
        format.ok_or_else(|| RuntimeResponseParseError::payload("payload format is required"))?;
    if id.trim().is_empty() {
        return Err(RuntimeResponseParseError::payload(
            "payload id cannot be empty",
        ));
    }
    if format.trim().is_empty() {
        return Err(RuntimeResponseParseError::payload(
            "payload format cannot be empty",
        ));
    }

    Ok(PayloadAttributes { id, format })
}

fn to_runtime_response(
    envelope: &Map<String, Value>,
    payloads: &[RuntimePayload],
) -> Result<RuntimeResponse, RuntimeResponseParseError> {
    let response_type = required_str(envelope, "response_type")?;
    let activity = Activity::parse(required_str(envelope, "activity")?)?;
    let message = required_str(envelope, "message")?.to_owned();
    let manifest = manifest_from_envelope(envelope)?;

    match response_type {
        "answer" => {
            validate_allowed_fields(envelope, &[COMMON_FIELDS, ANSWER_FIELDS].concat())?;
            validate_activity_pair(response_type, activity)?;
            let answer_payload_id = optional_str(envelope, "answer_payload_id")?.map(str::to_owned);
            Ok(RuntimeResponse::Answer(RuntimeAnswer {
                activity,
                message,
                answer_payload_id,
                manifest,
            }))
        }
        "tool" => {
            validate_allowed_fields(envelope, &[COMMON_FIELDS, TOOL_FIELDS].concat())?;
            validate_activity_pair(response_type, activity)?;
            let arguments = required_object_value(envelope, "arguments")?.clone();
            reject_forbidden_raw_argument_fields(&arguments)?;
            Ok(RuntimeResponse::Tool(RuntimeToolCandidate {
                activity,
                message,
                tool_name: required_str(envelope, "tool_name")?.to_owned(),
                arguments,
                reason: optional_str(envelope, "reason")?.unwrap_or("").to_owned(),
                manifest,
            }))
        }
        "plan" => {
            validate_allowed_fields(envelope, &[COMMON_FIELDS, PLAN_FIELDS].concat())?;
            validate_activity_pair(response_type, activity)?;
            reject_unreferenced_payloads(payloads)?;
            Ok(RuntimeResponse::Plan(RuntimePlan {
                activity,
                message,
                plan_items: parse_plan_items(envelope)?,
                reason: optional_str(envelope, "reason")?.unwrap_or("").to_owned(),
                manifest,
            }))
        }
        "clarify" => {
            validate_allowed_fields(envelope, &[COMMON_FIELDS, REASON_FIELD].concat())?;
            validate_activity_pair(response_type, activity)?;
            reject_unreferenced_payloads(payloads)?;
            Ok(RuntimeResponse::Clarify(RuntimeClarification {
                activity,
                message,
                reason: optional_str(envelope, "reason")?.unwrap_or("").to_owned(),
                manifest,
            }))
        }
        "blocked" => {
            validate_allowed_fields(envelope, &[COMMON_FIELDS, REASON_FIELD].concat())?;
            validate_activity_pair(response_type, activity)?;
            reject_unreferenced_payloads(payloads)?;
            Ok(RuntimeResponse::Blocked(RuntimeBlocked {
                activity,
                message,
                reason: optional_str(envelope, "reason")?.unwrap_or("").to_owned(),
                manifest,
            }))
        }
        value => Err(RuntimeResponseParseError::schema(format!(
            "unknown response_type: {value}"
        ))),
    }
}

fn parse_plan_items(
    envelope: &Map<String, Value>,
) -> Result<Vec<RuntimePlanItem>, RuntimeResponseParseError> {
    let items = envelope
        .get("plan_items")
        .and_then(Value::as_array)
        .ok_or_else(|| RuntimeResponseParseError::schema("plan_items must be an array"))?;
    if items.is_empty() {
        return Err(RuntimeResponseParseError::schema(
            "plan_items cannot be empty",
        ));
    }
    if items.len() > 40 {
        return Err(RuntimeResponseParseError::schema(
            "plan_items cannot exceed 40 entries",
        ));
    }

    items
        .iter()
        .map(|item| {
            let item = item.as_object().ok_or_else(|| {
                RuntimeResponseParseError::schema("plan item must be a JSON object")
            })?;
            validate_allowed_fields(item, &["operation", "target"])?;
            let operation = PlanOperation::parse(required_str(item, "operation")?)?;
            let target = optional_str(item, "target")?.map(str::to_owned);
            if matches!(
                operation,
                PlanOperation::Read
                    | PlanOperation::Create
                    | PlanOperation::Update
                    | PlanOperation::Delete
            ) && target.as_deref().is_none_or(str::is_empty)
            {
                return Err(RuntimeResponseParseError::schema(format!(
                    "plan operation {} requires target",
                    operation.as_str()
                )));
            }
            Ok(RuntimePlanItem { operation, target })
        })
        .collect()
}

fn manifest_from_envelope(
    envelope: &Map<String, Value>,
) -> Result<RuntimeManifest, RuntimeResponseParseError> {
    let manifest_id = required_str(envelope, "tool_manifest_id")?;
    let manifest_version = required_str(envelope, "tool_manifest_version")?;

    if manifest_id != TOOL_MANIFEST_ID {
        return Err(RuntimeResponseParseError::schema(format!(
            "tool_manifest_id mismatch: {manifest_id}"
        )));
    }
    if manifest_version != TOOL_MANIFEST_VERSION {
        return Err(RuntimeResponseParseError::schema(format!(
            "tool_manifest_version mismatch: {manifest_version}"
        )));
    }

    Ok(RuntimeManifest {
        tool_manifest_id: manifest_id.to_owned(),
        tool_manifest_version: manifest_version.to_owned(),
    })
}

fn validate_allowed_fields(
    envelope: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), RuntimeResponseParseError> {
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    for key in envelope.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(RuntimeResponseParseError::schema(format!(
                "unknown field: {key}"
            )));
        }
    }

    Ok(())
}

fn validate_activity_pair(
    response_type: &str,
    activity: Activity,
) -> Result<(), RuntimeResponseParseError> {
    if activity_pair_allowed(response_type, activity) {
        Ok(())
    } else {
        Err(RuntimeResponseParseError::schema(format!(
            "invalid response_type/activity pair: {response_type}/{}",
            activity.as_str()
        )))
    }
}

fn activity_pair_allowed(response_type: &str, activity: Activity) -> bool {
    RESPONSE_ACTIVITY_PAIRS
        .iter()
        .any(|pair| pair.allows(response_type, activity))
}

fn validate_payload_reference(
    response: &RuntimeResponse,
    payloads: &[RuntimePayload],
) -> Result<(), RuntimeResponseParseError> {
    match response {
        RuntimeResponse::Answer(answer) => {
            let Some(payload_id) = answer.answer_payload_id.as_deref() else {
                return reject_unreferenced_payloads(payloads);
            };
            validate_single_payload_reference(payload_id, payloads, Some("markdown"))
        }
        RuntimeResponse::Tool(candidate) => {
            let Some(payload_id) = candidate
                .arguments
                .as_object()
                .and_then(|arguments| arguments.get("payload_id"))
            else {
                reject_unreferenced_payloads(payloads)?;
                return Ok(());
            };
            let payload_id = payload_id
                .as_str()
                .ok_or_else(|| RuntimeResponseParseError::payload("payload_id must be a string"))?;
            validate_single_payload_reference(payload_id, payloads, None)
        }
        RuntimeResponse::Plan(_) | RuntimeResponse::Clarify(_) | RuntimeResponse::Blocked(_) => {
            reject_unreferenced_payloads(payloads)
        }
    }
}

fn validate_single_payload_reference(
    payload_id: &str,
    payloads: &[RuntimePayload],
    expected_format: Option<&str>,
) -> Result<(), RuntimeResponseParseError> {
    let mut matching_payload = None;
    for payload in payloads.iter().filter(|payload| payload.id == payload_id) {
        if matching_payload.replace(payload).is_some() {
            return Err(RuntimeResponseParseError::payload(format!(
                "duplicate payload block: {payload_id}"
            )));
        }
    }

    let Some(payload) = matching_payload else {
        return Err(RuntimeResponseParseError::payload(format!(
            "missing payload block: {payload_id}"
        )));
    };

    if let Some(expected_format) = expected_format {
        if payload.format != expected_format {
            return Err(RuntimeResponseParseError::payload(format!(
                "payload format must be {expected_format}: {}",
                payload.format
            )));
        }
    }

    let referenced_ids = HashSet::from([payload_id]);
    if let Some(unreferenced) = payloads
        .iter()
        .find(|payload| !referenced_ids.contains(payload.id.as_str()))
    {
        return Err(RuntimeResponseParseError::payload(format!(
            "payload block exists without payload_id reference: {}",
            unreferenced.id
        )));
    }

    Ok(())
}

fn reject_unreferenced_payloads(
    payloads: &[RuntimePayload],
) -> Result<(), RuntimeResponseParseError> {
    if payloads.is_empty() {
        return Ok(());
    }

    Err(RuntimeResponseParseError::payload(
        "payload block exists without payload_id reference",
    ))
}

fn reject_forbidden_raw_argument_fields(
    arguments: &Value,
) -> Result<(), RuntimeResponseParseError> {
    let Some(arguments) = arguments.as_object() else {
        return Err(RuntimeResponseParseError::schema(
            "arguments must be a JSON object",
        ));
    };

    for field in FORBIDDEN_RAW_ARGUMENT_FIELDS {
        if arguments.contains_key(*field) {
            return Err(RuntimeResponseParseError::schema(format!(
                "raw payload field must use payload_id: {field}"
            )));
        }
    }

    Ok(())
}

fn required_str<'a>(
    envelope: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, RuntimeResponseParseError> {
    envelope
        .get(key)
        .ok_or_else(|| RuntimeResponseParseError::schema(format!("missing field: {key}")))?
        .as_str()
        .ok_or_else(|| RuntimeResponseParseError::schema(format!("field must be string: {key}")))
}

fn optional_str<'a>(
    envelope: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, RuntimeResponseParseError> {
    let Some(value) = envelope.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(Some)
        .ok_or_else(|| RuntimeResponseParseError::schema(format!("field must be string: {key}")))
}

fn required_object_value<'a>(
    envelope: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Value, RuntimeResponseParseError> {
    let value = envelope
        .get(key)
        .ok_or_else(|| RuntimeResponseParseError::schema(format!("missing field: {key}")))?;
    if value.as_object().is_none() {
        return Err(RuntimeResponseParseError::schema(format!(
            "field must be object: {key}"
        )));
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_runtime_response, parse_runtime_response_envelope_diagnostic, Activity,
        PlanOperation, RuntimeResponse, RuntimeResponseParseErrorKind,
    };

    fn manifest_fields() -> &'static str {
        r#""tool_manifest_id":"ahreumcode.local-llm.tool-manifest.v1","tool_manifest_version":"1""#
    }

    #[test]
    fn parses_answer_response() {
        let raw = format!(
            r#"{{"response_type":"answer","activity":"None","message":"ready",{}}}"#,
            manifest_fields()
        );

        let parsed = parse_runtime_response(&raw).expect("answer should parse");

        let RuntimeResponse::Answer(answer) = parsed.response else {
            panic!("expected answer");
        };
        assert_eq!(answer.activity, Activity::None);
        assert_eq!(answer.message, "ready");
        assert_eq!(answer.answer_payload_id, None);
        assert!(parsed.payloads.is_empty());
    }

    #[test]
    fn parses_clarify_with_ask_activity() {
        let raw = format!(
            r#"{{"response_type":"clarify","activity":"Ask","message":"Which file should I inspect?","reason":"target is ambiguous",{}}}"#,
            manifest_fields()
        );

        let parsed = parse_runtime_response(&raw).expect("clarify should parse");

        let RuntimeResponse::Clarify(clarify) = parsed.response else {
            panic!("expected clarify");
        };
        assert_eq!(clarify.activity, Activity::Ask);
        assert_eq!(clarify.message, "Which file should I inspect?");
        assert_eq!(clarify.reason, "target is ambiguous");
    }

    #[test]
    fn parses_plan_response_with_target_ledger_items() {
        let raw = format!(
            r#"{{"response_type":"plan","activity":"None","message":"계획을 세웠습니다.","plan_items":[{{"operation":"create","target":"web/index.html"}},{{"operation":"create","target":"web/app.js"}},{{"operation":"verify"}}],"reason":"multi-target request",{}}}"#,
            manifest_fields()
        );

        let parsed = parse_runtime_response(&raw).expect("plan should parse");

        let RuntimeResponse::Plan(plan) = parsed.response else {
            panic!("expected plan");
        };
        assert_eq!(plan.activity, Activity::None);
        assert_eq!(plan.plan_items.len(), 3);
        assert_eq!(plan.plan_items[0].operation, PlanOperation::Create);
        assert_eq!(plan.plan_items[0].target.as_deref(), Some("web/index.html"));
        assert_eq!(plan.plan_items[2].operation, PlanOperation::Verify);
        assert_eq!(plan.plan_items[2].target, None);
    }

    #[test]
    fn rejects_plan_response_with_tool_arguments() {
        let raw = format!(
            r#"{{"response_type":"plan","activity":"None","message":"plan","plan_items":[{{"operation":"create","target":"web/index.html"}}],"tool_name":"apply_patch","arguments":{{"payload_id":"patch_001"}},{}}}"#,
            manifest_fields()
        );

        let error = parse_runtime_response(&raw).expect_err("plan must not carry tool fields");

        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::SchemaValidationFailed
        );
        assert!(error.message.contains("unknown field"));
    }

    #[test]
    fn rejects_missing_manifest_fields() {
        let raw = r#"{"response_type":"answer","activity":"None","message":"ready"}"#;

        let error = parse_runtime_response(raw).expect_err("manifest echo is required");

        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::SchemaValidationFailed
        );
        assert!(error.message.contains("missing field: tool_manifest_id"));
    }

    #[test]
    fn parses_tool_without_reason_as_empty_metadata() {
        let raw = format!(
            r#"{{"response_type":"tool","activity":"Explore","message":"read","tool_name":"read_file","arguments":{{"path":"Cargo.toml","start_line":1,"max_lines":80}},{}}}"#,
            manifest_fields()
        );

        let parsed = parse_runtime_response(&raw).expect("reason is metadata");

        let RuntimeResponse::Tool(candidate) = parsed.response else {
            panic!("expected tool candidate");
        };
        assert_eq!(candidate.reason, "");
    }

    #[test]
    fn rejects_clarify_with_none_activity() {
        let raw = format!(
            r#"{{"response_type":"clarify","activity":"None","message":"Which file should I inspect?","reason":"target is ambiguous",{}}}"#,
            manifest_fields()
        );

        let error = parse_runtime_response(&raw).expect_err("clarify/None should fail");

        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::SchemaValidationFailed
        );
        assert_eq!(
            error.message,
            "invalid response_type/activity pair: clarify/None"
        );
    }

    #[test]
    fn parses_answer_response_with_markdown_payload() {
        let raw = format!(
            r#"<AHREUM_ACTION>
{{"response_type":"answer","activity":"None","message":"summary","answer_payload_id":"answer_001",{} }}
</AHREUM_ACTION>

<AHREUM_PAYLOAD id="answer_001" format="markdown">
```typescript
const greeting: string = "Hello, World!";
console.log(greeting);
```
</AHREUM_PAYLOAD>"#,
            manifest_fields()
        );

        let parsed = parse_runtime_response(&raw).expect("answer payload should parse");

        let RuntimeResponse::Answer(answer) = parsed.response else {
            panic!("expected answer");
        };
        assert_eq!(answer.answer_payload_id.as_deref(), Some("answer_001"));
        assert_eq!(parsed.payloads[0].format, "markdown");
        assert!(parsed.payloads[0].body.contains("Hello, World!"));
    }

    #[test]
    fn parses_action_block_with_leading_model_text() {
        let raw = format!(
            r#"Here is the contract:
<AHREUM_ACTION>
{{"response_type":"answer","activity":"None","message":"ready",{} }}
</AHREUM_ACTION>"#,
            manifest_fields()
        );

        let parsed = parse_runtime_response(&raw).expect("tagged contract should parse");

        let RuntimeResponse::Answer(answer) = parsed.response else {
            panic!("expected answer");
        };
        assert_eq!(answer.message, "ready");
    }

    #[test]
    fn rejects_payload_answer_without_action_framing() {
        let raw = format!(
            r#"{{"response_type":"answer","activity":"None","message":"summary","answer_payload_id":"answer_001",{} }}
<AHREUM_PAYLOAD id="answer_001" format="markdown">
answer body
</AHREUM_PAYLOAD>"#,
            manifest_fields()
        );

        let error =
            parse_runtime_response(&raw).expect_err("payload without action framing should fail");

        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::SchemaValidationFailed
        );
        assert!(error.message.contains("AHREUM_ACTION"));
    }

    #[test]
    fn rejects_answer_payload_without_markdown_format() {
        let raw = format!(
            r#"<AHREUM_ACTION>
{{"response_type":"answer","activity":"None","message":"summary","answer_payload_id":"answer_001",{} }}
</AHREUM_ACTION>

<AHREUM_PAYLOAD id="answer_001" format="text">
body
</AHREUM_PAYLOAD>"#,
            manifest_fields()
        );

        let error =
            parse_runtime_response(&raw).expect_err("wrong answer payload format should fail");

        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::PayloadValidationFailed
        );
        assert!(error.message.contains("markdown"));
    }

    #[test]
    fn parses_payload_block_with_interstitial_model_text() {
        let raw = format!(
            r#"<AHREUM_ACTION>
{{"response_type":"answer","activity":"None","message":"summary","answer_payload_id":"answer_001",{} }}
</AHREUM_ACTION>
summary:
<AHREUM_PAYLOAD id="answer_001" format="markdown">
body
</AHREUM_PAYLOAD>"#,
            manifest_fields()
        );

        let parsed = parse_runtime_response(&raw).expect("tagged payload should parse");

        assert_eq!(parsed.payloads.len(), 1);
        assert_eq!(parsed.payloads[0].body, "body");
    }

    #[test]
    fn rejects_unknown_field() {
        let raw = format!(
            r#"{{"response_type":"answer","activity":"None","message":"ready","extra":true,{}}}"#,
            manifest_fields()
        );

        let error = parse_runtime_response(&raw).expect_err("unknown field should fail");

        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::SchemaValidationFailed
        );
        assert!(error.message.contains("unknown field"));
    }

    #[test]
    fn parses_tool_candidate_with_payload() {
        let raw = format!(
            r#"<AHREUM_ACTION>
{{"response_type":"tool","activity":"Change","message":"patch ready","tool_name":"apply_patch","arguments":{{"payload_id":"patch_001"}},"reason":"needs patch",{} }}
</AHREUM_ACTION>

<AHREUM_PAYLOAD id="patch_001" format="apply_patch">
*** Begin Patch
*** Update File: src/main.rs
@@
-old
+new
*** End Patch
</AHREUM_PAYLOAD>"#,
            manifest_fields()
        );

        let parsed = parse_runtime_response(&raw).expect("framed response should parse");

        let RuntimeResponse::Tool(candidate) = parsed.response else {
            panic!("expected tool candidate");
        };
        assert_eq!(candidate.activity, Activity::Change);
        assert_eq!(candidate.tool_name, "apply_patch");
        assert_eq!(parsed.payloads[0].id, "patch_001");
        assert_eq!(parsed.payloads[0].format, "apply_patch");
    }

    #[test]
    fn rejects_missing_payload_reference() {
        let raw = format!(
            r#"{{"response_type":"tool","activity":"Change","message":"patch ready","tool_name":"apply_patch","arguments":{{"payload_id":"patch_001"}},"reason":"needs patch",{} }}"#,
            manifest_fields()
        );

        let error = parse_runtime_response(&raw).expect_err("missing payload should fail");

        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::PayloadValidationFailed
        );
    }

    #[test]
    fn rejects_raw_payload_text_in_json_arguments() {
        let raw = format!(
            r#"{{"response_type":"tool","activity":"Change","message":"patch ready","tool_name":"apply_patch","arguments":{{"patch":"*** Begin Patch"}},"reason":"needs patch",{} }}"#,
            manifest_fields()
        );

        let error = parse_runtime_response(&raw).expect_err("raw patch argument should fail");

        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::SchemaValidationFailed
        );
        assert!(error.message.contains("payload_id"));
    }

    #[test]
    fn rejects_unreferenced_extra_payload_block() {
        let raw = format!(
            r#"<AHREUM_ACTION>
{{"response_type":"tool","activity":"Change","message":"patch ready","tool_name":"apply_patch","arguments":{{"payload_id":"patch_001"}},"reason":"needs patch",{} }}
</AHREUM_ACTION>

<AHREUM_PAYLOAD id="patch_001" format="apply_patch">
*** Begin Patch
*** End Patch
</AHREUM_PAYLOAD>
<AHREUM_PAYLOAD id="patch_002" format="apply_patch">
*** Begin Patch
*** End Patch
</AHREUM_PAYLOAD>"#,
            manifest_fields()
        );

        let error = parse_runtime_response(&raw).expect_err("extra payload should fail");

        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::PayloadValidationFailed
        );
        assert!(error.message.contains("without payload_id"));
    }

    #[test]
    fn extracts_envelope_diagnostic_from_payload_invalid_response() {
        let raw = format!(
            r#"<AHREUM_ACTION>
{{"response_type":"tool","activity":"Explore","message":"read","tool_name":"read_file","arguments":{{"path":"Cargo.toml","start_line":1,"max_lines":120}},{} }}
</AHREUM_ACTION>
<AHREUM_PAYLOAD id="orphan" format="markdown">
unused
</AHREUM_PAYLOAD>"#,
            manifest_fields()
        );

        let diagnostic = parse_runtime_response_envelope_diagnostic(&raw)
            .expect("envelope diagnostic should parse");

        assert_eq!(diagnostic.response_type.as_deref(), Some("tool"));
        assert_eq!(diagnostic.activity.as_deref(), Some("Explore"));
        assert_eq!(diagnostic.tool_name.as_deref(), Some("read_file"));
        let error = parse_runtime_response(&raw).expect_err("payload remains invalid");
        assert_eq!(
            error.kind,
            RuntimeResponseParseErrorKind::PayloadValidationFailed
        );
    }

    #[test]
    fn extracts_envelope_diagnostic_from_unframed_payload_response() {
        let raw = format!(
            r#"{{"response_type":"tool","activity":"Change","message":"patch","tool_name":"apply_patch","arguments":{{"payload_id":"patch_001"}},"reason":"change",{} }}
<AHREUM_PAYLOAD id="patch_001" format="apply_patch">
*** Begin Patch
*** Update File: src/main.rs
*** End Patch
</AHREUM_PAYLOAD>"#,
            manifest_fields()
        );

        let diagnostic = parse_runtime_response_envelope_diagnostic(&raw)
            .expect("unframed payload diagnostic should parse");

        assert_eq!(diagnostic.response_type.as_deref(), Some("tool"));
        assert_eq!(diagnostic.activity.as_deref(), Some("Change"));
        assert_eq!(diagnostic.tool_name.as_deref(), Some("apply_patch"));
        let error = parse_runtime_response(&raw).expect_err("framing should still fail");
        assert_eq!(
            error.message,
            "framed response must start with AHREUM_ACTION"
        );
    }

    #[test]
    fn unwraps_whole_markdown_fence_only() {
        let raw = format!(
            "```json\n{{\"response_type\":\"answer\",\"activity\":\"None\",\"message\":\"ready\",{}}}\n```",
            manifest_fields()
        );

        let parsed = parse_runtime_response(&raw).expect("whole fence should unwrap");

        let RuntimeResponse::Answer(answer) = parsed.response else {
            panic!("expected answer");
        };
        assert_eq!(answer.message, "ready");
    }

    #[test]
    fn detects_partial_action_block() {
        let raw = format!(
            r#"<AHREUM_ACTION>
{{"response_type":"answer","activity":"None","message":"ready",{}}}"#,
            manifest_fields()
        );

        let error = parse_runtime_response(&raw).expect_err("partial action should fail");

        assert_eq!(error.kind, RuntimeResponseParseErrorKind::PartialResponse);
    }
}
