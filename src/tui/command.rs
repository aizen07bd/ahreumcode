#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandId {
    Exit,
    Quit,
    Status,
    PersonaFull,
    PersonaOff,
    PersonaClose,
}

impl CommandId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exit => "/exit",
            Self::Quit => "/quit",
            Self::Status => "/status",
            Self::PersonaFull => "/persona full",
            Self::PersonaOff => "/persona off",
            Self::PersonaClose => "/persona close",
        }
    }
}

#[derive(Clone, Copy)]
pub enum CommandPresentation {
    InlineAction,
}

impl CommandPresentation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InlineAction => "InlineAction",
        }
    }
}

#[derive(Clone, Copy)]
pub enum CommandRisk {
    Low,
}

impl CommandRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
        }
    }
}

pub struct CommandMetadata {
    pub id: CommandId,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub group: &'static str,
    pub availability: &'static [&'static str],
    pub presentation: CommandPresentation,
    pub risk: CommandRisk,
}

pub struct CommandRegistry {
    commands: Vec<CommandMetadata>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: vec![
                CommandMetadata {
                    id: CommandId::Exit,
                    name: "/exit",
                    aliases: &[],
                    description: "exit the app",
                    group: "Session",
                    availability: &["intro", "workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::Quit,
                    name: "/quit",
                    aliases: &[],
                    description: "exit the app",
                    group: "Session",
                    availability: &["intro", "workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::Status,
                    name: "/status",
                    aliases: &[],
                    description: "show status shell",
                    group: "Session",
                    availability: &["intro", "workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::PersonaFull,
                    name: "/persona full",
                    aliases: &[],
                    description: "open persona messenger",
                    group: "Persona",
                    availability: &["intro", "workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::PersonaOff,
                    name: "/persona off",
                    aliases: &[],
                    description: "turn persona off",
                    group: "Persona",
                    availability: &["intro", "workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::PersonaClose,
                    name: "/persona close",
                    aliases: &[],
                    description: "close persona messenger",
                    group: "Persona",
                    availability: &["intro", "workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
            ],
        }
    }

    pub fn filtered(&self, query: &str) -> Vec<&CommandMetadata> {
        self.commands
            .iter()
            .filter(|command| command.matches(query))
            .collect()
    }

    pub fn command(&self, id: CommandId) -> Option<&CommandMetadata> {
        self.commands.iter().find(|command| command.id == id)
    }
}

impl CommandMetadata {
    fn matches(&self, query: &str) -> bool {
        let normalized = normalize_query(query);
        normalized.is_empty()
            || self.name.starts_with(&normalized)
            || self
                .aliases
                .iter()
                .any(|alias| alias.starts_with(&normalized))
    }
}

#[derive(Default)]
pub struct CommandSurfaceState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub scroll: usize,
}

impl CommandSurfaceState {
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_owned();
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn move_selection(&mut self, delta: isize, item_count: usize) {
        if item_count == 0 {
            self.selected = 0;
            self.scroll = 0;
            return;
        }

        let current = self.selected as isize;
        let max = item_count as isize - 1;
        self.selected = (current + delta).clamp(0, max) as usize;

        if self.selected < self.scroll {
            self.scroll = self.selected;
        }

        let visible_height = COMMAND_VISIBLE_ROWS;
        if self.selected >= self.scroll + visible_height {
            self.scroll = self.selected + 1 - visible_height;
        }
    }
}

pub const COMMAND_VISIBLE_ROWS: usize = 5;

pub enum CommandInputEvent {
    SurfaceOpened,
    FilterChanged { query: String },
    CommandSelected { command: CommandId },
    ActionDispatched { command: CommandId },
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum CommandDispatch {
    None,
    ExitRequested,
    StatusShell,
    PersonaFull,
    PersonaOff,
    PersonaClose,
}

pub struct CommandInputOutcome {
    pub events: Vec<CommandInputEvent>,
    pub dispatch: CommandDispatch,
}

impl CommandInputOutcome {
    pub fn none() -> Self {
        Self {
            events: Vec::new(),
            dispatch: CommandDispatch::None,
        }
    }
}

pub fn confirm_command(
    surface: &mut CommandSurfaceState,
    registry: &CommandRegistry,
) -> CommandInputOutcome {
    let filtered = registry.filtered(&surface.query);
    let Some(command) = filtered.get(surface.selected) else {
        return CommandInputOutcome::none();
    };

    let command_id = command.id;
    surface.close();

    CommandInputOutcome {
        events: vec![
            CommandInputEvent::CommandSelected {
                command: command_id,
            },
            CommandInputEvent::ActionDispatched {
                command: command_id,
            },
        ],
        dispatch: dispatch_for(command_id),
    }
}

fn dispatch_for(command: CommandId) -> CommandDispatch {
    match command {
        CommandId::Exit | CommandId::Quit => CommandDispatch::ExitRequested,
        CommandId::Status => CommandDispatch::StatusShell,
        CommandId::PersonaFull => CommandDispatch::PersonaFull,
        CommandId::PersonaOff => CommandDispatch::PersonaOff,
        CommandId::PersonaClose => CommandDispatch::PersonaClose,
    }
}

pub fn normalize_query(query: &str) -> String {
    let trimmed = query.trim_start();
    if trimmed.is_empty() {
        String::new()
    } else if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    }
}
