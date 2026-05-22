use super::persona::{PersonaMessage, PersonaSpeaker};
use super::persona_runtime::{
    PersonaPeerMessage, PersonaRuntimeStage, PersonaTeamTask, PersonaTeamTaskStatus, PersonaTurn,
    PersonaTurnKind,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaTurnDecision {
    Speak,
    Pass,
}

impl PersonaTurnDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Speak => "speak",
            Self::Pass => "pass",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaRoleProfile {
    pub speaker: PersonaSpeaker,
    pub speaker_id: &'static str,
    pub display_name: &'static str,
    pub role_label: Option<&'static str>,
    pub mission: &'static str,
    pub speak_when: &'static str,
    pub pass_when: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaTurnOutputContract {
    pub speaker: PersonaSpeaker,
    pub allow_pass: bool,
}

impl PersonaTurnOutputContract {
    fn new(speaker: PersonaSpeaker, allow_pass: bool) -> Self {
        Self {
            speaker,
            allow_pass,
        }
    }

    pub fn speaker_id(&self) -> &'static str {
        speaker_id(self.speaker)
    }

    fn render(&self) -> String {
        let speak_shape = format!(
            "Return exactly one JSON object. Speak shape: {{\"speaker\":\"{}\",\"decision\":\"{}\",\"body\":\"...\",\"peer_messages\":[{{\"to\":\"지윤\",\"body\":\"...\"}}]}}.",
            self.speaker_id(),
            PersonaTurnDecision::Speak.as_str(),
        );
        if self.allow_pass {
            return format!(
                "{speak_shape} Pass shape: {{\"speaker\":\"{}\",\"decision\":\"{}\",\"body\":\"\",\"peer_messages\":[]}}.",
                self.speaker_id(),
                PersonaTurnDecision::Pass.as_str(),
            );
        }

        format!(
            "{speak_shape} This turn requires a visible speaker message; do not return decision=\"{}\".",
            PersonaTurnDecision::Pass.as_str()
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaTurnPrompt {
    pub turn_id: String,
    pub speaker: PersonaSpeaker,
    pub role: PersonaRoleProfile,
    pub output_contract: PersonaTurnOutputContract,
    pub system_prompt: String,
    pub user_prompt: String,
}

pub fn persona_role_profile(speaker: PersonaSpeaker) -> PersonaRoleProfile {
    match speaker {
        PersonaSpeaker::Lead => PersonaRoleProfile {
            speaker,
            speaker_id: speaker_id(speaker),
            display_name: "팀장",
            role_label: None,
            mission: "작업 목적을 정리하고 필요한 팀원 의견만 연결한 뒤 진행 방향을 짧게 조율한다.",
            speak_when: "새 작업을 시작하거나 팀원 의견을 종합해야 할 때 말한다.",
            pass_when: "이미 다음 진행 방향이 명확하고 조율할 내용이 없으면 패스한다.",
        },
        PersonaSpeaker::Planning => PersonaRoleProfile {
            speaker,
            speaker_id: speaker_id(speaker),
            display_name: "지윤",
            role_label: Some("기획/설계"),
            mission: "요구사항, 범위, 사용자 기대, 구조적 선택지를 점검한다.",
            speak_when: "요구사항이 모호하거나 범위, 화면 구조, 작업 순서 결정이 필요할 때 말한다.",
            pass_when:
                "요구사항과 범위가 이미 충분히 분명하고 설계 관점의 추가 의견이 없으면 패스한다.",
        },
        PersonaSpeaker::Implementation => PersonaRoleProfile {
            speaker,
            speaker_id: speaker_id(speaker),
            display_name: "민호",
            role_label: Some("구현"),
            mission: "구현 경로, 변경 단위, 기존 코드와의 연결 방식을 점검한다.",
            speak_when:
                "코드 변경 방식, 기술 선택, 영향 범위, 실행 순서에 의견이 필요할 때 말한다.",
            pass_when:
                "구현 관점에서 새로 더할 판단이 없거나 다른 역할 의견만으로 충분하면 패스한다.",
        },
        PersonaSpeaker::Verification => PersonaRoleProfile {
            speaker,
            speaker_id: speaker_id(speaker),
            display_name: "서연",
            role_label: Some("검증"),
            mission: "검증 기준, 실패 가능성, 재현 조건, 확인 범위를 점검한다.",
            speak_when: "테스트 기준, 실패 위험, 확인해야 할 경계 조건이 있을 때 말한다.",
            pass_when: "검증 관점의 별도 위험이나 확인 포인트가 없으면 패스한다.",
        },
        PersonaSpeaker::Documentation => PersonaRoleProfile {
            speaker,
            speaker_id: speaker_id(speaker),
            display_name: "하준",
            role_label: Some("문서"),
            mission: "사용자에게 남길 결정, 변경 기록, 문서 반영 필요성을 점검한다.",
            speak_when: "결정 기록, 사용자 전달, 문서 업데이트 판단이 필요할 때 말한다.",
            pass_when: "기록하거나 전달할 새 결정이 없으면 패스한다.",
        },
    }
}

#[cfg(test)]
pub fn fixed_persona_role_profiles() -> [PersonaRoleProfile; 5] {
    [
        persona_role_profile(PersonaSpeaker::Lead),
        persona_role_profile(PersonaSpeaker::Planning),
        persona_role_profile(PersonaSpeaker::Implementation),
        persona_role_profile(PersonaSpeaker::Verification),
        persona_role_profile(PersonaSpeaker::Documentation),
    ]
}

pub fn build_persona_turn_prompt(
    turn: &PersonaTurn,
    speaker_history: &[PersonaMessage],
) -> PersonaTurnPrompt {
    let role = persona_role_profile(turn.speaker);
    let output_contract =
        PersonaTurnOutputContract::new(turn.speaker, !turn.kind.requires_visible_message());
    let system_prompt = persona_turn_system_prompt(&role, turn, &output_contract);
    let user_prompt = persona_turn_user_prompt(turn, speaker_history);

    PersonaTurnPrompt {
        turn_id: turn.id.clone(),
        speaker: turn.speaker,
        role,
        output_contract,
        system_prompt,
        user_prompt,
    }
}

fn persona_turn_system_prompt(
    role: &PersonaRoleProfile,
    turn: &PersonaTurn,
    output_contract: &PersonaTurnOutputContract,
) -> String {
    if turn.stage == PersonaRuntimeStage::Completion {
        return persona_completion_system_prompt(role, output_contract);
    }

    let contribution_rule = if output_contract.allow_pass {
        "If your role has no useful contribution for this turn, return pass. Do not write a visible 'no opinion' message."
    } else {
        "This turn is mandatory: return a speak decision for the expected speaker, not pass."
    };

    vec![
        "You are one independent AhreumCode persona session, not a script writer for multiple people.".to_owned(),
        "You answer only as the requested speaker.".to_owned(),
        "The main runtime owns tools, evidence, approvals, filesystem changes, execution, and final answers.".to_owned(),
        "Persona sessions own only team discussion: scope, tradeoffs, questions, risk, and coordination.".to_owned(),
        "Use the supplied task state as context only; do not turn raw runtime logs or internal status into persona chat.".to_owned(),
        "Do not state literal file paths, filenames, config keys, provider names, package names, versions, extracted values, or repository facts in visible body or peer_messages; those belong only in the main answer.".to_owned(),
        "Do not propose example path candidates or likely config filenames. Persona discussion stays at role coordination level.".to_owned(),
        "If task state has no successful observation, discuss only coordination and evidence needed; do not state concrete repository facts, file formats, extracted values, versions, paths, filenames, or configuration contents.".to_owned(),
        "If task state reports a failed, limited, canceled, or blocked main runtime status, treat it as unresolved evidence and do not present it as analysis completion.".to_owned(),
        "The team task list is the persona collaboration state. Lead peer_messages open team tasks; addressed members answer those tasks; lead summary/completion closes reported tasks.".to_owned(),
        "Do not mention tool names, command names, function names, API names, or internal operation names in visible body or peer_messages.".to_owned(),
        "Do not expose internal speaker ids such as lead, planning, implementation, verification, or documentation in visible body; use display labels instead.".to_owned(),
        "Do not create new people or specialist roles.".to_owned(),
        "Do not return a messages array, transcript, markdown, or more than one speaker.".to_owned(),
        "Keep body under 140 Korean characters and each peer message body under 120 Korean characters.".to_owned(),
        "Do not repeat the user's action request as a persona action plan; reframe it as task scope, risk, coordination need, or question.".to_owned(),
        contribution_rule.to_owned(),
        "Use peer_messages only when another fixed teammate should answer a concrete role question in a later independent turn.".to_owned(),
        "Peer messages create advisory team tasks for persona discussion; they are not main-runtime execution assignments.".to_owned(),
        format!(
            "Allowed peer message recipients for peer_messages.to: {}. Do not send a peer message to yourself.",
            peer_recipient_options()
        ),
        output_contract.render(),
        format!("Requested speaker id: {}", role.speaker_id),
        format!(
            "Requested speaker label: {}{}",
            role.display_name,
            role.role_label
                .map(|label| format!("({label})"))
                .unwrap_or_default()
        ),
        format!("Role mission: {}", role.mission),
        format!("Speak when: {}", role.speak_when),
        format!("Pass when: {}", role.pass_when),
        stage_rule(turn.stage, turn.kind).to_owned(),
    ]
    .join("\n")
}

fn persona_completion_system_prompt(
    role: &PersonaRoleProfile,
    output_contract: &PersonaTurnOutputContract,
) -> String {
    vec![
        "You are one AhreumCode persona session.".to_owned(),
        "You answer only as the requested speaker.".to_owned(),
        "Completion turn only: close the team discussion from supplied task state.".to_owned(),
        "Use reported team tasks only as teammate contribution. Passed or unreported tasks are not teammate decisions.".to_owned(),
        "If supplied task state is failed, limited, canceled, blocked, or has no successful observation, say only that coordination state closed around an unresolved result; do not state concrete repository facts.".to_owned(),
        "Use Korean, completed/past-tense coordination wording only.".to_owned(),
        "Do not mention tools, commands, files read by persona, internal operation names, literal paths, filenames, config keys, provider names, package values, versions, or final answer facts.".to_owned(),
        "Do not ask teammates. Return peer_messages as an empty array.".to_owned(),
        "Keep body under 100 Korean characters.".to_owned(),
        output_contract.render(),
        format!("Requested speaker id: {}", role.speaker_id),
        format!(
            "Requested speaker label: {}{}",
            role.display_name,
            role.role_label
                .map(|label| format!("({label})"))
                .unwrap_or_default()
        ),
    ]
    .join("\n")
}

fn persona_turn_user_prompt(turn: &PersonaTurn, speaker_history: &[PersonaMessage]) -> String {
    let (history, peer_messages) = if turn.stage == PersonaRuntimeStage::Completion {
        ("없음".to_owned(), "없음".to_owned())
    } else {
        (
            render_speaker_history(speaker_history),
            render_peer_messages(turn.speaker, &turn.peer_messages),
        )
    };
    let user_prompt = if turn.stage == PersonaRuntimeStage::Completion {
        "omitted: completion closure uses task state only"
    } else {
        turn.user_prompt.as_str()
    };

    let team_tasks = render_team_tasks(turn.speaker, &turn.team_tasks);

    format!(
        "turn_id: {turn_id}\ntask_id: {task_id}\nstage: {stage:?}\nkind: {kind:?}\nspeaker: {speaker}\n\n사용자 요청:\n{user_prompt}\n\n현재 작업 상태 요약:\n{task_state_summary}\n\n팀 작업 항목:\n{team_tasks}\n\n이 speaker의 최근 발화:\n{history}\n\n이 speaker에게 전달된 팀 메시지:\n{peer_messages}",
        turn_id = turn.id,
        task_id = turn.task_id,
        stage = turn.stage,
        kind = turn.kind,
        speaker = speaker_display_label(turn.speaker),
        user_prompt = user_prompt,
        task_state_summary = turn.task_state_summary,
    )
}

fn render_speaker_history(messages: &[PersonaMessage]) -> String {
    let recent = messages.iter().rev().take(4).collect::<Vec<_>>();
    if recent.is_empty() {
        return "없음".to_owned();
    }

    recent
        .into_iter()
        .rev()
        .map(|message| message.body.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_peer_messages(speaker: PersonaSpeaker, messages: &[PersonaPeerMessage]) -> String {
    let addressed = messages
        .iter()
        .filter(|message| message.to == speaker)
        .collect::<Vec<_>>();
    if addressed.is_empty() {
        return "없음".to_owned();
    }

    addressed
        .into_iter()
        .map(|message| {
            format!(
                "{} -> {}: {}",
                speaker_display_label(message.from),
                speaker_display_label(message.to),
                message.body
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_team_tasks(speaker: PersonaSpeaker, tasks: &[PersonaTeamTask]) -> String {
    let relevant = tasks
        .iter()
        .filter(|task| task.owner == speaker || speaker == PersonaSpeaker::Lead)
        .collect::<Vec<_>>();
    if relevant.is_empty() {
        return "없음".to_owned();
    }

    relevant
        .into_iter()
        .map(|task| {
            let report = task.report.as_deref().unwrap_or("-");
            format!(
                "{} [{}] owner={} title={} report={}",
                task.id,
                team_task_status_label(task.status),
                speaker_display_label(task.owner),
                task.title,
                report
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn team_task_status_label(status: PersonaTeamTaskStatus) -> &'static str {
    match status {
        PersonaTeamTaskStatus::Open => "open",
        PersonaTeamTaskStatus::Claimed => "claimed",
        PersonaTeamTaskStatus::Reported => "reported",
        PersonaTeamTaskStatus::Passed => "passed",
        PersonaTeamTaskStatus::Closed => "closed",
    }
}

fn stage_rule(stage: PersonaRuntimeStage, kind: PersonaTurnKind) -> &'static str {
    match (stage, kind) {
        (PersonaRuntimeStage::Kickoff, PersonaTurnKind::LeadSummon) => {
            "Turn rule: frame the work as scope and coordination only. Do not repeat the user's action request as an instruction. Invite only relevant fixed team members."
        }
        (PersonaRuntimeStage::Kickoff, PersonaTurnKind::MemberResponse) => {
            "Turn rule: answer the lead from your own role only, or pass if your role adds nothing useful."
        }
        (PersonaRuntimeStage::Kickoff, PersonaTurnKind::LeadSummary) => {
            "Turn rule: summarize the useful team direction briefly without pretending to be the final answer."
        }
        (PersonaRuntimeStage::Progress, PersonaTurnKind::ProgressLead) => {
            "Turn rule: use the supplied task lifecycle state only as coordination context, explain how the team discussion should adapt, and ask at most three fixed teammates through peer_messages only when their role perspective is needed for the next step. Do not restate tool output as facts."
        }
        (PersonaRuntimeStage::Progress, PersonaTurnKind::MemberResponse) => {
            "Turn rule: answer only the progress question addressed to your role, or pass if your role adds nothing useful."
        }
        (PersonaRuntimeStage::Progress, PersonaTurnKind::LeadSummary) => {
            "Turn rule: briefly summarize the coordination direction based on the supplied lifecycle state and teammate viewpoints. Do not claim final-answer authority or announce execution."
        }
        (PersonaRuntimeStage::FollowUp, PersonaTurnKind::FollowUpLead) => {
            "Turn rule: treat the user message as a possible follow-up, retry, correction, complaint, or unrelated new task. Use prior task context only if the current message depends on it. Ask at most three fixed teammates through peer_messages when their role perspective is needed."
        }
        (PersonaRuntimeStage::FollowUp, PersonaTurnKind::MemberResponse) => {
            "Turn rule: answer only the follow-up question addressed to your role, or pass if your role adds nothing useful."
        }
        (PersonaRuntimeStage::FollowUp, PersonaTurnKind::LeadSummary) => {
            "Turn rule: briefly connect the useful role feedback to the user's current message without claiming final-answer authority."
        }
        (PersonaRuntimeStage::Completion, PersonaTurnKind::Completion) => {
            "Turn rule: close from the supplied task state only, using completed or past-tense coordination wording. Do not ask teammates, do not continue kickoff or progress planning, do not announce future work, do not say you are entering or starting verification, implementation, reading, extraction, or analysis, and return peer_messages as an empty array."
        }
        _ => "Turn rule: stay within the requested speaker role and pass if there is no useful contribution.",
    }
}

pub fn speaker_id(speaker: PersonaSpeaker) -> &'static str {
    match speaker {
        PersonaSpeaker::Lead => "lead",
        PersonaSpeaker::Planning => "planning",
        PersonaSpeaker::Implementation => "implementation",
        PersonaSpeaker::Verification => "verification",
        PersonaSpeaker::Documentation => "documentation",
    }
}

pub fn speaker_display_label(speaker: PersonaSpeaker) -> String {
    let role = persona_role_profile(speaker);
    match role.role_label {
        Some(label) => format!("{}({label})", role.display_name),
        None => role.display_name.to_owned(),
    }
}

fn peer_recipient_options() -> String {
    [
        PersonaSpeaker::Lead,
        PersonaSpeaker::Planning,
        PersonaSpeaker::Implementation,
        PersonaSpeaker::Verification,
        PersonaSpeaker::Documentation,
    ]
    .into_iter()
    .map(speaker_display_label)
    .collect::<Vec<_>>()
    .join(", ")
}

pub fn speaker_from_peer_recipient(value: &str) -> Option<PersonaSpeaker> {
    PersonaSpeaker::from_id(value).or_else(|| match value {
        "팀장" => Some(PersonaSpeaker::Lead),
        "지윤" | "지윤(기획/설계)" | "기획/설계" => Some(PersonaSpeaker::Planning),
        "민호" | "민호(구현)" | "구현" => Some(PersonaSpeaker::Implementation),
        "서연" | "서연(검증)" | "검증" => Some(PersonaSpeaker::Verification),
        "하준" | "하준(문서)" | "문서" => Some(PersonaSpeaker::Documentation),
        _ => None,
    })
}

pub fn speaker_from_output_value(value: &str) -> Option<PersonaSpeaker> {
    speaker_from_peer_recipient(value)
}
