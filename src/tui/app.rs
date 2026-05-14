use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::Terminal;
use serde_json::json;

use crate::cli::{AppCommand, RunMode, SceneCommand};
use crate::config::{ConfigLoadOutcome, ConfigLoadSource, RuntimeConfig};
use crate::llm::{
    LlmChatReport, LlmChatRequest, LlmChatStatus, LlmHealthReport, LlmHealthStatus,
    LlmProviderFactory,
};
use crate::logging::{LogEvent, Logger};

use super::approval::ApprovalInputEvent;
use super::command::{CommandDispatch, CommandInputEvent, CommandRegistry};
use super::expanded_form::ExpandedFormEvent;
use super::persona::{PersonaEvent, PersonaRendered};
use super::scenes::epilogue::print_epilogue;
use super::scenes::intro::{handle_intro_event, render_intro};
use super::scenes::main::{handle_main_event, render_main};
use super::state::{Scene, TuiState};
use super::working_process::{WorkingFinishReason, WorkingProcessEvent, WorkingProcessEvents};
use super::workspace::{WorkspaceEvent, WorkspaceEvents, WorkspaceRendered};

const TUI_01_SCOPE: &str = "tui-01-intro-scene";
const TUI_02_SCOPE: &str = "tui-02-epilogue-scene";
const TUI_03_SCOPE: &str = "tui-03-main-scene-layout";
const TUI_04_SCOPE: &str = "tui-04-command-area-basic-actions";
const TUI_05_SCOPE: &str = "tui-05-approval-area";
const TUI_06_SCOPE: &str = "tui-06-working-process-area";
const TUI_07_SCOPE: &str = "tui-07-workspace-output-layout";
const TUI_08_SCOPE: &str = "tui-08-persona-message-detail";
const TUI_09_SCOPE: &str = "tui-09-complex-commands";
const TUI_10_SCOPE: &str = "tui-10-modal-expanded-form";
const LLM_01_SCOPE: &str = "llm-01-config-runtime";
const LLM_02_SCOPE: &str = "llm-02-provider-connection";
const LLM_03_SCOPE: &str = "llm-03-plain-prompt-request";
const EVENT_APP_STARTED: &str = "app_started";
const EVENT_TERMINAL_ENTERED: &str = "terminal_entered";
const EVENT_INTRO_RENDERED: &str = "intro_rendered";
const EVENT_PROMPT_FOCUS_READY: &str = "prompt_focus_ready";
const EVENT_EXIT_REQUESTED: &str = "exit_requested";
const EVENT_SESSION_SUMMARY_CREATED: &str = "session_summary_created";
const EVENT_EPILOGUE_RENDERED: &str = "epilogue_rendered";
const EVENT_TERMINAL_RESTORED: &str = "terminal_restored";
const EVENT_MAIN_SCENE_RENDERED: &str = "main_scene_rendered";
const EVENT_LAYOUT_CALCULATED: &str = "layout_calculated";
const EVENT_PERSONA_LAYOUT_ABSENT: &str = "persona_layout_absent";
const EVENT_STATUSLINE_POSITIONED: &str = "statusline_positioned";
const EVENT_COMMAND_SURFACE_OPENED: &str = "command_surface_opened";
const EVENT_COMMAND_FILTER_CHANGED: &str = "command_filter_changed";
const EVENT_COMMAND_SELECTED: &str = "command_selected";
const EVENT_COMMAND_ACTION_DISPATCHED: &str = "command_action_dispatched";
const EVENT_COMMAND_AVAILABILITY_CHECKED: &str = "command_availability_checked";
const EVENT_STEPPED_PICKER_OPENED: &str = "stepped_picker_opened";
const EVENT_STEPPED_PICKER_SELECTION_CHANGED: &str = "stepped_picker_selection_changed";
const EVENT_STEPPED_PICKER_CONFIRMED: &str = "stepped_picker_confirmed";
const EVENT_EXPANDED_FORM_OPENED: &str = "expanded_form_opened";
const EVENT_EXPANDED_FORM_FIELD_CHANGED: &str = "expanded_form_field_changed";
const EVENT_EXPANDED_FORM_SUBMITTED: &str = "expanded_form_submitted";
const EVENT_EXPANDED_FORM_CANCELLED: &str = "expanded_form_cancelled";
const EVENT_APPROVAL_SURFACE_OPENED: &str = "approval_surface_opened";
const EVENT_APPROVAL_OPTION_SELECTED: &str = "approval_option_selected";
const EVENT_APPROVAL_RESULT_RECORDED: &str = "approval_result_recorded";
const EVENT_WORKING_PROCESS_STARTED: &str = "working_process_started";
const EVENT_WORKING_PHASE_CHANGED: &str = "working_phase_changed";
const EVENT_WORKING_PROCESS_CANCEL_HINT_RENDERED: &str = "working_process_cancel_hint_rendered";
const EVENT_WORKING_PROCESS_FINISHED: &str = "working_process_finished";
const EVENT_WORKSPACE_PROMPT_BLOCK_ADDED: &str = "workspace_prompt_block_added";
const EVENT_WORKSPACE_OUTPUT_ADDED: &str = "workspace_output_added";
const EVENT_WORKSPACE_SCROLL_CHANGED: &str = "workspace_scroll_changed";
const EVENT_WORKSPACE_RENDERED: &str = "workspace_rendered";
const EVENT_PERSONA_PANEL_OPENED: &str = "persona_panel_opened";
const EVENT_PERSONA_PANEL_CLOSED: &str = "persona_panel_closed";
const EVENT_PERSONA_MESSAGE_RENDERED: &str = "persona_message_rendered";
const EVENT_PERSONA_WIDTH_REJECTED: &str = "persona_width_rejected";
const EVENT_CONFIG_LOAD_STARTED: &str = "config_load_started";
const EVENT_CONFIG_LOADED: &str = "config_loaded";
const EVENT_CONFIG_DEFAULT_APPLIED: &str = "config_default_applied";
const EVENT_CONFIG_LOAD_FAILED: &str = "config_load_failed";
const EVENT_LLM_HEALTH_CHECK_STARTED: &str = "llm_health_check_started";
const EVENT_LLM_HEALTH_CHECK_SUCCEEDED: &str = "llm_health_check_succeeded";
const EVENT_LLM_HEALTH_CHECK_FAILED: &str = "llm_health_check_failed";
const EVENT_LLM_LATENCY_RECORDED: &str = "llm_latency_recorded";
const EVENT_LLM_REQUEST_STARTED: &str = "llm_request_started";
const EVENT_LLM_RESPONSE_RECEIVED: &str = "llm_response_received";
const EVENT_LLM_REQUEST_CANCELLED: &str = "llm_request_cancelled";
const EVENT_LLM_REQUEST_FAILED: &str = "llm_request_failed";

