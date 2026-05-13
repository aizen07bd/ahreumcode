use std::time::{Duration, Instant};

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum WorkingPhase {
    Interpret,
    Classify,
    Validate,
    Execute,
    Apply,
    Answer,
}

impl WorkingPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Interpret => "해석",
            Self::Classify => "유형",
            Self::Validate => "검증",
            Self::Execute => "실행",
            Self::Apply => "반영",
            Self::Answer => "답변",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::Interpret => "사용자 요청을 해석합니다.",
            Self::Classify => "작업 유형을 분류합니다.",
            Self::Validate => "수행 조건을 검증합니다.",
            Self::Execute => "작업을 실행합니다.",
            Self::Apply => "결과를 화면 상태에 반영합니다.",
            Self::Answer => "응답 준비를 마무리합니다.",
        }
    }

    pub fn number(self) -> usize {
        PHASES
            .iter()
            .position(|phase| *phase == self)
            .map(|index| index + 1)
            .unwrap_or(1)
    }
}

pub const PHASES: [WorkingPhase; 6] = [
    WorkingPhase::Interpret,
    WorkingPhase::Classify,
    WorkingPhase::Validate,
    WorkingPhase::Execute,
    WorkingPhase::Apply,
    WorkingPhase::Answer,
];

const PHASE_DURATION: Duration = Duration::from_secs(1);

pub struct WorkingProcessState {
    active: bool,
    phase: WorkingPhase,
    detail: &'static str,
    started_at: Instant,
    last_phase_index: usize,
}

impl Default for WorkingProcessState {
    fn default() -> Self {
        Self {
            active: false,
            phase: WorkingPhase::Interpret,
            detail: WorkingPhase::Interpret.detail(),
            started_at: Instant::now(),
            last_phase_index: 0,
        }
    }
}

impl WorkingProcessState {
    pub fn start(&mut self) -> WorkingProcessEvents {
        self.active = true;
        self.phase = WorkingPhase::Interpret;
        self.detail = WorkingPhase::Interpret.detail();
        self.started_at = Instant::now();
        self.last_phase_index = 0;

        WorkingProcessEvents {
            events: vec![
                WorkingProcessEvent::Started,
                WorkingProcessEvent::PhaseChanged { phase: self.phase },
                WorkingProcessEvent::CancelHintRendered,
            ],
            workspace_line: None,
        }
    }

    pub fn tick(&mut self) -> WorkingProcessEvents {
        if !self.active {
            return WorkingProcessEvents::none();
        }

        let elapsed = self.started_at.elapsed();
        if elapsed >= PHASE_DURATION * PHASES.len() as u32 {
            return self.finish(WorkingFinishReason::Completed);
        }

        let phase_index = (elapsed.as_secs() as usize).min(PHASES.len() - 1);
        if phase_index == self.last_phase_index {
            return WorkingProcessEvents::none();
        }

        self.last_phase_index = phase_index;
        self.phase = PHASES[phase_index];
        self.detail = self.phase.detail();

        WorkingProcessEvents {
            events: vec![WorkingProcessEvent::PhaseChanged { phase: self.phase }],
            workspace_line: None,
        }
    }

    pub fn cancel(&mut self) -> WorkingProcessEvents {
        if !self.active {
            return WorkingProcessEvents::none();
        }
        self.finish(WorkingFinishReason::Canceled)
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn phase(&self) -> WorkingPhase {
        self.phase
    }

    pub fn detail(&self) -> &'static str {
        self.detail
    }

    pub fn elapsed_secs(&self) -> u64 {
        if self.active {
            self.started_at.elapsed().as_secs()
        } else {
            0
        }
    }

    fn finish(&mut self, reason: WorkingFinishReason) -> WorkingProcessEvents {
        self.active = false;
        WorkingProcessEvents {
            events: vec![WorkingProcessEvent::Finished { reason }],
            workspace_line: Some(reason.workspace_line().to_owned()),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum WorkingFinishReason {
    Completed,
    Canceled,
}

impl WorkingFinishReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Canceled => "canceled",
        }
    }

    fn workspace_line(self) -> &'static str {
        match self {
            Self::Completed => "Working process completed.",
            Self::Canceled => "Working process canceled.",
        }
    }
}

pub enum WorkingProcessEvent {
    Started,
    PhaseChanged { phase: WorkingPhase },
    CancelHintRendered,
    Finished { reason: WorkingFinishReason },
}

pub struct WorkingProcessEvents {
    pub events: Vec<WorkingProcessEvent>,
    pub workspace_line: Option<String>,
}

impl WorkingProcessEvents {
    pub fn none() -> Self {
        Self {
            events: Vec::new(),
            workspace_line: None,
        }
    }
}
