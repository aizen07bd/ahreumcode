use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::super::state::RuntimeStatus;
use super::super::style;

pub fn render_statusline(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    status: &RuntimeStatus,
) {
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
