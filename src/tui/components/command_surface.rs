use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::super::command::{
    CommandRegistry, CommandRuntimeLabels, CommandSurfaceState, COMMAND_VISIBLE_ROWS,
};
use super::super::style;

pub fn render_command_surface(
    frame: &mut Frame<'_>,
    area: Rect,
    surface: &CommandSurfaceState,
    registry: &CommandRegistry,
    scene: &str,
    runtime_labels: CommandRuntimeLabels<'_>,
) {
    if !surface.open || area.height == 0 {
        return;
    }

    if surface.stepped_picker.is_some() {
        render_stepped_picker(frame, area, surface, runtime_labels);
        return;
    }

    let filtered = registry.filtered_for(&surface.query, scene);
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

fn render_stepped_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    surface: &CommandSurfaceState,
    runtime_labels: CommandRuntimeLabels<'_>,
) {
    let title = surface.step_title().unwrap_or("Select Option");
    let selected = surface
        .stepped_picker
        .as_ref()
        .map(|picker| picker.selected())
        .unwrap_or(0);
    let options = surface.step_options_for(runtime_labels);

    let mut lines = vec![Line::from(vec![
        Span::styled(title, style::panel_bold()),
        Span::styled("  esc back", style::muted()),
    ])];

    for (index, option) in options
        .iter()
        .enumerate()
        .take(COMMAND_VISIBLE_ROWS.saturating_sub(1))
    {
        let is_selected = index == selected;
        let marker = if is_selected { "> " } else { "  " };
        let label_style = if is_selected {
            style::cyan()
        } else {
            style::panel()
        };
        let detail_style = if is_selected {
            style::panel()
        } else {
            style::muted()
        };

        lines.push(Line::from(vec![
            Span::styled(marker, detail_style),
            Span::styled(option.label.clone(), label_style),
            Span::raw("  "),
            Span::styled(option.detail.clone(), detail_style),
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
