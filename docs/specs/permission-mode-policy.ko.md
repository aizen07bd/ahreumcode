---
id: permission-mode-policy-ko
type: spec
status: draft
topics:
  - permissions
  - modes
  - guide-mode
  - crew-mode
  - pilot-mode
  - safety
summary: Korean specification draft for AhreumCode work modes and initial permission policy.
last_updated: 2026-05-11
related:
  - docs/product/tool-call-benchmark.ko.md
  - docs/specs/model-response-contract.ko.md
  - docs/specs/intent-frame-uncertainty-gate.ko.md
  - docs/product/tui-ui-command-benchmark.ko.md
  - docs/architecture/agent-operating-guardrails.md
  - docs/PROJECT-CONTEXT.md
---

# Permission Mode Policy Korean Draft

## 목적

이 문서는 아름코드의 초기 work mode와 permission policy를 정의한다.

전 프로젝트의 `Human / Angel / God` 모드 개념은 다음 방향으로 재해석한다.

```text
Human  -> Guide
Angel  -> Crew
God    -> Pilot
```

이름은 새 프로젝트의 현재 기준이다. 이전 이름은 참조용이며 현재 제품 표면에는 사용하지 않는다.

## Core Principles

```text
Mode = user-facing work autonomy preset
Permission matrix = internal allow/ask/deny capability policy
Intent/uncertainty gate = whether the requested action is clear enough
Hard safety limits = mode보다 우선하는 실행 금지/강제 확인 경계
```

모드는 권한을 완전히 대체하지 않는다.

모드는 사용자가 이해하기 쉬운 preset이고, 실제 실행 권한은 다음을 함께 평가한다.

- activity group: `Explore`, `Change`, `Execute`, `Configure`, `Ask`
- concrete tool
- target path
- command/cwd
- network/external access
- destructive risk
- intent clarity
- hard safety limits

## Work Modes

```text
Guide
Crew
Pilot
```

### Guide Mode

목적:

```text
사용자가 주도하고 아름코드는 안내/보조한다.
```

성격:

- ask-first
- high-control
- 중요한 행동 전에 사용자 확인
- 기획, 리뷰, 위험 작업, 초보자 흐름에 적합

초기 정책:

```text
Explore/local docs       allow
Explore/local workspace  ask or limited allow
Explore/web              ask
Change                   ask
Execute                  ask
Configure                ask
Ask                      allow
```

설명 문구 후보:

```text
Guide
Ask-first, user-led work.
Best for careful review, planning, and high-control tasks.
```

### Crew Mode

목적:

```text
아름코드 팀이 안전 경계 안에서 함께 일한다.
```

성격:

- recommended default
- bounded Explore 자동 허용
- search/read는 사용자의 의중을 만족하기 위해 적극 수행
- Change/Execute/Configure는 승인 필요
- 일상적인 로컬 개발 작업에 적합

초기 정책:

```text
Explore/local docs       allow
Explore/local workspace  allow if bounded
Explore/web              ask
Change                   ask
Execute                  ask
Configure                ask
Ask                      allow
```

설명 문구 후보:

```text
Crew
Safe team autonomy within configured boundaries.
Recommended for everyday local development.
```

### Pilot Mode

목적:

```text
아름코드가 더 주도적으로 작업을 몰고 간다.
```

성격:

- broad autonomy
- 사용자가 명시적으로 켜야 함
- 세션 단위 활성화를 기본 후보로 둔다
- hard safety limits는 계속 적용
- "무제한 권한"이 아니다

초기 정책:

```text
Explore/local docs       allow
Explore/local workspace  allow
Explore/web              ask or configured allow
Change/apply_patch       ask initially, later limited allow candidate
Change/delete/rename     ask or deny
Execute/safe verify      ask initially, later allowlist candidate
Execute/general command  ask
Configure                ask
Ask                      allow
```

설명 문구 후보:

```text
Pilot
Agent-led work with broader autonomy.
Hard safety limits remain.
```

## Default Mode

초기 기본 모드는 `Crew`로 둔다.

이유:

- read/search까지 매번 묻는 것은 로컬 코딩 에이전트 UX를 답답하게 만든다.
- bounded workspace Explore는 로컬 LLM의 불확실성을 줄이는 데 필요하다.
- Change/Execute/Configure는 여전히 승인받으므로 안전 경계를 유지한다.
- persona/team 컨셉과 가장 잘 맞는다.

## Mode Picker

`/mode`는 prompt-adjacent stepped picker로 제공한다.

예:

```text
Select Work Mode

  Guide   ask-first, user-led work
> Crew    safe team autonomy (recommended)
  Pilot   agent-led work with hard safety limits

Enter select  Esc back
```

Pilot 선택 시 explicit confirmation을 요구한다.