pub fn run_app(command: AppCommand) -> io::Result<()> {
    match (command.scene, command.run_mode) {
        (SceneCommand::Intro, RunMode::Smoke) => run_intro_smoke(command),
        (SceneCommand::Main, RunMode::Smoke) => run_main_smoke(command),
        (SceneCommand::Main, _) => run_main_terminal(command),
        (SceneCommand::Epilogue, RunMode::Smoke) => run_epilogue_smoke(command),
        (SceneCommand::Epilogue, _) => run_epilogue_terminal(command),
        (SceneCommand::Intro, _) => run_intro_terminal(command),
    }
}

fn run_intro_terminal(command: AppCommand) -> io::Result<()> {
    let project_root = current_workspace_path()?;
    let workspace = workspace_display(&project_root);
    let logger = Logger::start()?;
    let config_outcome = load_runtime_config(&logger, &project_root)?;
    logger.ui(LogEvent::ui(
        TUI_01_SCOPE,
        EVENT_APP_STARTED,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;

    let mut terminal = TerminalSession::enter()?;
    logger.ui(LogEvent::ui(
        TUI_01_SCOPE,
        EVENT_TERMINAL_ENTERED,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;

    let mut app = TuiApp::new(
        logger,
        workspace,
        &config_outcome.config,
        config_outcome.source,
        config_outcome
            .warning
            .as_ref()
            .map(|warning| warning.message.as_str()),
        command.run_mode.as_str(),
    );
    app.run(terminal.terminal_mut())?;
    terminal.restore()?;
    app.log_terminal_restored()?;
    app.print_epilogue_after_restore()
}

fn run_intro_smoke(command: AppCommand) -> io::Result<()> {
    let project_root = current_workspace_path()?;
    let workspace = workspace_display(&project_root);
    let logger = Logger::start()?;
    let config_outcome = load_runtime_config(&logger, &project_root)?;
    logger.ui(LogEvent::ui(
        TUI_01_SCOPE,
        EVENT_APP_STARTED,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;

    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend)?;
    let state = TuiState::intro(
        workspace,
        &config_outcome.config,
        config_outcome.source,
        config_outcome
            .warning
            .as_ref()
            .map(|warning| warning.message.as_str()),
    );

    logger.ui(LogEvent::ui(
        TUI_01_SCOPE,
        EVENT_PROMPT_FOCUS_READY,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;
    terminal.draw(|frame| render_intro(frame, &state))?;
    logger.ui(LogEvent::ui(
        TUI_01_SCOPE,
        EVENT_INTRO_RENDERED,
        json!({ "run_mode": command.run_mode.as_str(), "backend": "test" }),
    ))?;

    println!("tui-01 intro smoke ok");
    println!("scene=intro");
    println!("run_mode={}", command.run_mode.as_str());
    println!("log_dir={}", logger.session_dir().display());

    Ok(())
}

fn run_main_terminal(command: AppCommand) -> io::Result<()> {
    let project_root = current_workspace_path()?;
    let workspace = workspace_display(&project_root);
    let logger = Logger::start()?;
    let config_outcome = load_runtime_config(&logger, &project_root)?;
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_APP_STARTED,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;

    let mut terminal = TerminalSession::enter()?;
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_TERMINAL_ENTERED,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;

    let mut app = TuiApp::new_main(
        logger,
        workspace,
        &config_outcome.config,
        config_outcome.source,
        config_outcome
            .warning
            .as_ref()
            .map(|warning| warning.message.as_str()),
        command.run_mode.as_str(),
    );
    app.run(terminal.terminal_mut())?;
    terminal.restore()?;
    app.log_terminal_restored()?;
    app.print_epilogue_after_restore()
}

fn run_main_smoke(command: AppCommand) -> io::Result<()> {
    let project_root = current_workspace_path()?;
    let workspace = workspace_display(&project_root);
    let logger = Logger::start()?;
    let config_outcome = load_runtime_config(&logger, &project_root)?;
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend)?;
    let state = TuiState::main(
        workspace,
        &config_outcome.config,
        config_outcome.source,
        config_outcome
            .warning
            .as_ref()
            .map(|warning| warning.message.as_str()),
    );

    terminal.draw(|frame| render_main(frame, &state))?;
    log_main_scene_rendered(&logger, command.run_mode.as_str())?;

    println!("tui-03 main smoke ok");
    println!("scene=main");
    println!("run_mode={}", command.run_mode.as_str());
    println!("log_dir={}", logger.session_dir().display());

    Ok(())
}

fn run_epilogue_terminal(command: AppCommand) -> io::Result<()> {
    let project_root = current_workspace_path()?;
    let workspace = workspace_display(&project_root);
    let logger = Logger::start()?;
    let config_outcome = load_runtime_config(&logger, &project_root)?;
    let app = TuiApp::new_epilogue(
        logger,
        workspace,
        &config_outcome.config,
        config_outcome.source,
        config_outcome
            .warning
            .as_ref()
            .map(|warning| warning.message.as_str()),
        command.run_mode.as_str(),
    );

    app.log_exit_requested(command.run_mode.as_str(), "scene")?;
    app.log_session_summary_created()?;
    app.print_epilogue_after_restore()
}

fn run_epilogue_smoke(command: AppCommand) -> io::Result<()> {
    let project_root = current_workspace_path()?;
    let workspace = workspace_display(&project_root);
    let logger = Logger::start()?;
    let config_outcome = load_runtime_config(&logger, &project_root)?;
    let app = TuiApp::new_epilogue(
        logger,
        workspace,
        &config_outcome.config,
        config_outcome.source,
        config_outcome
            .warning
            .as_ref()
            .map(|warning| warning.message.as_str()),
        command.run_mode.as_str(),
    );

    app.log_exit_requested(command.run_mode.as_str(), "smoke")?;
    app.log_session_summary_created()?;
    app.print_epilogue_after_restore()?;

    println!("tui-02 epilogue smoke ok");
    println!("scene=epilogue");
    println!("run_mode={}", command.run_mode.as_str());
    println!("log_dir={}", app.logger.session_dir().display());

    Ok(())
}

struct TuiApp {
    state: TuiState,
    logger: Logger,
    runtime_config: RuntimeConfig,
    active_plain_request: Option<ActivePlainRequest>,
    next_run_index: u64,
    run_mode: &'static str,
    intro_render_logged: bool,
    main_render_logged: bool,
    terminal_restore_scope: Option<&'static str>,
}

struct ActivePlainRequest {
    run_id: String,
    prompt: String,
    receiver: Receiver<LlmChatReport>,
    cancelled: bool,
}

impl TuiApp {
    fn new(
        logger: Logger,
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
        run_mode: &'static str,
    ) -> Self {
        Self {
            state: TuiState::intro(workspace, config, config_source, config_warning),
            logger,
            runtime_config: config.clone(),
            active_plain_request: None,
            next_run_index: 1,
            run_mode,
            intro_render_logged: false,
            main_render_logged: false,
            terminal_restore_scope: None,
        }
    }

    fn new_main(
        logger: Logger,
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
        run_mode: &'static str,
    ) -> Self {
        Self {
            state: TuiState::main(workspace, config, config_source, config_warning),
            logger,
            runtime_config: config.clone(),
            active_plain_request: None,
            next_run_index: 1,
            run_mode,
            intro_render_logged: true,
            main_render_logged: false,
            terminal_restore_scope: None,
        }
    }

    fn new_epilogue(
        logger: Logger,
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
        run_mode: &'static str,
    ) -> Self {
        Self {
            state: TuiState::epilogue(workspace, config, config_source, config_warning),
            logger,
            runtime_config: config.clone(),
            active_plain_request: None,
            next_run_index: 1,
            run_mode,
            intro_render_logged: true,
            main_render_logged: true,
            terminal_restore_scope: Some(TUI_02_SCOPE),
        }
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        if matches!(self.state.scene, Scene::Intro) {
            self.logger.ui(LogEvent::ui(
                TUI_01_SCOPE,
                EVENT_PROMPT_FOCUS_READY,
                json!({}),
            ))?;
        }

        while !self.state.should_quit {
            self.poll_plain_prompt_request()?;

            if matches!(self.state.scene, Scene::Main) {
                let runtime_outcome = self.state.tick_working_process();
                self.log_working_process_events(&runtime_outcome.working_process_events.events)?;
                self.log_workspace_events(&runtime_outcome.workspace_events.events)?;
            }

            terminal.draw(|frame| match self.state.scene {
                Scene::Intro => render_intro(frame, &self.state),
                Scene::Main => render_main(frame, &self.state),
                Scene::Epilogue => {}
            })?;

            if !self.intro_render_logged {
                self.logger
                    .ui(LogEvent::ui(TUI_01_SCOPE, EVENT_INTRO_RENDERED, json!({})))?;
                self.intro_render_logged = true;
            }
            if matches!(self.state.scene, Scene::Main) && !self.main_render_logged {
                log_main_scene_rendered(&self.logger, self.run_mode)?;
                self.main_render_logged = true;
            }
            self.log_workspace_render_if_pending()?;
            self.log_persona_render_if_pending()?;

            if event::poll(Duration::from_millis(100))? {
                let Event::Key(key_event) = event::read()? else {
                    continue;
                };
                match self.state.scene {
                    Scene::Intro => {
                        let action = handle_intro_event(key_event, &mut self.state);
                        self.log_command_events(&action.command_outcome.events)?;
                        self.log_working_process_events(&action.working_process_events.events)?;
                        self.log_workspace_events(&action.workspace_events.events)?;
                        self.handle_runtime_dispatch(action.command_outcome.dispatch)?;
                        self.handle_plain_prompt_events(&action.working_process_events)?;
                        if action.command_outcome.dispatch == CommandDispatch::ExitRequested {
                            self.terminal_restore_scope = Some(TUI_02_SCOPE);
                            self.log_exit_requested(self.run_mode, "intro_prompt")?;
                            self.log_session_summary_created()?;
                        }
                    }
                    Scene::Main => {
                        let action = handle_main_event(key_event, &mut self.state);
                        self.log_command_events(&action.command_outcome.events)?;
                        self.log_approval_events(&action.approval_outcome.events)?;
                        self.log_working_process_events(&action.working_process_events.events)?;
                        self.log_workspace_events(&action.workspace_events.events)?;
                        self.log_persona_events(&action.persona_events.events)?;
                        self.log_expanded_form_events(&action.expanded_form_events.events)?;
                        self.handle_runtime_dispatch(action.command_outcome.dispatch)?;
                        self.handle_plain_prompt_events(&action.working_process_events)?;
                        if action.command_outcome.dispatch == CommandDispatch::ExitRequested {
                            self.terminal_restore_scope = Some(TUI_02_SCOPE);
                            self.log_exit_requested(self.run_mode, "main_prompt")?;
                            self.log_session_summary_created()?;
                        }
                    }
                    Scene::Epilogue => {}
                }
            }
        }

        Ok(())
    }

    fn handle_runtime_dispatch(&mut self, dispatch: CommandDispatch) -> io::Result<()> {
        match dispatch {
            CommandDispatch::HealthCheck => self.run_health_check(),
            _ => Ok(()),
        }
    }

    fn run_health_check(&mut self) -> io::Result<()> {
        self.state.enter_main_for_runtime_output();
        self.log_health_check_started()?;

        let provider = LlmProviderFactory::from_config(&self.runtime_config);
        let report = provider.health_check();

        self.log_health_latency(&report)?;
        match &report.status {
            LlmHealthStatus::Succeeded { .. } => self.log_health_check_succeeded(&report)?,
            LlmHealthStatus::Failed(_) => self.log_health_check_failed(&report)?,
        }

        let events = self.record_health_report(&report);
        self.log_workspace_events(&events.events)
    }

    fn record_health_report(&mut self, report: &LlmHealthReport) -> WorkspaceEvents {
        match &report.status {
            LlmHealthStatus::Succeeded { available_models } => {
                let mut events = self.state.record_system_notice("health ok");
                events.extend(self.state.record_system_notice(format!(
                    "provider {} | model {}",
                    report.provider, report.model
                )));
                events.extend(self.state.record_system_notice(format!(
                    "latency {} ms | models {} | endpoint {}",
                    report.latency_ms, available_models, report.models_url
                )));
                events
            }
            LlmHealthStatus::Failed(failure) => {
                let mut events = self
                    .state
                    .record_system_notice(format!("health failed: {}", failure.kind.as_str()));
                events.extend(self.state.record_system_notice(format!(
                    "provider {} | model {}",
                    report.provider, report.model
                )));
                events.extend(
                    self.state
                        .record_system_notice(format!("endpoint {}", report.models_url)),
                );
                events.extend(
                    self.state
                        .record_system_notice(format!("message {}", failure.message)),
                );
                events
            }
        }
    }

    fn log_health_check_started(&self) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_02_SCOPE,
            EVENT_LLM_HEALTH_CHECK_STARTED,
            json!({
                "provider": &self.runtime_config.provider.active,
                "provider_type": self.runtime_config.provider.provider_type.as_str(),
                "base_url": &self.runtime_config.provider.base_url,
                "model": &self.runtime_config.provider.model,
                "timeout_ms": self.runtime_config.limits.command_timeout_ms,
            }),
        ))
    }

    fn log_health_latency(&self, report: &LlmHealthReport) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_02_SCOPE,
            EVENT_LLM_LATENCY_RECORDED,
            json!({
                "provider": &report.provider,
                "model": &report.model,
                "endpoint": &report.models_url,
                "latency_ms": report.latency_ms,
            }),
        ))
    }

    fn log_health_check_succeeded(&self, report: &LlmHealthReport) -> io::Result<()> {
        let LlmHealthStatus::Succeeded { available_models } = &report.status else {
            return Ok(());
        };

        self.logger.llm(LogEvent::ui(
            LLM_02_SCOPE,
            EVENT_LLM_HEALTH_CHECK_SUCCEEDED,
            json!({
                "provider": &report.provider,
                "base_url": &report.base_url,
                "model": &report.model,
                "endpoint": &report.models_url,
                "latency_ms": report.latency_ms,
                "available_models": available_models,
            }),
        ))
    }

    fn log_health_check_failed(&self, report: &LlmHealthReport) -> io::Result<()> {
        let LlmHealthStatus::Failed(failure) = &report.status else {
            return Ok(());
        };

        self.logger.llm(LogEvent::ui(
            LLM_02_SCOPE,
            EVENT_LLM_HEALTH_CHECK_FAILED,
            json!({
                "provider": &report.provider,
                "base_url": &report.base_url,
                "model": &report.model,
                "endpoint": &report.models_url,
                "latency_ms": report.latency_ms,
                "failure_kind": failure.kind.as_str(),
                "http_status": failure.http_status,
                "message": &failure.message,
                "recoverable": true,
            }),
        ))
    }

    fn handle_plain_prompt_events(&mut self, events: &WorkingProcessEvents) -> io::Result<()> {
        if working_started(events) {
            if let Some(prompt) = self.state.pending_prompt.clone() {
                self.start_plain_prompt_request(prompt)?;
            }
        }

        if working_cancelled(events) {
            self.cancel_active_plain_request()?;
        }

        Ok(())
    }

    fn start_plain_prompt_request(&mut self, prompt: String) -> io::Result<()> {
        if self.active_plain_request.is_some() {
            return Ok(());
        }

        let run_id = self.next_run_id();
        self.log_plain_request_started(&run_id, &prompt)?;

        let (sender, receiver) = mpsc::channel();
        let config = self.runtime_config.clone();
        let thread_prompt = prompt.clone();
        thread::spawn(move || {
            let provider = LlmProviderFactory::from_config(&config);
            let report = provider.send_chat(LlmChatRequest {
                prompt: thread_prompt,
            });
            let _ = sender.send(report);
        });

        self.active_plain_request = Some(ActivePlainRequest {
            run_id,
            prompt,
            receiver,
            cancelled: false,
        });

        Ok(())
    }

    fn poll_plain_prompt_request(&mut self) -> io::Result<()> {
        let Some(active) = self.active_plain_request.as_ref() else {
            return Ok(());
        };

        let result = match active.receiver.try_recv() {
            Ok(report) => Some(report),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.log_plain_request_failed_channel(active)?;
                let complete_outcome = self.state.complete_working_process();
                self.log_working_process_events(&complete_outcome.working_process_events.events)?;
                self.log_workspace_events(&complete_outcome.workspace_events.events)?;
                let mut events = self
                    .state
                    .record_system_notice("request failed: runtime_channel_disconnected");
                events.extend(
                    self.state
                        .record_system_notice("message local request worker ended unexpectedly"),
                );
                self.log_workspace_events(&events.events)?;
                self.active_plain_request = None;
                return Ok(());
            }
        };

        let Some(report) = result else {
            return Ok(());
        };

        let Some(active) = self.active_plain_request.take() else {
            return Ok(());
        };

        if active.cancelled {
            return Ok(());
        }

        let complete_outcome = self.state.complete_working_process();
        self.log_working_process_events(&complete_outcome.working_process_events.events)?;
        self.log_workspace_events(&complete_outcome.workspace_events.events)?;
        self.state.pending_prompt = None;

        match &report.status {
            LlmChatStatus::Succeeded { .. } => {
                self.log_plain_response_received(&active, &report)?
            }
            LlmChatStatus::Failed(_) => self.log_plain_request_failed(&active, &report)?,
        }

        let events = self.record_plain_chat_report(&report);
        self.log_workspace_events(&events.events)
    }

    fn cancel_active_plain_request(&mut self) -> io::Result<()> {
        let Some(mut active) = self.active_plain_request.take() else {
            return Ok(());
        };
        active.cancelled = true;
        self.log_plain_request_cancelled(&active)?;
        self.state.pending_prompt = None;
        Ok(())
    }

    fn record_plain_chat_report(&mut self, report: &LlmChatReport) -> WorkspaceEvents {
        match &report.status {
            LlmChatStatus::Succeeded { answer } => self.state.record_answer(answer.clone()),
            LlmChatStatus::Failed(failure) => {
                let mut events = self
                    .state
                    .record_system_notice(format!("request failed: {}", failure.kind.as_str()));
                events.extend(self.state.record_system_notice(format!(
                    "provider {} | model {}",
                    report.provider, report.model
                )));
                events.extend(
                    self.state
                        .record_system_notice(format!("endpoint {}", report.chat_url)),
                );
                events.extend(
                    self.state
                        .record_system_notice(format!("message {}", failure.message)),
                );
                events
            }
        }
    }

    fn next_run_id(&mut self) -> String {
        let run_id = format!("run-{number:04}", number = self.next_run_index);
        self.next_run_index += 1;
        run_id
    }

    fn log_plain_request_started(&self, run_id: &str, prompt: &str) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_03_SCOPE,
            EVENT_LLM_REQUEST_STARTED,
            json!({
                "run_id": run_id,
                "provider": &self.runtime_config.provider.active,
                "provider_type": self.runtime_config.provider.provider_type.as_str(),
                "base_url": &self.runtime_config.provider.base_url,
                "model": &self.runtime_config.provider.model,
                "prompt_chars": prompt.chars().count(),
                "timeout_ms": self.runtime_config.limits.command_timeout_ms,
            }),
        ))
    }

    fn log_plain_response_received(
        &self,
        active: &ActivePlainRequest,
        report: &LlmChatReport,
    ) -> io::Result<()> {
        let LlmChatStatus::Succeeded { answer } = &report.status else {
            return Ok(());
        };

        self.logger.llm(LogEvent::ui(
            LLM_03_SCOPE,
            EVENT_LLM_RESPONSE_RECEIVED,
            json!({
                "run_id": &active.run_id,
                "provider": &report.provider,
                "base_url": &report.base_url,
                "model": &report.model,
                "endpoint": &report.chat_url,
                "latency_ms": report.latency_ms,
                "prompt_chars": active.prompt.chars().count(),
                "response_chars": answer.chars().count(),
            }),
        ))
    }

    fn log_plain_request_failed(
        &self,
        active: &ActivePlainRequest,
        report: &LlmChatReport,
    ) -> io::Result<()> {
        let LlmChatStatus::Failed(failure) = &report.status else {
            return Ok(());
        };

        self.logger.llm(LogEvent::ui(
            LLM_03_SCOPE,
            EVENT_LLM_REQUEST_FAILED,
            json!({
                "run_id": &active.run_id,
                "provider": &report.provider,
                "base_url": &report.base_url,
                "model": &report.model,
                "endpoint": &report.chat_url,
                "latency_ms": report.latency_ms,
                "prompt_chars": active.prompt.chars().count(),
                "failure_kind": failure.kind.as_str(),
                "http_status": failure.http_status,
                "message": &failure.message,
                "recoverable": true,
            }),
        ))
    }

    fn log_plain_request_failed_channel(&self, active: &ActivePlainRequest) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_03_SCOPE,
            EVENT_LLM_REQUEST_FAILED,
            json!({
                "run_id": &active.run_id,
                "provider": &self.runtime_config.provider.active,
                "model": &self.runtime_config.provider.model,
                "prompt_chars": active.prompt.chars().count(),
                "failure_kind": "runtime_channel_disconnected",
                "recoverable": true,
            }),
        ))
    }

    fn log_plain_request_cancelled(&self, active: &ActivePlainRequest) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_03_SCOPE,
            EVENT_LLM_REQUEST_CANCELLED,
            json!({
                "run_id": &active.run_id,
                "provider": &self.runtime_config.provider.active,
                "model": &self.runtime_config.provider.model,
                "prompt_chars": active.prompt.chars().count(),
            }),
        ))
    }

    fn log_exit_requested(&self, run_mode: &str, source: &str) -> io::Result<()> {
        self.logger.ui(LogEvent::ui(
            TUI_02_SCOPE,
            EVENT_EXIT_REQUESTED,
            json!({ "run_mode": run_mode, "source": source }),
        ))
    }

    fn log_session_summary_created(&self) -> io::Result<()> {
        let Some(summary) = &self.state.epilogue_summary else {
            return Ok(());
        };
        self.logger.ui(LogEvent::ui(
            TUI_02_SCOPE,
            EVENT_SESSION_SUMMARY_CREATED,
            json!({
                "workspace": summary.workspace,
                "model": summary.model,
                "mode": summary.mode,
                "session": summary.session,
                "tools_executed": summary.tools_executed,
                "tools_failed": summary.tools_failed,
            }),
        ))
    }

    fn log_terminal_restored(&self) -> io::Result<()> {
        let Some(scope_id) = self.terminal_restore_scope else {
            return Ok(());
        };
        self.logger
            .ui(LogEvent::ui(scope_id, EVENT_TERMINAL_RESTORED, json!({})))
    }

    fn print_epilogue_after_restore(&self) -> io::Result<()> {
        let Some(summary) = &self.state.epilogue_summary else {
            return Ok(());
        };
        print_epilogue(summary)?;
        self.logger.ui(LogEvent::ui(
            TUI_02_SCOPE,
            EVENT_EPILOGUE_RENDERED,
            json!({ "surface": "stdout" }),
        ))
    }

    fn log_command_events(&self, events: &[CommandInputEvent]) -> io::Result<()> {
        let registry = CommandRegistry::new();

        for event in events {
            match event {
                CommandInputEvent::SurfaceOpened => {
                    self.logger.ui(LogEvent::ui(
                        TUI_04_SCOPE,
                        EVENT_COMMAND_SURFACE_OPENED,
                        json!({ "scene": self.state.scene.as_str() }),
                    ))?;
                }
                CommandInputEvent::FilterChanged { query } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_04_SCOPE,
                        EVENT_COMMAND_FILTER_CHANGED,
                        json!({ "query": query }),
                    ))?;
                }
                CommandInputEvent::CommandSelected { command } => {
                    let data = command_log_data(&registry, *command);
                    self.logger
                        .ui(LogEvent::ui(TUI_04_SCOPE, EVENT_COMMAND_SELECTED, data))?;
                }
                CommandInputEvent::ActionDispatched { command } => {
                    let data = command_log_data(&registry, *command);
                    self.logger.ui(LogEvent::ui(
                        TUI_04_SCOPE,
                        EVENT_COMMAND_ACTION_DISPATCHED,
                        data,
                    ))?;
                }
                CommandInputEvent::CommandAvailabilityChecked {
                    command,
                    allowed,
                    reason,
                } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_09_SCOPE,
                        EVENT_COMMAND_AVAILABILITY_CHECKED,
                        json!({
                            "command": command.as_str(),
                            "allowed": allowed,
                            "reason": reason,
                        }),
                    ))?;
                }
                CommandInputEvent::SteppedPickerOpened { command, step } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_09_SCOPE,
                        EVENT_STEPPED_PICKER_OPENED,
                        json!({ "command": command.as_str(), "step": step }),
                    ))?;
                }
                CommandInputEvent::SteppedPickerSelectionChanged { command, selected } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_09_SCOPE,
                        EVENT_STEPPED_PICKER_SELECTION_CHANGED,
                        json!({ "command": command.as_str(), "selected": selected }),
                    ))?;
                }
                CommandInputEvent::SteppedPickerConfirmed { command, selected } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_09_SCOPE,
                        EVENT_STEPPED_PICKER_CONFIRMED,
                        json!({ "command": command.as_str(), "selected": selected }),
                    ))?;
                }
            }
        }

        Ok(())
    }

    fn log_approval_events(&self, events: &[ApprovalInputEvent]) -> io::Result<()> {
        for event in events {
            match event {
                ApprovalInputEvent::SurfaceOpened => {
                    self.logger.ui(LogEvent::ui(
                        TUI_05_SCOPE,
                        EVENT_APPROVAL_SURFACE_OPENED,
                        json!({ "scene": self.state.scene.as_str() }),
                    ))?;
                }
                ApprovalInputEvent::OptionSelected { option } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_05_SCOPE,
                        EVENT_APPROVAL_OPTION_SELECTED,
                        json!({ "option": option.as_str() }),
                    ))?;
                }
                ApprovalInputEvent::ResultRecorded { result } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_05_SCOPE,
                        EVENT_APPROVAL_RESULT_RECORDED,
                        json!({ "result": result.as_str() }),
                    ))?;
                }
            }
        }

        Ok(())
    }

    fn log_working_process_events(&self, events: &[WorkingProcessEvent]) -> io::Result<()> {
        for event in events {
            match event {
                WorkingProcessEvent::Started => {
                    self.logger.ui(LogEvent::ui(
                        TUI_06_SCOPE,
                        EVENT_WORKING_PROCESS_STARTED,
                        json!({ "scene": self.state.scene.as_str() }),
                    ))?;
                }
                WorkingProcessEvent::PhaseChanged { phase } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_06_SCOPE,
                        EVENT_WORKING_PHASE_CHANGED,
                        json!({ "phase": phase.label(), "step": phase.number() }),
                    ))?;
                }
                WorkingProcessEvent::CancelHintRendered => {
                    self.logger.ui(LogEvent::ui(
                        TUI_06_SCOPE,
                        EVENT_WORKING_PROCESS_CANCEL_HINT_RENDERED,
                        json!({ "hint": "esc 취소" }),
                    ))?;
                }
                WorkingProcessEvent::Finished { reason } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_06_SCOPE,
                        EVENT_WORKING_PROCESS_FINISHED,
                        json!({ "reason": reason.as_str() }),
                    ))?;
                }
            }
        }

        Ok(())
    }

    fn log_workspace_events(&self, events: &[WorkspaceEvent]) -> io::Result<()> {
        for event in events {
            match event {
                WorkspaceEvent::PromptBlockAdded => {
                    self.logger.ui(LogEvent::ui(
                        TUI_07_SCOPE,
                        EVENT_WORKSPACE_PROMPT_BLOCK_ADDED,
                        json!({ "scene": self.state.scene.as_str() }),
                    ))?;
                }
                WorkspaceEvent::OutputAdded { item_type } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_07_SCOPE,
                        EVENT_WORKSPACE_OUTPUT_ADDED,
                        json!({ "item_type": item_type }),
                    ))?;
                }
                WorkspaceEvent::ScrollChanged { scroll } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_07_SCOPE,
                        EVENT_WORKSPACE_SCROLL_CHANGED,
                        json!({ "scroll": scroll }),
                    ))?;
                }
            }
        }

        Ok(())
    }

    fn log_workspace_render_if_pending(&mut self) -> io::Result<()> {
        let Some(rendered) = self.state.take_workspace_render_event() else {
            return Ok(());
        };

        self.log_workspace_rendered(rendered)
    }

    fn log_workspace_rendered(&self, rendered: WorkspaceRendered) -> io::Result<()> {
        self.logger.ui(LogEvent::ui(
            TUI_07_SCOPE,
            EVENT_WORKSPACE_RENDERED,
            json!({ "item_count": rendered.item_count, "scroll": rendered.scroll }),
        ))
    }

    fn log_persona_events(&self, events: &[PersonaEvent]) -> io::Result<()> {
        for event in events {
            match event {
                PersonaEvent::PanelOpened => {
                    self.logger.ui(LogEvent::ui(
                        TUI_08_SCOPE,
                        EVENT_PERSONA_PANEL_OPENED,
                        json!({ "scene": self.state.scene.as_str() }),
                    ))?;
                }
                PersonaEvent::PanelClosed => {
                    self.logger.ui(LogEvent::ui(
                        TUI_08_SCOPE,
                        EVENT_PERSONA_PANEL_CLOSED,
                        json!({ "scene": self.state.scene.as_str() }),
                    ))?;
                }
                PersonaEvent::WidthRejected { width, min_width } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_08_SCOPE,
                        EVENT_PERSONA_WIDTH_REJECTED,
                        json!({ "width": width, "min_width": min_width }),
                    ))?;
                }
            }
        }

        Ok(())
    }

    fn log_persona_render_if_pending(&mut self) -> io::Result<()> {
        let Some(rendered) = self.state.take_persona_render_event() else {
            return Ok(());
        };

        self.log_persona_message_rendered(rendered)
    }

    fn log_persona_message_rendered(&self, rendered: PersonaRendered) -> io::Result<()> {
        self.logger.ui(LogEvent::ui(
            TUI_08_SCOPE,
            EVENT_PERSONA_MESSAGE_RENDERED,
            json!({ "message_count": rendered.message_count }),
        ))
    }

    fn log_expanded_form_events(&self, events: &[ExpandedFormEvent]) -> io::Result<()> {
        for event in events {
            match event {
                ExpandedFormEvent::Opened { kind } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_10_SCOPE,
                        EVENT_EXPANDED_FORM_OPENED,
                        json!({ "kind": kind.as_str() }),
                    ))?;
                }
                ExpandedFormEvent::FieldChanged {
                    kind,
                    field,
                    masked,
                } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_10_SCOPE,
                        EVENT_EXPANDED_FORM_FIELD_CHANGED,
                        json!({
                            "kind": kind.as_str(),
                            "field": field,
                            "masked": masked,
                        }),
                    ))?;
                }
                ExpandedFormEvent::Submitted { kind } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_10_SCOPE,
                        EVENT_EXPANDED_FORM_SUBMITTED,
                        json!({ "kind": kind.as_str() }),
                    ))?;
                }
                ExpandedFormEvent::Cancelled { kind } => {
                    self.logger.ui(LogEvent::ui(
                        TUI_10_SCOPE,
                        EVENT_EXPANDED_FORM_CANCELLED,
                        json!({ "kind": kind.as_str() }),
                    ))?;
                }
            }
        }

        Ok(())
    }
}

