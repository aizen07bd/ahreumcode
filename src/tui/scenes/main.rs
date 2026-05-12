use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::product;

use super::super::components::statusline;
use super::super::state::TuiState;
use super::super::style;

pub fn render_main(frame: &mut Frame<'_>, state: &TuiState) {
    let area = frame.area();
    let layout = layout_main(area);

    render_header(frame, layout.header);
    render_workspace(frame, layout.workspace);
    render_prompt_composer(frame, layout.prompt, state);
    statusline::render_statusline(frame, layout.statusline, &state.runtime_status);
}

pub fn handle_main_event(event: KeyEvent, state: &mut TuiState) {
    match event.code {
        KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
        }
        KeyCode::Esc => {
            state.should_quit = true;
        }
        KeyCode::Backspace => {
            state.main_input.pop();
        }
        KeyCode::Char(value) => {
            state.main_input.push(value);
        }
        _ => {}
    }
}

struct MainLayout {
    header: Rect,
    workspace: Rect,
    prompt: Rect,
    statusline: Rect,
}

fn layout_main(area: Rect) -> MainLayout {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    MainLayout {
        header: root[0],
        workspace: root[1],
        prompt: root[2],
        statusline: root[3],
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
