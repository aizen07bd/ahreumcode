use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::product;

use super::super::command::{CommandInputOutcome, CommandRegistry};
use super::super::components::{command_surface, statusline};
use super::super::state::TuiState;
use super::super::style;
use super::prompt::handle_prompt_event;

pub fn render_main(frame: &mut Frame<'_>, state: &TuiState) {
    let area = frame.area();
    let layout = layout_main(area, state.command_surface.open);

    render_header(frame, layout.header);
    render_workspace(frame, layout.workspace);
    render_prompt_composer(frame, layout.prompt, state);
    command_surface::render_command_surface(
        frame,
        layout.command,
        &state.command_surface,
        &CommandRegistry::new(),
    );
    statusline::render_statusline(frame, layout.statusline, &state.runtime_status);
}

pub fn handle_main_event(event: KeyEvent, state: &mut TuiState) -> CommandInputOutcome {
    if state.command_surface.open && !event.modifiers.contains(KeyModifiers::CONTROL) {
        let outcome = handle_prompt_event(event, &mut state.main_input, &mut state.command_surface);
        state.apply_command_dispatch(outcome.dispatch);
        return outcome;
    }

    match event.code {
        KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
            CommandInputOutcome::none()
        }
        KeyCode::Esc => {
            state.should_quit = true;
            CommandInputOutcome::none()
        }
        _ => {
            let outcome =
                handle_prompt_event(event, &mut state.main_input, &mut state.command_surface);
            state.apply_command_dispatch(outcome.dispatch);
            outcome
        }
    }
}

struct MainLayout {
    header: Rect,
    workspace: Rect,
    prompt: Rect,
    command: Rect,
    statusline: Rect,
}

fn layout_main(area: Rect, command_open: bool) -> MainLayout {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(command_height(area, command_open)),
            Constraint::Length(1),
        ])
        .split(area);

    MainLayout {
        header: root[0],
        workspace: root[1],
        prompt: root[2],
        command: root[3],
        statusline: root[4],
    }
}

fn command_height(area: Rect, command_open: bool) -> u16 {
    if command_open && area.height >= 12 {
        5
    } else {
        0
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect) {
    let version = product::version_label();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(product::APP_NAME, style::panel_bold()),
        Span::raw(" "),
        Span::styled(version, style::muted()),
    ]));
    frame.render_widget(header, rows[0]);

    let divider = Paragraph::new("─".repeat(area.width as usize)).style(style::muted());
    frame.render_widget(divider, rows[1]);
}

fn render_workspace(frame: &mut Frame<'_>, area: Rect) {
    let paragraph = Paragraph::new("");
    frame.render_widget(paragraph, area);
}

fn render_prompt_composer(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  > ", style::panel()),
            Span::styled(state.main_input.as_str(), style::panel()),
        ]),
        Line::from(""),
    ];

    let paragraph = Paragraph::new(lines)
        .block(Block::default().style(style::prompt_background()))
        .style(style::prompt_background());
    frame.render_widget(paragraph, area);
}
