use crossterm::event::{KeyCode, KeyEvent};

use super::super::command::{
    confirm_command, CommandInputEvent, CommandInputOutcome, CommandRegistry, CommandSurfaceState,
};

pub fn handle_prompt_event(
    event: KeyEvent,
    input: &mut String,
    surface: &mut CommandSurfaceState,
    registry: &CommandRegistry,
    scene: &str,
    runtime_busy: bool,
) -> CommandInputOutcome {
    if surface.open {
        return handle_command_event(event, input, surface, registry, scene, runtime_busy);
    }

    match event.code {
        KeyCode::Char('/') => {
            input.clear();
            input.push('/');
            surface.open();
            CommandInputOutcome {
                events: vec![CommandInputEvent::SurfaceOpened],
                dispatch: super::super::command::CommandDispatch::None,
            }
        }
        KeyCode::Backspace => {
            input.pop();
            CommandInputOutcome::none()
        }
        KeyCode::Char(value) => {
            input.push(value);
            CommandInputOutcome::none()
        }
        _ => CommandInputOutcome::none(),
    }
}

fn handle_command_event(
    event: KeyEvent,
    input: &mut String,
    surface: &mut CommandSurfaceState,
    registry: &CommandRegistry,
    scene: &str,
    runtime_busy: bool,
) -> CommandInputOutcome {
    if surface.stepped_picker.is_some() {
        return handle_stepped_picker_event(event, input, surface);
    }

    match event.code {
        KeyCode::Esc => {
            surface.close();
            input.clear();
            CommandInputOutcome::none()
        }
        KeyCode::Up => {
            let item_count = registry.filtered_for(&surface.query, scene).len();
            surface.move_selection(-1, item_count);
            CommandInputOutcome::none()
        }
        KeyCode::Down => {
            let item_count = registry.filtered_for(&surface.query, scene).len();
            surface.move_selection(1, item_count);
            CommandInputOutcome::none()
        }
        KeyCode::Enter => {
            let outcome = confirm_command(surface, registry, scene, runtime_busy);
            if !outcome.events.is_empty() {
                input.clear();
            }
            outcome
        }
        KeyCode::Backspace => {
            input.pop();
            if input.is_empty() {
                surface.close();
                return CommandInputOutcome::none();
            }
            update_query(input, surface)
        }
        KeyCode::Char(value) => {
            input.push(value);
            update_query(input, surface)
        }
        _ => CommandInputOutcome::none(),
    }
}

fn handle_stepped_picker_event(
    event: KeyEvent,
    input: &mut String,
    surface: &mut CommandSurfaceState,
) -> CommandInputOutcome {
    match event.code {
        KeyCode::Esc | KeyCode::Backspace => {
            surface.back_picker_step();
            CommandInputOutcome::none()
        }
        KeyCode::Up => {
            let Some(selected) = surface.move_picker_selection(-1) else {
                return CommandInputOutcome::none();
            };
            let Some(picker) = surface.stepped_picker.as_ref() else {
                return CommandInputOutcome::none();
            };
            CommandInputOutcome {
                events: vec![CommandInputEvent::SteppedPickerSelectionChanged {
                    command: picker.command(),
                    selected,
                }],
                dispatch: super::super::command::CommandDispatch::None,
            }
        }
        KeyCode::Down => {
            let Some(selected) = surface.move_picker_selection(1) else {
                return CommandInputOutcome::none();
            };
            let Some(picker) = surface.stepped_picker.as_ref() else {
                return CommandInputOutcome::none();
            };
            CommandInputOutcome {
                events: vec![CommandInputEvent::SteppedPickerSelectionChanged {
                    command: picker.command(),
                    selected,
                }],
                dispatch: super::super::command::CommandDispatch::None,
            }
        }
        KeyCode::Enter => {
            let outcome = super::super::command::confirm_picker_selection(surface);
            if !outcome.events.is_empty() {
                input.clear();
            }
            outcome
        }
        _ => CommandInputOutcome::none(),
    }
}

fn update_query(input: &str, surface: &mut CommandSurfaceState) -> CommandInputOutcome {
    let query = input.strip_prefix('/').unwrap_or(input);
    surface.set_query(query);
    CommandInputOutcome {
        events: vec![CommandInputEvent::FilterChanged {
            query: query.to_owned(),
        }],
        dispatch: super::super::command::CommandDispatch::None,
    }
}
