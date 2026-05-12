use crossterm::event::{KeyCode, KeyEvent};

use super::super::command::{
    confirm_command, CommandInputEvent, CommandInputOutcome, CommandRegistry, CommandSurfaceState,
};

pub fn handle_prompt_event(
    event: KeyEvent,
    input: &mut String,
    surface: &mut CommandSurfaceState,
) -> CommandInputOutcome {
    let registry = CommandRegistry::new();

    if surface.open {
        return handle_command_event(event, input, surface, &registry);
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
) -> CommandInputOutcome {
    match event.code {
        KeyCode::Esc => {
            surface.close();
            input.clear();
            CommandInputOutcome::none()
        }
        KeyCode::Up => {
            let item_count = registry.filtered(&surface.query).len();
            surface.move_selection(-1, item_count);
            CommandInputOutcome::none()
        }
        KeyCode::Down => {
            let item_count = registry.filtered(&surface.query).len();
            surface.move_selection(1, item_count);
            CommandInputOutcome::none()
        }
        KeyCode::Enter => {
            let outcome = confirm_command(surface, registry);
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
