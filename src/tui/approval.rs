#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ApprovalOption {
    ApproveOnce,
    Deny,
    ViewDetails,
}

impl ApprovalOption {
    pub fn label(self) -> &'static str {
        match self {
            Self::ApproveOnce => "Yes, approve once",
            Self::Deny => "No, deny",
            Self::ViewDetails => "View details",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApproveOnce => "approve_once",
            Self::Deny => "deny",
            Self::ViewDetails => "view_details",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ApprovalResult {
    ApprovedOnce,
    Denied,
}

impl ApprovalResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApprovedOnce => "approved_once",
            Self::Denied => "denied",
        }
    }

    pub fn workspace_line(self, request: &ApprovalRequest) -> String {
        match self {
            Self::ApprovedOnce => format!("Approval approved once: {}", request.action),
            Self::Denied => format!("Approval denied: {}", request.action),
        }
    }
}

pub struct ApprovalRequest {
    pub title: String,
    pub reason: String,
    pub action: String,
    pub details: String,
}

impl ApprovalRequest {
    pub fn shell() -> Self {
        Self {
            title: "Approval required".to_owned(),
            reason: "Permission shell is waiting for user decision.".to_owned(),
            action: "pending tool action".to_owned(),
            details:
                "This shell is the TUI approval surface before the policy engine is connected."
                    .to_owned(),
        }
    }
}

pub struct ApprovalSurfaceState {
    pub open: bool,
    pub selected: usize,
    pub details_open: bool,
    pub request: Option<ApprovalRequest>,
}

impl Default for ApprovalSurfaceState {
    fn default() -> Self {
        Self {
            open: false,
            selected: 0,
            details_open: false,
            request: None,
        }
    }
}

impl ApprovalSurfaceState {
    pub fn open(&mut self, request: ApprovalRequest) {
        self.open = true;
        self.selected = 0;
        self.details_open = false;
        self.request = Some(request);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.selected = 0;
        self.details_open = false;
        self.request = None;
    }

    pub fn move_selection(&mut self, delta: isize) {
        let max = APPROVAL_OPTIONS.len() as isize - 1;
        let current = self.selected as isize;
        self.selected = (current + delta).clamp(0, max) as usize;
    }

    pub fn selected_option(&self) -> ApprovalOption {
        APPROVAL_OPTIONS[self.selected]
    }
}

pub const APPROVAL_OPTIONS: [ApprovalOption; 3] = [
    ApprovalOption::ApproveOnce,
    ApprovalOption::Deny,
    ApprovalOption::ViewDetails,
];

pub enum ApprovalInputEvent {
    SurfaceOpened,
    OptionSelected { option: ApprovalOption },
    ResultRecorded { result: ApprovalResult },
}

pub struct ApprovalInputOutcome {
    pub events: Vec<ApprovalInputEvent>,
    pub workspace_line: Option<String>,
}

impl ApprovalInputOutcome {
    pub fn none() -> Self {
        Self {
            events: Vec::new(),
            workspace_line: None,
        }
    }
}

pub fn open_approval_surface(surface: &mut ApprovalSurfaceState) -> ApprovalInputOutcome {
    surface.open(ApprovalRequest::shell());
    ApprovalInputOutcome {
        events: vec![ApprovalInputEvent::SurfaceOpened],
        workspace_line: None,
    }
}

pub fn confirm_approval_selection(surface: &mut ApprovalSurfaceState) -> ApprovalInputOutcome {
    if !surface.open {
        return ApprovalInputOutcome::none();
    }

    let option = surface.selected_option();
    let events = vec![ApprovalInputEvent::OptionSelected { option }];

    match option {
        ApprovalOption::ApproveOnce => record_result(surface, ApprovalResult::ApprovedOnce, events),
        ApprovalOption::Deny => record_result(surface, ApprovalResult::Denied, events),
        ApprovalOption::ViewDetails => {
            surface.details_open = true;
            ApprovalInputOutcome {
                events,
                workspace_line: None,
            }
        }
    }
}

fn record_result(
    surface: &mut ApprovalSurfaceState,
    result: ApprovalResult,
    mut events: Vec<ApprovalInputEvent>,
) -> ApprovalInputOutcome {
    let workspace_line = surface
        .request
        .as_ref()
        .map(|request| result.workspace_line(request));

    events.push(ApprovalInputEvent::ResultRecorded { result });
    surface.close();

    ApprovalInputOutcome {
        events,
        workspace_line,
    }
}
