use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::product;

use super::super::command::{CommandInputOutcome, CommandRegistry};
use super::super::components::{command_surface, statusline, wordmark};
use super::super::persona::MIN_PERSONA_TERMINAL_WIDTH;
use super::super::state::TuiState;
use super::super::style;
use super::super::working_process::WorkingProcessEvents;
use super::super::workspace::WorkspaceEvents;
use super::prompt::handle_prompt_event;

pub struct IntroAction {
    pub command_outcome: CommandInputOutcome,
    pub working_process_events: WorkingProcessEvents,
    pub workspace_events: WorkspaceEvents,
}

pub fn render_intro(frame: &mut Frame<'_>, state: &TuiState, registry: &CommandRegistry) {
    let area = frame.area();
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    render_intro_body(frame, root[0], state, registry);
    statusline::render_statusline(frame, root[1], &state.runtime_status);
}

pub fn handle_intro_event(
    event: KeyEvent,
    state: &mut TuiState,
    registry: &CommandRegistry,
) -> IntroAction {
    if state.command_surface.open && !event.modifiers.contains(KeyModifiers::CONTROL) {
        let outcome = handle_prompt_event(
            event,
            &mut state.intro_input,
            &mut state.command_surface,
            registry,
            state.scene.as_str(),
            false,
        );
        let _ = state.apply_command_dispatch(outcome.dispatch, current_terminal_width());
        return IntroAction {
            command_outcome: outcome,
            working_process_events: WorkingProcessEvents::none(),
            workspace_events: WorkspaceEvents::none(),
        };
    }

    match event.code {
        KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
            IntroAction {
                command_outcome: CommandInputOutcome::none(),
                working_process_events: WorkingProcessEvents::none(),
                workspace_events: WorkspaceEvents::none(),
            }
        }
        KeyCode::Esc => IntroAction {
            command_outcome: CommandInputOutcome::none(),
            working_process_events: WorkingProcessEvents::none(),
            workspace_events: WorkspaceEvents::none(),
        },
        KeyCode::Enter => {
            let prompt_outcome = state.enter_main_with_prompt();
            IntroAction {
                command_outcome: CommandInputOutcome::none(),
                working_process_events: prompt_outcome.working_process_events,
                workspace_events: prompt_outcome.workspace_events,
            }
        }
        _ => {
            let outcome = handle_prompt_event(
                event,
                &mut state.intro_input,
                &mut state.command_surface,
                registry,
                state.scene.as_str(),
                false,
            );
            let _ = state.apply_command_dispatch(outcome.dispatch, current_terminal_width());
            IntroAction {
                command_outcome: outcome,
                working_process_events: WorkingProcessEvents::none(),
                workspace_events: WorkspaceEvents::none(),
            }
        }
    }
}

fn render_intro_body(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    registry: &CommandRegistry,
) {
    let command_height = if state.command_surface.open { 5 } else { 0 };
    let content_height = (17 + command_height).min(area.height);
    let vertical_margin = area.height.saturating_sub(content_height) / 2;
    let centered = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(vertical_margin),
            Constraint::Length(content_height),
            Constraint::Min(0),
        ])
        .split(area)[1];

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),
            Constraint::Length(0),
            Constraint::Length(5),
            Constraint::Length(command_height),
            Constraint::Length(2),
        ])
        .split(centered);

    render_logo(frame, chunks[0]);
    render_prompt_panel(frame, chunks[2], state);
    command_surface::render_command_surface(
        frame,
        centered_width(chunks[3], 84),
        &state.command_surface,
        registry,
        state.scene.as_str(),
        state.runtime_status.command_labels(),
    );
    render_intro_hint(frame, chunks[4]);
}

fn render_logo(frame: &mut Frame<'_>, area: Rect) {
    let paragraph = Paragraph::new(wordmark::lines());
    frame.render_widget(paragraph, area);
}

fn render_prompt_panel(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    const PROMPT_PREFIX_WIDTH: u16 = 4;

    let panel = centered_width(area, 84);
    let input_text = if state.intro_input.is_empty() {
        product::INTRO_PROMPT_PLACEHOLDER.to_owned()
    } else {
        state.intro_input.clone()
    };

    let input_style = if state.intro_input.is_empty() {
        style::muted()
    } else {
        style::panel()
    };

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  > ", style::panel()),
            Span::styled(input_text, input_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Mode: ", style::panel()),
            Span::styled(state.runtime_status.mode.clone(), style::cyan()),
            Span::raw("     "),
            Span::styled(
                state.runtime_status.provider_display.clone(),
                style::panel(),
            ),
            Span::styled(" / ", style::muted()),
            Span::styled(state.runtime_status.model.clone(), style::panel()),
        ]),
        Line::from(""),
    ];

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(style::cyan())
                .style(style::prompt_background()),
        )
        .style(style::prompt_background());

    frame.render_widget(paragraph, panel);

    let inner_x = panel.x.saturating_add(1);
    let input_width = state.intro_input.as_str().width() as u16;
    let cursor_x = inner_x
        .saturating_add(PROMPT_PREFIX_WIDTH)
        .saturating_add(input_width)
        .min(panel.right().saturating_sub(1));
    let cursor_y = panel
        .y
        .saturating_add(1)
        .min(panel.bottom().saturating_sub(1));
    frame.set_cursor_position(Position::new(cursor_x, cursor_y));
}

fn render_intro_hint(frame: &mut Frame<'_>, area: Rect) {
    let hint_area = centered_width(area, 84);
    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(product::INTRO_HEALTH_HINT, style::panel()),
        Span::styled(product::INTRO_HEALTH_HINT_TEXT, style::muted()),
        Span::raw("     "),
        Span::styled(product::INTRO_COMMAND_HINT, style::panel()),
        Span::styled(product::INTRO_COMMAND_HINT_TEXT, style::muted()),
    ]))
    .alignment(Alignment::Right);

    frame.render_widget(paragraph, hint_area);
}

fn centered_width(area: Rect, max_width: u16) -> Rect {
    let width = area.width.min(max_width);
    let horizontal_margin = area.width.saturating_sub(width) / 2;
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(horizontal_margin),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(area)[1]
}

fn current_terminal_width() -> u16 {
    let width = crossterm::terminal::size()
        .map(|(width, _)| width)
        .unwrap_or_default();
    if width > 0 {
        return width;
    }

    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|width| *width > 0)
        .unwrap_or(MIN_PERSONA_TERMINAL_WIDTH)
}
