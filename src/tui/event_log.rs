use std::io;

use serde_json::json;

use crate::logging::{LogEvent, Logger};

use super::approval::ApprovalInputEvent;
use super::command::{CommandId, CommandInputEvent, CommandRegistry};
use super::expanded_form::ExpandedFormEvent;
use super::persona::{PersonaEvent, PersonaRendered};
use super::working_process::{WorkingFinishReason, WorkingProcessEvent};
use super::workspace::{WorkspaceEvent, WorkspaceRendered};

pub(super) const TUI_01_SCOPE: &str = "tui-01-intro-scene";
pub(super) const TUI_02_SCOPE: &str = "tui-02-epilogue-scene";
pub(super) const TUI_03_SCOPE: &str = "tui-03-main-scene-layout";

const TUI_04_SCOPE: &str = "tui-04-command-area-basic-actions";
const TUI_05_SCOPE: &str = "tui-05-approval-area";
const TUI_06_SCOPE: &str = "tui-06-working-process-area";
const TUI_07_SCOPE: &str = "tui-07-workspace-output-layout";
const TUI_08_SCOPE: &str = "tui-08-persona-message-detail";
const TUI_09_SCOPE: &str = "tui-09-complex-commands";
const TUI_10_SCOPE: &str = "tui-10-modal-expanded-form";

const EVENT_MAIN_SCENE_RENDERED: &str = "main_scene_rendered";
const EVENT_LAYOUT_CALCULATED: &str = "layout_calculated";
const EVENT_PERSONA_LAYOUT_ABSENT: &str = "persona_layout_absent";
const EVENT_STATUSLINE_POSITIONED: &str = "statusline_positioned";
const EVENT_COMMAND_SURFACE_OPENED: &str = "command_surface_opened";
const EVENT_COMMAND_FILTER_CHANGED: &str = "command_filter_changed";
const EVENT_COMMAND_SELECTED: &str = "command_selected";
const EVENT_COMMAND_ACTION_DISPATCHED: &str = "command_action_dispatched";
const EVENT_COMMAND_AVAILABILITY_CHECKED: &str = "command_availability_checked";
const EVENT_STEPPED_PICKER_OPENED: &str = "stepped_picker_opened";
const EVENT_STEPPED_PICKER_SELECTION_CHANGED: &str = "stepped_picker_selection_changed";
const EVENT_STEPPED_PICKER_CONFIRMED: &str = "stepped_picker_confirmed";
const EVENT_EXPANDED_FORM_OPENED: &str = "expanded_form_opened";
const EVENT_EXPANDED_FORM_FIELD_CHANGED: &str = "expanded_form_field_changed";
const EVENT_EXPANDED_FORM_SUBMITTED: &str = "expanded_form_submitted";
const EVENT_EXPANDED_FORM_CANCELLED: &str = "expanded_form_cancelled";
const EVENT_APPROVAL_SURFACE_OPENED: &str = "approval_surface_opened";
const EVENT_APPROVAL_OPTION_SELECTED: &str = "approval_option_selected";
const EVENT_APPROVAL_RESULT_RECORDED: &str = "approval_result_recorded";
const EVENT_WORKING_PROCESS_STARTED: &str = "working_process_started";
const EVENT_WORKING_PHASE_CHANGED: &str = "working_phase_changed";
const EVENT_WORKING_PROCESS_CANCEL_HINT_RENDERED: &str = "working_process_cancel_hint_rendered";
const EVENT_WORKING_PROCESS_FINISHED: &str = "working_process_finished";
const EVENT_WORKSPACE_PROMPT_BLOCK_ADDED: &str = "workspace_prompt_block_added";
const EVENT_WORKSPACE_OUTPUT_ADDED: &str = "workspace_output_added";
const EVENT_WORKSPACE_SCROLL_CHANGED: &str = "workspace_scroll_changed";
const EVENT_WORKSPACE_RENDERED: &str = "workspace_rendered";
const EVENT_PERSONA_PANEL_OPENED: &str = "persona_panel_opened";
const EVENT_PERSONA_PANEL_CLOSED: &str = "persona_panel_closed";
const EVENT_PERSONA_MESSAGE_RENDERED: &str = "persona_message_rendered";
const EVENT_PERSONA_WIDTH_REJECTED: &str = "persona_width_rejected";

pub(super) fn log_main_scene_rendered(logger: &Logger, run_mode: &str) -> io::Result<()> {
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_LAYOUT_CALCULATED,
        json!({ "run_mode": run_mode }),
    ))?;
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_PERSONA_LAYOUT_ABSENT,
        json!({ "persona": "off" }),
    ))?;
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_STATUSLINE_POSITIONED,
        json!({ "position": "bottom" }),
    ))?;
    logger.ui(LogEvent::ui(
        TUI_03_SCOPE,
        EVENT_MAIN_SCENE_RENDERED,
        json!({ "run_mode": run_mode }),
    ))
}

