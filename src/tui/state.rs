use crate::{
    config::{ConfigLoadSource, RuntimeConfig},
    product,
};

use super::approval::{open_approval_surface, ApprovalInputOutcome, ApprovalSurfaceState};
use super::command::{CommandDispatch, CommandRuntimeLabels, CommandSurfaceState};
use super::expanded_form::{
    ExpandedFormEvents, ExpandedFormKind, ExpandedFormState, ExpandedFormSubmit,
};
use super::persona::{
    PersonaBuffer, PersonaEvent, PersonaEvents, PersonaMessage, PersonaRendered,
    MIN_PERSONA_TERMINAL_WIDTH,
};
use super::working_process::{WorkingProcessEvents, WorkingProcessState};
use super::workspace::{WorkspaceBuffer, WorkspaceEvents, WorkspaceRendered};

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
    pub expanded_form: ExpandedFormState,
    pub working_process: WorkingProcessState,
    pub workspace: WorkspaceBuffer,
    pub persona_panel: PersonaPanelState,
    pub persona: PersonaBuffer,
    pub status_shell_open: bool,
    pub should_quit: bool,
    pub runtime_status: RuntimeStatus,
    pub epilogue_summary: Option<EpilogueSummary>,
}

impl TuiState {
    pub fn intro(
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
    ) -> Self {
        Self {
            scene: Scene::Intro,
            intro_input: String::new(),
            main_input: String::new(),
            pending_prompt: None,
            command_surface: CommandSurfaceState::default(),
            approval_surface: ApprovalSurfaceState::default(),
            expanded_form: ExpandedFormState::default(),
            working_process: WorkingProcessState::default(),
            workspace: WorkspaceBuffer::default(),
            persona_panel: PersonaPanelState::Off,
            persona: PersonaBuffer::default(),
            status_shell_open: false,
            should_quit: false,
            runtime_status: RuntimeStatus::new(workspace, config, config_source, config_warning),
            epilogue_summary: None,
        }
    }

    pub fn main(
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
    ) -> Self {
        let mut state = Self::intro(workspace, config, config_source, config_warning);
        state.scene = Scene::Main;
        state
    }

    pub fn epilogue(
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
    ) -> Self {
        let mut state = Self::intro(workspace, config, config_source, config_warning);
        state.request_exit();
        state
    }

    pub fn request_exit(&mut self) {
        self.epilogue_summary = Some(EpilogueSummary::from_runtime(&self.runtime_status));
        self.scene = Scene::Epilogue;
        self.should_quit = true;
    }

    pub fn enter_main_with_prompt(&mut self) -> PromptSubmitOutcome {
        let prompt = self.intro_input.trim().to_owned();
        if prompt.is_empty() {
            return PromptSubmitOutcome::none();
        }

        self.pending_prompt = Some(prompt);
        self.intro_input.clear();
        self.main_input.clear();
        self.command_surface.close();
        self.scene = Scene::Main;
        self.start_working_process_for_prompt()
    }

    pub fn apply_command_dispatch(
        &mut self,
        dispatch: CommandDispatch,
        terminal_width: u16,
    ) -> CommandDispatchOutcome {
        match dispatch {
            CommandDispatch::None => CommandDispatchOutcome::none(),
            CommandDispatch::HealthCheck => CommandDispatchOutcome::none(),
            CommandDispatch::ExitRequested => {
                self.request_exit();
                CommandDispatchOutcome::none()
            }
            CommandDispatch::StatusShell => {
                self.status_shell_open = true;
                CommandDispatchOutcome {
                    approval_outcome: ApprovalInputOutcome::none(),
                    workspace_events: self.push_status_summary(),
                    persona_events: PersonaEvents::none(),
                    expanded_form_events: ExpandedFormEvents::none(),
                }
            }
            CommandDispatch::ApprovalShell => {
                self.command_surface.close();
                CommandDispatchOutcome {
                    approval_outcome: open_approval_surface(&mut self.approval_surface),
                    workspace_events: WorkspaceEvents::none(),
                    persona_events: PersonaEvents::none(),
                    expanded_form_events: ExpandedFormEvents::none(),
                }
            }
            CommandDispatch::ModeGuide => self.set_mode("Guide"),
            CommandDispatch::ModeCrew => self.set_mode("Crew"),
            CommandDispatch::ModePilot => self.set_mode("Pilot"),
            CommandDispatch::ProviderLmStudio => self.set_provider(product::DEFAULT_PROVIDER),
            CommandDispatch::ModelGemma => self.set_model(product::DEFAULT_MODEL),
            CommandDispatch::OpenLocalProviderForm => {
                self.open_expanded_form(ExpandedFormKind::LocalProvider)
            }
            CommandDispatch::OpenLocalModelForm => {
                self.open_expanded_form(ExpandedFormKind::LocalModel)
            }
            CommandDispatch::OpenDocsInitForm => {
                self.open_expanded_form(ExpandedFormKind::DocsInit)
            }
            CommandDispatch::OpenInitForm => {
                self.open_expanded_form(ExpandedFormKind::InitInstructions)
            }
            CommandDispatch::PersonaFull => self.open_persona_full(terminal_width),
            CommandDispatch::PersonaOff | CommandDispatch::PersonaClose => self.close_persona(),
        }
    }

