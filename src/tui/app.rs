use std::io::{self, Stdout};
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
use crate::logging::{LogEvent, Logger};

use super::scenes::epilogue::print_epilogue;
use super::scenes::intro::{handle_intro_event, render_intro, IntroAction};
use super::state::{Scene, TuiState};

const TUI_01_SCOPE: &str = "tui-01-intro-scene";
const TUI_02_SCOPE: &str = "tui-02-epilogue-scene";
const EVENT_APP_STARTED: &str = "app_started";
const EVENT_TERMINAL_ENTERED: &str = "terminal_entered";
const EVENT_INTRO_RENDERED: &str = "intro_rendered";
const EVENT_PROMPT_FOCUS_READY: &str = "prompt_focus_ready";
const EVENT_EXIT_REQUESTED: &str = "exit_requested";
const EVENT_SESSION_SUMMARY_CREATED: &str = "session_summary_created";
const EVENT_EPILOGUE_RENDERED: &str = "epilogue_rendered";
const EVENT_TERMINAL_RESTORED: &str = "terminal_restored";

pub fn run_app(command: AppCommand) -> io::Result<()> {
    match (command.scene, command.run_mode) {
        (SceneCommand::Intro, RunMode::Smoke) => run_intro_smoke(command),
        (SceneCommand::Epilogue, RunMode::Smoke) => run_epilogue_smoke(command),
        (SceneCommand::Epilogue, _) => run_epilogue_terminal(command),
        (SceneCommand::Intro, _) => run_intro_terminal(command),
    }
}

fn run_intro_terminal(command: AppCommand) -> io::Result<()> {
    let workspace = current_workspace()?;
    let logger = Logger::start()?;
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

    let mut app = TuiApp::new(logger, workspace);
    app.run(terminal.terminal_mut())?;
    terminal.restore()?;
    app.log_terminal_restored()?;
    app.print_epilogue_after_restore()
}

fn run_intro_smoke(command: AppCommand) -> io::Result<()> {
    let workspace = current_workspace()?;
    let logger = Logger::start()?;
    logger.ui(LogEvent::ui(
        TUI_01_SCOPE,
        EVENT_APP_STARTED,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;

    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend)?;
    let state = TuiState::intro(workspace);

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

fn run_epilogue_terminal(command: AppCommand) -> io::Result<()> {
    let workspace = current_workspace()?;
    let logger = Logger::start()?;
    let app = TuiApp::new_epilogue(logger, workspace);

    app.log_exit_requested(command.run_mode.as_str(), "scene")?;
    app.log_session_summary_created()?;
    app.print_epilogue_after_restore()
}

fn run_epilogue_smoke(command: AppCommand) -> io::Result<()> {
    let workspace = current_workspace()?;
    let logger = Logger::start()?;
    let app = TuiApp::new_epilogue(logger, workspace);

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
    intro_render_logged: bool,
    terminal_restore_scope: Option<&'static str>,
}

impl TuiApp {
    fn new(logger: Logger, workspace: String) -> Self {
        Self {
            state: TuiState::intro(workspace),
            logger,
            intro_render_logged: false,
            terminal_restore_scope: None,
        }
    }

    fn new_epilogue(logger: Logger, workspace: String) -> Self {
        Self {
            state: TuiState::epilogue(workspace),
            logger,
            intro_render_logged: true,
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
            terminal.draw(|frame| match self.state.scene {
                Scene::Intro => render_intro(frame, &self.state),
                Scene::Epilogue => {}
            })?;

            if !self.intro_render_logged {
                self.logger
                    .ui(LogEvent::ui(TUI_01_SCOPE, EVENT_INTRO_RENDERED, json!({})))?;
                self.intro_render_logged = true;
            }

            if event::poll(Duration::from_millis(100))? {
                let Event::Key(key_event) = event::read()? else {
                    continue;
                };
                match self.state.scene {
                    Scene::Intro => {
                        let action = handle_intro_event(key_event, &mut self.state);
                        if matches!(action, IntroAction::ExitRequested) {
                            self.terminal_restore_scope = Some(TUI_02_SCOPE);
                            self.log_exit_requested("normal", "intro_prompt")?;
                            self.log_session_summary_created()?;
                        }
                    }
                    Scene::Epilogue => {}
                }
            }
        }

        Ok(())
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
}

fn current_workspace() -> io::Result<String> {
    Ok(std::env::current_dir()?.display().to_string())
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