fn command_log_data(
    registry: &CommandRegistry,
    command: super::command::CommandId,
) -> serde_json::Value {
    let Some(metadata) = registry.command(command) else {
        return json!({ "command": command.as_str() });
    };

    json!({
        "command": metadata.name,
        "group": metadata.group,
        "presentation": metadata.presentation.as_str(),
        "risk": metadata.risk.as_str(),
        "availability": metadata.availability,
    })
}

fn working_started(events: &WorkingProcessEvents) -> bool {
    events
        .events
        .iter()
        .any(|event| matches!(event, WorkingProcessEvent::Started))
}

fn working_cancelled(events: &WorkingProcessEvents) -> bool {
    events.events.iter().any(|event| {
        matches!(
            event,
            WorkingProcessEvent::Finished {
                reason: WorkingFinishReason::Canceled
            }
        )
    })
}

fn log_main_scene_rendered(logger: &Logger, run_mode: &str) -> io::Result<()> {
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_LAYOUT_CALCULATED,
        json!({ "run_mode": run_mode }),
    ))?;
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_PERSONA_LAYOUT_ABSENT,
        json!({ "persona": "off" }),
    ))?;
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_STATUSLINE_POSITIONED,
        json!({ "position": "bottom" }),
    ))?;
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_MAIN_SCENE_RENDERED,
        json!({ "run_mode": run_mode }),
    ))
}

