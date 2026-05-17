use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::style;
use super::super::workspace::{WorkspaceBuffer, WorkspaceItem};

pub fn render_workspace_items(frame: &mut Frame<'_>, area: Rect, workspace: &WorkspaceBuffer) {
    let lines = workspace_lines(workspace, area.width);
    let visible = lines
        .into_iter()
        .skip(workspace.scroll_offset())
        .take(area.height as usize)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(visible), area);
}

fn workspace_lines(workspace: &WorkspaceBuffer, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for item in workspace.items() {
        match item {
            WorkspaceItem::UserPrompt { text } => {
                lines.push(prompt_line("", width));
                lines.push(prompt_line(&format!("  > {text}"), width));
                lines.push(prompt_line("", width));
            }
            WorkspaceItem::ManagerMessage { text } => {
                lines.push(Line::from(vec![
                    Span::styled("[Manager] ", style::cyan()),
                    Span::styled(text.clone(), style::panel()),
                ]));
            }
            WorkspaceItem::SystemNotice { text } => {
                lines.push(Line::from(vec![
                    Span::styled("[system] ", style::muted()),
                    Span::styled(text.clone(), style::panel()),
                ]));
            }
            WorkspaceItem::AssistantAnswer { text } => {
                lines.push(Line::from(Span::styled("Answer", style::cyan())));
                for line in text.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("  ", style::muted()),
                        Span::styled(line.to_owned(), style::panel()),
                    ]));
                }
            }
            WorkspaceItem::ActivityOutput { group, summary } => {
                lines.push(Line::from(vec![
                    Span::styled(group.label(), style::cyan()),
                    Span::styled("  ", style::muted()),
                    Span::styled(summary.clone(), style::panel()),
                ]));
            }
            WorkspaceItem::EvidenceBlock { title, body } => {
                lines.push(Line::from(vec![
                    Span::styled("Evidence  ", style::cyan()),
                    Span::styled(title.clone(), style::panel_bold()),
                ]));
                for line in body.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("  ", style::muted()),
                        Span::styled(line.to_owned(), style::muted()),
                    ]));
                }
            }
            WorkspaceItem::DiffSummary {
                path,
                additions,
                deletions,
                expanded,
            } => {
                let marker = if *expanded { "v" } else { ">" };
                lines.push(Line::from(vec![
                    Span::styled("Change  ", style::cyan()),
                    Span::styled(marker, style::muted()),
                    Span::styled(" ", style::muted()),
                    Span::styled(path.clone(), style::panel()),
                    Span::styled(format!(" (+{additions} -{deletions})"), style::muted()),
                ]));
                if *expanded {
                    lines.push(Line::from(vec![
                        Span::styled("+ ", style::cyan()),
                        Span::styled("inline diff added lines", style::panel()),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("- ", style::muted()),
                        Span::styled("inline diff removed lines", style::muted()),
                    ]));
                }
            }
            WorkspaceItem::Result { text } => {
                lines.push(Line::from(vec![
                    Span::styled("Result  ", style::cyan()),
                    Span::styled(text.clone(), style::panel()),
                ]));
            }
        }
    }
    lines
}

fn prompt_line(text: &str, width: u16) -> Line<'static> {
    let padded = format!("{text:<width$}", width = width as usize);
    Line::from(Span::styled(padded, style::prompt_background()))
}