    pub fn record_workspace_line(&mut self, text: String) -> WorkspaceEvents {
        self.workspace.push_result(text)
    }

    pub fn record_system_notice(&mut self, text: impl Into<String>) -> WorkspaceEvents {
        self.workspace.push_system_notice(text)
    }

    pub fn record_answer(&mut self, text: impl Into<String>) -> WorkspaceEvents {
        self.workspace.push_answer(text)
    }

    pub fn complete_working_process(&mut self) -> WorkingRuntimeOutcome {
        let working_process_events = self.working_process.complete();
        let workspace_events = self.record_working_workspace_items(&working_process_events);
        WorkingRuntimeOutcome {
            working_process_events,
            workspace_events,
        }
    }

    pub fn enter_main_for_runtime_output(&mut self) {
        if matches!(self.scene, Scene::Intro) {
            self.scene = Scene::Main;
            self.intro_input.clear();
            self.main_input.clear();
        }
        self.command_surface.close();
        self.approval_surface.close();
        self.expanded_form.cancel();
    }

    pub fn start_working_process(&mut self) -> PromptSubmitOutcome {
        let prompt = self.main_input.trim().to_owned();
        if prompt.is_empty() {
            return PromptSubmitOutcome::none();
        }

        self.pending_prompt = Some(prompt);
        self.start_working_process_for_prompt()
    }

    pub fn tick_working_process(&mut self) -> WorkingRuntimeOutcome {
        let working_process_events = self.working_process.tick();
        let workspace_events = self.record_working_workspace_items(&working_process_events);
        WorkingRuntimeOutcome {
            working_process_events,
            workspace_events,
        }
    }

    pub fn cancel_working_process(&mut self) -> WorkingRuntimeOutcome {
        let working_process_events = self.working_process.cancel();
        let workspace_events = self.record_working_workspace_items(&working_process_events);
        WorkingRuntimeOutcome {
            working_process_events,
            workspace_events,
        }
    }

    pub fn scroll_workspace(&mut self, delta: isize) -> WorkspaceEvents {
        self.workspace.scroll(delta)
    }

    pub fn take_workspace_render_event(&mut self) -> Option<WorkspaceRendered> {
        self.workspace.take_render_event()
    }

    pub fn cancel_expanded_form(&mut self) -> ExpandedFormOutcome {
        ExpandedFormOutcome {
            workspace_events: WorkspaceEvents::none(),
            expanded_form_events: self.expanded_form.cancel(),
        }
    }

    pub fn focus_next_expanded_form_field(&mut self) {
        self.expanded_form.focus_next();
    }

    pub fn focus_previous_expanded_form_field(&mut self) {
        self.expanded_form.focus_previous();
    }

    pub fn update_expanded_form_char(&mut self, value: char) -> ExpandedFormOutcome {
        ExpandedFormOutcome {
            workspace_events: WorkspaceEvents::none(),
            expanded_form_events: self.expanded_form.push_char(value),
        }
    }

    pub fn backspace_expanded_form(&mut self) -> ExpandedFormOutcome {
        ExpandedFormOutcome {
            workspace_events: WorkspaceEvents::none(),
            expanded_form_events: self.expanded_form.backspace(),
        }
    }

    pub fn submit_expanded_form(&mut self) -> ExpandedFormOutcome {
        let ExpandedFormSubmit {
            submitted,
            events,
            notice,
        } = self.expanded_form.submit();
        let workspace_events = if submitted {
            notice
                .map(|value| self.workspace.push_system_notice(value))
                .unwrap_or_else(WorkspaceEvents::none)
        } else {
            WorkspaceEvents::none()
        };

        ExpandedFormOutcome {
            workspace_events,
            expanded_form_events: events,
        }
    }

    pub fn take_persona_render_event(&mut self) -> Option<PersonaRendered> {
        self.persona.take_render_event()
    }