pub(super) fn log_command_events(
    logger: &Logger,
    scene: &str,
    registry: &CommandRegistry,
    events: &[CommandInputEvent],
) -> io::Result<()> {
    for event in events {
        match event {
            CommandInputEvent::SurfaceOpened => {
                logger.ui(LogEvent::ui(
                    TUI_04_SCOPE,
                    EVENT_COMMAND_SURFACE_OPENED,
                    json!({ "scene": scene }),
                ))?;
            }
            CommandInputEvent::FilterChanged { query } => {
                logger.ui(LogEvent::ui(
                    TUI_04_SCOPE,
                    EVENT_COMMAND_FILTER_CHANGED,
                    json!({ "query": query }),
                ))?;
            }
            CommandInputEvent::CommandSelected { command } => {
                logger.ui(LogEvent::ui(
                    TUI_04_SCOPE,
                    EVENT_COMMAND_SELECTED,
                    command_log_data(registry, *command),
                ))?;
            }
            CommandInputEvent::ActionDispatched { command } => {
                logger.ui(LogEvent::ui(
                    TUI_04_SCOPE,
                    EVENT_COMMAND_ACTION_DISPATCHED,
                    command_log_data(registry, *command),
                ))?;
            }
            CommandInputEvent::CommandAvailabilityChecked {
                command,
                allowed,
                reason,
            } => {
                logger.ui(LogEvent::ui(
                    TUI_09_SCOPE,
                    EVENT_COMMAND_AVAILABILITY_CHECKED,
                    json!({
                        "command": command.as_str(),
                        "allowed": allowed,
                        "reason": reason,
                    }),
                ))?;
            }
            CommandInputEvent::SteppedPickerOpened { command, step } => {
                logger.ui(LogEvent::ui(
                    TUI_09_SCOPE,
                    EVENT_STEPPED_PICKER_OPENED,
                    json!({ "command": command.as_str(), "step": step }),
                ))?;
            }
            CommandInputEvent::SteppedPickerSelectionChanged { command, selected } => {
                logger.ui(LogEvent::ui(
                    TUI_09_SCOPE,
                    EVENT_STEPPED_PICKER_SELECTION_CHANGED,
                    json!({ "command": command.as_str(), "selected": selected }),
                ))?;
            }
            CommandInputEvent::SteppedPickerConfirmed { command, selected } => {
                logger.ui(LogEvent::ui(
                    TUI_09_SCOPE,
                    EVENT_STEPPED_PICKER_CONFIRMED,
                    json!({ "command": command.as_str(), "selected": selected }),
                ))?;
            }
        }
    }

    Ok(())
}

pub(super) fn log_approval_events(
    logger: &Logger,
    scene: &str,
    events: &[ApprovalInputEvent],
) -> io::Result<()> {
    for event in events {
        match event {
            ApprovalInputEvent::SurfaceOpened => {
                logger.ui(LogEvent::ui(
                    TUI_05_SCOPE,
                    EVENT_APPROVAL_SURFACE_OPENED,
                    json!({ "scene": scene }),
                ))?;
            }
            ApprovalInputEvent::OptionSelected { option } => {
                logger.ui(LogEvent::ui(
                    TUI_05_SCOPE,
                    EVENT_APPROVAL_OPTION_SELECTED,
                    json!({ "option": option.as_str() }),
                ))?;
            }
            ApprovalInputEvent::ResultRecorded { result } => {
                logger.ui(LogEvent::ui(
                    TUI_05_SCOPE,
                    EVENT_APPROVAL_RESULT_RECORDED,
                    json!({ "result": result.as_str() }),
                ))?;
            }
        }
    }

    Ok(())
}

pub(super) fn log_working_process_events(
    logger: &Logger,
    scene: &str,
    events: &[WorkingProcessEvent],
) -> io::Result<()> {
    for event in events {
        match event {
            WorkingProcessEvent::Started => {
                logger.ui(LogEvent::ui(
                    TUI_06_SCOPE,
                    EVENT_WORKING_PROCESS_STARTED,
                    json!({ "scene": scene }),
                ))?;
            }
            WorkingProcessEvent::PhaseChanged { phase } => {
                logger.ui(LogEvent::ui(
                    TUI_06_SCOPE,
                    EVENT_WORKING_PHASE_CHANGED,
                    json!({ "phase": phase.label(), "step": phase.number() }),
                ))?;
            }
            WorkingProcessEvent::CancelHintRendered => {
                logger.ui(LogEvent::ui(
                    TUI_06_SCOPE,
                    EVENT_WORKING_PROCESS_CANCEL_HINT_RENDERED,
                    json!({ "hint": "esc 취소" }),
                ))?;
            }
            WorkingProcessEvent::Finished { reason } => {
                logger.ui(LogEvent::ui(
                    TUI_06_SCOPE,
                    EVENT_WORKING_PROCESS_FINISHED,
                    json!({ "reason": reason.as_str() }),
                ))?;
            }
        }
    }

    Ok(())
}

