use crate::product;

use super::approval::{open_approval_surface, ApprovalInputOutcome, ApprovalSurfaceState};
use super::command::{CommandDispatch, CommandSurfaceState};
use super::working_process::{WorkingProcessEvents, WorkingProcessState};

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
    pub approval_surface: ApprovalSurfaceState,
    pub working_process: WorkingProcessState,
    pub workspace_entries: Vec<WorkspaceEntry>,
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
            approval_surface: ApprovalSurfaceState::default(),
            working_process: WorkingProcessState::default(),
            workspace_entries: Vec::new(),
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

    pub fn enter_main_with_prompt(&mut self) -> WorkingProcessEvents {
        let prompt = self.intro_input.trim().to_owned();
        if prompt.is_empty() {
            return WorkingProcessEvents::none();
        }

        self.pending_prompt = Some(prompt);
        self.intro_input.clear();
        self.main_input.clear();
        self.command_surface.close();
        self.scene = Scene::Main;
        self.working_process.start()
    }

    pub fn apply_command_dispatch(&mut self, dispatch: CommandDispatch) -> ApprovalInputOutcome {
        match dispatch {
            CommandDispatch::None => ApprovalInputOutcome::none(),
            CommandDispatch::ExitRequested => {
                self.request_exit();
                ApprovalInputOutcome::none()
            }
            CommandDispatch::StatusShell => {
                self.status_shell_open = true;
                ApprovalInputOutcome::none()
            }
            CommandDispatch::ApprovalShell => {
                self.command_surface.close();
                open_approval_surface(&mut self.approval_surface)
            }
            CommandDispatch::PersonaFull => {
                self.persona_panel = PersonaPanelState::Full;
                ApprovalInputOutcome::none()
            }
            CommandDispatch::PersonaOff | CommandDispatch::PersonaClose => {
                self.persona_panel = PersonaPanelState::Off;
                ApprovalInputOutcome::none()
            }
        }
    }

    pub fn record_workspace_line(&mut self, text: String) {
        self.workspace_entries.push(WorkspaceEntry { text });
    }

    pub fn start_working_process(&mut self) -> WorkingProcessEvents {
        self.main_input.clear();
        self.command_surface.close();
        self.approval_surface.close();
        self.working_process.start()
    }

    pub fn tick_working_process(&mut self) -> WorkingProcessEvents {
        let events = self.working_process.tick();
        self.record_working_workspace_line(&events);
        events
    }

    pub fn cancel_working_process(&mut self) -> WorkingProcessEvents {
        let events = self.working_process.cancel();
        self.record_working_workspace_line(&events);
        events
    }

    fn record_working_workspace_line(&mut self, events: &WorkingProcessEvents) {
        if let Some(workspace_line) = &events.workspace_line {
            self.record_workspace_line(workspace_line.clone());
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum PersonaPanelState {
    Off,
    Full,
}

pub struct WorkspaceEntry {
    pub text: String,
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
