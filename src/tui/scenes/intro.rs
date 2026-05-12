use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::product;

use super::super::components::wordmark;
use super::super::state::TuiState;
use super::super::style;

pub fn render_intro(frame: &mut Frame<'_>, state: &TuiState) {
    let area = frame.area();
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    render_intro_body(frame, root[0], state);
    render_statusline(frame, root[1], state);
}

pub fn handle_intro_event(event: KeyEvent, state: &mut TuiState) {
    match event.code {
        KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
        }
        KeyCode::Esc => {
            state.should_quit = true;
        }
        KeyCode::Enter => {
            let input = state.intro_input.trim();
            if input == "/exit" || input == "/quit" {
                state.should_quit = true;
            }
        }
        KeyCode::Backspace => {
            state.intro_input.pop();
        }
        KeyCode::Char(value) => {
            state.intro_input.push(value);
        }
        _ => {}
    }
}

fn render_intro_body(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let content_height = 17.min(area.height);
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
            Constraint::Length(2),
        ])
        .split(centered);

    render_logo(frame, chunks[0]);
    render_prompt_panel(frame, chunks[2], state);
    render_intro_hint(frame, chunks[3]);
}

fn render_logo(frame: &mut Frame<'_>, area: Rect) {
    let paragraph = Paragraph::new(wordmark::lines());
    frame.render_widget(paragraph, area);
}

fn render_prompt_panel(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
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
            Span::styled(product::DEFAULT_MODE, style::cyan()),
            Span::raw("     "),
            Span::styled(product::DEFAULT_PROVIDER_DISPLAY, style::panel()),
            Span::styled(" / ", style::muted()),
            Span::styled(product::DEFAULT_MODEL, style::panel()),
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

fn render_statusline(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let status = &state.runtime_status;
    let text = format!(
        "{} · {}/{} · {} · {} · {} · {} · {}",
        status.mode,
        status.provider,
        status.model,
        status.workspace,
        status.context,
        status.tokens,
        status.web,
        status.runtime_state
    );

    let paragraph =
        Paragraph::new(fit_to_width(&text, area.width as usize)).style(style::statusline());
    frame.render_widget(paragraph, area);
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

fn fit_to_width(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }

    if width <= 3 {
        return ".".repeat(width);
    }

    let target = width - 3;
    let mut output = String::new();
    let mut current_width = 0;

    for value in text.chars() {
        let value_width = value.to_string().width();
        if current_width + value_width > target {
            break;
        }
        current_width += value_width;
        output.push(value);
    }

    output.push_str("...");
    output
}
