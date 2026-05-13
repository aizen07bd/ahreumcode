use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::product;

use super::super::approval::ApprovalInputOutcome;
use super::super::command::{CommandInputOutcome, CommandRegistry};
use super::super::components::{
    approval_surface, command_surface, expanded_form, persona, statusline, working_process,
    workspace,
};
use super::super::expanded_form::ExpandedFormEvents;
use super::super::persona::{PersonaEvents, MIN_PERSONA_PANEL_WIDTH, MIN_PERSONA_TERMINAL_WIDTH};
use super::super::state::TuiState;
use super::super::style;
use super::super::working_process::WorkingProcessEvents;
use super::super::workspace::WorkspaceEvents;
use super::approval::handle_approval_event;
use super::prompt::handle_prompt_event;

pub struct MainAction {
    pub command_outcome: CommandInputOutcome,
    pub approval_outcome: ApprovalInputOutcome,
    pub working_process_events: WorkingProcessEvents,
    pub workspace_events: WorkspaceEvents,
    pub persona_events: PersonaEvents,
    pub expanded_form_events: ExpandedFormEvents,
}

pub fn render_main(frame: &mut Frame<'_>, state: &TuiState) {
    let area = frame.area();
    let layout = layout_main(
        area,
        state.command_surface.open,
        state.approval_surface.open,
        state.working_process.is_active(),
    );

    render_header(frame, layout.header);
    render_body(frame, layout.workspace, state);
    working_process::render_working_process(frame, layout.working_process, &state.working_process);
    render_prompt_composer(frame, layout.prompt, state);
    approval_surface::render_approval_surface(frame, layout.approval, &state.approval_surface);
    command_surface::render_command_surface(
        frame,
        layout.command,
        &state.command_surface,
        &CommandRegistry::new(),
        state.scene.as_str(),
    );
    statusline::render_statusline(frame, layout.statusline, &state.runtime_status);
    expanded_form::render_expanded_form(frame, expanded_form_area(area), &state.expanded_form);
}

pub fn handle_main_event(event: KeyEvent, state: &mut TuiState) -> MainAction {
    if state.working_process.is_active() && matches!(event.code, KeyCode::Esc) {
        let runtime_outcome = state.cancel_working_process();
        return MainAction {
            command_outcome: CommandInputOutcome::none(),
            approval_outcome: ApprovalInputOutcome::none(),
            working_process_events: runtime_outcome.working_process_events,
            workspace_events: runtime_outcome.workspace_events,
            persona_events: PersonaEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        };
    }

    if state.expanded_form.open && !event.modifiers.contains(KeyModifiers::CONTROL) {
        return handle_expanded_form_event(event, state);
    }

    if state.approval_surface.open && !event.modifiers.contains(KeyModifiers::CONTROL) {
        let approval_outcome = handle_approval_event(event, &mut state.approval_surface);
        let mut workspace_events = WorkspaceEvents::none();
        if let Some(workspace_line) = &approval_outcome.workspace_line {
            workspace_events.extend(state.record_workspace_line(workspace_line.clone()));
        }
        return MainAction {
            command_outcome: CommandInputOutcome::none(),
            approval_outcome,
            working_process_events: WorkingProcessEvents::none(),
            workspace_events,
            persona_events: PersonaEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        };
    }

    if state.command_surface.open && !event.modifiers.contains(KeyModifiers::CONTROL) {
        let command_outcome = handle_prompt_event(
            event,
            &mut state.main_input,
            &mut state.command_surface,
            state.scene.as_str(),
            state.working_process.is_active(),
        );
        let dispatch_outcome =
            state.apply_command_dispatch(command_outcome.dispatch, current_terminal_width());
        return MainAction {
            command_outcome,
            approval_outcome: dispatch_outcome.approval_outcome,
            working_process_events: WorkingProcessEvents::none(),
            workspace_events: dispatch_outcome.workspace_events,
            persona_events: dispatch_outcome.persona_events,
            expanded_form_events: dispatch_outcome.expanded_form_events,
        };
    }

    match event.code {
        KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
            MainAction::none()
        }
        KeyCode::Esc => {
            state.should_quit = true;
            MainAction::none()
        }
        KeyCode::Enter => {
            if state.main_input.trim().is_empty() {
                MainAction::none()
            } else {
                let prompt_outcome = state.start_working_process();
                MainAction {
                    command_outcome: CommandInputOutcome::none(),
                    approval_outcome: ApprovalInputOutcome::none(),
                    working_process_events: prompt_outcome.working_process_events,
                    workspace_events: prompt_outcome.workspace_events,
                    persona_events: PersonaEvents::none(),
                    expanded_form_events: ExpandedFormEvents::none(),
                }
            }
        }
        KeyCode::Up => MainAction {
            command_outcome: CommandInputOutcome::none(),
            approval_outcome: ApprovalInputOutcome::none(),
            working_process_events: WorkingProcessEvents::none(),
            workspace_events: state.scroll_workspace(-1),
            persona_events: PersonaEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        },
        KeyCode::Down => MainAction {
            command_outcome: CommandInputOutcome::none(),
            approval_outcome: ApprovalInputOutcome::none(),
            working_process_events: WorkingProcessEvents::none(),
            workspace_events: state.scroll_workspace(1),
            persona_events: PersonaEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        },
        _ => {
            let command_outcome = handle_prompt_event(
                event,
                &mut state.main_input,
                &mut state.command_surface,
                state.scene.as_str(),
                state.working_process.is_active(),
            );
            let dispatch_outcome =
                state.apply_command_dispatch(command_outcome.dispatch, current_terminal_width());
            MainAction {
                command_outcome,
                approval_outcome: dispatch_outcome.approval_outcome,
                working_process_events: WorkingProcessEvents::none(),
                workspace_events: dispatch_outcome.workspace_events,
                persona_events: dispatch_outcome.persona_events,
                expanded_form_events: dispatch_outcome.expanded_form_events,
            }
        }
    }
}

