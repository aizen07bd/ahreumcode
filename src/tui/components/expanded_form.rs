use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::super::expanded_form::{ExpandedFormField, ExpandedFormState};
use super::super::style;

pub fn render_expanded_form(frame: &mut Frame<'_>, area: Rect, form: &ExpandedFormState) {
    if !form.open || area.height == 0 {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style::cyan())
        .style(style::prompt_background());
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let mut lines = vec![Line::from(vec![
        Span::styled(form.kind.title(), style::panel_bold()),
        Span::styled(
            "  tab next  shift+tab prev  enter submit  esc cancel",
            style::muted(),
        ),
    ])];

    for (index, field) in form.fields.iter().enumerate() {
        lines.push(render_field_line(field, index == form.focused, inner.width));
    }

    if let Some(message) = &form.validation_message {
        lines.push(Line::from(vec![
            Span::styled("validation  ", style::muted()),
            Span::styled(message.clone(), style::panel()),
        ]));
    }

    let visible = lines
        .into_iter()
        .take(inner.height as usize)
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(visible).style(style::prompt_background()),
        inner,
    );
}

fn render_field_line(field: &ExpandedFormField, focused: bool, width: u16) -> Line<'static> {
    let marker = if focused { "> " } else { "  " };
    let label_style = if focused {
        style::cyan()
    } else {
        style::muted()
    };
    let value_style = if focused {
        style::panel()
    } else {
        style::muted()
    };
    let value = display_value(field);
    let label = format!("{:<15}", field.label);
    let text = fit_to_width(&value, width.saturating_sub(19) as usize);

    Line::from(vec![
        Span::styled(marker, label_style),
        Span::styled(label, label_style),
        Span::styled(text, value_style),
    ])
}

fn display_value(field: &ExpandedFormField) -> String {
    if !field.secret || field.value.is_empty() {
        return field.value.clone();
    }

    "*".repeat(field.value.width().max(1))
}

fn fit_to_width(text: &str, width: usize) -> String {
    if width == 0 || text.width() <= width {
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