fn load_runtime_config(logger: &Logger, project_root: &Path) -> io::Result<ConfigLoadOutcome> {
    let config_path = project_root.join(crate::config::CONFIG_RELATIVE_PATH);
    logger.llm(LogEvent::ui(
        LLM_01_SCOPE,
        EVENT_CONFIG_LOAD_STARTED,
        json!({ "path": config_path.display().to_string() }),
    ))?;

    match RuntimeConfig::load(project_root) {
        Ok(outcome) => {
            if matches!(
                outcome.source,
                ConfigLoadSource::DefaultCreated | ConfigLoadSource::DefaultApplied
            ) {
                logger.llm(LogEvent::ui(
                    LLM_01_SCOPE,
                    EVENT_CONFIG_DEFAULT_APPLIED,
                    json!({
                        "path": outcome.config.config_path.display().to_string(),
                        "created": outcome.warning.is_none(),
                    }),
                ))?;
            }
            if let Some(warning) = &outcome.warning {
                logger.llm(LogEvent::ui(
                    LLM_01_SCOPE,
                    EVENT_CONFIG_LOAD_FAILED,
                    json!({
                        "path": outcome.config.config_path.display().to_string(),
                        "recoverable": true,
                        "message": &warning.message,
                    }),
                ))?;
            }
            log_config_loaded(logger, &outcome.config, outcome.source)?;
            Ok(outcome)
        }
        Err(error) => {
            logger.llm(LogEvent::ui(
                LLM_01_SCOPE,
                EVENT_CONFIG_LOAD_FAILED,
                json!({
                    "path": config_path.display().to_string(),
                    "recoverable": true,
                    "message": error.message(),
                }),
            ))?;
            logger.llm(LogEvent::ui(
                LLM_01_SCOPE,
                EVENT_CONFIG_DEFAULT_APPLIED,
                json!({
                    "path": config_path.display().to_string(),
                    "created": false,
                    "reason": "load_failed",
                }),
            ))?;
            let config = RuntimeConfig::default_local(config_path);
            log_config_loaded(logger, &config, ConfigLoadSource::DefaultApplied)?;

            Ok(ConfigLoadOutcome {
                config,
                source: ConfigLoadSource::DefaultApplied,
                warning: Some(crate::config::ConfigWarning {
                    message: error.message(),
                }),
            })
        }
    }
}