    fn open_expanded_form(&mut self, kind: ExpandedFormKind) -> CommandDispatchOutcome {
        self.command_surface.close();
        CommandDispatchOutcome {
            approval_outcome: ApprovalInputOutcome::none(),
            workspace_events: WorkspaceEvents::none(),
            persona_events: PersonaEvents::none(),
            expanded_form_events: self.expanded_form.open(kind),
        }
    }

    fn open_persona_full(&mut self, terminal_width: u16) -> CommandDispatchOutcome {
        if terminal_width < MIN_PERSONA_TERMINAL_WIDTH {
            let workspace_events = self.workspace.push_system_notice(format!(
                "페르소나 메시지는 터미널 가로폭 {} 이상에서만 동작합니다.",
                MIN_PERSONA_TERMINAL_WIDTH
            ));
            return CommandDispatchOutcome {
                approval_outcome: ApprovalInputOutcome::none(),
                workspace_events,
                persona_events: PersonaEvents::single(PersonaEvent::WidthRejected {
                    width: terminal_width,
                    min_width: MIN_PERSONA_TERMINAL_WIDTH,
                }),
                expanded_form_events: ExpandedFormEvents::none(),
            };
        }

        if self.persona_panel == PersonaPanelState::Full {
            return CommandDispatchOutcome::none();
        }

        self.persona_panel = PersonaPanelState::Full;
        CommandDispatchOutcome {
            approval_outcome: ApprovalInputOutcome::none(),
            workspace_events: WorkspaceEvents::none(),
            persona_events: PersonaEvents::single(PersonaEvent::PanelOpened),
            expanded_form_events: ExpandedFormEvents::none(),
        }
    }

    fn close_persona(&mut self) -> CommandDispatchOutcome {
        if self.persona_panel == PersonaPanelState::Off {
            return CommandDispatchOutcome::none();
        }

        self.persona_panel = PersonaPanelState::Off;
        CommandDispatchOutcome {
            approval_outcome: ApprovalInputOutcome::none(),
            workspace_events: WorkspaceEvents::none(),
            persona_events: PersonaEvents::single(PersonaEvent::PanelClosed),
            expanded_form_events: ExpandedFormEvents::none(),
        }
    }

    fn set_mode(&mut self, mode: &'static str) -> CommandDispatchOutcome {
        self.runtime_status.mode = mode.to_owned();
        CommandDispatchOutcome {
            approval_outcome: ApprovalInputOutcome::none(),
            workspace_events: self
                .workspace
                .push_system_notice(format!("mode set to {mode}")),
            persona_events: PersonaEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        }
    }

    fn set_provider(&mut self, provider: &'static str) -> CommandDispatchOutcome {
        self.runtime_status.provider = provider.to_owned();
        self.runtime_status.provider_display = provider.to_owned();
        CommandDispatchOutcome {
            approval_outcome: ApprovalInputOutcome::none(),
            workspace_events: self
                .workspace
                .push_system_notice(format!("provider set to {provider}")),
            persona_events: PersonaEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        }
    }

    fn set_model(&mut self, model: &'static str) -> CommandDispatchOutcome {
        self.runtime_status.model = model.to_owned();
        CommandDispatchOutcome {
            approval_outcome: ApprovalInputOutcome::none(),
            workspace_events: self
                .workspace
                .push_system_notice(format!("model set to {model}")),
            persona_events: PersonaEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        }
    }

    fn start_working_process_for_prompt(&mut self) -> PromptSubmitOutcome {
        let prompt = self.pending_prompt.clone().unwrap_or_default();
        let mut workspace_events = self.workspace.push_user_prompt(prompt);
        self.main_input.clear();
        self.command_surface.close();
        self.approval_surface.close();
        self.expanded_form.cancel();
        let working_process_events = self.working_process.start();
        workspace_events.extend(
            self.workspace
                .push_manager_message("요청을 작업 흐름으로 정리했습니다."),
        );
        if self.persona_panel == PersonaPanelState::Full {
            self.persona.push_message(PersonaMessage::team_lead(
                "요청을 확인했습니다. 작업 흐름은 왼쪽 기록과 분리해서 지켜보겠습니다.",
            ));
        }

        PromptSubmitOutcome {
            working_process_events,
            workspace_events,
        }
    }

