use crossterm::event::{KeyCode, KeyEvent};

use super::super::approval::{
    confirm_approval_selection, ApprovalInputOutcome, ApprovalSurfaceState,
};

pub fn handle_approval_event(
    event: KeyEvent,
    approval: &mut ApprovalSurfaceState,
) -> ApprovalInputOutcome {
    match event.code {
        KeyCode::Up => {
            approval.move_selection(-1);
            ApprovalInputOutcome::none()
        }
        KeyCode::Down => {
            approval.move_selection(1);
            ApprovalInputOutcome::none()
        }
        KeyCode::Enter => confirm_approval_selection(approval),
        _ => ApprovalInputOutcome::none(),
    }
}
