use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::super::persona::{PersonaBuffer, PersonaMessage, PersonaSpeakerRole};
use super::super::style;

pub fn render_persona_panel(frame: &mut Frame<'_>, area: Rect, persona: &PersonaBuffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(style::muted())
        .title(Span::styled(" Persona ", style::panel_bold()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = persona_lines(persona, inner.width);
    frame.render_widget(Paragraph::new(lines), inner);
}

fn persona_lines(persona: &PersonaBuffer, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for message in persona.messages() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        render_persona_message(&mut lines, message, width);
    }

    lines
}

fn render_persona_message(lines: &mut Vec<Line<'static>>, message: &PersonaMessage, width: u16) {
    match message.role {
        PersonaSpeakerRole::Lead => render_lead_message(lines, message, width),
        PersonaSpeakerRole::Member => render_member_message(lines, message, width),
    }
}

fn render_lead_message(lines: &mut Vec<Line<'static>>, message: &PersonaMessage, width: u16) {
    lines.push(Line::from(vec![
        Span::styled(format!("[{}]", message.speaker), style::persona_lead()),
        Span::styled(format!(" {}", message.time_label), style::persona_time()),
    ]));

    let body_width = width.saturating_sub(2).max(1) as usize;
    for (index, part) in wrap_text(&message.body, body_width).into_iter().enumerate() {
        if index == 0 {
            lines.push(Line::from(vec![
                Span::styled("│ ", style::persona_accent()),
                Span::styled(part, style::panel()),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(part, style::panel()),
            ]));
        }
    }
}

fn render_member_message(lines: &mut Vec<Line<'static>>, message: &PersonaMessage, width: u16) {
    lines.push(Line::from(vec![
        Span::styled(message.speaker.clone(), style::persona_speaker()),
        Span::styled(format!(" {}", message.time_label), style::persona_time()),
    ]));

    let body_width = width.saturating_sub(2).max(1) as usize;
    for part in wrap_text(&message.body, body_width) {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(part, style::panel()),
        ]));
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 || text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;

    for value in text.chars() {
        let value_width = value.to_string().width();
        if current_width > 0 && current_width + value_width > width {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }

        current.push(value);
        current_width += value_width;
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}
