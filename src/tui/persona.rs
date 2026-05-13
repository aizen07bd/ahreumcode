use time::OffsetDateTime;

pub const MIN_PERSONA_TERMINAL_WIDTH: u16 = 140;
pub const MIN_PERSONA_PANEL_WIDTH: u16 = 36;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum PersonaSpeakerRole {
    Lead,
    Member,
}

pub struct PersonaMessage {
    pub speaker: String,
    pub role: PersonaSpeakerRole,
    pub time_label: String,
    pub body: String,
}

impl PersonaMessage {
    pub fn team_lead(body: impl Into<String>) -> Self {
        Self {
            speaker: "팀장".to_owned(),
            role: PersonaSpeakerRole::Lead,
            time_label: current_time_label(),
            body: body.into(),
        }
    }

    #[allow(dead_code)]
    pub fn member(speaker: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            speaker: speaker.into(),
            role: PersonaSpeakerRole::Member,
            time_label: current_time_label(),
            body: body.into(),
        }
    }
}

#[derive(Default)]
pub struct PersonaBuffer {
    messages: Vec<PersonaMessage>,
    render_pending: bool,
}

impl PersonaBuffer {
    pub fn push_message(&mut self, message: PersonaMessage) {
        self.messages.push(message);
        self.render_pending = true;
    }

    pub fn messages(&self) -> &[PersonaMessage] {
        &self.messages
    }

    pub fn take_render_event(&mut self) -> Option<PersonaRendered> {
        if !self.render_pending {
            return None;
        }

        self.render_pending = false;
        Some(PersonaRendered {
            message_count: self.messages.len(),
        })
    }
}

pub enum PersonaEvent {
    PanelOpened,
    PanelClosed,
    WidthRejected { width: u16, min_width: u16 },
}

#[derive(Default)]
pub struct PersonaEvents {
    pub events: Vec<PersonaEvent>,
}

impl PersonaEvents {
    pub fn none() -> Self {
        Self { events: Vec::new() }
    }

    pub fn single(event: PersonaEvent) -> Self {
        Self {
            events: vec![event],
        }
    }
}

pub struct PersonaRendered {
    pub message_count: usize,
}

fn current_time_label() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    format!("{:02}:{:02}", now.hour(), now.minute())
}
