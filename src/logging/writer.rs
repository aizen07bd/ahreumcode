use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::json;
use time::format_description::FormatItem;
use time::macros::format_description;
use time::OffsetDateTime;

use super::LogEvent;
use crate::product;

const LOG_ROOT: &str = ".ahreumcode/logs/sessions";

pub struct Logger {
    session_id: String,
    session_dir: PathBuf,
}

impl Logger {
    pub fn start() -> io::Result<Self> {
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        let date = format_time(now, format_description!("[year]-[month]-[day]"))?;
        let session_id = format_time(
            now,
            format_description!("[year][month][day]-[hour][minute][second]"),
        )?;
        let session_dir = Path::new(LOG_ROOT).join(date).join(&session_id);

        fs::create_dir_all(&session_dir)?;

        let logger = Self {
            session_id,
            session_dir,
        };
        logger.write_session_summary()?;
        Ok(logger)
    }

    pub fn ui(&self, event: LogEvent) -> io::Result<()> {
        self.append_jsonl("ui.jsonl", &event.to_json(&self.session_id))
    }

    pub fn llm(&self, event: LogEvent) -> io::Result<()> {
        self.append_jsonl("llm.jsonl", &event.to_json(&self.session_id))
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    fn write_session_summary(&self) -> io::Result<()> {
        let summary = json!({
            "session_id": self.session_id,
            "app": product::APP_NAME,
            "version": product::VERSION,
            "provider": product::DEFAULT_PROVIDER,
            "model": product::DEFAULT_MODEL,
            "mode": product::DEFAULT_MODE,
        });

        let mut file = File::create(self.session_dir.join("session.json"))?;
        writeln!(file, "{summary}")?;
        Ok(())
    }

    fn append_jsonl(&self, file_name: &str, value: &serde_json::Value) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.session_dir.join(file_name))?;
        writeln!(file, "{value}")?;
        Ok(())
    }
}

fn format_time(time: OffsetDateTime, format: &'static [FormatItem<'static>]) -> io::Result<String> {
    time.format(format)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