fn log_config_loaded(
    logger: &Logger,
    config: &RuntimeConfig,
    source: ConfigLoadSource,
) -> io::Result<()> {
    logger.llm(LogEvent::ui(
        LLM_01_SCOPE,
        EVENT_CONFIG_LOADED,
        json!({
            "path": config.config_path.display().to_string(),
            "source": source.as_str(),
            "provider": &config.provider.active,
            "provider_type": config.provider.provider_type.as_str(),
            "base_url": &config.provider.base_url,
            "model": &config.provider.model,
            "context_tokens": config.provider.context_tokens,
            "api_key_env_configured": config.provider.api_key_env.is_some(),
            "workspace_root": &config.workspace.root,
            "mode": &config.mode.default,
            "persona_default": &config.persona.default,
            "persona_min_terminal_width": config.persona.min_terminal_width,
            "max_model_turns": config.limits.max_model_turns,
            "max_tool_calls": config.limits.max_tool_calls,
            "max_same_tool_repeats": config.limits.max_same_tool_repeats,
            "read_max_lines": config.limits.read_max_lines,
            "search_max_results": config.limits.search_max_results,
            "command_timeout_ms": config.limits.command_timeout_ms,
            "web_enabled": config.web.enabled,
        }),
    ))
}

fn current_workspace_path() -> io::Result<PathBuf> {
    std::env::current_dir()
}

fn workspace_display(path: &Path) -> String {
    path.display().to_string()
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    restored: bool,
}

impl TerminalSession {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        Ok(Self {
            terminal,
            restored: false,
        })
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }

        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
