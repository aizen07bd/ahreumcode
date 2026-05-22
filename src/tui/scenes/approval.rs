use crossterm::event::{KeyCode, KeyEvent};

use super::super::approval::{
    confirm_approval_selection, ApprovalInputOutcome, ApprovalOption, ApprovalSurfaceState,
    APPROVAL_OPTIONS,
};
use super::prompt::is_enter_event;

pub fn handle_approval_event(
    event: KeyEvent,
    approval: &mut ApprovalSurfaceState,
) -> ApprovalInputOutcome {
    match event.code {
        KeyCode::Char('1') => confirm_numbered_selection(approval, ApprovalOption::ApproveOnce),
        KeyCode::Char('2') => confirm_numbered_selection(approval, ApprovalOption::Deny),
        KeyCode::Char('3') => confirm_numbered_selection(approval, ApprovalOption::ViewDetails),
        KeyCode::Esc => confirm_numbered_selection(approval, ApprovalOption::Deny),
        KeyCode::Up => {
            approval.move_selection(-1);
            ApprovalInputOutcome::none()
        }
        KeyCode::Down => {
            approval.move_selection(1);
            ApprovalInputOutcome::none()
        }
        _ if is_enter_event(&event) => confirm_approval_selection(approval),
        _ => ApprovalInputOutcome::none(),
    }
}

fn confirm_numbered_selection(
    approval: &mut ApprovalSurfaceState,
    option: ApprovalOption,
) -> ApprovalInputOutcome {
    if let Some(index) = APPROVAL_OPTIONS
        .iter()
        .position(|candidate| *candidate == option)
    {
        approval.selected = index;
    }
    confirm_approval_selection(approval)
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::handle_approval_event;
    use crate::tui::approval::{ApprovalInputEvent, ApprovalResult, ApprovalSurfaceState};

    #[test]
    fn control_j_confirms_approval_like_enter() {
        let mut approval = ApprovalSurfaceState::default();
        approval.open(crate::tui::approval::ApprovalRequest {
            title: "Approval required".to_owned(),
            reason: "test".to_owned(),
            action: "apply_patch".to_owned(),
            details: "details".to_owned(),
        });

        let outcome = handle_approval_event(
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
            &mut approval,
        );

        assert!(outcome.events.iter().any(|event| {
            matches!(
                event,
                ApprovalInputEvent::ResultRecorded {
                    result: ApprovalResult::ApprovedOnce
                }
            )
        }));
        assert!(!approval.open);
    }

    #[test]
    fn number_key_confirms_matching_approval_option() {
        let mut approval = ApprovalSurfaceState::default();
        approval.open(crate::tui::approval::ApprovalRequest {
            title: "Approval required".to_owned(),
            reason: "test".to_owned(),
            action: "apply_patch".to_owned(),
            details: "details".to_owned(),
        });

        let outcome = handle_approval_event(
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
            &mut approval,
        );

        assert!(outcome.events.iter().any(|event| {
            matches!(
                event,
                ApprovalInputEvent::ResultRecorded {
                    result: ApprovalResult::Denied
                }
            )
        }));
        assert!(!approval.open);
    }

    #[test]
    fn escape_denies_open_approval_surface() {
        let mut approval = ApprovalSurfaceState::default();
        approval.open(crate::tui::approval::ApprovalRequest {
            title: "Approval required".to_owned(),
            reason: "test".to_owned(),
            action: "apply_patch".to_owned(),
            details: "details".to_owned(),
        });

        let outcome = handle_approval_event(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut approval,
        );

        assert!(outcome.events.iter().any(|event| {
            matches!(
                event,
                ApprovalInputEvent::ResultRecorded {
                    result: ApprovalResult::Denied
                }
            )
        }));
        assert!(!approval.open);
    }
}
