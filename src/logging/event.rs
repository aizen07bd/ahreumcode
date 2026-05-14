use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[derive(Clone, Copy)]
pub enum LogLevel {
    Info,
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
        }
    }
}

pub struct LogEvent {
    pub scope_id: &'static str,
    pub level: LogLevel,
    pub event: &'static str,
    pub data: Value,
}

impl LogEvent {
    pub fn ui(scope_id: &'static str, event: &'static str, data: Value) -> Self {
        Self {
            scope_id,
            level: LogLevel::Info,
            event,
            data,
        }
    }

    pub fn to_json(&self, session_id: &str) -> Value {
        json!({
            "ts": local_timestamp(),
            "session_id": session_id,
            "scope_id": self.scope_id,
            "level": self.level.as_str(),
            "event": self.event,
            "data": self.data,
        })
    }
}

pub(crate) fn local_timestamp() -> String {
    OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}