fn handle_expanded_form_event(event: KeyEvent, state: &mut TuiState) -> MainAction {
    let outcome = match event.code {
        KeyCode::Esc => state.cancel_expanded_form(),
        KeyCode::Tab => {
            state.focus_next_expanded_form_field();
            super::super::state::ExpandedFormOutcome {
                workspace_events: WorkspaceEvents::none(),
                expanded_form_events: ExpandedFormEvents::none(),
            }
        }
        KeyCode::BackTab => {
            state.focus_previous_expanded_form_field();
            super::super::state::ExpandedFormOutcome {
                workspace_events: WorkspaceEvents::none(),
                expanded_form_events: ExpandedFormEvents::none(),
            }
        }
        KeyCode::Enter => state.submit_expanded_form(),
        KeyCode::Backspace => state.backspace_expanded_form(),
        KeyCode::Char(value) => state.update_expanded_form_char(value),
        _ => super::super::state::ExpandedFormOutcome {
            workspace_events: WorkspaceEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        },
    };

    MainAction {
        command_outcome: CommandInputOutcome::none(),
        approval_outcome: ApprovalInputOutcome::none(),
        working_process_events: WorkingProcessEvents::none(),
        workspace_events: outcome.workspace_events,
        persona_events: PersonaEvents::none(),
        expanded_form_events: outcome.expanded_form_events,
    }
}

impl MainAction {
    fn none() -> Self {
        Self {
            command_outcome: CommandInputOutcome::none(),
            approval_outcome: ApprovalInputOutcome::none(),
            working_process_events: WorkingProcessEvents::none(),
            workspace_events: WorkspaceEvents::none(),
            persona_events: PersonaEvents::none(),
            expanded_form_events: ExpandedFormEvents::none(),
        }
    }
}

struct MainLayout {
    header: Rect,
    workspace: Rect,
    working_process: Rect,
    prompt: Rect,
    approval: Rect,
    command: Rect,
    statusline: Rect,
}

fn layout_main(
    area: Rect,
    command_open: bool,
    approval_open: bool,
    working_process_open: bool,
) -> MainLayout {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(working_process_height(area, working_process_open)),
            Constraint::Length(3),
            Constraint::Length(approval_height(area, approval_open)),
            Constraint::Length(command_height(area, command_open)),
            Constraint::Length(1),
        ])
        .split(area);

    MainLayout {
        header: root[0],
        workspace: root[1],
        working_process: root[2],
        prompt: root[3],
        approval: root[4],
        command: root[5],
        statusline: root[6],
    }
}

fn working_process_height(area: Rect, working_process_open: bool) -> u16 {
    if working_process_open && area.height >= 10 {
        2
    } else {
        0
    }
}

fn approval_height(area: Rect, approval_open: bool) -> u16 {
    if approval_open && area.height >= 14 {
        6
    } else {
        0
    }
}

fn command_height(area: Rect, command_open: bool) -> u16 {
    if command_open && area.height >= 12 {
        5
    } else {
        0
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect) {
    let version = product::version_label();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(product::APP_NAME, style::panel_bold()),
        Span::raw(" "),
        Span::styled(version, style::muted()),
    ]));
    frame.render_widget(header, rows[0]);

    let divider = Paragraph::new("─".repeat(area.width as usize)).style(style::muted());
    frame.render_widget(divider, rows[1]);
}

fn render_body(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    if !state.persona_panel.is_full() {
        workspace::render_workspace_items(frame, area, &state.workspace);
        return;
    }

    let panel_width = persona_panel_width(area.width);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(panel_width)])
        .split(area);

    workspace::render_workspace_items(frame, body[0], &state.workspace);
    persona::render_persona_panel(frame, body[1], &state.persona);
}

fn render_prompt_composer(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  > ", style::panel()),
            Span::styled(state.main_input.as_str(), style::panel()),
        ]),
        Line::from(""),
    ];

    let paragraph = Paragraph::new(lines)
        .block(Block::default().style(style::prompt_background()))
        .style(style::prompt_background());
    frame.render_widget(paragraph, area);
}

fn persona_panel_width(total_width: u16) -> u16 {
    let candidate = (total_width / 4).max(MIN_PERSONA_PANEL_WIDTH);
    candidate.min(total_width.saturating_sub(1))
}

fn current_terminal_width() -> u16 {
    crossterm::terminal::size()
        .map(|(width, _)| width)
        .unwrap_or(MIN_PERSONA_TERMINAL_WIDTH)
}

fn expanded_form_area(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(88);
    let height = area.height.saturating_sub(4).min(12);
    let horizontal_margin = area.width.saturating_sub(width) / 2;
    let vertical_margin = area.height.saturating_sub(height) / 2;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(vertical_margin),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(horizontal_margin),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(rows[1])[1]
}
