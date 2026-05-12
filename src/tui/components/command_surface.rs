use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::super::command::{CommandRegistry, CommandSurfaceState, COMMAND_VISIBLE_ROWS};
use super::super::style;

pub fn render_command_surface(
    frame: &mut Frame<'_>,
    area: Rect,
    surface: &CommandSurfaceState,
    registry: &CommandRegistry,
) {
    if !surface.open || area.height == 0 {
        return;
    }

    let filtered = registry.filtered(&surface.query);
    let visible = filtered
        .iter()
        .skip(surface.scroll)
        .take(COMMAND_VISIBLE_ROWS);

    let mut lines = Vec::new();
    for (visible_index, command) in visible.enumerate() {
        let index = surface.scroll + visible_index;
        let selected = index == surface.selected;
        let marker = if selected { "> " } else { "  " };
        let command_style = if selected {
            style::cyan()
        } else {
            style::panel()
        };
        let detail_style = if selected {
            style::panel()
        } else {
            style::muted()
        };

        lines.push(Line::from(vec![
            Span::styled(marker, detail_style),
            Span::styled(command.name, command_style),
            Span::raw("  "),
            Span::styled(command.description, detail_style),
        ]));
    }

    while lines.len() < COMMAND_VISIBLE_ROWS {
        lines.push(Line::from(""));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().style(style::prompt_background()))
        .style(style::prompt_background());
    frame.render_widget(paragraph, area);
}
