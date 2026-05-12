use crate::product;

#[derive(Clone, Copy)]
pub enum Scene {
    Intro,
    Epilogue,
}

pub struct TuiState {
    pub scene: Scene,
    pub intro_input: String,
    pub should_quit: bool,
    pub runtime_status: RuntimeStatus,
    pub epilogue_summary: Option<EpilogueSummary>,
}

impl TuiState {
    pub fn intro(workspace: String) -> Self {
        Self {
            scene: Scene::Intro,
            intro_input: String::new(),
            should_quit: false,
            runtime_status: RuntimeStatus::new(workspace),
            epilogue_summary: None,
        }
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
