use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::style;
use super::super::working_process::{WorkingProcessState, PHASES};

pub fn render_working_process(
    frame: &mut Frame<'_>,
    area: Rect,
    working_process: &WorkingProcessState,
) {
    if !working_process.is_active() || area.height == 0 {
        return;
    }

    let mut phase_line = vec![Span::styled("[working process] ", style::muted())];
    for (index, phase) in PHASES.iter().enumerate() {
        if index > 0 {
            phase_line.push(Span::styled(" > ", style::muted()));
        }

        let phase_style = if *phase == working_process.phase() {
            style::cyan()
        } else {
            style::muted()
        };
        phase_line.push(Span::styled(phase.label(), phase_style));
    }

    let detail = format!(
        "[{}/6] {} ({}s, esc 취소)",
        working_process.phase().number(),
        working_process.detail(),
        working_process.elapsed_secs()
    );

    let paragraph = Paragraph::new(vec![
        Line::from(phase_line),
        Line::from(vec![Span::styled(detail, style::muted())]),
    ]);
    frame.render_widget(paragraph, area);
}
