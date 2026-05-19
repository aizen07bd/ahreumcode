use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
        KeyCode::Char('/') if input.is_empty() => open_command_surface(input, surface),
        KeyCode::Char('/') => {
            input.push('/');
            CommandInputOutcome::none()
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

pub fn is_enter_event(event: &KeyEvent) -> bool {
    matches!(event.code, KeyCode::Enter)
        || matches!(event.code, KeyCode::Char('\n' | '\r'))
        || (event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(event.code, KeyCode::Char('j' | 'm')))
}

fn open_command_surface(
    input: &mut String,
    surface: &mut CommandSurfaceState,
) -> CommandInputOutcome {
    input.clear();
    input.push('/');
    surface.open();
    CommandInputOutcome {
        events: vec![CommandInputEvent::SurfaceOpened],
        dispatch: super::super::command::CommandDispatch::None,
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

    if is_enter_event(&event) {
        let outcome = confirm_command(surface, registry, scene, runtime_busy);
        if !outcome.events.is_empty() {
            input.clear();
        }
        return outcome;
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
    if is_enter_event(&event) {
        let outcome = super::super::command::confirm_picker_selection(surface);
        if !outcome.events.is_empty() {
            input.clear();
        }
        return outcome;
    }

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

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::tui::command::CommandDispatch;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn slash_opens_command_surface_when_prompt_is_empty() {
        let registry = CommandRegistry::new();
        let mut input = String::new();
        let mut surface = CommandSurfaceState::default();

        let outcome = handle_prompt_event(
            key(KeyCode::Char('/')),
            &mut input,
            &mut surface,
            &registry,
            "workspace",
            false,
        );

        assert_eq!(input, "/");
        assert!(surface.open);
        assert!(matches!(
            outcome.events.as_slice(),
            [CommandInputEvent::SurfaceOpened]
        ));
        assert!(matches!(outcome.dispatch, CommandDispatch::None));
    }

    #[test]
    fn slash_inside_prompt_is_plain_text() {
        let registry = CommandRegistry::new();
        let mut input = "src".to_owned();
        let mut surface = CommandSurfaceState::default();

        let outcome = handle_prompt_event(
            key(KeyCode::Char('/')),
            &mut input,
            &mut surface,
            &registry,
            "main",
            false,
        );

        assert_eq!(input, "src/");
        assert!(!surface.open);
        assert!(outcome.events.is_empty());
        assert!(matches!(outcome.dispatch, CommandDispatch::None));
    }

    #[test]
    fn control_j_confirms_command_surface_like_enter() {
        let registry = CommandRegistry::new();
        let mut input = "/persona full".to_owned();
        let mut surface = CommandSurfaceState::default();
        surface.open();
        surface.set_query("persona full");

        let outcome = handle_prompt_event(
            ctrl_key(KeyCode::Char('j')),
            &mut input,
            &mut surface,
            &registry,
            "workspace",
            false,
        );

        assert!(input.is_empty());
        assert!(matches!(outcome.dispatch, CommandDispatch::PersonaFull));
    }
}
