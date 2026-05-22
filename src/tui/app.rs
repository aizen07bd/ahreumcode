use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::Terminal;
use serde_json::{json, Value};

use crate::cli::{AppCommand, RunMode, SceneCommand};
use crate::config::{ConfigLoadOutcome, ConfigLoadSource, RuntimeConfig};
use crate::llm::{
    attach_schema_prompt, parse_runtime_response, Activity, ChangePreview, ChatFailureKind,
    DecisionGate, LlmChatFailure, LlmChatReport, LlmChatStatus, LlmDiagnostics,
    LlmDiagnosticsRuntime, LlmDiagnosticsSnapshot, LlmDiagnosticsState, LlmHealthReport,
    LlmHealthStatus, LlmMessage, LlmMessageRole, LlmMessageVisibility, LlmProviderFactory,
    MessageHistory, ParsedRuntimeResponse, RepairLimitReached, RepairLoop, RepairRequest,
    RuntimeDecision, RuntimeDecisionError, RuntimeResponseParseError,
    RuntimeResponseParseErrorKind, SchemaPrompt, SchemaPromptBuilder,
};
use crate::logging::{LogEvent, Logger};
use crate::tool::{
    apply_approved_change, capture_change_precondition, redacted_tool_arguments,
    validate_approved_change, ApprovedChange, ChangePrecondition, ObservationStatus,
    PermissionDecision, PermissionDenial, PermissionGate, PermissionRequest,
    PostEditDiagnosticRequest, ToolCall, ToolErrorKind, ToolName, ToolObservation, ToolRuntime,
};

use super::approval::{ApprovalInputEvent, ApprovalRequest, ApprovalResult};
use super::command::{CommandDispatch, CommandInputEvent, CommandRegistry};
use super::event_log::{self, TUI_01_SCOPE, TUI_02_SCOPE, TUI_03_SCOPE};
use super::expanded_form::ExpandedFormEvent;
use super::persona::{PersonaEvent, PersonaRendered, PersonaRuntimeEvent};
use super::persona_runtime::{
    parse_persona_turn_result_for_turn, PersonaRuntime, PersonaRuntimeMode, PersonaTurn,
    PersonaTurnKind, PersonaTurnOutcome, PersonaTurnPrompt,
};
use super::runtime_request::{self, ActivePlainRequest};
use super::runtime_workspace;
use super::scenes::epilogue::print_epilogue;
use super::scenes::intro::{handle_intro_event, render_intro};
use super::scenes::main::{handle_main_event, render_main};
use super::state::{Scene, TuiState};
use super::working_process::{WorkingPhase, WorkingProcessEvent, WorkingProcessEvents};
use super::workspace::{ActivityGroup, WorkspaceEvent, WorkspaceEvents, WorkspaceRendered};

const LLM_01_SCOPE: &str = "llm-01-config-runtime";
const LLM_02_SCOPE: &str = "llm-02-provider-connection";
const LLM_03_SCOPE: &str = "llm-03-plain-prompt-request";
const LLM_04_SCOPE: &str = "llm-04-message-history";
const LLM_05_SCOPE: &str = "llm-05-schema-prompt-builder";
const LLM_06_SCOPE: &str = "llm-06-json-response-parser";
const LLM_07_SCOPE: &str = "llm-07-repair-request-loop";
const LLM_08_SCOPE: &str = "llm-08-runtime-decision-gate";
const LLM_09_SCOPE: &str = "llm-09-tui-process-binding";
const LLM_10_SCOPE: &str = "llm-10-diagnostics-and-status";
const TOOL_EXPLORE_SCOPE: &str = "explore-tool-runtime";
const TOOL_03_SCOPE: &str = "tool-03-tool-loop-binding";
const TOOL_04_SCOPE: &str = "tool-04-permission-branches";
const TOOL_06_SCOPE: &str = "tool-06-change-approval-execution";
const TOOL_07_SCOPE: &str = "tool-07-post-edit-diagnostics";
const TOOL_08_SCOPE: &str = "tool-08-command-execution-approval";
const TOOL_09_SCOPE: &str = "tool-09-web-network-runtime";
const TOOL_10_SCOPE: &str = "tool-10-external-path-policy";
const EVENT_APP_STARTED: &str = "app_started";
const EVENT_TERMINAL_ENTERED: &str = "terminal_entered";
const EVENT_INTRO_RENDERED: &str = "intro_rendered";
const EVENT_PROMPT_FOCUS_READY: &str = "prompt_focus_ready";
const EVENT_EXIT_REQUESTED: &str = "exit_requested";
const EVENT_SESSION_SUMMARY_CREATED: &str = "session_summary_created";
const EVENT_EPILOGUE_RENDERED: &str = "epilogue_rendered";
const EVENT_TERMINAL_RESTORED: &str = "terminal_restored";
const EVENT_WORKING_PROCESS_STARTED: &str = "working_process_started";
const EVENT_WORKING_PROCESS_PHASE_CHANGED: &str = "working_process_phase_changed";
const EVENT_WORKING_PROCESS_CANCELLED: &str = "working_process_cancelled";
const EVENT_WORKING_PROCESS_COMPLETED: &str = "working_process_completed";
const EVENT_LLM_DIAGNOSTICS_REQUESTED: &str = "llm_diagnostics_requested";
const EVENT_LLM_DIAGNOSTICS_RENDERED: &str = "llm_diagnostics_rendered";
const EVENT_LLM_STATUS_SNAPSHOT_RECORDED: &str = "llm_status_snapshot_recorded";
const EVENT_LLM_RUNTIME_READY_FOR_TOOL_STAGE: &str = "llm_runtime_ready_for_tool_stage";
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
const EVENT_MESSAGE_HISTORY_CREATED: &str = "message_history_created";
const EVENT_MESSAGE_RECORDED: &str = "message_recorded";
const EVENT_TURN_ID_ASSIGNED: &str = "turn_id_assigned";
const EVENT_HISTORY_WRITE_FAILED: &str = "history_write_failed";
const EVENT_SCHEMA_PROMPT_BUILT: &str = "schema_prompt_built";
const EVENT_SCHEMA_PROMPT_ATTACHED: &str = "schema_prompt_attached";
const EVENT_SCHEMA_PROMPT_BUILD_FAILED: &str = "schema_prompt_build_failed";
const EVENT_RAW_RESPONSE_RECEIVED: &str = "raw_response_received";
const EVENT_JSON_PARSE_SUCCEEDED: &str = "json_parse_succeeded";
const EVENT_JSON_PARSE_FAILED: &str = "json_parse_failed";
const EVENT_SCHEMA_VALIDATION_FAILED: &str = "schema_validation_failed";
const EVENT_REPAIR_REQUEST_STARTED: &str = "repair_request_started";
const EVENT_REPAIR_RESPONSE_RECEIVED: &str = "repair_response_received";
const EVENT_REPAIR_SUCCEEDED: &str = "repair_succeeded";
const EVENT_REPAIR_LIMIT_REACHED: &str = "repair_limit_reached";
const EVENT_RUNTIME_DECISION_STARTED: &str = "runtime_decision_started";
const EVENT_RUNTIME_DECISION_RECORDED: &str = "runtime_decision_recorded";
const EVENT_TOOL_CANDIDATE_CLASSIFIED: &str = "tool_candidate_classified";
const EVENT_RUNTIME_DECISION_FAILED: &str = "runtime_decision_failed";
const EVENT_TOOL_CALL_RECEIVED: &str = "tool_call_received";
const EVENT_TOOL_ARGUMENT_RESOLVED: &str = "tool_argument_resolved";
const EVENT_TOOL_PATH_BOUNDARY_CHECKED: &str = "tool_path_boundary_checked";
const EVENT_TOOL_EXECUTION_STARTED: &str = "tool_execution_started";
const EVENT_TOOL_EXECUTION_SUCCEEDED: &str = "tool_execution_succeeded";
const EVENT_TOOL_EXECUTION_FAILED: &str = "tool_execution_failed";
const EVENT_TOOL_OBSERVATION_RECORDED: &str = "tool_observation_recorded";
const EVENT_TOOL_WORKSPACE_SUMMARY_RENDERED: &str = "tool_workspace_summary_rendered";
const EVENT_TOOL_OBSERVATION_ATTACHED: &str = "tool_observation_attached";
const EVENT_TOOL_LOOP_REQUEST_STARTED: &str = "tool_loop_request_started";
const EVENT_TOOL_LOOP_DUPLICATE_REDIRECTED: &str = "tool_loop_duplicate_redirected";
const EVENT_TOOL_LOOP_LIMIT_REACHED: &str = "tool_loop_limit_reached";
const EVENT_TOOL_PERMISSION_EVALUATED: &str = "tool_permission_evaluated";
const EVENT_TOOL_PERMISSION_APPROVAL_OPENED: &str = "tool_permission_approval_opened";
const EVENT_TOOL_PERMISSION_DENIED: &str = "tool_permission_denied";

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
        project_root,
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
    let command_registry = CommandRegistry::new();
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
    terminal.draw(|frame| render_intro(frame, &state, &command_registry))?;
    logger.ui(LogEvent::ui(
        TUI_01_SCOPE,
        EVENT_INTRO_RENDERED,
        json!({ "run_mode": command.run_mode.as_str(), "backend": "test" }),
    ))?;

    println!("tui-01 intro smoke ok");
    println!("scene=intro");
    println!("run_mode={}", command.run_mode.as_str());
    println!("log_bucket={}", logger.log_bucket_dir().display());

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
        project_root,
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
    let command_registry = CommandRegistry::new();
    let state = TuiState::main(
        workspace,
        &config_outcome.config,
        config_outcome.source,
        config_outcome
            .warning
            .as_ref()
            .map(|warning| warning.message.as_str()),
    );

    terminal.draw(|frame| render_main(frame, &state, &command_registry))?;
    event_log::log_main_scene_rendered(&logger, command.run_mode.as_str())?;

    println!("tui-03 main smoke ok");
    println!("scene=main");
    println!("run_mode={}", command.run_mode.as_str());
    println!("log_bucket={}", logger.log_bucket_dir().display());

    Ok(())
}

