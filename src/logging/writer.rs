use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::json;
use time::format_description::FormatItem;
use time::macros::format_description;
use time::OffsetDateTime;

use super::{event::local_timestamp, LogEvent};
use crate::product;

const LOG_ROOT: &str = ".ahreumcode/logs/sessions";
const LOG_01_SCOPE: &str = "log-01-daily-bucket-layout";
const EVENT_LOG_BUCKET_CREATED: &str = "log_bucket_created";
const EVENT_LOG_SESSION_STARTED: &str = "log_session_started";
const EVENT_LOG_RECORD_APPENDED: &str = "log_record_appended";
const EVENT_LOG_WRITE_FAILED: &str = "log_write_failed";
const SESSION_INDEX_FILE: &str = "sessions.jsonl";

pub struct Logger {
    session_id: String,
    bucket_dir: PathBuf,
}

impl Logger {
    pub fn start() -> io::Result<Self> {
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        Self::start_at(Path::new(LOG_ROOT), now)
    }

    fn start_at(root: &Path, now: OffsetDateTime) -> io::Result<Self> {
        let date = format_time(now, format_description!("[year]-[month]-[day]"))?;
        let session_id = format_time(
            now,
            format_description!("[year][month][day]-[hour][minute][second]-[subsecond digits:3]"),
        )?;
        let bucket_dir = root.join(date);
        let bucket_existed = bucket_dir.exists();

        fs::create_dir_all(&bucket_dir)?;

        let logger = Self {
            session_id,
            bucket_dir,
        };
        logger.write_internal_event(
            EVENT_LOG_BUCKET_CREATED,
            json!({
                "bucket_dir": logger.bucket_dir.display().to_string(),
                "existed_before": bucket_existed,
            }),
        )?;
        logger.write_session_started()?;
        Ok(logger)
    }

    pub fn ui(&self, event: LogEvent) -> io::Result<()> {
        self.append_jsonl("ui.jsonl", &event.to_json(&self.session_id))
    }

    pub fn llm(&self, event: LogEvent) -> io::Result<()> {
        self.append_jsonl("llm.jsonl", &event.to_json(&self.session_id))
    }

    pub fn log_bucket_dir(&self) -> &Path {
        &self.bucket_dir
    }

    fn write_session_started(&self) -> io::Result<()> {
        self.write_internal_event(
            EVENT_LOG_SESSION_STARTED,
            json!({
                "app": product::APP_NAME,
                "version": product::VERSION,
                "provider": product::DEFAULT_PROVIDER,
                "model": product::DEFAULT_MODEL,
                "mode": product::DEFAULT_MODE,
            }),
        )
    }

    fn append_jsonl(&self, file_name: &str, value: &serde_json::Value) -> io::Result<()> {
        match self.append_jsonl_raw(file_name, value) {
            Ok(()) => {
                self.write_internal_event(
                    EVENT_LOG_RECORD_APPENDED,
                    json!({
                        "file": file_name,
                        "scope_id": value.get("scope_id").and_then(|value| value.as_str()),
                        "event": value.get("event").and_then(|value| value.as_str()),
                    }),
                )?;
                Ok(())
            }
            Err(error) => {
                let _ = self.write_internal_event(
                    EVENT_LOG_WRITE_FAILED,
                    json!({
                        "file": file_name,
                        "error_kind": error.kind().to_string(),
                        "message": error.to_string(),
                    }),
                );
                Err(error)
            }
        }
    }

    fn write_internal_event(&self, event: &'static str, data: serde_json::Value) -> io::Result<()> {
        let value = json!({
            "ts": local_timestamp(),
            "session_id": self.session_id,
            "scope_id": LOG_01_SCOPE,
            "level": "info",
            "event": event,
            "data": data,
        });
        self.append_jsonl_raw(SESSION_INDEX_FILE, &value)
    }

    fn append_jsonl_raw(&self, file_name: &str, value: &serde_json::Value) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.bucket_dir.join(file_name))?;
        writeln!(file, "{value}")?;
        Ok(())
    }
}

fn format_time(time: OffsetDateTime, format: &'static [FormatItem<'static>]) -> io::Result<String> {
    time.format(format)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use serde_json::json;
    use time::OffsetDateTime;

    use super::{Logger, SESSION_INDEX_FILE};
    use crate::logging::LogEvent;

    #[test]
    fn stores_logs_in_daily_bucket_without_session_directory() {
        let root = test_log_root("daily-bucket");
        let logger = Logger::start_at(&root, OffsetDateTime::from_unix_timestamp(0).unwrap())
            .expect("logger should start");

        let bucket = root.join("1970-01-01");
        assert_eq!(logger.log_bucket_dir(), bucket.as_path());
        assert!(bucket.join(SESSION_INDEX_FILE).is_file());
        assert!(!bucket.join("19700101-000000-000").exists());

        logger
            .ui(LogEvent::ui("log-test", "test_event", json!({"ok": true})))
            .expect("ui log should append");

        assert!(bucket.join("ui.jsonl").is_file());
        let sessions = fs::read_to_string(bucket.join(SESSION_INDEX_FILE))
            .expect("session index should be readable");
        assert!(sessions.contains("\"event\":\"log_bucket_created\""));
        assert!(sessions.contains("\"event\":\"log_session_started\""));
        assert!(sessions.contains("\"event\":\"log_record_appended\""));
    }

    fn test_log_root(name: &str) -> std::path::PathBuf {
        let mut root = Path::new("target").join("logging-tests");
        root.push(format!(
            "{}-{}-{}",
            name,
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        root
    }
}