예:

```text
Pilot mode enables broader agent-led work.
Hard safety limits still apply.

> 1. Enable Pilot for this session
  2. Cancel
```

## Hard Safety Limits

다음 경계는 모든 모드보다 우선한다.

초기 정책:

| Capability/Risk | Initial Policy |
| --- | --- |
| secrets, `.env`, private keys, tokens | deny or hard ask |
| outside workspace access | ask or deny |
| sudo/admin/elevated privilege | deny by default |
| destructive commands | deny or hard ask |
| dependency install | ask |
| network access | ask |
| web login/session interaction | deny |
| kill unrelated processes | deny |
| delete files/directories | ask or deny |
| repeated identical tool calls | ask or block by loop guard |

`Pilot`도 이 hard safety limits를 우회하지 못한다.

## Activity Uncertainty Policy

상세 기준은 `docs/specs/intent-frame-uncertainty-gate.ko.md`를 따른다.

불확실성 정책은 모드보다 우선한다.

핵심 문장:

```text
Uncertainty is allowed for bounded read/search exploration.
Uncertainty is not allowed for mutation, execution, or configuration.
```

### Explore

정책:

- workspace-local, bounded read/search 범위에서는 불확실성이 있어도 진행 가능하다.
- 사용자가 파일을 모르는 경우에도 후보 탐색과 여러 파일 읽기는 가능하다.
- 단, workspace 밖, network, 대량 파일 읽기는 별도 정책/승인 대상이다.

예:

```text
"권한 관련 문서가 뭐였는지 알려줘"
-> Search text "permission|approval"
-> Read matching docs
-> Summarize with evidence
```

### Change

정책:

- target/scope가 명확하지 않으면 실행 금지.
- 후보가 여러 개이면 사용자에게 묻는다.
- 변경 전 preview와 approval이 필요하다.
- delete/rename은 더 엄격하게 다룬다.

### Execute

정책:

- command/cwd/purpose가 명확하지 않으면 실행 금지.
- test/build도 초기에는 approval 대상이다.
- safe verification allowlist는 나중에 열 수 있다.

### Configure

정책:

- explicit user intent 없이는 설정 변경 금지.
- provider/model/config 변경은 expanded form 또는 명시적 approval을 거친다.

### Ask

정책:

- 사용자 판단이 필요한 지점에서 사용한다.
- Change/Execute/Configure에서 불확실성이 있으면 Ask로 전환한다.

## Permission Levels

전 프로젝트의 lv0~lv3 모델은 내부 개념으로 유지할 수 있다.

초기 후보:

| Level | Meaning |
| --- | --- |
| lv0 | low-risk read, inspect, analyze, status-check |
| lv1 | limited workspace create/update and safe verification candidates |
| lv2 | delete, outside-workspace, general command, network |
| lv3 | elevated privilege, process control, listening ports, web login, destructive operations |

주의:

- level은 위험도 분류다.
- 같은 level 안에서도 mode별 allow/ask/deny는 다를 수 있다.
- 사용자 UI는 mode summary를 먼저 보여주고, 상세 permission matrix는 고급 설정으로 미룬다.

## Initial Matrix Summary

| Activity/Capability | Guide | Crew | Pilot |
| --- | --- | --- | --- |
| Explore/local docs | allow | allow | allow |
| Explore/local workspace bounded | ask or limited allow | allow | allow |
| Explore/web | ask | ask | ask/configured allow |
| Change/apply_patch | ask | ask | ask initially |
| Change/delete/rename | ask/deny | ask/deny | ask/deny |
| Execute/safe verification | ask | ask | ask initially |
| Execute/general command | ask | ask | ask |
| Configure | ask | ask | ask |
| Ask | allow | allow | allow |

## Non-Goals

- `Pilot`은 no-permission mode가 아니다.
- mode는 intent ambiguity를 해결하지 않는다.
- mode는 hard safety limits를 우회하지 않는다.
- mode는 persona style이나 team dialogue를 바꾸는 기능이 아니다.
- 처음부터 모든 low-level permission을 사용자 설정 UI에 노출하지 않는다.

## Open Implementation Checks

- `Guide`에서 bounded workspace Explore를 `ask`로 둘지 `limited allow`로 둘지 실제 UX에서 확인한다.
- `Pilot`에서 `apply_patch` limited allow를 언제 열지 결정한다.
- safe verification command allowlist 기준을 정한다.
- `/mode` 변경이 session-only인지 config-persistent인지 결정한다.
- permission matrix 저장 형식을 `.ahreumcode/config.toml`에 어떻게 표현할지 결정한다.

## Change History

### 2026-05-11

- Created Guide/Crew/Pilot mode policy draft from the previous Human/Angel/God concept.
