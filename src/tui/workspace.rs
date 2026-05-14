#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum ActivityGroup {
    Explore,
    Change,
    Execute,
    Configure,
    Ask,
}

impl ActivityGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::Explore => "Explore",
            Self::Change => "Change",
            Self::Execute => "Execute",
            Self::Configure => "Configure",
            Self::Ask => "Ask",
        }
    }
}

#[allow(dead_code)]
pub enum WorkspaceItem {
    UserPrompt {
        text: String,
    },
    ManagerMessage {
        text: String,
    },
    SystemNotice {
        text: String,
    },
    AssistantAnswer {
        text: String,
    },
    ActivityOutput {
        group: ActivityGroup,
        summary: String,
    },
    EvidenceBlock {
        title: String,
        body: String,
    },
    DiffSummary {
        path: String,
        additions: u16,
        deletions: u16,
        expanded: bool,
    },
    Result {
        text: String,
    },
}

impl WorkspaceItem {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UserPrompt { .. } => "user_prompt",
            Self::ManagerMessage { .. } => "manager_message",
            Self::SystemNotice { .. } => "system_notice",
            Self::AssistantAnswer { .. } => "assistant_answer",
            Self::ActivityOutput { .. } => "activity_output",
            Self::EvidenceBlock { .. } => "evidence",
            Self::DiffSummary { .. } => "diff_summary",
            Self::Result { .. } => "result",
        }
    }

    pub fn line_count(&self) -> usize {
        match self {
            Self::UserPrompt { .. } => 3,
            Self::ManagerMessage { .. } => 1,
            Self::SystemNotice { .. } => 1,
            Self::AssistantAnswer { text } => text.lines().count().max(1) + 1,
            Self::ActivityOutput { .. } => 1,
            Self::EvidenceBlock { .. } => 2,
            Self::DiffSummary { expanded, .. } => {
                if *expanded {
                    3
                } else {
                    1
                }
            }
            Self::Result { .. } => 1,
        }
    }
}

#[derive(Default)]
pub struct WorkspaceBuffer {
    items: Vec<WorkspaceItem>,
    scroll: usize,
    render_pending: bool,
}

impl WorkspaceBuffer {
    pub fn push_user_prompt(&mut self, text: String) -> WorkspaceEvents {
        self.items.push(WorkspaceItem::UserPrompt { text });
        self.render_pending = true;
        WorkspaceEvents {
            events: vec![WorkspaceEvent::PromptBlockAdded],
        }
    }

    pub fn push_manager_message(&mut self, text: impl Into<String>) -> WorkspaceEvents {
        self.push_output(WorkspaceItem::ManagerMessage { text: text.into() })
    }

    pub fn push_system_notice(&mut self, text: impl Into<String>) -> WorkspaceEvents {
        self.push_output(WorkspaceItem::SystemNotice { text: text.into() })
    }

    pub fn push_answer(&mut self, text: impl Into<String>) -> WorkspaceEvents {
        self.push_output(WorkspaceItem::AssistantAnswer { text: text.into() })
    }

    #[allow(dead_code)]
    pub fn push_work_output(
        &mut self,
        group: ActivityGroup,
        summary: impl Into<String>,
    ) -> WorkspaceEvents {
        self.push_output(WorkspaceItem::ActivityOutput {
            group,
            summary: summary.into(),
        })
    }

    #[allow(dead_code)]
    pub fn push_evidence(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> WorkspaceEvents {
        self.push_output(WorkspaceItem::EvidenceBlock {
            title: title.into(),
            body: body.into(),
        })
    }

    #[allow(dead_code)]
    pub fn push_diff_summary(
        &mut self,
        path: impl Into<String>,
        additions: u16,
        deletions: u16,
    ) -> WorkspaceEvents {
        self.push_output(WorkspaceItem::DiffSummary {
            path: path.into(),
            additions,
            deletions,
            expanded: false,
        })
    }

    pub fn push_result(&mut self, text: impl Into<String>) -> WorkspaceEvents {
        self.push_output(WorkspaceItem::Result { text: text.into() })
    }

    pub fn scroll(&mut self, delta: isize) -> WorkspaceEvents {
        let previous = self.scroll;
        let max_scroll = self.total_lines().saturating_sub(1);
        let next = (self.scroll as isize + delta).clamp(0, max_scroll as isize) as usize;
        self.scroll = next;

        if self.scroll == previous {
            return WorkspaceEvents::none();
        }

        self.render_pending = true;
        WorkspaceEvents {
            events: vec![WorkspaceEvent::ScrollChanged {
                scroll: self.scroll,
            }],
        }
    }

    pub fn items(&self) -> &[WorkspaceItem] {
        &self.items
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll
    }

    pub fn take_render_event(&mut self) -> Option<WorkspaceRendered> {
        if !self.render_pending {
            return None;
        }

        self.render_pending = false;
        Some(WorkspaceRendered {
            item_count: self.items.len(),
            scroll: self.scroll,
        })
    }

    fn push_output(&mut self, item: WorkspaceItem) -> WorkspaceEvents {
        let kind = item.kind();
        self.items.push(item);
        self.render_pending = true;
        WorkspaceEvents {
            events: vec![WorkspaceEvent::OutputAdded { item_type: kind }],
        }
    }

    fn total_lines(&self) -> usize {
        self.items.iter().map(WorkspaceItem::line_count).sum()
    }
}

pub enum WorkspaceEvent {
    PromptBlockAdded,
    OutputAdded { item_type: &'static str },
    ScrollChanged { scroll: usize },
}

pub struct WorkspaceRendered {
    pub item_count: usize,
    pub scroll: usize,
}

#[derive(Default)]
pub struct WorkspaceEvents {
    pub events: Vec<WorkspaceEvent>,
}

impl WorkspaceEvents {
    pub fn none() -> Self {
        Self { events: Vec::new() }
    }

    pub fn extend(&mut self, other: WorkspaceEvents) {
        self.events.extend(other.events);
    }
}
