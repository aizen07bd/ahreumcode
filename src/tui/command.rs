#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandId {
    Exit,
    Quit,
    Status,
    Approval,
    Mode,
    Provider,
    Model,
    Persona,
    DocsInit,
    Init,
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
            Self::Approval => "/approval",
            Self::Mode => "/mode",
            Self::Provider => "/provider",
            Self::Model => "/model",
            Self::Persona => "/persona",
            Self::DocsInit => "/docs-init",
            Self::Init => "/init",
            Self::PersonaFull => "/persona full",
            Self::PersonaOff => "/persona off",
            Self::PersonaClose => "/persona close",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum CommandPresentation {
    InlineAction,
    SteppedPicker,
}

impl CommandPresentation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InlineAction => "InlineAction",
            Self::SteppedPicker => "SteppedPicker",
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
                    id: CommandId::Approval,
                    name: "/approval",
                    aliases: &[],
                    description: "show approval shell",
                    group: "Permission",
                    availability: &["workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::Mode,
                    name: "/mode",
                    aliases: &[],
                    description: "choose permission mode",
                    group: "Runtime",
                    availability: &["workspace"],
                    presentation: CommandPresentation::SteppedPicker,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::Provider,
                    name: "/provider",
                    aliases: &[],
                    description: "choose local LLM provider",
                    group: "Runtime",
                    availability: &["workspace"],
                    presentation: CommandPresentation::SteppedPicker,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::Model,
                    name: "/model",
                    aliases: &[],
                    description: "choose model",
                    group: "Runtime",
                    availability: &["workspace"],
                    presentation: CommandPresentation::SteppedPicker,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::Persona,
                    name: "/persona",
                    aliases: &[],
                    description: "choose persona visibility",
                    group: "Persona",
                    availability: &["workspace"],
                    presentation: CommandPresentation::SteppedPicker,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::DocsInit,
                    name: "/docs-init",
                    aliases: &[],
                    description: "prepare docs template setup",
                    group: "Project",
                    availability: &["workspace"],
                    presentation: CommandPresentation::SteppedPicker,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::Init,
                    name: "/init",
                    aliases: &[],
                    description: "prepare AGENTS.md setup",
                    group: "Project",
                    availability: &["workspace"],
                    presentation: CommandPresentation::SteppedPicker,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::PersonaFull,
                    name: "/persona full",
                    aliases: &[],
                    description: "open persona messenger",
                    group: "Persona",
                    availability: &["workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::PersonaOff,
                    name: "/persona off",
                    aliases: &[],
                    description: "turn persona off",
                    group: "Persona",
                    availability: &["workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
                CommandMetadata {
                    id: CommandId::PersonaClose,
                    name: "/persona close",
                    aliases: &[],
                    description: "close persona messenger",
                    group: "Persona",
                    availability: &["workspace"],
                    presentation: CommandPresentation::InlineAction,
                    risk: CommandRisk::Low,
                },
            ],
        }
    }

    pub fn filtered_for(&self, query: &str, scene: &str) -> Vec<&CommandMetadata> {
        self.commands
            .iter()
            .filter(|command| command.matches(query) && command.available_in(scene))
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

    fn available_in(&self, scene: &str) -> bool {
        self.availability
            .iter()
            .any(|available| *available == scene)
    }
}

#[derive(Default)]
pub struct CommandSurfaceState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub scroll: usize,
    pub stepped_picker: Option<SteppedPickerState>,
}

impl CommandSurfaceState {
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.scroll = 0;
        self.stepped_picker = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
        self.scroll = 0;
        self.stepped_picker = None;
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_owned();
        self.selected = 0;
        self.scroll = 0;
        self.stepped_picker = None;
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

