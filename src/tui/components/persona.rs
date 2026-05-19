use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::super::persona::{PersonaBuffer, PersonaMessage, PersonaSpeaker, PersonaSpeakerRole};
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
    let visible_lines = visible_lines(lines, inner.height, persona.scroll_offset());
    frame.render_widget(Paragraph::new(visible_lines), inner);
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
    match message.role() {
        PersonaSpeakerRole::Lead => render_lead_message(lines, message, width),
        PersonaSpeakerRole::Member => render_member_message(lines, message, width),
    }
}

fn render_lead_message(lines: &mut Vec<Line<'static>>, message: &PersonaMessage, width: u16) {
    let mut label = speaker_label_spans(message.speaker);
    label.push(Span::styled(
        persona_time_label(message),
        style::persona_time(),
    ));
    lines.push(Line::from(label));

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
    let mut label = speaker_label_spans(message.speaker);
    label.push(Span::styled(
        persona_time_label(message),
        style::persona_time(),
    ));
    lines.push(Line::from(label));

    let body_width = width.saturating_sub(2).max(1) as usize;
    for part in wrap_text(&message.body, body_width) {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(part, style::panel()),
        ]));
    }
}

fn persona_time_label(message: &PersonaMessage) -> String {
    if message.repeat_count > 1 {
        format!(" {} x{}", message.time_label, message.repeat_count)
    } else {
        format!(" {}", message.time_label)
    }
}

fn speaker_label_spans(speaker: PersonaSpeaker) -> Vec<Span<'static>> {
    let (name_style, role_style) = speaker_label_styles(speaker);
    let mut spans = vec![Span::styled(speaker.name().to_owned(), name_style)];

    if let Some(role) = speaker.role_label() {
        spans.push(Span::styled(format!("({role})"), role_style));
    }

    spans
}

fn speaker_label_styles(speaker: PersonaSpeaker) -> (Style, Style) {
    let color = match speaker {
        PersonaSpeaker::Lead => Color::Cyan,
        PersonaSpeaker::Planning => Color::Magenta,
        PersonaSpeaker::Implementation => Color::Green,
        PersonaSpeaker::Verification => Color::Yellow,
        PersonaSpeaker::Documentation => Color::Blue,
    };

    (
        Style::default().fg(color).add_modifier(Modifier::BOLD),
        Style::default().fg(color),
    )
}

fn visible_lines(
    lines: Vec<Line<'static>>,
    height: u16,
    scroll_from_bottom: usize,
) -> Vec<Line<'static>> {
    let height = height as usize;
    if height == 0 || lines.len() <= height {
        return lines;
    }

    let latest_start = lines.len() - height;
    let start = latest_start.saturating_sub(scroll_from_bottom);
    lines.into_iter().skip(start).take(height).collect()
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

#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::visible_lines;

    fn lines(count: usize) -> Vec<Line<'static>> {
        (0..count)
            .map(|index| Line::from(format!("line {index}")))
            .collect()
    }

    #[test]
    fn persona_view_defaults_to_latest_lines() {
        let visible = visible_lines(lines(10), 4, 0);

        assert_eq!(visible.len(), 4);
        assert_eq!(visible[0], Line::from("line 6"));
        assert_eq!(visible[3], Line::from("line 9"));
    }

    #[test]
    fn persona_view_scrolls_up_from_latest_position() {
        let visible = visible_lines(lines(10), 4, 2);

        assert_eq!(visible.len(), 4);
        assert_eq!(visible[0], Line::from("line 4"));
        assert_eq!(visible[3], Line::from("line 7"));
    }
}