fn run_epilogue_terminal(command: AppCommand) -> io::Result<()> {
    let project_root = current_workspace_path()?;
    let workspace = workspace_display(&project_root);
    let logger = Logger::start()?;
    let config_outcome = load_runtime_config(&logger, &project_root)?;
    let app = TuiApp::new_epilogue(
        logger,
        project_root,
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
        project_root,
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
    println!("log_bucket={}", app.logger.log_bucket_dir().display());

    Ok(())
}

struct TuiApp {
    state: TuiState,
    logger: Logger,
    runtime_config: RuntimeConfig,
    tool_runtime: ToolRuntime,
    command_registry: CommandRegistry,
    llm_diagnostics: LlmDiagnosticsState,
    active_plain_request: Option<ActivePlainRequest>,
    pending_change_approval: Option<PendingChangeApproval>,
    pending_command_approval: Option<PendingCommandApproval>,
    pending_web_approval: Option<PendingWebApproval>,
    active_persona_request: Option<ActivePersonaRequest>,
    persona_runtime: PersonaRuntime,
    previous_task_frame: Option<ConversationTaskFrame>,
    next_run_index: u64,
    run_mode: &'static str,
    intro_render_logged: bool,
    main_render_logged: bool,
    terminal_restore_scope: Option<&'static str>,
}

struct ActivePersonaRequest {
    receiver: Receiver<LlmChatReport>,
    prompt: String,
    turn: PersonaTurn,
    start_plain_after: bool,
    persona_context_start: usize,
}

struct PendingChangeApproval {
    active: ActivePlainRequest,
    tool_name: String,
    arguments: Value,
    signature: String,
    preview: ChangePreview,
    precondition: ChangePrecondition,
}

struct PendingCommandApproval {
    active: ActivePlainRequest,
    tool_name: String,
    arguments: Value,
}

struct PendingWebApproval {
    active: ActivePlainRequest,
    tool_name: String,
    arguments: Value,
}

enum PendingChangePreparation {
    NotChange,
    Pending {
        tool_name: String,
        arguments: Value,
        signature: String,
        preview: ChangePreview,
        precondition: ChangePrecondition,
    },
    Failed {
        tool_name: String,
        arguments: Value,
        signature: String,
        observation: ToolObservation,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConversationTaskFrame {
    user_prompt: String,
    task_state_summary: String,
    runtime_context_summary: String,
}

impl TuiApp {
    fn new(
        logger: Logger,
        project_root: PathBuf,
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
        run_mode: &'static str,
    ) -> Self {
        let artifact_root = logger.log_bucket_dir().join("artifacts/tool");
        Self {
            state: TuiState::intro(workspace, config, config_source, config_warning),
            logger,
            runtime_config: config.clone(),
            tool_runtime: ToolRuntime::new(project_root, artifact_root),
            command_registry: CommandRegistry::new(),
            llm_diagnostics: LlmDiagnosticsState::default(),
            active_plain_request: None,
            pending_change_approval: None,
            pending_command_approval: None,
            pending_web_approval: None,
            active_persona_request: None,
            persona_runtime: PersonaRuntime::new(),
            previous_task_frame: None,
            next_run_index: 1,
            run_mode,
            intro_render_logged: false,
            main_render_logged: false,
            terminal_restore_scope: None,
        }
    }

    fn new_main(
        logger: Logger,
        project_root: PathBuf,
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
        run_mode: &'static str,
    ) -> Self {
        let artifact_root = logger.log_bucket_dir().join("artifacts/tool");
        Self {
            state: TuiState::main(workspace, config, config_source, config_warning),
            logger,
            runtime_config: config.clone(),
            tool_runtime: ToolRuntime::new(project_root, artifact_root),
            command_registry: CommandRegistry::new(),
            llm_diagnostics: LlmDiagnosticsState::default(),
            active_plain_request: None,
            pending_change_approval: None,
            pending_command_approval: None,
            pending_web_approval: None,
            active_persona_request: None,
            persona_runtime: PersonaRuntime::new(),
            previous_task_frame: None,
            next_run_index: 1,
            run_mode,
            intro_render_logged: true,
            main_render_logged: false,
            terminal_restore_scope: None,
        }
    }

    fn new_epilogue(
        logger: Logger,
        project_root: PathBuf,
        workspace: String,
        config: &RuntimeConfig,
        config_source: ConfigLoadSource,
        config_warning: Option<&str>,
        run_mode: &'static str,
    ) -> Self {
        let artifact_root = logger.log_bucket_dir().join("artifacts/tool");
        Self {
            state: TuiState::epilogue(workspace, config, config_source, config_warning),
            logger,
            runtime_config: config.clone(),
            tool_runtime: ToolRuntime::new(project_root, artifact_root),
            command_registry: CommandRegistry::new(),
            llm_diagnostics: LlmDiagnosticsState::default(),
            active_plain_request: None,
            pending_change_approval: None,
            pending_command_approval: None,
            pending_web_approval: None,
            active_persona_request: None,
            persona_runtime: PersonaRuntime::new(),
            previous_task_frame: None,
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
            self.poll_persona_request()?;
            self.poll_plain_prompt_request()?;

            if matches!(self.state.scene, Scene::Main) {
                let runtime_outcome = self.state.tick_working_process();
                self.log_working_process_events(&runtime_outcome.working_process_events.events)?;
                self.log_workspace_events(&runtime_outcome.workspace_events.events)?;
            }

            terminal.draw(|frame| match self.state.scene {
                Scene::Intro => render_intro(frame, &self.state, &self.command_registry),
                Scene::Main => render_main(frame, &self.state, &self.command_registry),
                Scene::Epilogue => {}
            })?;

            if !self.intro_render_logged {
                self.logger
                    .ui(LogEvent::ui(TUI_01_SCOPE, EVENT_INTRO_RENDERED, json!({})))?;
                self.intro_render_logged = true;
            }
            if matches!(self.state.scene, Scene::Main) && !self.main_render_logged {
                event_log::log_main_scene_rendered(&self.logger, self.run_mode)?;
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
                        let action =
                            handle_intro_event(key_event, &mut self.state, &self.command_registry);
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
                        let action =
                            handle_main_event(key_event, &mut self.state, &self.command_registry);
                        self.log_command_events(&action.command_outcome.events)?;
                        self.log_approval_events(&action.approval_outcome.events)?;
                        self.log_working_process_events(&action.working_process_events.events)?;
                        self.log_workspace_events(&action.workspace_events.events)?;
                        self.log_persona_events(&action.persona_events.events)?;
                        self.log_expanded_form_events(&action.expanded_form_events.events)?;
                        self.handle_approval_runtime_outcome(&action.approval_outcome.events)?;
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
            CommandDispatch::StatusShell => self.render_llm_diagnostics(),
            CommandDispatch::HealthCheck => self.run_health_check(),
            _ => Ok(()),
        }
    }

    fn render_llm_diagnostics(&mut self) -> io::Result<()> {
        self.log_llm_diagnostics_requested("status")?;
        let snapshot = self.llm_diagnostics_snapshot();
        self.log_llm_status_snapshot_recorded(&snapshot)?;
        self.log_llm_runtime_ready_for_tool_stage(&snapshot)?;
        let events = runtime_workspace::record_llm_diagnostics(&mut self.state, &snapshot);
        self.log_workspace_events(&events.events)?;
        self.log_llm_diagnostics_rendered(&snapshot)
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
        self.llm_diagnostics.record_health(&report);

        let events = runtime_workspace::record_health_report(&mut self.state, &report);
        self.log_workspace_events(&events.events)?;
        let snapshot = self.llm_diagnostics_snapshot();
        self.log_llm_status_snapshot_recorded(&snapshot)
    }

    fn llm_diagnostics_snapshot(&self) -> LlmDiagnosticsSnapshot {
        LlmDiagnostics::snapshot(
            LlmDiagnosticsRuntime {
                provider: &self.runtime_config.provider.active,
                model: &self.runtime_config.provider.model,
                base_url: &self.runtime_config.provider.base_url,
                context_tokens: self.runtime_config.provider.context_tokens,
                mode: &self.state.runtime_status.mode,
                web: self.state.runtime_status.web,
            },
            &self.llm_diagnostics,
        )
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

    fn log_llm_diagnostics_requested(&self, source: &str) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_10_SCOPE,
            EVENT_LLM_DIAGNOSTICS_REQUESTED,
            json!({
                "source": source,
                "provider": &self.runtime_config.provider.active,
                "model": &self.runtime_config.provider.model,
            }),
        ))
    }

    fn log_llm_diagnostics_rendered(&self, snapshot: &LlmDiagnosticsSnapshot) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_10_SCOPE,
            EVENT_LLM_DIAGNOSTICS_RENDERED,
            json!({
                "line_count": snapshot.lines().len(),
                "last_request": &snapshot.last_request,
                "last_parse": &snapshot.last_parse,
                "last_decision": &snapshot.last_decision,
            }),
        ))
    }

    fn log_llm_status_snapshot_recorded(
        &self,
        snapshot: &LlmDiagnosticsSnapshot,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_10_SCOPE,
            EVENT_LLM_STATUS_SNAPSHOT_RECORDED,
            json!({
                "provider": &snapshot.provider,
                "model": &snapshot.model,
                "base_url": &snapshot.base_url,
                "context_tokens": snapshot.context_tokens,
                "mode": &snapshot.mode,
                "web": &snapshot.web,
                "last_health": &snapshot.last_health,
                "last_request": &snapshot.last_request,
                "last_parse": &snapshot.last_parse,
                "last_repair": &snapshot.last_repair,
                "last_decision": &snapshot.last_decision,
                "last_failure": &snapshot.last_failure,
            }),
        ))
    }

    fn log_llm_runtime_ready_for_tool_stage(
        &self,
        snapshot: &LlmDiagnosticsSnapshot,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_10_SCOPE,
            EVENT_LLM_RUNTIME_READY_FOR_TOOL_STAGE,
            json!({
                "ready": snapshot.tool_stage_ready,
                "reason": &snapshot.tool_stage_reason,
                "e2e_verified": false,
            }),
        ))
    }

    fn handle_plain_prompt_events(&mut self, events: &WorkingProcessEvents) -> io::Result<()> {
        if event_log::working_started(&events.events) {
            if let Some(prompt) = self.state.pending_prompt.clone() {
                let persona_context_start = self.state.persona.message_count();
                self.active_persona_request = None;
                if !self.start_persona_kickoff_request(prompt.clone(), true, persona_context_start)
                {
                    self.start_plain_prompt_request(prompt, persona_context_start)?;
                }
            }
        }

        if event_log::working_cancelled(&events.events) {
            self.cancel_active_persona_request();
            self.cancel_active_plain_request()?;
        }

        Ok(())
    }

    fn start_persona_kickoff_request(
        &mut self,
        prompt: String,
        start_plain_after: bool,
        persona_context_start: usize,
    ) -> bool {
        if !self.state.persona_panel.is_full() || self.active_persona_request.is_some() {
            self.persona_runtime
                .start_kickoff_for_mode(PersonaRuntimeMode::Off, prompt);
            return false;
        }

        let turn = if let Some(frame) = self.previous_task_frame.as_ref() {
            self.persona_runtime.start_follow_up_for_mode(
                PersonaRuntimeMode::Full,
                prompt.clone(),
                persona_follow_up_task_state_summary(frame, &prompt),
            )
        } else {
            self.persona_runtime
                .start_kickoff_for_mode(PersonaRuntimeMode::Full, prompt.clone())
        };

        if turn.is_none() {
            return false;
        }

        self.start_next_persona_turn_request(prompt, start_plain_after, persona_context_start)
    }

    fn enqueue_persona_completion_request(&mut self, task_state_summary: String) -> bool {
        if !self.state.persona_panel.is_full() {
            return false;
        }

        if let Some(active) = self.active_persona_request.as_ref() {
            if active.turn.kind == PersonaTurnKind::Completion
                && self.persona_runtime.absorb_completion_into_active_closure(
                    Some(&active.turn),
                    task_state_summary.clone(),
                )
            {
                return true;
            }
        }

        if self
            .persona_runtime
            .enqueue_completion_for_mode(PersonaRuntimeMode::Full, task_state_summary)
            .is_none()
        {
            return false;
        }

        if let Some(active) = self.active_persona_request.as_ref() {
            if active.turn.kind != PersonaTurnKind::Completion {
                self.active_persona_request = None;
            } else {
                return true;
            }
        }

        let Some(prompt) = self
            .persona_runtime
            .current_task()
            .map(|task| task.user_prompt.clone())
        else {
            return false;
        };
        let persona_context_start = self.state.persona.message_count();
        self.start_next_persona_turn_request(prompt, false, persona_context_start)
    }

    fn enqueue_persona_progress_request(&mut self, task_state_summary: String) -> bool {
        if !self.state.persona_panel.is_full() {
            return false;
        }

        if self
            .persona_runtime
            .enqueue_progress_for_mode(PersonaRuntimeMode::Full, task_state_summary)
            .is_none()
        {
            return false;
        }

        if self.active_persona_request.is_some() {
            return true;
        }

        let Some(prompt) = self
            .persona_runtime
            .current_task()
            .map(|task| task.user_prompt.clone())
        else {
            return false;
        };
        let persona_context_start = self.state.persona.message_count();
        self.start_next_persona_turn_request(prompt, false, persona_context_start)
    }

    fn start_next_persona_turn_request(
        &mut self,
        prompt: String,
        start_plain_after: bool,
        persona_context_start: usize,
    ) -> bool {
        if !self.state.persona_panel.is_full() || self.active_persona_request.is_some() {
            return false;
        }

        let Some(turn) = self.persona_runtime.pop_next_turn() else {
            return false;
        };
        let history = self
            .persona_runtime
            .session_history(turn.speaker)
            .map(|messages| messages.to_vec())
            .unwrap_or_default();
        let prompt_bundle = self.persona_runtime.build_turn_prompt(&turn, &history);
        let messages = persona_turn_prompt_messages(prompt_bundle);
        let receiver = runtime_request::spawn_chat_request(&self.runtime_config, messages);
        self.active_persona_request = Some(ActivePersonaRequest {
            receiver,
            prompt,
            turn,
            start_plain_after,
            persona_context_start,
        });
        true
    }

    fn poll_persona_request(&mut self) -> io::Result<()> {
        let Some(active) = self.active_persona_request.as_ref() else {
            return Ok(());
        };

        let report = match active.receiver.try_recv() {
            Ok(report) => Some(report),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.active_persona_request = None;
                None
            }
        };

        let Some(report) = report else {
            return Ok(());
        };

        let Some(active) = self.active_persona_request.take() else {
            return Ok(());
        };

        let outcome = match report.status {
            LlmChatStatus::Succeeded { answer } => {
                parse_persona_turn_result_for_turn(&answer, &active.turn).unwrap_or_else(|_| {
                    PersonaTurnOutcome::Passed {
                        speaker: active.turn.speaker,
                    }
                    .into()
                })
            }
            LlmChatStatus::Failed(_) => PersonaTurnOutcome::Passed {
                speaker: active.turn.speaker,
            }
            .into(),
        };

        let stale_after_completion = self.persona_runtime.is_task_completed()
            && active.turn.kind != PersonaTurnKind::Completion;
        if !stale_after_completion {
            if let PersonaTurnOutcome::Spoken(message) = &outcome.outcome {
                self.state.record_persona_message(message.clone());
            }
        }
        self.persona_runtime
            .record_turn_result(&active.turn, outcome);

        if self.active_persona_request.is_none()
            && self.start_next_persona_turn_request(
                active.prompt.clone(),
                active.start_plain_after,
                active.persona_context_start,
            )
        {
            return Ok(());
        }

        if active.start_plain_after && self.active_plain_request.is_none() {
            self.start_plain_prompt_request(active.prompt, active.persona_context_start)?;
        }

        Ok(())
    }

    fn cancel_active_persona_request(&mut self) {
        self.active_persona_request = None;
        self.persona_runtime.clear();
    }

    fn start_plain_prompt_request(
        &mut self,
        prompt: String,
        persona_context_start: usize,
    ) -> io::Result<()> {
        if self.active_plain_request.is_some() {
            return Ok(());
        }

        let run_id = runtime_request::next_run_id(&mut self.next_run_index);
        let mut history = MessageHistory::new(run_id.clone());
        self.log_message_history_created(&history)?;

        let turn_id = history.next_turn_id();
        self.log_turn_id_assigned(&history, &turn_id)?;

        let schema_prompt = self.build_schema_prompt()?;
        let schema_message = attach_schema_prompt(&mut history, turn_id.clone(), &schema_prompt);
        self.log_schema_prompt_attached(&history, &schema_message, &schema_prompt)?;
        self.log_message_recorded(&history, &schema_message)?;

        if let Some(frame) = self.previous_task_frame.as_ref() {
            let context_message = history.append(
                turn_id.clone(),
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                conversation_context_message(frame),
            );
            self.log_message_recorded(&history, &context_message)?;
        }

        let user_message = history.append(
            turn_id.clone(),
            LlmMessageRole::User,
            LlmMessageVisibility::UserVisible,
            prompt.clone(),
        );
        self.log_message_recorded(&history, &user_message)?;

        let request_messages = history.for_request(None);
        self.log_plain_request_started(&run_id, &turn_id, &prompt)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestStarted);
        self.llm_diagnostics
            .record_request_started(&run_id, &turn_id);
        self.log_runtime_process_started(&run_id, &turn_id)?;
        self.set_runtime_working_phase(WorkingPhase::Interpret, "로컬 LLM 응답을 기다립니다.")?;

        let receiver = runtime_request::spawn_chat_request(&self.runtime_config, request_messages);

        self.active_plain_request = Some(ActivePlainRequest::new(
            run_id,
            turn_id,
            prompt,
            schema_message,
            user_message,
            history,
            receiver,
            persona_context_start,
        ));

        Ok(())
    }

    fn poll_plain_prompt_request(&mut self) -> io::Result<()> {
        let Some(active) = self.active_plain_request.as_ref() else {
            return Ok(());
        };

        let request_timeout = runtime_request::effective_chat_timeout(&self.runtime_config);
        if active.request_timed_out(request_timeout) {
            let Some(mut active) = self.active_plain_request.take() else {
                return Ok(());
            };
            if let Some(final_decision) =
                runtime_request::completed_tool_fallback_final_decision(&active, "timeout")
            {
                active.record_final_decision(&final_decision);
                let events =
                    runtime_workspace::record_runtime_decision(&mut self.state, &final_decision);
                self.finish_plain_request_with_events(
                    events,
                    "성공한 도구 observation으로 요청 시간 초과를 종료합니다.",
                    Some((&active.run_id, &active.turn_id)),
                    active.persona_context_start,
                    persona_task_state_summary(
                        &active,
                        "completed",
                        "tool observation succeeded before model follow-up timed out",
                    ),
                    Some(runtime_request::conversation_task_context_summary(&active)),
                )?;
                return Ok(());
            }
            let report = plain_request_timeout_report(&self.runtime_config, &active);
            self.llm_diagnostics.record_request_report(&report);
            let failure_message = active.history.append(
                active.turn_id.clone(),
                LlmMessageRole::System,
                LlmMessageVisibility::Internal,
                "request_failed:timeout",
            );
            self.log_message_recorded(&active.history, &failure_message)?;
            self.log_plain_request_failed(&active, &report)?;
            self.state
                .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestFailed);
            self.set_runtime_working_phase(
                WorkingPhase::Execute,
                "요청 시간이 초과되어 실행하지 않습니다.",
            )?;
            self.set_runtime_working_phase(
                WorkingPhase::Apply,
                "요청 시간 초과를 workspace에 반영합니다.",
            )?;
            let events = runtime_workspace::record_plain_chat_failure(&mut self.state, &report);
            self.finish_plain_request_with_events(
                events,
                "요청 시간 초과를 보고합니다.",
                Some((&active.run_id, &active.turn_id)),
                active.persona_context_start,
                persona_task_state_summary(&active, "request_failed", "timeout"),
                Some(runtime_request::conversation_task_context_summary(&active)),
            )?;
            return Ok(());
        }

        let result = match active.receiver.try_recv() {
            Ok(report) => Some(report),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                let Some(mut active) = self.active_plain_request.take() else {
                    return Ok(());
                };
                let failure_message = active.history.append(
                    active.turn_id.clone(),
                    LlmMessageRole::System,
                    LlmMessageVisibility::Internal,
                    "request_failed:runtime_channel_disconnected",
                );
                self.log_message_recorded(&active.history, &failure_message)?;
                self.log_plain_request_failed_channel(&active)?;
                self.state
                    .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestFailed);
                let mut events = self
                    .state
                    .record_system_notice("request failed: runtime_channel_disconnected");
                events.extend(
                    self.state
                        .record_system_notice("message local request worker ended unexpectedly"),
                );
                self.set_runtime_working_phase(
                    WorkingPhase::Execute,
                    "요청 채널이 끊겨 실행하지 않습니다.",
                )?;
                self.set_runtime_working_phase(
                    WorkingPhase::Apply,
                    "요청 채널 실패를 workspace에 반영합니다.",
                )?;
                self.finish_plain_request_with_events(
                    events,
                    "요청 실패 원인을 보고합니다.",
                    Some((&active.run_id, &active.turn_id)),
                    active.persona_context_start,
                    persona_task_state_summary(
                        &active,
                        "request_failed",
                        "runtime channel disconnected",
                    ),
                    Some(runtime_request::conversation_task_context_summary(&active)),
                )?;
                return Ok(());
            }
        };

        let Some(report) = result else {
            return Ok(());
        };

        let Some(mut active) = self.active_plain_request.take() else {
            return Ok(());
        };

        if active.cancelled {
            return Ok(());
        }

        let events = match &report.status {
            LlmChatStatus::Succeeded { answer } => {
                self.llm_diagnostics.record_request_report(&report);
                let assistant_message = active.history.append(
                    active.turn_id.clone(),
                    LlmMessageRole::Assistant,
                    LlmMessageVisibility::UserVisible,
                    answer.clone(),
                );
                self.log_message_recorded(&active.history, &assistant_message)?;
                self.log_plain_response_received(&active, &report)?;
                self.state
                    .record_persona_runtime_event(PersonaRuntimeEvent::LlmResponseReceived);
                self.log_raw_response_received(&active, answer)?;
                self.set_runtime_working_phase(
                    WorkingPhase::Classify,
                    "모델 응답 형식을 분류합니다.",
                )?;
                if active.repair_attempts > 0 {
                    self.log_repair_response_received(&active, answer)?;
                }
                match parse_runtime_response(answer) {
                    Ok(parsed) => {
                        self.log_runtime_response_parsed(&active, &parsed)?;
                        self.state.record_persona_runtime_event(
                            PersonaRuntimeEvent::RuntimeResponseParsed,
                        );
                        self.enqueue_persona_progress_request(persona_task_state_summary(
                            &active,
                            "model_response_parsed",
                            "main runtime parsed a structured response and is validating the next action",
                        ));
                        self.llm_diagnostics.record_parse_success(
                            parsed.response.response_type(),
                            parsed.response.activity().as_str(),
                            parsed.payloads.len(),
                        );
                        if active.repair_attempts > 0 {
                            self.log_repair_succeeded(&active, &parsed)?;
                            self.state
                                .record_persona_runtime_event(PersonaRuntimeEvent::RepairSucceeded);
                            self.llm_diagnostics.record_repair_succeeded(
                                active.repair_attempts,
                                RepairLoop::default_local().max_attempts(),
                            );
                        }
                        self.set_runtime_working_phase(
                            WorkingPhase::Validate,
                            "응답 후보를 runtime decision으로 검증합니다.",
                        )?;
                        self.log_runtime_decision_started(&active, &parsed)?;
                        match DecisionGate::classify(&parsed) {
                            Ok(decision) => {
                                let decision =
                                    runtime_request::apply_final_answer_evidence_boundary(
                                        &active, decision,
                                    );
                                if let Some(target) = runtime_request::change_target_requiring_read(
                                    &active, &decision,
                                ) {
                                    active.pending_change_after_read_target = Some(target);
                                }
                                let decision = runtime_request::apply_change_evidence_boundary(
                                    &active, decision,
                                );
                                self.log_runtime_decision_recorded(&active, &decision)?;
                                self.llm_diagnostics.record_decision(
                                    decision.kind(),
                                    decision.activity().map(|activity| activity.as_str()),
                                    decision.tool_name(),
                                );
                                if let RuntimeDecision::ApprovalNeeded {
                                    tool_name: _,
                                    arguments: _,
                                    ..
                                } = &decision
                                {
                                    if let Some(final_decision) =
                                        runtime_request::repeated_completed_add_change_final_decision(
                                            &active, &decision,
                                        )
                                    {
                                        active.record_final_decision(&final_decision);
                                        let events = runtime_workspace::record_runtime_decision(
                                            &mut self.state,
                                            &final_decision,
                                        );
                                        self.finish_plain_request_with_events(
                                            events,
                                            "기존 observation으로 반복 파일 추가 후보를 종료합니다.",
                                            Some((&active.run_id, &active.turn_id)),
                                            active.persona_context_start,
                                            persona_task_state_summary(
                                                &active,
                                                "completed",
                                                "repeated add-file change candidate finalized from existing observation",
                                            ),
                                            Some(
                                                runtime_request::conversation_task_context_summary(
                                                    &active,
                                                ),
                                            ),
                                        )?;
                                        return Ok(());
                                    }
                                    let signature = approval_signature(&decision);
                                    if let Some((redirect, execution_record)) =
                                        runtime_request::duplicate_approval_repeat_redirect(
                                            &active, &signature,
                                        )
                                    {
                                        let duplicate_redirect_count = active
                                            .duplicate_redirect_count_for_signature(&signature);
                                        if should_finalize_duplicate_approval_redirect(
                                            redirect,
                                            duplicate_redirect_count,
                                            self.runtime_config.limits.max_same_tool_repeats,
                                        ) {
                                            let final_decision =
                                                runtime_request::duplicate_approval_final_decision(
                                                    &active, &signature,
                                                )
                                                .expect(
                                                    "duplicate approval should have a final decision",
                                                );
                                            active.record_final_decision(&final_decision);
                                            let events = runtime_workspace::record_runtime_decision(
                                                &mut self.state,
                                                &final_decision,
                                            );
                                            self.finish_plain_request_with_events(
                                                events,
                                                "반복 승인 후보를 기존 observation으로 종료합니다.",
                                                Some((&active.run_id, &active.turn_id)),
                                                active.persona_context_start,
                                                persona_task_state_summary(
                                                    &active,
                                                    "completed",
                                                    "duplicate approval candidate finalized after retry guidance",
                                                ),
                                                Some(
                                                    runtime_request::conversation_task_context_summary(
                                                        &active,
                                                    ),
                                                ),
                                            )?;
                                            return Ok(());
                                        }

                                        active.last_tool_signature = Some(signature.clone());
                                        active.duplicate_redirect_count =
                                            duplicate_redirect_count.saturating_add(1);
                                        let next_turn_id = active.history.next_turn_id();
                                        self.log_turn_id_assigned(&active.history, &next_turn_id)?;
                                        let request_messages = match redirect {
                                            runtime_request::ToolLoopRepeatRedirect::SettledDuplicate => {
                                                runtime_request::tool_repeat_answer_request_messages(
                                                    &active.schema_message,
                                                    &active.user_message,
                                                    &execution_record,
                                                    &active.executed_tool_records,
                                                    &active.executed_tool_signatures,
                                                    &next_turn_id,
                                                )
                                            }
                                            runtime_request::ToolLoopRepeatRedirect::TruncatedContinuation => {
                                                runtime_request::tool_repeat_continuation_request_messages(
                                                    &active.schema_message,
                                                    &active.user_message,
                                                    &execution_record,
                                                    &active.executed_tool_records,
                                                    &active.executed_tool_signatures,
                                                    &next_turn_id,
                                                )
                                            }
                                            runtime_request::ToolLoopRepeatRedirect::FailedDuplicate => {
                                                runtime_request::tool_repeat_failure_request_messages(
                                                    &active.schema_message,
                                                    &active.user_message,
                                                    &execution_record,
                                                    &active.executed_tool_records,
                                                    &active.executed_tool_signatures,
                                                    &next_turn_id,
                                                )
                                            }
                                        };
                                        self.log_plain_request_started(
                                            &active.run_id,
                                            &next_turn_id,
                                            &active.prompt,
                                        )?;
                                        self.state.record_persona_runtime_event(
                                            PersonaRuntimeEvent::LlmRequestStarted,
                                        );
                                        self.log_tool_loop_duplicate_redirected(
                                            &active,
                                            &next_turn_id,
                                            &signature,
                                            redirect.as_str(),
                                        )?;
                                        self.state.record_persona_runtime_event(
                                            PersonaRuntimeEvent::ToolLoopDuplicateRedirected,
                                        );
                                        self.llm_diagnostics
                                            .record_request_started(&active.run_id, &next_turn_id);
                                        self.set_runtime_working_phase(
                                            WorkingPhase::Interpret,
                                            "반복 승인 후보를 재승인하지 않고 기존 observation을 반영한 다음 응답을 요청합니다.",
                                        )?;

                                        active.turn_id = next_turn_id;
                                        active.receiver = runtime_request::spawn_chat_request(
                                            &self.runtime_config,
                                            request_messages,
                                        );
                                        active.reset_request_timer();
                                        active.reset_repair_state();
                                        self.active_plain_request = Some(active);
                                        return Ok(());
                                    }
                                    if let Some(final_decision) =
                                        runtime_request::duplicate_approval_final_decision(
                                            &active, &signature,
                                        )
                                    {
                                        active.record_final_decision(&final_decision);
                                        let events = runtime_workspace::record_runtime_decision(
                                            &mut self.state,
                                            &final_decision,
                                        );
                                        self.finish_plain_request_with_events(
                                            events,
                                            "기존 observation으로 반복 승인 후보를 종료합니다.",
                                            Some((&active.run_id, &active.turn_id)),
                                            active.persona_context_start,
                                            persona_task_state_summary(
                                                &active,
                                                "completed",
                                                "duplicate approval candidate finalized from existing observation",
                                            ),
                                            Some(
                                                runtime_request::conversation_task_context_summary(
                                                    &active,
                                                ),
                                            ),
                                        )?;
                                        return Ok(());
                                    }
                                }
                                if decision.tool_name().is_some() {
                                    self.log_tool_candidate_classified(&active, &decision)?;
                                    self.state.record_persona_runtime_event(
                                        PersonaRuntimeEvent::ToolCandidateClassified,
                                    );
                                }
                                self.set_runtime_working_phase(
                                    WorkingPhase::Execute,
                                    runtime_request::runtime_execute_detail(&decision),
                                )?;
                                let permission =
                                    PermissionGate::evaluate(&self.runtime_config, &decision);
                                self.log_tool_permission_evaluated(
                                    &active,
                                    &decision,
                                    &permission,
                                )?;
                                match permission {
                                    PermissionDecision::Allow => {
                                        if decision.tool_name().is_some() {
                                            self.state.record_persona_runtime_event(
                                                PersonaRuntimeEvent::ToolPermissionAllowed,
                                            );
                                        }
                                        if matches!(
                                            decision,
                                            RuntimeDecision::ToolCandidatePending { .. }
                                        ) {
                                            self.set_runtime_working_phase(
                                                WorkingPhase::Apply,
                                                "도구 observation을 workspace와 history에 반영합니다.",
                                            )?;
                                            self.handle_tool_decision_loop(active, &decision)?;
                                            return Ok(());
                                        }

                                        self.set_runtime_working_phase(
                                            WorkingPhase::Apply,
                                            "결정 결과를 workspace에 반영합니다.",
                                        )?;
                                        active.record_final_decision(&decision);
                                        runtime_workspace::record_runtime_decision(
                                            &mut self.state,
                                            &decision,
                                        )
                                    }
                                    PermissionDecision::Ask(request) => {
                                        self.state.record_persona_runtime_event(
                                            PersonaRuntimeEvent::ToolPermissionApprovalNeeded,
                                        );
                                        self.set_runtime_working_phase(
                                            WorkingPhase::Apply,
                                            "승인 요청을 workspace와 approval surface에 반영합니다.",
                                        )?;
                                        let change_preview = decision_change_preview(&decision);
                                        let pending_change = self.prepare_pending_change_approval(
                                            &active,
                                            &decision,
                                            change_preview.as_ref(),
                                        )?;
                                        if let PendingChangePreparation::Failed {
                                            tool_name,
                                            arguments,
                                            signature,
                                            observation,
                                        } = pending_change
                                        {
                                            self.handle_change_observation_after_approval(
                                                active,
                                                tool_name,
                                                arguments,
                                                signature,
                                                observation,
                                                None,
                                                "change_preview_precondition_failed",
                                            )?;
                                            return Ok(());
                                        }
                                        let events = self.open_permission_approval(
                                            &active,
                                            request,
                                            change_preview.clone(),
                                        )?;
                                        if let PendingChangePreparation::Pending {
                                            tool_name,
                                            arguments,
                                            signature,
                                            preview,
                                            precondition,
                                        } = pending_change
                                        {
                                            self.pending_change_approval =
                                                Some(PendingChangeApproval {
                                                    active,
                                                    tool_name,
                                                    arguments,
                                                    signature,
                                                    preview,
                                                    precondition,
                                                });
                                            self.log_workspace_events(&events.events)?;
                                            return Ok(());
                                        }
                                        if let Some((tool_name, arguments)) =
                                            command_approval_parts(&decision)
                                        {
                                            self.pending_command_approval =
                                                Some(PendingCommandApproval {
                                                    active,
                                                    tool_name,
                                                    arguments,
                                                });
                                            self.log_workspace_events(&events.events)?;
                                            return Ok(());
                                        }
                                        if let Some((tool_name, arguments)) =
                                            web_approval_parts(&decision)
                                        {
                                            self.pending_web_approval = Some(PendingWebApproval {
                                                active,
                                                tool_name,
                                                arguments,
                                            });
                                            self.log_workspace_events(&events.events)?;
                                            return Ok(());
                                        }
                                        events
                                    }
                                    PermissionDecision::Deny(denial) => {
                                        self.state.record_persona_runtime_event(
                                            PersonaRuntimeEvent::ToolPermissionDenied,
                                        );
                                        self.set_runtime_working_phase(
                                            WorkingPhase::Apply,
                                            "권한 거부 결과를 workspace에 반영합니다.",
                                        )?;
                                        if let Some((tool_name, arguments)) =
                                            web_approval_parts(&decision)
                                        {
                                            self.handle_web_denial_observation(
                                                active, tool_name, arguments, denial,
                                            )?;
                                            return Ok(());
                                        }
                                        if external_path_denial_reason(&denial) {
                                            if let Some((tool_name, arguments, target)) =
                                                decision_tool_parts_for_denial(&decision)
                                            {
                                                self.handle_external_path_denial_observation(
                                                    active, tool_name, arguments, target, denial,
                                                )?;
                                                return Ok(());
                                            }
                                        }
                                        self.record_permission_denial(&active, denial)?
                                    }
                                }
                            }
                            Err(error) => {
                                self.log_runtime_decision_failed(&active, &parsed, &error)?;
                                self.state.record_persona_runtime_event(
                                    PersonaRuntimeEvent::RuntimeDecisionFailed,
                                );
                                self.enqueue_persona_progress_request(persona_task_state_summary(
                                    &active,
                                    "runtime_decision_failed",
                                    error.kind.as_str(),
                                ));
                                self.llm_diagnostics
                                    .record_decision_failure(error.kind.as_str());
                                match RepairLoop::default_local().next_request_for_runtime_decision(
                                    active.repair_attempts_for_source("runtime_decision"),
                                    &parsed,
                                    &error,
                                    answer,
                                ) {
                                    Ok(repair_request) => {
                                        self.llm_diagnostics.record_repair_started(
                                            repair_request.attempt,
                                            repair_request.max_attempts,
                                        );
                                        self.set_runtime_working_phase(
                                            WorkingPhase::Classify,
                                            "검증 실패를 repair 요청으로 재구성합니다.",
                                        )?;
                                        self.start_repair_request(active, repair_request)?;
                                        return Ok(());
                                    }
                                    Err(limit) => {
                                        self.log_repair_limit_reached(&active, &limit)?;
                                        self.state.record_persona_runtime_event(
                                            PersonaRuntimeEvent::RepairLimitReached,
                                        );
                                        self.llm_diagnostics.record_repair_limited(
                                            limit.attempts,
                                            limit.max_attempts,
                                        );
                                        self.set_runtime_working_phase(
                                            WorkingPhase::Execute,
                                            "실행 가능한 후보가 없어 실행하지 않습니다.",
                                        )?;
                                        self.set_runtime_working_phase(
                                            WorkingPhase::Apply,
                                            "검증 실패를 workspace에 반영합니다.",
                                        )?;
                                        runtime_workspace::record_runtime_decision_error(
                                            &mut self.state,
                                            &error,
                                        )
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        self.log_runtime_response_parse_failed(&active, answer, &error)?;
                        self.state.record_persona_runtime_event(
                            persona_event_for_runtime_response_parse_error(&error),
                        );
                        self.enqueue_persona_progress_request(persona_task_state_summary(
                            &active,
                            "runtime_response_parse_failed",
                            error.kind.as_str(),
                        ));
                        self.llm_diagnostics
                            .record_parse_failure(error.kind.as_str());
                        if let Some(final_decision) =
                            runtime_request::completed_tool_fallback_final_decision(
                                &active,
                                error.kind.as_str(),
                            )
                        {
                            active.record_final_decision(&final_decision);
                            let events = runtime_workspace::record_runtime_decision(
                                &mut self.state,
                                &final_decision,
                            );
                            self.finish_plain_request_with_events(
                                events,
                                "성공한 도구 observation으로 파싱 실패를 종료합니다.",
                                Some((&active.run_id, &active.turn_id)),
                                active.persona_context_start,
                                persona_task_state_summary(
                                    &active,
                                    "completed",
                                    "tool observation succeeded before model follow-up parse failure",
                                ),
                                Some(runtime_request::conversation_task_context_summary(&active)),
                            )?;
                            return Ok(());
                        }
                        match RepairLoop::default_local().next_request_with_raw(
                            active.repair_attempts_for_source("response_parse"),
                            &error,
                            Some(answer),
                        ) {
                            Ok(repair_request) => {
                                self.llm_diagnostics.record_repair_started(
                                    repair_request.attempt,
                                    repair_request.max_attempts,
                                );
                                self.set_runtime_working_phase(
                                    WorkingPhase::Classify,
                                    "응답 오류를 repair 요청으로 재구성합니다.",
                                )?;
                                self.start_repair_request(active, repair_request)?;
                                return Ok(());
                            }
                            Err(limit) => {
                                self.log_repair_limit_reached(&active, &limit)?;
                                self.state.record_persona_runtime_event(
                                    PersonaRuntimeEvent::RepairLimitReached,
                                );
                                self.llm_diagnostics
                                    .record_repair_limited(limit.attempts, limit.max_attempts);
                                self.set_runtime_working_phase(
                                    WorkingPhase::Execute,
                                    "실행 가능한 응답이 없어 실행하지 않습니다.",
                                )?;
                                self.set_runtime_working_phase(
                                    WorkingPhase::Apply,
                                    "repair 제한 초과를 workspace에 반영합니다.",
                                )?;
                                runtime_workspace::record_runtime_response_parse_error(
                                    &mut self.state,
                                    &error,
                                )
                            }
                        }
                    }
                }
            }
            LlmChatStatus::Failed(failure) => {
                if let Some(final_decision) =
                    runtime_request::completed_tool_fallback_final_decision(
                        &active,
                        failure.kind.as_str(),
                    )
                {
                    active.record_final_decision(&final_decision);
                    let events = runtime_workspace::record_runtime_decision(
                        &mut self.state,
                        &final_decision,
                    );
                    self.finish_plain_request_with_events(
                        events,
                        "성공한 도구 observation으로 요청 실패를 종료합니다.",
                        Some((&active.run_id, &active.turn_id)),
                        active.persona_context_start,
                        persona_task_state_summary(
                            &active,
                            "completed",
                            "tool observation succeeded before model follow-up request failure",
                        ),
                        Some(runtime_request::conversation_task_context_summary(&active)),
                    )?;
                    return Ok(());
                }
                self.llm_diagnostics.record_request_report(&report);
                let failure_message = active.history.append(
                    active.turn_id.clone(),
                    LlmMessageRole::System,
                    LlmMessageVisibility::Internal,
                    format!("request_failed:{}", failure.kind.as_str()),
                );
                self.log_message_recorded(&active.history, &failure_message)?;
                self.log_plain_request_failed(&active, &report)?;
                self.state
                    .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestFailed);
                self.set_runtime_working_phase(
                    WorkingPhase::Execute,
                    "요청이 실패해 실행하지 않습니다.",
                )?;
                self.set_runtime_working_phase(
                    WorkingPhase::Apply,
                    "요청 실패를 workspace에 반영합니다.",
                )?;
                runtime_workspace::record_plain_chat_failure(&mut self.state, &report)
            }
        };
        self.finish_plain_request_with_events(
            events,
            "응답 준비를 마무리합니다.",
            Some((&active.run_id, &active.turn_id)),
            active.persona_context_start,
            persona_task_state_summary(&active, "completed", "main runtime reached answer phase"),
            Some(runtime_request::conversation_task_context_summary(&active)),
        )
    }

    fn cancel_active_plain_request(&mut self) -> io::Result<()> {
        let Some(mut active) = self.active_plain_request.take() else {
            return Ok(());
        };
        active.cancelled = true;
        let cancel_message = active.history.append(
            active.turn_id.clone(),
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            "request_cancelled",
        );
        self.log_message_recorded(&active.history, &cancel_message)?;
        self.log_plain_request_cancelled(&active)?;
        self.log_runtime_process_cancelled(&active)?;
        self.state.pending_prompt = None;
        Ok(())
    }

    fn open_permission_approval(
        &mut self,
        active: &ActivePlainRequest,
        request: PermissionRequest,
        change_preview: Option<ChangePreview>,
    ) -> io::Result<WorkspaceEvents> {
        let action = request.action.clone();
        let reason = request.reason.clone();
        self.state.approval_surface.open(ApprovalRequest {
            title: request.title,
            reason: request.reason,
            action: request.action,
            details: request.details,
        });
        self.log_approval_events(&[ApprovalInputEvent::SurfaceOpened])?;
        self.log_tool_permission_approval_opened(active, &action, &reason)?;

        let mut events = self
            .state
            .record_system_notice(format!("approval needed: {action}"));
        events.extend(self.state.record_system_notice(reason));
        if let Some(preview) = change_preview {
            events.extend(self.state.workspace.push_diff_summary(
                preview.target_path,
                preview.additions,
                preview.deletions,
            ));
        }
        Ok(events)
    }

    fn record_permission_denial(
        &mut self,
        active: &ActivePlainRequest,
        denial: PermissionDenial,
    ) -> io::Result<WorkspaceEvents> {
        self.log_tool_permission_denied(active, &denial.reason, &denial.message)?;
        let mut events = self
            .state
            .record_system_notice(format!("permission denied: {}", denial.reason));
        events.extend(self.state.record_system_notice(denial.message));
        Ok(events)
    }

    fn prepare_pending_change_approval(
        &self,
        _active: &ActivePlainRequest,
        decision: &RuntimeDecision,
        change_preview: Option<&ChangePreview>,
    ) -> io::Result<PendingChangePreparation> {
        let RuntimeDecision::ApprovalNeeded {
            activity: Activity::Change,
            tool_name,
            arguments,
            ..
        } = decision
        else {
            return Ok(PendingChangePreparation::NotChange);
        };
        let Some(preview) = change_preview else {
            return Ok(PendingChangePreparation::NotChange);
        };
        if tool_name != "apply_patch" {
            return Ok(PendingChangePreparation::NotChange);
        }
        let signature = change_approval_signature(tool_name, arguments, preview);

        match capture_change_precondition(self.tool_runtime.workspace_root(), preview) {
            Ok(precondition) => {
                let approved = ApprovedChange {
                    preview: preview.clone(),
                    precondition,
                };
                if let Err(observation) = validate_approved_change(&approved) {
                    return Ok(PendingChangePreparation::Failed {
                        tool_name: tool_name.clone(),
                        arguments: arguments.clone(),
                        signature,
                        observation,
                    });
                }
                Ok(PendingChangePreparation::Pending {
                    tool_name: tool_name.clone(),
                    arguments: arguments.clone(),
                    signature,
                    preview: approved.preview,
                    precondition: approved.precondition,
                })
            }
            Err(observation) => Ok(PendingChangePreparation::Failed {
                tool_name: tool_name.clone(),
                arguments: arguments.clone(),
                signature,
                observation,
            }),
        }
    }

    fn handle_approval_runtime_outcome(&mut self, events: &[ApprovalInputEvent]) -> io::Result<()> {
        match approval_result(events) {
            Some(ApprovalResult::ApprovedOnce) => {
                if self.pending_change_approval.is_some() {
                    self.apply_pending_change_approval()
                } else if self.pending_command_approval.is_some() {
                    self.apply_pending_command_approval()
                } else {
                    self.apply_pending_web_approval()
                }
            }
            Some(ApprovalResult::Denied) => {
                if self.pending_change_approval.is_some() {
                    self.deny_pending_change_approval()
                } else if self.pending_command_approval.is_some() {
                    self.deny_pending_command_approval()
                } else {
                    self.deny_pending_web_approval()
                }
            }
            None => Ok(()),
        }
    }

    fn apply_pending_change_approval(&mut self) -> io::Result<()> {
        let Some(pending) = self.pending_change_approval.take() else {
            return Ok(());
        };
        let PendingChangeApproval {
            active,
            tool_name,
            arguments,
            signature,
            preview,
            precondition,
        } = pending;
        let observation = apply_approved_change(
            self.tool_runtime.workspace_root(),
            ApprovedChange {
                preview: preview.clone(),
                precondition,
            },
        );
        self.handle_change_observation_after_approval(
            active,
            tool_name,
            arguments,
            signature,
            observation,
            Some(preview.target_path),
            "approved_change_observation_recorded",
        )
    }

    fn deny_pending_change_approval(&mut self) -> io::Result<()> {
        let Some(pending) = self.pending_change_approval.take() else {
            return Ok(());
        };
        let observation = ToolObservation::failed(
            pending.tool_name.clone(),
            Some(pending.preview.target_path.clone()),
            ToolErrorKind::PermissionError,
            "change approval was denied by the user",
        );
        let signature =
            change_approval_signature(&pending.tool_name, &pending.arguments, &pending.preview);
        self.handle_change_observation_after_approval(
            pending.active,
            pending.tool_name,
            pending.arguments,
            signature,
            observation,
            None,
            "change_approval_denied",
        )
    }

    fn apply_pending_command_approval(&mut self) -> io::Result<()> {
        let Some(pending) = self.pending_command_approval.take() else {
            return Ok(());
        };
        let observation = self.tool_runtime.execute_approved_command(
            &pending.active.run_id,
            &pending.active.turn_id,
            pending.arguments.clone(),
            self.runtime_config.limits.command_timeout_ms,
        );
        self.handle_command_observation_after_approval(
            pending.active,
            pending.tool_name,
            pending.arguments,
            observation,
            "approved_command_observation_recorded",
        )
    }

    fn deny_pending_command_approval(&mut self) -> io::Result<()> {
        let Some(pending) = self.pending_command_approval.take() else {
            return Ok(());
        };
        let observation = ToolObservation::failed(
            pending.tool_name.clone(),
            command_cwd_for_observation(&pending.arguments),
            ToolErrorKind::PermissionError,
            "command approval was denied by the user",
        );
        self.handle_command_observation_after_approval(
            pending.active,
            pending.tool_name,
            pending.arguments,
            observation,
            "command_approval_denied",
        )
    }

    fn apply_pending_web_approval(&mut self) -> io::Result<()> {
        let Some(pending) = self.pending_web_approval.take() else {
            return Ok(());
        };
        let observation = self.tool_runtime.execute_approved_web(
            &pending.active.run_id,
            &pending.active.turn_id,
            pending.tool_name.clone(),
            pending.arguments.clone(),
            self.runtime_config.web.enabled,
            self.runtime_config.limits.command_timeout_ms,
        );
        self.handle_web_observation_after_approval(
            pending.active,
            pending.tool_name,
            pending.arguments,
            observation,
            "approved_web_observation_recorded",
        )
    }

    fn deny_pending_web_approval(&mut self) -> io::Result<()> {
        let Some(pending) = self.pending_web_approval.take() else {
            return Ok(());
        };
        let observation = ToolObservation::failed(
            pending.tool_name.clone(),
            web_target_for_observation(&pending.tool_name, &pending.arguments),
            ToolErrorKind::PermissionError,
            "web approval was denied by the user",
        );
        self.handle_web_observation_after_approval(
            pending.active,
            pending.tool_name,
            pending.arguments,
            observation,
            "web_approval_denied",
        )
    }

    fn handle_web_denial_observation(
        &mut self,
        active: ActivePlainRequest,
        tool_name: String,
        arguments: Value,
        denial: PermissionDenial,
    ) -> io::Result<()> {
        self.log_tool_permission_denied(&active, &denial.reason, &denial.message)?;
        let observation = ToolObservation::failed(
            tool_name.clone(),
            web_target_for_observation(&tool_name, &arguments),
            ToolErrorKind::PermissionError,
            denial.message,
        );
        self.handle_web_observation_after_approval(
            active,
            tool_name,
            arguments,
            observation,
            "web_permission_denied",
        )
    }

    fn handle_external_path_denial_observation(
        &mut self,
        active: ActivePlainRequest,
        tool_name: String,
        arguments: Value,
        target_raw: Option<String>,
        denial: PermissionDenial,
    ) -> io::Result<()> {
        self.log_tool_permission_denied(&active, &denial.reason, &denial.message)?;
        let observation = ToolObservation::failed(
            tool_name.clone(),
            target_raw,
            ToolErrorKind::PathOutsideWorkspace,
            denial.message,
        );
        self.handle_external_path_observation_after_denial(
            active,
            tool_name,
            arguments,
            observation,
            "external_path_denied",
        )
    }

    fn handle_change_observation_after_approval(
        &mut self,
        mut active: ActivePlainRequest,
        tool_name: String,
        arguments: Value,
        signature: String,
        observation: ToolObservation,
        diagnostic_target: Option<String>,
        persona_status: &str,
    ) -> io::Result<()> {
        active.tool_call_count = active.tool_call_count.saturating_add(1);
        active.last_tool_signature = Some(signature.clone());
        active.same_tool_repeat_count = 1;

        self.log_change_tool_call_received(&active, &tool_name, &arguments)?;
        self.log_change_execution_started(&active, &tool_name, &arguments)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::ToolExecutionStarted);
        self.log_change_observation(&active, &observation)?;
        self.state
            .record_persona_runtime_event(persona_event_for_tool_observation(&observation));

        let mut observation_message =
            self.record_change_loop_observation(&mut active, signature, &observation)?;
        self.enqueue_persona_progress_request(persona_task_state_summary(
            &active,
            persona_status,
            "main runtime recorded a change observation and is asking the model to continue from that evidence",
        ));

        let mut workspace_events = self
            .state
            .workspace
            .push_work_output(ActivityGroup::Change, observation.summary());
        workspace_events.extend(
            self.state
                .workspace
                .push_evidence(observation.tool_name.clone(), observation.preview_text()),
        );
        self.log_workspace_events(&workspace_events.events)?;

        if observation.status == ObservationStatus::Succeeded {
            if let Some(target_path) = diagnostic_target {
                if let Some(diagnostic) =
                    self.tool_runtime
                        .post_edit_diagnostics(PostEditDiagnosticRequest {
                            run_id: active.run_id.clone(),
                            turn_id: active.turn_id.clone(),
                            target_path,
                        })
                {
                    let diagnostic_signature = tool_signature(
                        "post_edit_diagnostics",
                        &json!({"path": diagnostic.target_raw.clone().unwrap_or_default()}),
                    );
                    observation_message = self.record_diagnostic_loop_observation(
                        &mut active,
                        diagnostic_signature,
                        &diagnostic,
                    )?;
                    let mut diagnostic_events = self
                        .state
                        .workspace
                        .push_work_output(ActivityGroup::Change, diagnostic.summary());
                    diagnostic_events.extend(
                        self.state
                            .workspace
                            .push_evidence(diagnostic.tool_name.clone(), diagnostic.preview_text()),
                    );
                    self.log_workspace_events(&diagnostic_events.events)?;
                    self.enqueue_persona_progress_request(persona_task_state_summary(
                        &active,
                        "post_edit_diagnostics_recorded",
                        "main runtime recorded post-edit diagnostics as observation only and did not auto-fix from it",
                    ));
                }
            }
        }

        let next_turn_id = active.history.next_turn_id();
        self.log_turn_id_assigned(&active.history, &next_turn_id)?;
        let request_messages = runtime_request::tool_loop_request_messages(
            &active.schema_message,
            &active.user_message,
            &observation_message,
            &active.executed_tool_records,
            &active.executed_tool_signatures,
            &next_turn_id,
        );
        self.log_plain_request_started(&active.run_id, &next_turn_id, &active.prompt)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestStarted);
        self.log_tool_loop_request_started(&active, &next_turn_id)?;
        self.llm_diagnostics
            .record_request_started(&active.run_id, &next_turn_id);
        self.set_runtime_working_phase(
            WorkingPhase::Interpret,
            "변경 observation을 반영한 다음 LLM 응답을 기다립니다.",
        )?;

        active.turn_id = next_turn_id;
        active.receiver =
            runtime_request::spawn_chat_request(&self.runtime_config, request_messages);
        active.reset_request_timer();
        active.reset_repair_state();
        self.active_plain_request = Some(active);

        Ok(())
    }

    fn handle_command_observation_after_approval(
        &mut self,
        mut active: ActivePlainRequest,
        tool_name: String,
        arguments: Value,
        observation: ToolObservation,
        persona_status: &str,
    ) -> io::Result<()> {
        let signature = tool_signature(&tool_name, &arguments);
        active.tool_call_count = active.tool_call_count.saturating_add(1);
        active.last_tool_signature = Some(signature.clone());
        active.same_tool_repeat_count = 1;

        self.log_command_tool_call_received(&active, &tool_name, &arguments)?;
        self.log_command_execution_started(&active, &tool_name, &arguments)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::ToolExecutionStarted);
        self.log_command_observation(&active, &observation)?;
        self.state
            .record_persona_runtime_event(persona_event_for_tool_observation(&observation));

        let observation_message =
            self.record_command_loop_observation(&mut active, signature, &observation)?;
        self.enqueue_persona_progress_request(persona_task_state_summary(
            &active,
            persona_status,
            "main runtime recorded an approved command observation and is asking the model to continue from that evidence",
        ));

        let mut workspace_events = self
            .state
            .workspace
            .push_work_output(ActivityGroup::Execute, observation.summary());
        workspace_events.extend(
            self.state
                .workspace
                .push_evidence(observation.tool_name.clone(), observation.preview_text()),
        );
        self.log_workspace_events(&workspace_events.events)?;

        let next_turn_id = active.history.next_turn_id();
        self.log_turn_id_assigned(&active.history, &next_turn_id)?;
        let request_messages = runtime_request::tool_loop_request_messages(
            &active.schema_message,
            &active.user_message,
            &observation_message,
            &active.executed_tool_records,
            &active.executed_tool_signatures,
            &next_turn_id,
        );
        self.log_plain_request_started(&active.run_id, &next_turn_id, &active.prompt)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestStarted);
        self.log_tool_loop_request_started(&active, &next_turn_id)?;
        self.llm_diagnostics
            .record_request_started(&active.run_id, &next_turn_id);
        self.set_runtime_working_phase(
            WorkingPhase::Interpret,
            "승인된 명령 observation을 반영한 다음 LLM 응답을 기다립니다.",
        )?;

        active.turn_id = next_turn_id;
        active.receiver =
            runtime_request::spawn_chat_request(&self.runtime_config, request_messages);
        active.reset_request_timer();
        active.reset_repair_state();
        self.active_plain_request = Some(active);

        Ok(())
    }

    fn handle_external_path_observation_after_denial(
        &mut self,
        mut active: ActivePlainRequest,
        tool_name: String,
        arguments: Value,
        observation: ToolObservation,
        persona_status: &str,
    ) -> io::Result<()> {
        let signature = tool_signature(&tool_name, &arguments);
        active.tool_call_count = active.tool_call_count.saturating_add(1);
        active.last_tool_signature = Some(signature.clone());
        active.same_tool_repeat_count = 1;

        self.log_external_path_observation(&active, &observation)?;
        self.state
            .record_persona_runtime_event(persona_event_for_tool_observation(&observation));
        let observation_message =
            self.record_external_path_loop_observation(&mut active, signature, &observation)?;
        self.enqueue_persona_progress_request(persona_task_state_summary(
            &active,
            persona_status,
            "main runtime recorded an external path policy observation and is asking the model to continue from that evidence",
        ));

        let mut workspace_events = self
            .state
            .workspace
            .push_work_output(ActivityGroup::Explore, observation.summary());
        workspace_events.extend(
            self.state
                .workspace
                .push_evidence(observation.tool_name.clone(), observation.preview_text()),
        );
        self.log_workspace_events(&workspace_events.events)?;

        let next_turn_id = active.history.next_turn_id();
        self.log_turn_id_assigned(&active.history, &next_turn_id)?;
        let request_messages = runtime_request::tool_loop_request_messages(
            &active.schema_message,
            &active.user_message,
            &observation_message,
            &active.executed_tool_records,
            &active.executed_tool_signatures,
            &next_turn_id,
        );
        self.log_plain_request_started(&active.run_id, &next_turn_id, &active.prompt)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestStarted);
        self.log_tool_loop_request_started(&active, &next_turn_id)?;
        self.llm_diagnostics
            .record_request_started(&active.run_id, &next_turn_id);
        self.set_runtime_working_phase(
            WorkingPhase::Interpret,
            "external path policy observation을 반영한 다음 LLM 응답을 기다립니다.",
        )?;

        active.turn_id = next_turn_id;
        active.receiver =
            runtime_request::spawn_chat_request(&self.runtime_config, request_messages);
        active.reset_request_timer();
        active.reset_repair_state();
        self.active_plain_request = Some(active);

        Ok(())
    }

    fn handle_web_observation_after_approval(
        &mut self,
        mut active: ActivePlainRequest,
        tool_name: String,
        arguments: Value,
        observation: ToolObservation,
        persona_status: &str,
    ) -> io::Result<()> {
        let signature = tool_signature(&tool_name, &arguments);
        active.tool_call_count = active.tool_call_count.saturating_add(1);
        active.last_tool_signature = Some(signature.clone());
        active.same_tool_repeat_count = 1;

        self.log_web_tool_call_received(&active, &tool_name, &arguments)?;
        self.log_web_execution_started(&active, &tool_name, &arguments)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::ToolExecutionStarted);
        self.log_web_observation(&active, &observation)?;
        self.state
            .record_persona_runtime_event(persona_event_for_tool_observation(&observation));

        let observation_message =
            self.record_web_loop_observation(&mut active, signature, &observation)?;
        self.enqueue_persona_progress_request(persona_task_state_summary(
            &active,
            persona_status,
            "main runtime recorded a web/network observation and is asking the model to continue from that evidence",
        ));

        let mut workspace_events = self
            .state
            .workspace
            .push_work_output(ActivityGroup::Explore, observation.summary());
        workspace_events.extend(
            self.state
                .workspace
                .push_evidence(observation.tool_name.clone(), observation.preview_text()),
        );
        self.log_workspace_events(&workspace_events.events)?;

        let next_turn_id = active.history.next_turn_id();
        self.log_turn_id_assigned(&active.history, &next_turn_id)?;
        let request_messages = runtime_request::tool_loop_request_messages(
            &active.schema_message,
            &active.user_message,
            &observation_message,
            &active.executed_tool_records,
            &active.executed_tool_signatures,
            &next_turn_id,
        );
        self.log_plain_request_started(&active.run_id, &next_turn_id, &active.prompt)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestStarted);
        self.log_tool_loop_request_started(&active, &next_turn_id)?;
        self.llm_diagnostics
            .record_request_started(&active.run_id, &next_turn_id);
        self.set_runtime_working_phase(
            WorkingPhase::Interpret,
            "web/network observation을 반영한 다음 LLM 응답을 기다립니다.",
        )?;

        active.turn_id = next_turn_id;
        active.receiver =
            runtime_request::spawn_chat_request(&self.runtime_config, request_messages);
        active.reset_request_timer();
        active.reset_repair_state();
        self.active_plain_request = Some(active);

        Ok(())
    }

    fn record_change_loop_observation(
        &mut self,
        active: &mut ActivePlainRequest,
        signature: String,
        observation: &ToolObservation,
    ) -> io::Result<LlmMessage> {
        let observation_message = active.history.append(
            active.turn_id.clone(),
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            observation.history_message(),
        );
        self.log_message_recorded(&active.history, &observation_message)?;
        self.log_tool_observation_attached(active, &observation_message)?;
        self.log_change_observation_recorded(active, observation)?;
        active.record_tool_execution(signature, observation, observation_message.clone());
        Ok(observation_message)
    }

    fn record_command_loop_observation(
        &mut self,
        active: &mut ActivePlainRequest,
        signature: String,
        observation: &ToolObservation,
    ) -> io::Result<LlmMessage> {
        let observation_message = active.history.append(
            active.turn_id.clone(),
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            observation.history_message(),
        );
        self.log_message_recorded(&active.history, &observation_message)?;
        self.log_tool_observation_attached(active, &observation_message)?;
        self.log_command_observation_recorded(active, observation)?;
        active.record_tool_execution(signature, observation, observation_message.clone());
        Ok(observation_message)
    }

    fn record_web_loop_observation(
        &mut self,
        active: &mut ActivePlainRequest,
        signature: String,
        observation: &ToolObservation,
    ) -> io::Result<LlmMessage> {
        let observation_message = active.history.append(
            active.turn_id.clone(),
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            observation.history_message(),
        );
        self.log_message_recorded(&active.history, &observation_message)?;
        self.log_tool_observation_attached(active, &observation_message)?;
        self.log_web_observation_recorded(active, observation)?;
        active.record_tool_execution(signature, observation, observation_message.clone());
        Ok(observation_message)
    }

    fn record_external_path_loop_observation(
        &mut self,
        active: &mut ActivePlainRequest,
        signature: String,
        observation: &ToolObservation,
    ) -> io::Result<LlmMessage> {
        let observation_message = active.history.append(
            active.turn_id.clone(),
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            observation.history_message(),
        );
        self.log_message_recorded(&active.history, &observation_message)?;
        self.log_tool_observation_attached(active, &observation_message)?;
        self.log_external_path_observation_recorded(active, observation)?;
        active.record_tool_execution(signature, observation, observation_message.clone());
        Ok(observation_message)
    }

    fn record_diagnostic_loop_observation(
        &mut self,
        active: &mut ActivePlainRequest,
        signature: String,
        observation: &ToolObservation,
    ) -> io::Result<LlmMessage> {
        self.log_diagnostic_observation(active, observation)?;
        let observation_message = active.history.append(
            active.turn_id.clone(),
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            observation.history_message(),
        );
        self.log_message_recorded(&active.history, &observation_message)?;
        self.log_tool_observation_attached(active, &observation_message)?;
        self.log_diagnostic_observation_recorded(active, observation)?;
        active.record_tool_execution(signature, observation, observation_message.clone());
        Ok(observation_message)
    }

    fn handle_tool_decision_loop(
        &mut self,
        mut active: ActivePlainRequest,
        decision: &RuntimeDecision,
    ) -> io::Result<()> {
        let RuntimeDecision::ToolCandidatePending {
            activity,
            tool_name,
            arguments,
            summary,
        } = decision
        else {
            return Ok(());
        };

        let signature = tool_signature(tool_name, arguments);
        if active.tool_call_count >= self.runtime_config.limits.max_tool_calls {
            let diagnosis = runtime_request::diagnose_tool_loop_limit(
                "max_tool_calls",
                active.last_tool_observation.as_ref(),
            );
            self.log_tool_loop_limit_reached(
                &active,
                "max_tool_calls",
                active.tool_call_count,
                self.runtime_config.limits.max_tool_calls,
                &signature,
                diagnosis.as_str(),
            )?;
            self.state
                .record_persona_runtime_event(PersonaRuntimeEvent::ToolLoopLimitReached);
            let events = runtime_workspace::record_tool_loop_limit(
                &mut self.state,
                "max_tool_calls",
                diagnosis.as_str(),
            );
            self.finish_plain_request_with_events(
                events,
                "도구 루프 제한을 보고합니다.",
                Some((&active.run_id, &active.turn_id)),
                active.persona_context_start,
                persona_task_state_summary(&active, "tool_loop_limited", diagnosis.as_str()),
                Some(runtime_request::conversation_task_context_summary(&active)),
            )?;
            return Ok(());
        }

        let same_tool_repeat_count =
            if active.last_tool_signature.as_deref() == Some(signature.as_str()) {
                active.same_tool_repeat_count.saturating_add(1)
            } else {
                1
            };
        if let Some((redirect, execution_record)) = active
            .repeat_redirect_for_tool_candidate(&signature, tool_name, arguments)
            .map(|(redirect, record)| (redirect, record.clone()))
        {
            let duplicate_redirect_count =
                active.duplicate_redirect_count_for_signature(&signature);
            if redirect == runtime_request::ToolLoopRepeatRedirect::SettledDuplicate
                && duplicate_redirect_count > 0
            {
                let decision =
                    runtime_request::settled_duplicate_final_decision(&active, &execution_record);
                active.record_final_decision(&decision);
                let events = runtime_workspace::record_runtime_decision(&mut self.state, &decision);
                self.finish_plain_request_with_events(
                    events,
                    "기존 observation으로 반복 도구 후보를 종료합니다.",
                    Some((&active.run_id, &active.turn_id)),
                    active.persona_context_start,
                    persona_task_state_summary(
                        &active,
                        "completed",
                        "settled duplicate tool candidate finalized from existing observation",
                    ),
                    Some(runtime_request::conversation_task_context_summary(&active)),
                )?;
                return Ok(());
            }

            if duplicate_redirect_count >= self.runtime_config.limits.max_same_tool_repeats {
                let diagnosis = runtime_request::diagnose_tool_loop_limit(
                    "max_same_tool_repeats",
                    active.last_tool_observation.as_ref(),
                );
                self.log_tool_loop_limit_reached(
                    &active,
                    "max_same_tool_repeats",
                    duplicate_redirect_count.saturating_add(1),
                    self.runtime_config.limits.max_same_tool_repeats,
                    &signature,
                    diagnosis.as_str(),
                )?;
                self.state
                    .record_persona_runtime_event(PersonaRuntimeEvent::ToolLoopLimitReached);
                let events = runtime_workspace::record_tool_loop_limit(
                    &mut self.state,
                    "max_same_tool_repeats",
                    diagnosis.as_str(),
                );
                self.finish_plain_request_with_events(
                    events,
                    "반복 도구 루프 제한을 보고합니다.",
                    Some((&active.run_id, &active.turn_id)),
                    active.persona_context_start,
                    persona_task_state_summary(&active, "tool_loop_limited", diagnosis.as_str()),
                    Some(runtime_request::conversation_task_context_summary(&active)),
                )?;
                return Ok(());
            }
            active.last_tool_signature = Some(signature.clone());
            active.same_tool_repeat_count = same_tool_repeat_count;
            active.duplicate_redirect_count = duplicate_redirect_count.saturating_add(1);

            let next_turn_id = active.history.next_turn_id();
            self.log_turn_id_assigned(&active.history, &next_turn_id)?;
            let request_messages = match redirect {
                runtime_request::ToolLoopRepeatRedirect::SettledDuplicate => {
                    runtime_request::tool_repeat_answer_request_messages(
                        &active.schema_message,
                        &active.user_message,
                        &execution_record,
                        &active.executed_tool_records,
                        &active.executed_tool_signatures,
                        &next_turn_id,
                    )
                }
                runtime_request::ToolLoopRepeatRedirect::TruncatedContinuation => {
                    runtime_request::tool_repeat_continuation_request_messages(
                        &active.schema_message,
                        &active.user_message,
                        &execution_record,
                        &active.executed_tool_records,
                        &active.executed_tool_signatures,
                        &next_turn_id,
                    )
                }
                runtime_request::ToolLoopRepeatRedirect::FailedDuplicate => {
                    runtime_request::tool_repeat_failure_request_messages(
                        &active.schema_message,
                        &active.user_message,
                        &execution_record,
                        &active.executed_tool_records,
                        &active.executed_tool_signatures,
                        &next_turn_id,
                    )
                }
            };
            self.log_plain_request_started(&active.run_id, &next_turn_id, &active.prompt)?;
            self.state
                .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestStarted);
            self.log_tool_loop_duplicate_redirected(
                &active,
                &next_turn_id,
                &signature,
                redirect.as_str(),
            )?;
            self.state
                .record_persona_runtime_event(PersonaRuntimeEvent::ToolLoopDuplicateRedirected);
            self.llm_diagnostics
                .record_request_started(&active.run_id, &next_turn_id);
            self.set_runtime_working_phase(
                WorkingPhase::Interpret,
                "동일 도구 호출을 재실행하지 않고 기존 observation을 반영한 다음 응답을 요청합니다.",
            )?;

            active.turn_id = next_turn_id;
            active.receiver =
                runtime_request::spawn_chat_request(&self.runtime_config, request_messages);
            active.reset_request_timer();
            active.reset_repair_state();
            self.active_plain_request = Some(active);
            return Ok(());
        }

        if same_tool_repeat_count > self.runtime_config.limits.max_same_tool_repeats {
            let diagnosis = runtime_request::diagnose_tool_loop_limit(
                "max_same_tool_repeats",
                active.last_tool_observation.as_ref(),
            );
            self.log_tool_loop_limit_reached(
                &active,
                "max_same_tool_repeats",
                same_tool_repeat_count,
                self.runtime_config.limits.max_same_tool_repeats,
                &signature,
                diagnosis.as_str(),
            )?;
            self.state
                .record_persona_runtime_event(PersonaRuntimeEvent::ToolLoopLimitReached);
            let events = runtime_workspace::record_tool_loop_limit(
                &mut self.state,
                "max_same_tool_repeats",
                diagnosis.as_str(),
            );
            self.finish_plain_request_with_events(
                events,
                "반복 도구 루프 제한을 보고합니다.",
                Some((&active.run_id, &active.turn_id)),
                active.persona_context_start,
                persona_task_state_summary(&active, "tool_loop_limited", diagnosis.as_str()),
                Some(runtime_request::conversation_task_context_summary(&active)),
            )?;
            return Ok(());
        }

        active.tool_call_count = active.tool_call_count.saturating_add(1);
        active.last_tool_signature = Some(signature.clone());
        active.same_tool_repeat_count = same_tool_repeat_count;

        self.log_tool_call_received(&active, *activity, tool_name, summary, arguments)?;
        let call = ToolCall::new(
            active.run_id.clone(),
            active.turn_id.clone(),
            *activity,
            tool_name.clone(),
            arguments.clone(),
        );
        self.log_tool_execution_started(&active, &call)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::ToolExecutionStarted);
        let observation = self.tool_runtime.execute(call);
        self.log_tool_observation(&active, &observation)?;
        self.state
            .record_persona_runtime_event(persona_event_for_tool_observation(&observation));

        let observation_message = active.history.append(
            active.turn_id.clone(),
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            observation.history_message(),
        );
        self.log_message_recorded(&active.history, &observation_message)?;
        self.log_tool_observation_attached(&active, &observation_message)?;
        self.log_tool_observation_recorded(&active, &observation)?;
        active.record_tool_execution(signature.clone(), &observation, observation_message.clone());
        self.enqueue_persona_progress_request(persona_task_state_summary(
            &active,
            "tool_observation_recorded",
            "main runtime recorded a workspace observation and is asking the model to continue from that evidence",
        ));

        let mut events = self
            .state
            .workspace
            .push_work_output(ActivityGroup::Explore, observation.summary());
        events.extend(
            self.state
                .workspace
                .push_evidence(observation.tool_name.clone(), observation.preview_text()),
        );
        self.log_tool_workspace_summary_rendered(&active, &observation)?;
        self.log_workspace_events(&events.events)?;

        let next_turn_id = active.history.next_turn_id();
        self.log_turn_id_assigned(&active.history, &next_turn_id)?;
        let request_messages = runtime_request::tool_loop_request_messages(
            &active.schema_message,
            &active.user_message,
            &observation_message,
            &active.executed_tool_records,
            &active.executed_tool_signatures,
            &next_turn_id,
        );
        self.log_plain_request_started(&active.run_id, &next_turn_id, &active.prompt)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::LlmRequestStarted);
        self.log_tool_loop_request_started(&active, &next_turn_id)?;
        self.llm_diagnostics
            .record_request_started(&active.run_id, &next_turn_id);
        self.set_runtime_working_phase(
            WorkingPhase::Interpret,
            "도구 observation을 반영한 다음 LLM 응답을 기다립니다.",
        )?;

        active.turn_id = next_turn_id;
        active.receiver =
            runtime_request::spawn_chat_request(&self.runtime_config, request_messages);
        active.reset_request_timer();
        active.reset_repair_state();
        self.active_plain_request = Some(active);

        Ok(())
    }

    fn start_repair_request(
        &mut self,
        mut active: ActivePlainRequest,
        repair_request: RepairRequest,
    ) -> io::Result<()> {
        let repair_message = active.history.append(
            active.turn_id.clone(),
            LlmMessageRole::System,
            LlmMessageVisibility::Internal,
            repair_request.prompt.clone(),
        );
        self.log_message_recorded(&active.history, &repair_message)?;
        self.log_repair_request_started(&active, &repair_request)?;
        self.state
            .record_persona_runtime_event(PersonaRuntimeEvent::RepairRequestStarted);
        self.enqueue_persona_progress_request(persona_task_state_summary(
            &active,
            "repair_request_started",
            repair_request.failure_signature.as_str(),
        ));

        let request_messages = runtime_request::repair_request_messages(&active.history);
        let receiver = runtime_request::spawn_chat_request(&self.runtime_config, request_messages);

        active.receiver = receiver;
        active.reset_request_timer();
        active.repair_attempts = repair_request.attempt;
        active.repair_source = Some(repair_request.source);
        self.active_plain_request = Some(active);
        Ok(())
    }

    fn set_runtime_working_phase(
        &mut self,
        phase: WorkingPhase,
        detail: impl Into<String>,
    ) -> io::Result<()> {
        let detail = detail.into();
        let runtime_outcome = self.state.set_working_process_phase(phase, detail.clone());
        self.log_working_process_events(&runtime_outcome.working_process_events.events)?;
        self.log_workspace_events(&runtime_outcome.workspace_events.events)?;
        if !runtime_outcome.working_process_events.events.is_empty() {
            self.log_runtime_process_phase_changed(phase, &detail)?;
        }
        Ok(())
    }

    fn finish_plain_request_with_events(
        &mut self,
        events: WorkspaceEvents,
        answer_detail: &str,
        runtime_ids: Option<(&str, &str)>,
        persona_context_start: usize,
        persona_task_state_summary: String,
        runtime_context_summary: Option<String>,
    ) -> io::Result<()> {
        self.set_runtime_working_phase(WorkingPhase::Answer, answer_detail)?;
        let complete_outcome = self.state.complete_working_process();
        self.log_working_process_events(&complete_outcome.working_process_events.events)?;
        self.log_workspace_events(&complete_outcome.workspace_events.events)?;
        self.log_runtime_process_completed(runtime_ids)?;
        let _ = persona_context_start;
        self.enqueue_persona_completion_request(persona_task_state_summary.clone());
        if let Some(prompt) = self.state.pending_prompt.as_ref() {
            self.previous_task_frame = Some(ConversationTaskFrame {
                user_prompt: prompt.clone(),
                runtime_context_summary: runtime_context_summary
                    .unwrap_or_else(|| persona_task_state_summary.clone()),
                task_state_summary: persona_task_state_summary,
            });
        }
        self.state.pending_prompt = None;
        self.log_workspace_events(&events.events)
    }

    fn build_schema_prompt(&self) -> io::Result<SchemaPrompt> {
        match SchemaPromptBuilder::build() {
            Ok(prompt) => {
                self.logger.llm(LogEvent::ui(
                    LLM_05_SCOPE,
                    EVENT_SCHEMA_PROMPT_BUILT,
                    json!({
                        "tool_manifest_id": prompt.tool_manifest_id,
                        "tool_manifest_version": prompt.tool_manifest_version,
                        "prompt_chars": prompt.content.chars().count(),
                    }),
                ))?;
                Ok(prompt)
            }
            Err(error) => {
                self.logger.llm(LogEvent::ui(
                    LLM_05_SCOPE,
                    EVENT_SCHEMA_PROMPT_BUILD_FAILED,
                    json!({
                        "missing_rule": error.missing_rule,
                    }),
                ))?;
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "schema prompt validation failed",
                ))
            }
        }
    }

    fn log_schema_prompt_attached(
        &self,
        history: &MessageHistory,
        message: &LlmMessage,
        prompt: &SchemaPrompt,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_05_SCOPE,
            EVENT_SCHEMA_PROMPT_ATTACHED,
            json!({
                "run_id": history.run_id(),
                "turn_id": &message.turn_id,
                "tool_manifest_id": prompt.tool_manifest_id,
                "tool_manifest_version": prompt.tool_manifest_version,
                "role": message.role.as_str(),
                "visibility": message.visibility.as_str(),
                "content_chars": message.content.chars().count(),
            }),
        ))
    }

    fn log_message_history_created(&self, history: &MessageHistory) -> io::Result<()> {
        self.write_history_event(
            EVENT_MESSAGE_HISTORY_CREATED,
            json!({
                "run_id": history.run_id(),
            }),
        )
    }

    fn log_turn_id_assigned(&self, history: &MessageHistory, turn_id: &str) -> io::Result<()> {
        self.write_history_event(
            EVENT_TURN_ID_ASSIGNED,
            json!({
                "run_id": history.run_id(),
                "turn_id": turn_id,
            }),
        )
    }

    fn log_message_recorded(
        &self,
        history: &MessageHistory,
        message: &LlmMessage,
    ) -> io::Result<()> {
        self.write_history_event(
            EVENT_MESSAGE_RECORDED,
            json!({
                "run_id": history.run_id(),
                "turn_id": &message.turn_id,
                "role": message.role.as_str(),
                "visibility": message.visibility.as_str(),
                "content_chars": message.content.chars().count(),
            }),
        )
    }

    fn write_history_event(&self, event: &'static str, data: serde_json::Value) -> io::Result<()> {
        let result = self.logger.llm(LogEvent::ui(LLM_04_SCOPE, event, data));
        if let Err(error) = result {
            let _ = self.logger.llm(LogEvent::ui(
                LLM_04_SCOPE,
                EVENT_HISTORY_WRITE_FAILED,
                json!({
                    "failed_event": event,
                    "message": error.to_string(),
                }),
            ));
            return Err(error);
        }

        Ok(())
    }

    fn log_plain_request_started(
        &self,
        run_id: &str,
        turn_id: &str,
        prompt: &str,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_03_SCOPE,
            EVENT_LLM_REQUEST_STARTED,
            json!({
                "run_id": run_id,
                "turn_id": turn_id,
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
                "turn_id": &active.turn_id,
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

    fn log_raw_response_received(
        &self,
        active: &ActivePlainRequest,
        answer: &str,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_06_SCOPE,
            EVENT_RAW_RESPONSE_RECEIVED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "response_chars": answer.chars().count(),
            }),
        ))
    }

    fn log_runtime_response_parsed(
        &self,
        active: &ActivePlainRequest,
        parsed: &ParsedRuntimeResponse,
    ) -> io::Result<()> {
        let manifest = parsed.response.manifest();
        self.logger.llm(LogEvent::ui(
            LLM_06_SCOPE,
            EVENT_JSON_PARSE_SUCCEEDED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "response_type": parsed.response.response_type(),
                "activity": parsed.response.activity().as_str(),
                "tool_manifest_id": &manifest.tool_manifest_id,
                "tool_manifest_version": &manifest.tool_manifest_version,
                "payload_count": parsed.payloads.len(),
            }),
        ))
    }

    fn log_runtime_response_parse_failed(
        &self,
        active: &ActivePlainRequest,
        answer: &str,
        error: &RuntimeResponseParseError,
    ) -> io::Result<()> {
        let event = match error.kind {
            RuntimeResponseParseErrorKind::JsonParseFailed => EVENT_JSON_PARSE_FAILED,
            RuntimeResponseParseErrorKind::SchemaValidationFailed
            | RuntimeResponseParseErrorKind::PayloadValidationFailed
            | RuntimeResponseParseErrorKind::PartialResponse => EVENT_SCHEMA_VALIDATION_FAILED,
        };

        self.logger.llm(LogEvent::ui(
            LLM_06_SCOPE,
            event,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "response_chars": answer.chars().count(),
                "error_kind": error.kind.as_str(),
                "message": &error.message,
                "recoverable": true,
            }),
        ))
    }

    fn log_repair_request_started(
        &self,
        active: &ActivePlainRequest,
        repair_request: &RepairRequest,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_07_SCOPE,
            EVENT_REPAIR_REQUEST_STARTED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "attempt": repair_request.attempt,
                "max_attempts": repair_request.max_attempts,
                "source": repair_request.source,
                "failure_signature": &repair_request.failure_signature,
                "prompt_chars": repair_request.prompt.chars().count(),
            }),
        ))
    }

    fn log_repair_response_received(
        &self,
        active: &ActivePlainRequest,
        answer: &str,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_07_SCOPE,
            EVENT_REPAIR_RESPONSE_RECEIVED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "attempt": active.repair_attempts,
                "response_chars": answer.chars().count(),
            }),
        ))
    }

    fn log_repair_succeeded(
        &self,
        active: &ActivePlainRequest,
        parsed: &ParsedRuntimeResponse,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_07_SCOPE,
            EVENT_REPAIR_SUCCEEDED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "attempt": active.repair_attempts,
                "response_type": parsed.response.response_type(),
                "activity": parsed.response.activity().as_str(),
                "payload_count": parsed.payloads.len(),
            }),
        ))
    }

    fn log_repair_limit_reached(
        &self,
        active: &ActivePlainRequest,
        limit: &RepairLimitReached,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_07_SCOPE,
            EVENT_REPAIR_LIMIT_REACHED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "attempts": limit.attempts,
                "max_attempts": limit.max_attempts,
                "failure_signature": &limit.failure_signature,
                "reason": limit.reason.as_str(),
            }),
        ))
    }

    fn log_runtime_decision_started(
        &self,
        active: &ActivePlainRequest,
        parsed: &ParsedRuntimeResponse,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_08_SCOPE,
            EVENT_RUNTIME_DECISION_STARTED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "response_type": parsed.response.response_type(),
                "activity": parsed.response.activity().as_str(),
                "payload_count": parsed.payloads.len(),
            }),
        ))
    }

    fn log_runtime_decision_recorded(
        &self,
        active: &ActivePlainRequest,
        decision: &RuntimeDecision,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_08_SCOPE,
            EVENT_RUNTIME_DECISION_RECORDED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "decision": decision.kind(),
                "activity": decision.activity().map(|activity| activity.as_str()),
                "tool_name": decision.tool_name(),
            }),
        ))
    }

    fn log_tool_candidate_classified(
        &self,
        active: &ActivePlainRequest,
        decision: &RuntimeDecision,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_08_SCOPE,
            EVENT_TOOL_CANDIDATE_CLASSIFIED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "decision": decision.kind(),
                "activity": decision.activity().map(|activity| activity.as_str()),
                "tool_name": decision.tool_name(),
            }),
        ))
    }

    fn log_tool_permission_evaluated(
        &self,
        active: &ActivePlainRequest,
        decision: &RuntimeDecision,
        permission: &PermissionDecision,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_04_SCOPE,
            EVENT_TOOL_PERMISSION_EVALUATED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "decision": decision.kind(),
                "activity": decision.activity().map(|activity| activity.as_str()),
                "tool_name": decision.tool_name(),
                "branch": permission.branch(),
            }),
        ))
    }

    fn log_tool_permission_approval_opened(
        &self,
        active: &ActivePlainRequest,
        action: &str,
        reason: &str,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_04_SCOPE,
            EVENT_TOOL_PERMISSION_APPROVAL_OPENED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "action": action,
                "reason": reason,
            }),
        ))
    }

    fn log_tool_permission_denied(
        &self,
        active: &ActivePlainRequest,
        reason: &str,
        message: &str,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_04_SCOPE,
            EVENT_TOOL_PERMISSION_DENIED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "reason": reason,
                "message": message,
            }),
        ))
    }

    fn log_tool_call_received(
        &self,
        active: &ActivePlainRequest,
        activity: Activity,
        tool_name: &str,
        summary: &str,
        arguments: &serde_json::Value,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_EXPLORE_SCOPE,
            EVENT_TOOL_CALL_RECEIVED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "activity": activity.as_str(),
                "tool_name": tool_name,
                "summary_chars": summary.chars().count(),
                "argument_keys": argument_keys(arguments),
                "arguments_redacted": redacted_tool_arguments(tool_name, arguments),
            }),
        ))
    }

    fn log_tool_execution_started(
        &self,
        active: &ActivePlainRequest,
        call: &ToolCall,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_EXPLORE_SCOPE,
            EVENT_TOOL_EXECUTION_STARTED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &call.tool_name,
                "activity": call.activity.as_str(),
                "workspace_root": self.tool_runtime.workspace_root().display().to_string(),
            }),
        ))
    }

    fn log_tool_observation(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_EXPLORE_SCOPE,
            EVENT_TOOL_ARGUMENT_RESOLVED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
            }),
        ))?;
        self.logger.llm(LogEvent::ui(
            TOOL_EXPLORE_SCOPE,
            EVENT_TOOL_PATH_BOUNDARY_CHECKED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
            }),
        ))?;

        let event = match observation.status {
            ObservationStatus::Succeeded => EVENT_TOOL_EXECUTION_SUCCEEDED,
            ObservationStatus::Failed => EVENT_TOOL_EXECUTION_FAILED,
        };
        self.logger.llm(LogEvent::ui(
            TOOL_EXPLORE_SCOPE,
            event,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "preview_lines": observation.preview.len(),
                "total_lines": observation.total_lines,
                "total_bytes": observation.total_bytes,
                "truncated": observation.truncated,
                "source_truncated": observation.source_truncated,
                "preview_truncated": observation.preview_truncated,
                "artifact_path": &observation.artifact_path,
                "next_range_hint": &observation.next_range_hint,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
                "message": &observation.message,
            }),
        ))
    }

    fn log_tool_observation_recorded(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_EXPLORE_SCOPE,
            EVENT_TOOL_OBSERVATION_RECORDED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "preview_lines": observation.preview.len(),
                "total_lines": observation.total_lines,
                "total_bytes": observation.total_bytes,
                "truncated": observation.truncated,
                "source_truncated": observation.source_truncated,
                "preview_truncated": observation.preview_truncated,
                "artifact_path": &observation.artifact_path,
                "next_range_hint": &observation.next_range_hint,
            }),
        ))
    }

    fn log_tool_observation_attached(
        &self,
        active: &ActivePlainRequest,
        message: &LlmMessage,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_03_SCOPE,
            EVENT_TOOL_OBSERVATION_ATTACHED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "message_role": message.role.as_str(),
                "message_visibility": message.visibility.as_str(),
                "content_chars": message.content.chars().count(),
                "tool_call_count": active.tool_call_count,
                "same_tool_repeat_count": active.same_tool_repeat_count,
            }),
        ))
    }

    fn log_change_tool_call_received(
        &self,
        active: &ActivePlainRequest,
        tool_name: &str,
        arguments: &Value,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_06_SCOPE,
            EVENT_TOOL_CALL_RECEIVED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "activity": Activity::Change.as_str(),
                "tool_name": tool_name,
                "argument_keys": argument_keys(arguments),
                "arguments_redacted": redacted_tool_arguments(tool_name, arguments),
            }),
        ))
    }

    fn log_change_execution_started(
        &self,
        active: &ActivePlainRequest,
        tool_name: &str,
        arguments: &Value,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_06_SCOPE,
            EVENT_TOOL_EXECUTION_STARTED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": tool_name,
                "activity": Activity::Change.as_str(),
                "workspace_root": self.tool_runtime.workspace_root().display().to_string(),
                "arguments_redacted": redacted_tool_arguments(tool_name, arguments),
            }),
        ))
    }

    fn log_change_observation(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        let event = match observation.status {
            ObservationStatus::Succeeded => EVENT_TOOL_EXECUTION_SUCCEEDED,
            ObservationStatus::Failed => EVENT_TOOL_EXECUTION_FAILED,
        };
        self.logger.llm(LogEvent::ui(
            TOOL_06_SCOPE,
            event,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "preview_lines": observation.preview.len(),
                "total_lines": observation.total_lines,
                "total_bytes": observation.total_bytes,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
                "message": &observation.message,
            }),
        ))
    }

    fn log_change_observation_recorded(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_06_SCOPE,
            EVENT_TOOL_OBSERVATION_RECORDED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
            }),
        ))
    }

    fn log_diagnostic_observation(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        let event = match observation.status {
            ObservationStatus::Succeeded => EVENT_TOOL_EXECUTION_SUCCEEDED,
            ObservationStatus::Failed => EVENT_TOOL_EXECUTION_FAILED,
        };
        self.logger.llm(LogEvent::ui(
            TOOL_07_SCOPE,
            event,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "preview_lines": observation.preview.len(),
                "total_lines": observation.total_lines,
                "total_bytes": observation.total_bytes,
                "truncated": observation.truncated,
                "preview_truncated": observation.preview_truncated,
                "artifact_path": &observation.artifact_path,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
                "message": &observation.message,
            }),
        ))
    }

    fn log_diagnostic_observation_recorded(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_07_SCOPE,
            EVENT_TOOL_OBSERVATION_RECORDED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
                "artifact_path": &observation.artifact_path,
            }),
        ))
    }

    fn log_command_tool_call_received(
        &self,
        active: &ActivePlainRequest,
        tool_name: &str,
        arguments: &Value,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_08_SCOPE,
            EVENT_TOOL_CALL_RECEIVED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "activity": Activity::Execute.as_str(),
                "tool_name": tool_name,
                "argument_keys": argument_keys(arguments),
                "arguments_redacted": redacted_tool_arguments(tool_name, arguments),
            }),
        ))
    }

    fn log_command_execution_started(
        &self,
        active: &ActivePlainRequest,
        tool_name: &str,
        arguments: &Value,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_08_SCOPE,
            EVENT_TOOL_EXECUTION_STARTED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": tool_name,
                "activity": Activity::Execute.as_str(),
                "workspace_root": self.tool_runtime.workspace_root().display().to_string(),
                "arguments_redacted": redacted_tool_arguments(tool_name, arguments),
            }),
        ))
    }

    fn log_command_observation(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        let event = match observation.status {
            ObservationStatus::Succeeded => EVENT_TOOL_EXECUTION_SUCCEEDED,
            ObservationStatus::Failed => EVENT_TOOL_EXECUTION_FAILED,
        };
        self.logger.llm(LogEvent::ui(
            TOOL_08_SCOPE,
            event,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "preview_lines": observation.preview.len(),
                "total_lines": observation.total_lines,
                "total_bytes": observation.total_bytes,
                "truncated": observation.truncated,
                "preview_truncated": observation.preview_truncated,
                "artifact_path": &observation.artifact_path,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
                "message": &observation.message,
            }),
        ))
    }

    fn log_command_observation_recorded(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_08_SCOPE,
            EVENT_TOOL_OBSERVATION_RECORDED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
                "artifact_path": &observation.artifact_path,
            }),
        ))
    }

    fn log_web_tool_call_received(
        &self,
        active: &ActivePlainRequest,
        tool_name: &str,
        arguments: &Value,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_09_SCOPE,
            EVENT_TOOL_CALL_RECEIVED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "activity": Activity::Explore.as_str(),
                "tool_name": tool_name,
                "argument_keys": argument_keys(arguments),
                "arguments_redacted": redacted_tool_arguments(tool_name, arguments),
            }),
        ))
    }

    fn log_web_execution_started(
        &self,
        active: &ActivePlainRequest,
        tool_name: &str,
        arguments: &Value,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_09_SCOPE,
            EVENT_TOOL_EXECUTION_STARTED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": tool_name,
                "activity": Activity::Explore.as_str(),
                "web_enabled": self.runtime_config.web.enabled,
                "arguments_redacted": redacted_tool_arguments(tool_name, arguments),
            }),
        ))
    }

    fn log_web_observation(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        let event = match observation.status {
            ObservationStatus::Succeeded => EVENT_TOOL_EXECUTION_SUCCEEDED,
            ObservationStatus::Failed => EVENT_TOOL_EXECUTION_FAILED,
        };
        self.logger.llm(LogEvent::ui(
            TOOL_09_SCOPE,
            event,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "preview_lines": observation.preview.len(),
                "total_lines": observation.total_lines,
                "total_bytes": observation.total_bytes,
                "truncated": observation.truncated,
                "preview_truncated": observation.preview_truncated,
                "artifact_path": &observation.artifact_path,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
                "message": &observation.message,
            }),
        ))
    }

    fn log_web_observation_recorded(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_09_SCOPE,
            EVENT_TOOL_OBSERVATION_RECORDED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
                "artifact_path": &observation.artifact_path,
            }),
        ))
    }

    fn log_external_path_observation(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_10_SCOPE,
            EVENT_TOOL_EXECUTION_FAILED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "target_resolved": &observation.target_resolved,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
                "message": &observation.message,
            }),
        ))
    }

    fn log_external_path_observation_recorded(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_10_SCOPE,
            EVENT_TOOL_OBSERVATION_RECORDED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
                "target_raw": &observation.target_raw,
                "error_kind": observation.error_kind.map(|kind| kind.as_str()),
            }),
        ))
    }

    fn log_tool_loop_request_started(
        &self,
        active: &ActivePlainRequest,
        next_turn_id: &str,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_03_SCOPE,
            EVENT_TOOL_LOOP_REQUEST_STARTED,
            json!({
                "run_id": &active.run_id,
                "previous_turn_id": &active.turn_id,
                "next_turn_id": next_turn_id,
                "tool_call_count": active.tool_call_count,
                "same_tool_repeat_count": active.same_tool_repeat_count,
            }),
        ))
    }

    fn log_tool_loop_duplicate_redirected(
        &self,
        active: &ActivePlainRequest,
        next_turn_id: &str,
        signature: &str,
        reason: &str,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_03_SCOPE,
            EVENT_TOOL_LOOP_DUPLICATE_REDIRECTED,
            json!({
                "run_id": &active.run_id,
                "previous_turn_id": &active.turn_id,
                "next_turn_id": next_turn_id,
                "signature": signature,
                "tool_call_count": active.tool_call_count,
                "same_tool_repeat_count": active.same_tool_repeat_count,
                "reason": reason,
            }),
        ))
    }

    fn log_tool_loop_limit_reached(
        &self,
        active: &ActivePlainRequest,
        reason: &str,
        actual: u16,
        limit: u16,
        signature: &str,
        diagnosis: &str,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_03_SCOPE,
            EVENT_TOOL_LOOP_LIMIT_REACHED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "reason": reason,
                "actual": actual,
                "limit": limit,
                "signature": signature,
                "diagnosis": diagnosis,
                "tool_call_count": active.tool_call_count,
                "same_tool_repeat_count": active.same_tool_repeat_count,
                "last_observation": active.last_tool_observation.as_ref().map(|observation| json!({
                    "tool_name": observation.tool_name,
                    "target_raw": observation.target_raw,
                    "status": observation.status,
                    "error_kind": observation.error_kind,
                    "truncated": observation.truncated,
                    "source_truncated": observation.source_truncated,
                    "preview_truncated": observation.preview_truncated,
                    "has_next_range_hint": observation.has_next_range_hint,
                })),
            }),
        ))
    }

    fn log_tool_workspace_summary_rendered(
        &self,
        active: &ActivePlainRequest,
        observation: &ToolObservation,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            TOOL_EXPLORE_SCOPE,
            EVENT_TOOL_WORKSPACE_SUMMARY_RENDERED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "tool_name": &observation.tool_name,
                "status": observation.status.as_str(),
            }),
        ))
    }

    fn log_runtime_decision_failed(
        &self,
        active: &ActivePlainRequest,
        parsed: &ParsedRuntimeResponse,
        error: &RuntimeDecisionError,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_08_SCOPE,
            EVENT_RUNTIME_DECISION_FAILED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "error_kind": error.kind.as_str(),
                "message": &error.message,
                "recoverable": true,
                "tool_candidate": parsed.tool_candidate_for_logging().map(|candidate| json!({
                    "tool_name": candidate.tool_name,
                    "argument_keys": argument_keys(candidate.arguments),
                    "arguments_redacted": redacted_tool_arguments(candidate.tool_name, candidate.arguments),
                })),
            }),
        ))
    }

    fn log_runtime_process_started(&self, run_id: &str, turn_id: &str) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_09_SCOPE,
            EVENT_WORKING_PROCESS_STARTED,
            json!({
                "run_id": run_id,
                "turn_id": turn_id,
                "phase": WorkingPhase::Interpret.label(),
                "step": WorkingPhase::Interpret.number(),
            }),
        ))
    }

    fn log_runtime_process_phase_changed(
        &self,
        phase: WorkingPhase,
        detail: &str,
    ) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_09_SCOPE,
            EVENT_WORKING_PROCESS_PHASE_CHANGED,
            json!({
                "phase": phase.label(),
                "step": phase.number(),
                "detail": detail,
            }),
        ))
    }

    fn log_runtime_process_cancelled(&self, active: &ActivePlainRequest) -> io::Result<()> {
        self.logger.llm(LogEvent::ui(
            LLM_09_SCOPE,
            EVENT_WORKING_PROCESS_CANCELLED,
            json!({
                "run_id": &active.run_id,
                "turn_id": &active.turn_id,
                "reason": "canceled",
            }),
        ))
    }

    fn log_runtime_process_completed(&self, runtime_ids: Option<(&str, &str)>) -> io::Result<()> {
        let (run_id, turn_id) = runtime_ids.unwrap_or(("", ""));
        self.logger.llm(LogEvent::ui(
            LLM_09_SCOPE,
            EVENT_WORKING_PROCESS_COMPLETED,
            json!({
                "run_id": run_id,
                "turn_id": turn_id,
                "reason": "completed",
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
                "turn_id": &active.turn_id,
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
                "turn_id": &active.turn_id,
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
                "turn_id": &active.turn_id,
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
        event_log::log_command_events(
            &self.logger,
            self.state.scene.as_str(),
            &self.command_registry,
            events,
        )
    }

    fn log_approval_events(&self, events: &[ApprovalInputEvent]) -> io::Result<()> {
        event_log::log_approval_events(&self.logger, self.state.scene.as_str(), events)
    }

    fn log_working_process_events(&self, events: &[WorkingProcessEvent]) -> io::Result<()> {
        event_log::log_working_process_events(&self.logger, self.state.scene.as_str(), events)
    }

    fn log_workspace_events(&self, events: &[WorkspaceEvent]) -> io::Result<()> {
        event_log::log_workspace_events(&self.logger, self.state.scene.as_str(), events)
    }

    fn log_workspace_render_if_pending(&mut self) -> io::Result<()> {
        let Some(rendered) = self.state.take_workspace_render_event() else {
            return Ok(());
        };

        self.log_workspace_rendered(rendered)
    }

    fn log_workspace_rendered(&self, rendered: WorkspaceRendered) -> io::Result<()> {
        event_log::log_workspace_rendered(&self.logger, rendered)
    }

    fn log_persona_events(&self, events: &[PersonaEvent]) -> io::Result<()> {
        event_log::log_persona_events(&self.logger, self.state.scene.as_str(), events)
    }

    fn log_persona_render_if_pending(&mut self) -> io::Result<()> {
        let Some(rendered) = self.state.take_persona_render_event() else {
            return Ok(());
        };

        self.log_persona_message_rendered(rendered)
    }

    fn log_persona_message_rendered(&self, rendered: PersonaRendered) -> io::Result<()> {
        event_log::log_persona_message_rendered(&self.logger, rendered)
    }

    fn log_expanded_form_events(&self, events: &[ExpandedFormEvent]) -> io::Result<()> {
        event_log::log_expanded_form_events(&self.logger, events)
    }
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

fn argument_keys(arguments: &serde_json::Value) -> Vec<String> {
    arguments
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

fn approval_signature(decision: &RuntimeDecision) -> String {
    match decision {
        RuntimeDecision::ApprovalNeeded {
            tool_name,
            arguments,
            change_preview: Some(preview),
            ..
        } if tool_name == "apply_patch" => change_approval_signature(tool_name, arguments, preview),
        RuntimeDecision::ApprovalNeeded {
            tool_name,
            arguments,
            ..
        } => tool_signature(tool_name, arguments),
        _ => "non-approval".to_owned(),
    }
}

fn change_approval_signature(
    tool_name: &str,
    arguments: &serde_json::Value,
    preview: &ChangePreview,
) -> String {
    if tool_name != "apply_patch" {
        return tool_signature(tool_name, arguments);
    }
    format!(
        "apply_patch:target={}:operation={}:payload_hash={:016x}",
        preview.target_path,
        preview.operation.as_str(),
        stable_signature_hash(&preview.payload_body)
    )
}

fn stable_signature_hash(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn tool_signature(tool_name: &str, arguments: &serde_json::Value) -> String {
    format!("{tool_name}:{}", arguments)
}

fn decision_change_preview(decision: &RuntimeDecision) -> Option<ChangePreview> {
    match decision {
        RuntimeDecision::ApprovalNeeded { change_preview, .. } => change_preview.clone(),
        _ => None,
    }
}

fn command_approval_parts(decision: &RuntimeDecision) -> Option<(String, Value)> {
    match decision {
        RuntimeDecision::ApprovalNeeded {
            activity: Activity::Execute,
            tool_name,
            arguments,
            ..
        } if tool_name == "run_command" => Some((tool_name.clone(), arguments.clone())),
        _ => None,
    }
}

fn command_cwd_for_observation(arguments: &Value) -> Option<String> {
    arguments
        .as_object()
        .and_then(|object| object.get("cwd"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn web_approval_parts(decision: &RuntimeDecision) -> Option<(String, Value)> {
    match decision {
        RuntimeDecision::ToolCandidatePending {
            activity: Activity::Explore,
            tool_name,
            arguments,
            ..
        } if is_web_tool_name(tool_name) => Some((tool_name.clone(), arguments.clone())),
        _ => None,
    }
}

fn is_web_tool_name(tool_name: &str) -> bool {
    matches!(
        ToolName::parse(tool_name),
        Some(ToolName::WebSearch | ToolName::WebFetch)
    )
}

fn web_target_for_observation(tool_name: &str, arguments: &Value) -> Option<String> {
    match ToolName::parse(tool_name) {
        Some(ToolName::WebFetch) => arguments
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Some(ToolName::WebSearch) => arguments
            .get("query")
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

fn external_path_denial_reason(denial: &PermissionDenial) -> bool {
    matches!(
        denial.reason.as_str(),
        "external_path_manual_only" | "sensitive_external_path"
    )
}

fn decision_tool_parts_for_denial(
    decision: &RuntimeDecision,
) -> Option<(String, Value, Option<String>)> {
    match decision {
        RuntimeDecision::ToolCandidatePending {
            tool_name,
            arguments,
            ..
        } => Some((
            tool_name.clone(),
            arguments.clone(),
            path_target_for_observation(tool_name, arguments, None),
        )),
        RuntimeDecision::ApprovalNeeded {
            tool_name,
            arguments,
            change_preview,
            ..
        } => Some((
            tool_name.clone(),
            arguments.clone(),
            path_target_for_observation(tool_name, arguments, change_preview.as_ref()),
        )),
        _ => None,
    }
}

fn path_target_for_observation(
    tool_name: &str,
    arguments: &Value,
    change_preview: Option<&ChangePreview>,
) -> Option<String> {
    match ToolName::parse(tool_name) {
        Some(ToolName::ListFiles | ToolName::SearchText | ToolName::ReadFile) => arguments
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Some(ToolName::RunCommand) => arguments
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Some(ToolName::ApplyPatch) => change_preview.map(|preview| preview.target_path.clone()),
        _ => None,
    }
}

fn approval_result(events: &[ApprovalInputEvent]) -> Option<ApprovalResult> {
    events.iter().find_map(|event| match event {
        ApprovalInputEvent::ResultRecorded { result } => Some(*result),
        _ => None,
    })
}

fn persona_event_for_runtime_response_parse_error(
    error: &RuntimeResponseParseError,
) -> PersonaRuntimeEvent {
    match error.kind {
        RuntimeResponseParseErrorKind::JsonParseFailed => {
            PersonaRuntimeEvent::RuntimeResponseParseFailed
        }
        RuntimeResponseParseErrorKind::SchemaValidationFailed
        | RuntimeResponseParseErrorKind::PayloadValidationFailed
        | RuntimeResponseParseErrorKind::PartialResponse => {
            PersonaRuntimeEvent::SchemaValidationFailed
        }
    }
}

fn persona_event_for_tool_observation(observation: &ToolObservation) -> PersonaRuntimeEvent {
    match observation.status {
        ObservationStatus::Succeeded => PersonaRuntimeEvent::ToolExecutionSucceeded,
        ObservationStatus::Failed => PersonaRuntimeEvent::ToolExecutionFailed,
    }
}

fn persona_turn_prompt_messages(prompt: PersonaTurnPrompt) -> Vec<LlmMessage> {
    vec![
        LlmMessage {
            turn_id: prompt.turn_id.clone(),
            role: LlmMessageRole::System,
            visibility: LlmMessageVisibility::Internal,
            content: prompt.system_prompt,
        },
        LlmMessage {
            turn_id: prompt.turn_id,
            role: LlmMessageRole::User,
            visibility: LlmMessageVisibility::UserVisible,
            content: prompt.user_prompt,
        },
    ]
}

fn conversation_context_message(frame: &ConversationTaskFrame) -> String {
    format!(
        "<AHREUM_CONVERSATION_CONTEXT>\nprevious_task_user_request:\n{previous_prompt}\n\nprevious_task_state_summary:\n{previous_summary}\n\nprevious_runtime_context:\n{runtime_context}\n\ninstruction: The current user message may be a follow-up, retry, correction, complaint, or an unrelated new task. Decide from the current message semantics. Use previous_task only when the current message depends on it; otherwise ignore it. A previous task state that was limited, failed, canceled, or blocked is not completed evidence. Do not invent file contents, extracted values, package names, versions, path candidates, or successful execution facts from this context.\n</AHREUM_CONVERSATION_CONTEXT>",
        previous_prompt = frame.user_prompt,
        previous_summary = frame.task_state_summary,
        runtime_context = frame.runtime_context_summary,
    )
}

fn persona_follow_up_task_state_summary(
    frame: &ConversationTaskFrame,
    current_prompt: &str,
) -> String {
    format!(
        "follow_up_policy: current user message may be a follow-up, retry, correction, complaint, or unrelated new task; relate it to the previous task only when the message depends on it.\nprevious_user_request: {previous_prompt}\nprevious_task_state_summary:\n{previous_summary}\ncurrent_user_message: {current_prompt}\nfact_boundary: persona must not state literal paths, filenames, config keys, extracted values, file contents, package names, provider names, versions, or configuration values; those belong only in the main answer.",
        previous_prompt = frame.user_prompt,
        previous_summary = frame.task_state_summary,
        current_prompt = current_prompt,
    )
}

fn persona_task_state_summary(
    active: &ActivePlainRequest,
    runtime_status: &str,
    detail: &str,
) -> String {
    let mut lines = vec![
        format!("main_runtime_status: {runtime_status}"),
        format!("detail: {detail}"),
        format!("tool_call_count: {}", active.tool_call_count),
    ];

    if let Some(observation) = active.last_tool_observation.as_ref() {
        lines.push(format!(
            "last_observation: tool_name={} status={} target_raw={} error_kind={}",
            observation.tool_name,
            observation.status,
            observation.target_raw.as_deref().unwrap_or("-"),
            observation.error_kind.unwrap_or("-"),
        ));
        lines.push("fact_boundary: persona must not state literal paths, filenames, config keys, extracted values, file contents, package names, provider names, versions, or configuration values; those belong only in the main answer.".to_owned());
        if observation.status != "succeeded" || observation.error_kind.is_some() {
            lines.push("fact_boundary: last observation is not successful evidence; persona must not claim file contents or completed analysis from it.".to_owned());
        }
    } else {
        lines.push("last_observation: none".to_owned());
        lines.push("fact_boundary: no observation is available to persona; do not state literal paths, filenames, config keys, extracted values, file contents, package names, provider names, versions, or configuration values.".to_owned());
    }

    lines.join("\n")
}

fn should_finalize_duplicate_approval_redirect(
    redirect: runtime_request::ToolLoopRepeatRedirect,
    duplicate_redirect_count: u16,
    max_same_tool_repeats: u16,
) -> bool {
    let was_already_redirected = duplicate_redirect_count > 0;
    match redirect {
        runtime_request::ToolLoopRepeatRedirect::SettledDuplicate => was_already_redirected,
        runtime_request::ToolLoopRepeatRedirect::FailedDuplicate => was_already_redirected,
        runtime_request::ToolLoopRepeatRedirect::TruncatedContinuation => {
            duplicate_redirect_count >= max_same_tool_repeats
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn persona_request_state_marks_literal_values_as_main_answer_only() {
        let active = crate::tui::runtime_request::ActivePlainRequest::new(
            "run-0001".to_owned(),
            "turn-0001".to_owned(),
            "USER_REQUEST_MARKER".to_owned(),
            crate::llm::LlmMessage {
                turn_id: "turn-0001".to_owned(),
                role: crate::llm::LlmMessageRole::System,
                visibility: crate::llm::LlmMessageVisibility::Internal,
                content: "schema".to_owned(),
            },
            crate::llm::LlmMessage {
                turn_id: "turn-0001".to_owned(),
                role: crate::llm::LlmMessageRole::User,
                visibility: crate::llm::LlmMessageVisibility::UserVisible,
                content: "USER_REQUEST_MARKER".to_owned(),
            },
            crate::llm::MessageHistory::new("run-0001"),
            std::sync::mpsc::channel().1,
            0,
        );

        let summary = super::persona_task_state_summary(&active, "completed", "done");

        assert!(summary.contains("literal paths"));
        assert!(summary.contains("filenames"));
        assert!(summary.contains("last_observation: none"));
    }

    #[test]
    fn failed_duplicate_approval_finalizes_after_one_redirect() {
        assert!(!super::should_finalize_duplicate_approval_redirect(
            crate::tui::runtime_request::ToolLoopRepeatRedirect::FailedDuplicate,
            0,
            8,
        ));
        assert!(super::should_finalize_duplicate_approval_redirect(
            crate::tui::runtime_request::ToolLoopRepeatRedirect::FailedDuplicate,
            1,
            8,
        ));
    }

    #[test]
    fn conversation_context_message_keeps_previous_task_separate_from_current_prompt() {
        let frame = super::ConversationTaskFrame {
            user_prompt: "이전 HTML 작업".to_owned(),
            task_state_summary: "main_runtime_status: tool_loop_limited".to_owned(),
            runtime_context_summary: "previous_final_response: none".to_owned(),
        };

        let message = super::conversation_context_message(&frame);

        assert!(message.contains("<AHREUM_CONVERSATION_CONTEXT>"));
        assert!(message.contains("previous_task_user_request"));
        assert!(message.contains("이전 HTML 작업"));
        assert!(message.contains("Use previous_task only when the current message depends on it"));
        assert!(message.contains("otherwise ignore it"));
        assert!(message.contains("previous_runtime_context"));
        assert!(!message.contains("중간에 끊겼는데"));
    }

    #[test]
    fn persona_follow_up_summary_contains_prior_frame_and_current_message_without_deciding_relation(
    ) {
        let frame = super::ConversationTaskFrame {
            user_prompt: "이전 설정 분석".to_owned(),
            task_state_summary: "main_runtime_status: completed\nlast_observation: none".to_owned(),
            runtime_context_summary: "previous_final_response: config path noted".to_owned(),
        };

        let summary =
            super::persona_follow_up_task_state_summary(&frame, "아까 하던 것만 이어서 봐줘");

        assert!(summary.contains("previous_user_request: 이전 설정 분석"));
        assert!(summary.contains("current_user_message: 아까 하던 것만 이어서 봐줘"));
        assert!(
            summary.contains("relate it to the previous task only when the message depends on it")
        );
        assert!(summary.contains("fact_boundary"));
    }
}

fn plain_request_timeout_report(
    config: &RuntimeConfig,
    active: &ActivePlainRequest,
) -> LlmChatReport {
    LlmChatReport {
        provider: config.provider.active.clone(),
        base_url: config.provider.base_url.clone(),
        model: config.provider.model.clone(),
        chat_url: format!(
            "{}/chat/completions",
            config.provider.base_url.trim_end_matches('/')
        ),
        latency_ms: active.request_started_at.elapsed().as_millis(),
        status: LlmChatStatus::Failed(LlmChatFailure::new(
            ChatFailureKind::Timeout,
            "local LLM request exceeded the UI request timeout boundary",
        )),
    }
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