pub(super) fn log_workspace_events(
    logger: &Logger,
    scene: &str,
    events: &[WorkspaceEvent],
) -> io::Result<()> {
    for event in events {
        match event {
            WorkspaceEvent::PromptBlockAdded => {
                logger.ui(LogEvent::ui(
                    TUI_07_SCOPE,
                    EVENT_WORKSPACE_PROMPT_BLOCK_ADDED,
                    json!({ "scene": scene }),
                ))?;
            }
            WorkspaceEvent::OutputAdded { item_type } => {
                logger.ui(LogEvent::ui(
                    TUI_07_SCOPE,
                    EVENT_WORKSPACE_OUTPUT_ADDED,
                    json!({ "item_type": item_type }),
                ))?;
            }
            WorkspaceEvent::ScrollChanged { scroll } => {
                logger.ui(LogEvent::ui(
                    TUI_07_SCOPE,
                    EVENT_WORKSPACE_SCROLL_CHANGED,
                    json!({ "scroll": scroll }),
                ))?;
            }
        }
    }

    Ok(())
}

pub(super) fn log_workspace_rendered(
    logger: &Logger,
    rendered: WorkspaceRendered,
) -> io::Result<()> {
    logger.ui(LogEvent::ui(
        TUI_07_SCOPE,
        EVENT_WORKSPACE_RENDERED,
        json!({ "item_count": rendered.item_count, "scroll": rendered.scroll }),
    ))
}

pub(super) fn log_persona_events(
    logger: &Logger,
    scene: &str,
    events: &[PersonaEvent],
) -> io::Result<()> {
    for event in events {
        match event {
            PersonaEvent::PanelOpened => {
                logger.ui(LogEvent::ui(
                    TUI_08_SCOPE,
                    EVENT_PERSONA_PANEL_OPENED,
                    json!({ "scene": scene }),
                ))?;
            }
            PersonaEvent::PanelClosed => {
                logger.ui(LogEvent::ui(
                    TUI_08_SCOPE,
                    EVENT_PERSONA_PANEL_CLOSED,
                    json!({ "scene": scene }),
                ))?;
            }
            PersonaEvent::WidthRejected { width, min_width } => {
                logger.ui(LogEvent::ui(
                    TUI_08_SCOPE,
                    EVENT_PERSONA_WIDTH_REJECTED,
                    json!({ "width": width, "min_width": min_width }),
                ))?;
            }
        }
    }

    Ok(())
}

pub(super) fn log_persona_message_rendered(
    logger: &Logger,
    rendered: PersonaRendered,
) -> io::Result<()> {
    logger.ui(LogEvent::ui(
        TUI_08_SCOPE,
        EVENT_PERSONA_MESSAGE_RENDERED,
        json!({ "message_count": rendered.message_count }),
    ))
}

pub(super) fn log_expanded_form_events(
    logger: &Logger,
    events: &[ExpandedFormEvent],
) -> io::Result<()> {
    for event in events {
        match event {
            ExpandedFormEvent::Opened { kind } => {
                logger.ui(LogEvent::ui(
                    TUI_10_SCOPE,
                    EVENT_EXPANDED_FORM_OPENED,
                    json!({ "kind": kind.as_str() }),
                ))?;
            }
            ExpandedFormEvent::FieldChanged {
                kind,
                field,
                masked,
            } => {
                logger.ui(LogEvent::ui(
                    TUI_10_SCOPE,
                    EVENT_EXPANDED_FORM_FIELD_CHANGED,
                    json!({
                        "kind": kind.as_str(),
                        "field": field,
                        "masked": masked,
                    }),
                ))?;
            }
            ExpandedFormEvent::Submitted { kind } => {
                logger.ui(LogEvent::ui(
                    TUI_10_SCOPE,
                    EVENT_EXPANDED_FORM_SUBMITTED,
                    json!({ "kind": kind.as_str() }),
                ))?;
            }
            ExpandedFormEvent::Cancelled { kind } => {
                logger.ui(LogEvent::ui(
                    TUI_10_SCOPE,
                    EVENT_EXPANDED_FORM_CANCELLED,
                    json!({ "kind": kind.as_str() }),
                ))?;
            }
        }
    }

    Ok(())
}

fn command_log_data(registry: &CommandRegistry, command: CommandId) -> serde_json::Value {
    let Some(metadata) = registry.command(command) else {
        return json!({ "command": command.as_str() });
    };

    json!({
        "command": metadata.name,
        "group": metadata.group,
        "presentation": metadata.presentation.as_str(),
        "risk": metadata.risk.as_str(),
        "availability": metadata.availability,
    })
}

pub(super) fn working_started(events: &[WorkingProcessEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, WorkingProcessEvent::Started))
}

pub(super) fn working_cancelled(events: &[WorkingProcessEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            WorkingProcessEvent::Finished {
                reason: WorkingFinishReason::Canceled
            }
        )
    })
}