    pub fn open_stepped_picker(&mut self, command: CommandId) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.scroll = 0;
        self.stepped_picker = Some(SteppedPickerState::new(command));
    }

    pub fn step_title(&self) -> Option<&'static str> {
        self.stepped_picker.as_ref().map(SteppedPickerState::title)
    }

    pub fn step_options(&self) -> Vec<SteppedPickerOption> {
        self.stepped_picker
            .as_ref()
            .map(SteppedPickerState::options)
            .unwrap_or_default()
    }

    pub fn move_picker_selection(&mut self, delta: isize) -> Option<usize> {
        let picker = self.stepped_picker.as_mut()?;
        let item_count = picker.options().len();
        picker.move_selection(delta, item_count);
        Some(picker.selected)
    }

    pub fn back_picker_step(&mut self) -> bool {
        if self.stepped_picker.is_none() {
            return false;
        }

        self.stepped_picker = None;
        self.selected = 0;
        self.scroll = 0;
        true
    }
}

pub const COMMAND_VISIBLE_ROWS: usize = 5;

#[derive(Clone, Copy)]
pub struct SteppedPickerOption {
    pub label: &'static str,
    pub detail: &'static str,
    pub action: CommandDispatch,
}

pub struct SteppedPickerState {
    command: CommandId,
    selected: usize,
}

impl SteppedPickerState {
    fn new(command: CommandId) -> Self {
        Self {
            command,
            selected: 0,
        }
    }

    pub fn command(&self) -> CommandId {
        self.command
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn title(&self) -> &'static str {
        match self.command {
            CommandId::Mode => "Select Mode",
            CommandId::Provider => "Select Provider",
            CommandId::Model => "Select Model",
            CommandId::Persona => "Select Persona",
            CommandId::DocsInit => "Docs Init",
            CommandId::Init => "Project Init",
            _ => "Select Option",
        }
    }

    pub fn options(&self) -> Vec<SteppedPickerOption> {
        match self.command {
            CommandId::Mode => vec![
                SteppedPickerOption {
                    label: "Guide",
                    detail: "ask before mutation",
                    action: CommandDispatch::ModeGuide,
                },
                SteppedPickerOption {
                    label: "Crew (current)",
                    detail: "balanced approval flow",
                    action: CommandDispatch::ModeCrew,
                },
                SteppedPickerOption {
                    label: "Pilot",
                    detail: "faster trusted flow",
                    action: CommandDispatch::ModePilot,
                },
            ],
            CommandId::Provider => vec![SteppedPickerOption {
                label: "LM Studio (current)",
                detail: "OpenAI-compatible local endpoint",
                action: CommandDispatch::ProviderLmStudio,
            }],
            CommandId::Model => vec![SteppedPickerOption {
                label: "google/gemma-4-e4b (current)",
                detail: "LM Studio local model",
                action: CommandDispatch::ModelGemma,
            }],
            CommandId::Persona => vec![
                SteppedPickerOption {
                    label: "full",
                    detail: "open right messenger",
                    action: CommandDispatch::PersonaFull,
                },
                SteppedPickerOption {
                    label: "off",
                    detail: "remove right messenger",
                    action: CommandDispatch::PersonaOff,
                },
                SteppedPickerOption {
                    label: "close",
                    detail: "same as off",
                    action: CommandDispatch::PersonaClose,
                },
            ],
            CommandId::DocsInit => vec![
                SteppedPickerOption {
                    label: "prepare",
                    detail: "expanded setup opens in tui-10",
                    action: CommandDispatch::DocsInitPrepare,
                },
                SteppedPickerOption {
                    label: "cancel",
                    detail: "return to prompt",
                    action: CommandDispatch::None,
                },
            ],
            CommandId::Init => vec![
                SteppedPickerOption {
                    label: "prepare",
                    detail: "expanded setup opens in tui-10",
                    action: CommandDispatch::InitPrepare,
                },
                SteppedPickerOption {
                    label: "cancel",
                    detail: "return to prompt",
                    action: CommandDispatch::None,
                },
            ],
            _ => Vec::new(),
        }
    }

    fn move_selection(&mut self, delta: isize, item_count: usize) {
        if item_count == 0 {
            self.selected = 0;
            return;
        }

        let current = self.selected as isize;
        let max = item_count as isize - 1;
        self.selected = (current + delta).clamp(0, max) as usize;
    }
}

