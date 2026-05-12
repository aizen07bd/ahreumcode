use crate::product;

use super::command::{CommandDispatch, CommandSurfaceState};

#[derive(Clone, Copy)]
pub enum Scene {
    Intro,
    Main,
    Epilogue,
}

impl Scene {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intro => "intro",
            Self::Main => "workspace",
            Self::Epilogue => "epilogue",
        }
    }
}

pub struct TuiState {
    pub scene: Scene,
    pub intro_input: String,
    pub main_input: String,
    pub pending_prompt: Option<String>,
    pub command_surface: CommandSurfaceState,
    pub persona_panel: PersonaPanelState,
    pub status_shell_open: bool,
    pub should_quit: bool,
    pub runtime_status: RuntimeStatus,
    pub epilogue_summary: Option<EpilogueSummary>,
}

impl TuiState {
    pub fn intro(workspace: String) -> Self {
        Self {
            scene: Scene::Intro,
            intro_input: String::new(),
            main_input: String::new(),
            pending_prompt: None,
            command_surface: CommandSurfaceState::default(),
            persona_panel: PersonaPanelState::Off,
            status_shell_open: false,
            should_quit: false,
            runtime_status: RuntimeStatus::new(workspace),
            epilogue_summary: None,
        }
    }

    pub fn main(workspace: String) -> Self {
        let mut state = Self::intro(workspace);
        state.scene = Scene::Main;
        state
    }

    pub fn epilogue(workspace: String) -> Self {
        let mut state = Self::intro(workspace);
        state.request_exit();
        state
    }

    pub fn request_exit(&mut self) {
        self.epilogue_summary = Some(EpilogueSummary::from_runtime(&self.runtime_status));
        self.scene = Scene::Epilogue;
        self.should_quit = true;
    }

    pub fn enter_main_with_prompt(&mut self) {
        let prompt = self.intro_input.trim().to_owned();
        if prompt.is_empty() {
            return;
        }

        self.pending_prompt = Some(prompt);
        self.intro_input.clear();
        self.main_input.clear();
        self.command_surface.close();
        self.scene = Scene::Main;
    }

    pub fn apply_command_dispatch(&mut self, dispatch: CommandDispatch) {
        match dispatch {
            CommandDispatch::None => {}
            CommandDispatch::ExitRequested => self.request_exit(),
            CommandDispatch::StatusShell => {
                self.status_shell_open = true;
            }
            CommandDispatch::PersonaFull => {
                self.persona_panel = PersonaPanelState::Full;
            }
            CommandDispatch::PersonaOff | CommandDispatch::PersonaClose => {
                self.persona_panel = PersonaPanelState::Off;
            }
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum PersonaPanelState {
    Off,
    Full,
}

pub struct RuntimeStatus {
    pub mode: &'static str,
    pub provider: &'static str,
    pub model: &'static str,
    pub workspace: String,
    pub context: &'static str,
    pub tokens: &'static str,
    pub web: &'static str,
    pub runtime_state: &'static str,
}

impl RuntimeStatus {
    fn new(workspace: String) -> Self {
        Self {
            mode: product::DEFAULT_MODE,
            provider: product::DEFAULT_PROVIDER,
            model: product::DEFAULT_MODEL,
            workspace,
            context: product::DEFAULT_CONTEXT_STATUS,
            tokens: product::DEFAULT_TOKEN_STATUS,
            web: product::DEFAULT_WEB_STATUS,
            runtime_state: product::DEFAULT_RUNTIME_STATE,
        }
    }
}

pub struct EpilogueSummary {
    pub workspace: String,
    pub model: &'static str,
    pub mode: &'static str,
    pub session: &'static str,
    pub tools_executed: u16,
    pub tools_failed: u16,
    pub closing_message: &'static str,
}

impl EpilogueSummary {
    fn from_runtime(runtime_status: &RuntimeStatus) -> Self {
        Self {
            workspace: runtime_status.workspace.clone(),
            model: runtime_status.model,
            mode: runtime_status.mode,
            session: product::SESSION_SAVED_LABEL,
            tools_executed: 0,
            tools_failed: 0,
            closing_message: product::GOODBYE_LABEL,
        }
    }
}