    fn push_status_summary(&mut self) -> WorkspaceEvents {
        let mut events = self.workspace.push_system_notice(format!(
            "mode {} | config {}",
            self.runtime_status.mode, self.runtime_status.config_source,
        ));
        events.extend(self.workspace.push_system_notice(format!(
            "provider {} | model {}",
            self.runtime_status.provider, self.runtime_status.model,
        )));
        events.extend(self.workspace.push_system_notice(format!(
            "base_url {} | context {}",
            self.runtime_status.base_url, self.runtime_status.context_tokens,
        )));
        if let Some(warning) = &self.runtime_status.config_warning {
            events.extend(
                self.workspace
                    .push_system_notice(format!("config warning {warning}")),
            );
        }
        events
    }

    fn record_working_workspace_items(&mut self, events: &WorkingProcessEvents) -> WorkspaceEvents {
        let mut workspace_events = WorkspaceEvents::none();
        if let Some(workspace_line) = &events.workspace_line {
            workspace_events.extend(self.workspace.push_result(workspace_line.clone()));
        }
        workspace_events
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum PersonaPanelState {
    Off,
    Full,
}

impl PersonaPanelState {
    pub fn is_full(self) -> bool {
        self == Self::Full
    }
}

pub struct PromptSubmitOutcome {
    pub working_process_events: WorkingProcessEvents,
    pub workspace_events: WorkspaceEvents,
}

impl PromptSubmitOutcome {
    pub fn none() -> Self {
        Self {
            working_process_events: WorkingProcessEvents::none(),
            workspace_events: WorkspaceEvents::none(),
        }
    }
}

pub struct WorkingRuntimeOutcome {
    pub working_process_events: WorkingProcessEvents,
    pub workspace_events: WorkspaceEvents,
}

pub struct ExpandedFormOutcome {
    pub workspace_events: WorkspaceEvents,
    pub expanded_form_events: ExpandedFormEvents,
}

pub struct CommandDispatchOutcome {
    pub approval_outcome: ApprovalInputOutcome,
    pub workspace_events: WorkspaceEvents,
    pub persona_events: PersonaEvents,
    pub expanded_form_events: ExpandedFormEvents,
}

impl CommandDispatchOutcome {
    pub fn none() -> Self {
        Self {
            approval_outcome: ApprovalInputOutcome::none(),
            workspace_events: WorkspaceEvents::none(),
            persona_events: PersonaEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        }
    }
}

pub struct RuntimeStatus {
    pub mode: String,
    pub provider: String,
    pub provider_display: String,
    pub model: String,
    pub base_url: String,
    pub workspace: String,
    pub context: String,
    pub context_tokens: u32,
    pub tokens: &'static str,
    pub web: &'static str,
    pub runtime_state: &'static str,
    pub config_source: String,
    pub config_warning: Option<String>,
}

impl RuntimeStatus {
    fn new(
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
    ) -> Self {
        Self {
            mode: config.mode.default.clone(),
            provider: config.provider.active.clone(),
            provider_display: provider_display_name(&config.provider.active).to_owned(),
            model: config.provider.model.clone(),
            base_url: config.provider.base_url.clone(),
            workspace,
            context: format!("ctx {}", config.provider.context_tokens),
            context_tokens: config.provider.context_tokens,
            tokens: product::DEFAULT_TOKEN_STATUS,
            web: if config.web.enabled {
                "web on"
            } else {
                "web off"
            },
            runtime_state: product::DEFAULT_RUNTIME_STATE,
            config_source: config_source.as_str().to_owned(),
            config_warning: config_warning.map(ToOwned::to_owned),
        }
    }

    pub fn command_labels(&self) -> CommandRuntimeLabels<'_> {
        CommandRuntimeLabels {
            mode: self.mode.as_str(),
            provider_display: self.provider_display.as_str(),
            model: self.model.as_str(),
            base_url: self.base_url.as_str(),
        }
    }
}

pub struct EpilogueSummary {
    pub workspace: String,
    pub model: String,
    pub mode: String,
    pub session: &'static str,
    pub tools_executed: u16,
    pub tools_failed: u16,
    pub closing_message: &'static str,
}

impl EpilogueSummary {
    fn from_runtime(runtime_status: &RuntimeStatus) -> Self {
        Self {
            workspace: runtime_status.workspace.clone(),
            model: runtime_status.model.clone(),
            mode: runtime_status.mode.clone(),
            session: product::SESSION_SAVED_LABEL,
            tools_executed: 0,
            tools_failed: 0,
            closing_message: product::GOODBYE_LABEL,
        }
    }
}

fn provider_display_name(provider: &str) -> &str {
    if provider == product::DEFAULT_PROVIDER {
        product::DEFAULT_PROVIDER_DISPLAY
    } else {
        provider
    }
}
