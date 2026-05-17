use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::super::approval::{ApprovalOption, ApprovalSurfaceState, APPROVAL_OPTIONS};
use super::super::style;

pub fn render_approval_surface(frame: &mut Frame<'_>, area: Rect, approval: &ApprovalSurfaceState) {
    if !approval.open || area.height == 0 {
        return;
    }

    let Some(request) = &approval.request else {
        return;
    };

    let mut lines = vec![
        Line::from(vec![Span::styled(
            request.title.clone(),
            style::panel_bold(),
        )]),
        Line::from(vec![
            Span::styled("Reason: ", style::muted()),
            Span::styled(request.reason.clone(), style::panel()),
        ]),
    ];

    for (index, option) in APPROVAL_OPTIONS.iter().enumerate() {
        lines.push(render_option_line(
            index,
            *option,
            approval.selected == index,
        ));
    }

    if approval.details_open {
        lines.push(Line::from(vec![
            Span::styled("Details: ", style::muted()),
            Span::styled(request.details.clone(), style::panel()),
        ]));
    }

    while lines.len() < area.height as usize {
        lines.push(Line::from(""));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().style(style::prompt_background()))
        .style(style::prompt_background());
    frame.render_widget(paragraph, area);
}

fn render_option_line(index: usize, option: ApprovalOption, selected: bool) -> Line<'static> {
    let marker = if selected { "> " } else { "  " };
    let marker_style = if selected {
        style::cyan()
    } else {
        style::muted()
    };
    let option_style = if selected {
        style::cyan()
    } else {
        style::panel()
    };

    Line::from(vec![
        Span::styled(marker, marker_style),
        Span::styled(format!("{}. ", index + 1), style::muted()),
        Span::styled(option.label(), option_style),
    ])
}