pub enum CommandInputEvent {
    SurfaceOpened,
    FilterChanged {
        query: String,
    },
    CommandSelected {
        command: CommandId,
    },
    ActionDispatched {
        command: CommandId,
    },
    CommandAvailabilityChecked {
        command: CommandId,
        allowed: bool,
        reason: &'static str,
    },
    SteppedPickerOpened {
        command: CommandId,
        step: &'static str,
    },
    SteppedPickerSelectionChanged {
        command: CommandId,
        selected: usize,
    },
    SteppedPickerConfirmed {
        command: CommandId,
        selected: usize,
    },
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum CommandDispatch {
    None,
    ExitRequested,
    StatusShell,
    ApprovalShell,
    ModeGuide,
    ModeCrew,
    ModePilot,
    ProviderLmStudio,
    ModelGemma,
    DocsInitPrepare,
    InitPrepare,
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
    scene: &str,
    runtime_busy: bool,
) -> CommandInputOutcome {
    if surface.stepped_picker.is_some() {
        return confirm_picker_selection(surface);
    }

    let filtered = registry.filtered_for(&surface.query, scene);
    let Some(command) = filtered.get(surface.selected) else {
        return CommandInputOutcome::none();
    };

    let command_id = command.id;
    let availability = check_command_availability(command_id, runtime_busy);
    let availability_event = CommandInputEvent::CommandAvailabilityChecked {
        command: command_id,
        allowed: availability.allowed,
        reason: availability.reason,
    };

    if !availability.allowed {
        surface.close();
        return CommandInputOutcome {
            events: vec![availability_event],
            dispatch: CommandDispatch::None,
        };
    }

    if command.presentation == CommandPresentation::SteppedPicker {
        surface.open_stepped_picker(command_id);
        return CommandInputOutcome {
            events: vec![
                availability_event,
                CommandInputEvent::SteppedPickerOpened {
                    command: command_id,
                    step: surface.step_title().unwrap_or("Select Option"),
                },
            ],
            dispatch: CommandDispatch::None,
        };
    }

    surface.close();

    CommandInputOutcome {
        events: vec![
            availability_event,
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

pub fn confirm_picker_selection(surface: &mut CommandSurfaceState) -> CommandInputOutcome {
    let Some(picker) = surface.stepped_picker.as_ref() else {
        return CommandInputOutcome::none();
    };

    let command = picker.command();
    let selected = picker.selected();
    let options = picker.options();
    let dispatch = options
        .get(selected)
        .map(|option| option.action)
        .unwrap_or(CommandDispatch::None);
    surface.close();

    CommandInputOutcome {
        events: vec![
            CommandInputEvent::SteppedPickerConfirmed { command, selected },
            CommandInputEvent::ActionDispatched { command },
        ],
        dispatch,
    }
}

fn dispatch_for(command: CommandId) -> CommandDispatch {
    match command {
        CommandId::Exit | CommandId::Quit => CommandDispatch::ExitRequested,
        CommandId::Status => CommandDispatch::StatusShell,
        CommandId::Approval => CommandDispatch::ApprovalShell,
        CommandId::Mode
        | CommandId::Provider
        | CommandId::Model
        | CommandId::Persona
        | CommandId::DocsInit
        | CommandId::Init => CommandDispatch::None,
        CommandId::PersonaFull => CommandDispatch::PersonaFull,
        CommandId::PersonaOff => CommandDispatch::PersonaOff,
        CommandId::PersonaClose => CommandDispatch::PersonaClose,
    }
}

pub struct CommandAvailability {
    pub allowed: bool,
    pub reason: &'static str,
}

fn check_command_availability(command: CommandId, runtime_busy: bool) -> CommandAvailability {
    if runtime_busy {
        match command {
            CommandId::Exit | CommandId::Quit => CommandAvailability {
                allowed: true,
                reason: "always_allowed",
            },
            _ => CommandAvailability {
                allowed: false,
                reason: "runtime_busy",
            },
        }
    } else {
        CommandAvailability {
            allowed: true,
            reason: "available",
        }
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
