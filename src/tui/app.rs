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

use super::scenes::intro::{handle_intro_event, render_intro};
use super::state::{Scene, TuiState};

const TUI_SCOPE: &str = "tui-01-intro-scene";
const EVENT_APP_STARTED: &str = "app_started";
const EVENT_TERMINAL_ENTERED: &str = "terminal_entered";
const EVENT_INTRO_RENDERED: &str = "intro_rendered";
const EVENT_PROMPT_FOCUS_READY: &str = "prompt_focus_ready";

pub fn run_app(command: AppCommand) -> io::Result<()> {
    match (command.scene, command.run_mode) {
        (SceneCommand::Intro, RunMode::Smoke) => run_intro_smoke(command),
        (SceneCommand::Intro, _) => run_intro_terminal(command),
    }
}

fn run_intro_terminal(command: AppCommand) -> io::Result<()> {
    let workspace = current_workspace()?;
    let logger = Logger::start()?;
    logger.ui(LogEvent::ui(
        TUI_SCOPE,
        EVENT_APP_STARTED,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;

    let mut terminal = TerminalSession::enter()?;
    logger.ui(LogEvent::ui(
        TUI_SCOPE,
        EVENT_TERMINAL_ENTERED,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;

    let mut app = TuiApp::new(logger, workspace);
    app.run(terminal.terminal_mut())?;
    terminal.restore()
}

fn run_intro_smoke(command: AppCommand) -> io::Result<()> {
    let workspace = current_workspace()?;
    let logger = Logger::start()?;
    logger.ui(LogEvent::ui(
        TUI_SCOPE,
        EVENT_APP_STARTED,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;

    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend)?;
    let state = TuiState::intro(workspace);

    logger.ui(LogEvent::ui(
        TUI_SCOPE,
        EVENT_PROMPT_FOCUS_READY,
        json!({ "run_mode": command.run_mode.as_str() }),
    ))?;
    terminal.draw(|frame| render_intro(frame, &state))?;
    logger.ui(LogEvent::ui(
        TUI_SCOPE,
        EVENT_INTRO_RENDERED,
        json!({ "run_mode": command.run_mode.as_str(), "backend": "test" }),
    ))?;

    println!("tui-01 intro smoke ok");
    println!("scene=intro");
    println!("run_mode={}", command.run_mode.as_str());
    println!("log_dir={}", logger.session_dir().display());

    Ok(())
}

struct TuiApp {
    state: TuiState,
    logger: Logger,
    intro_render_logged: bool,
}

impl TuiApp {
    fn new(logger: Logger, workspace: String) -> Self {
        Self {
            state: TuiState::intro(workspace),
            logger,
            intro_render_logged: false,
        }
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        self.logger
            .ui(LogEvent::ui(TUI_SCOPE, EVENT_PROMPT_FOCUS_READY, json!({})))?;

        while !self.state.should_quit {
            terminal.draw(|frame| match self.state.scene {
                Scene::Intro => render_intro(frame, &self.state),
            })?;

            if !self.intro_render_logged {
                self.logger
                    .ui(LogEvent::ui(TUI_SCOPE, EVENT_INTRO_RENDERED, json!({})))?;
                self.intro_render_logged = true;
            }

            if event::poll(Duration::from_millis(100))? {
                let Event::Key(key_event) = event::read()? else {
                    continue;
                };
                match self.state.scene {
                    Scene::Intro => handle_intro_event(key_event, &mut self.state),
                }
            }
        }

        Ok(())
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
