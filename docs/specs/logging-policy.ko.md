---
id: logging-policy-ko
type: spec
status: draft
topics:
  - logging
  - diagnostics
  - local-llm
  - tool-calling
  - privacy
summary: Korean specification draft for local runtime logging and diagnostic evidence in AhreumCode.
last_updated: 2026-05-15
related:
  - docs/architecture/llmagent-failure-analysis.md
  - docs/architecture/implementation-sequence.ko.md
  - docs/tasks/tui-implementation-todo.ko.md
  - docs/specs/model-response-contract.ko.md
  - docs/specs/intent-frame-uncertainty-gate.ko.md
  - docs/specs/configuration-policy.ko.md
  - docs/PROJECT-CONTEXT.md
---

# Logging Policy Korean Draft

## 목적

이 문서는 아름코드의 로컬 runtime logging 정책을 정의한다.

전 프로젝트는 충분한 runtime log가 없어 실패 원인을 사후 분석하기 어려웠다. 아름코드는 로컬 LLM 도구 호출 실패를 제품 핵심 문제로 다루기 때문에, 로그는 선택 기능이 아니라 초기 아키텍처 필수 기능이다.

핵심 문장:

```text
No logs, no diagnosis.
No diagnosis, no reliable local-LLM tool calling.
```

## Core Principles

```text
로그는 많이 남긴다.
화면에는 필요한 것만 보여준다.
로그는 로컬 전용이다.
민감정보는 저장 전에 제거한다.
로그가 없으면 해당 구현 단계는 완료가 아니다.
```

정책:

- 모든 구현 단계는 해당 단계의 runtime log를 남겨야 완료로 본다.
- 로그는 분석용이고, TUI 화면은 사용자 가독성용이다.
- TUI workspace/persona에 모든 로그를 노출하지 않는다.
- log file은 git에 올리지 않는다.
- secret, token, key, credential, `.env` 값은 저장하지 않는다.

## Storage Location

기본 저장 위치:

```text
.ahreumcode/logs/
```

현재 구조:

```text
.ahreumcode/logs/
  sessions/
    2026-05-15/
      sessions.jsonl
      events.jsonl
      llm.jsonl
      tools.jsonl
      permissions.jsonl
      ui.jsonl
      persona.jsonl
      errors.jsonl
```

정책:

- 날짜 폴더는 유지한다.
- 실행마다 session directory를 만들지 않는다.
- session start/end 요약은 `sessions.jsonl`에 append한다.
- 각 JSONL event는 `session_id`를 포함하므로 날짜 bucket 안에서 session별 필터링이 가능해야 한다.

`.ahreumcode/`는 `.gitignore` 대상이다.

## Log Files

| File | Purpose |
| --- | --- |
| `sessions.jsonl` | session start/end, workspace, mode, provider/model, config snapshot summary |
| `events.jsonl` | high-level run events, working phases, answers, blocked/manual-only states |
| `llm.jsonl` | local LLM request/response metadata, parse result, repair attempt |
| `tools.jsonl` | tool candidate, validated arguments, execution start/end, observation summary |
| `permissions.jsonl` | approval request/result, blocked, ManualOnly, policy decision |
| `ui.jsonl` | scene transition, command action, prompt submit, cancel, exit |
| `persona.jsonl` | persona message source event, rendered speaker/message metadata |
| `errors.jsonl` | network failure, parse failure, timeout, unexpected recoverable error |

## Common Event Shape

모든 JSONL event는 공통 envelope를 가진다.

```json
{
  "ts": "2026-05-11T12:34:56.123+09:00",
  "session_id": "20260511-123456",
  "run_id": "run-0001",
  "turn_id": 3,
  "scope_id": "tui-01-intro-scene",
  "level": "info",
  "event": "intro_rendered",
  "data": {}
}
```

필드:

| Field | Required | Meaning |
| --- | --- | --- |
| `ts` | yes | local timestamp with timezone |
| `session_id` | yes | session id |
| `run_id` | when applicable | user prompt/run id |
| `turn_id` | when applicable | local LLM turn id |
| `scope_id` | yes | active implementation/runtime scope such as `tui-01-intro-scene` |
| `level` | yes | `trace`, `debug`, `info`, `warn`, `error` |
| `event` | yes | stable event name |
| `data` | yes | event-specific structured data |

## Scope Id Rule

각 영역 todo는 자기 번호 체계를 가진다.

예:

```text
tui-01-intro-scene
tui-02-epilogue-scene

llm-01-lm-studio-connection
llm-02-plain-response-display

loop-01-response-envelope-parse
loop-02-repair-on-invalid-json

tool-01-read-file
tool-02-list-files

config-01-load-default-config
policy-01-manual-only-gate
session-01-run-records
```

금지:

- `llm-02`, `loop-03`, `tool-05`처럼 전체 구현 순서 번호를 영역별 todo 번호로 섞지 않는다.
- 각 영역 todo는 `01`부터 시작한다.
- log `scope_id`는 실제 작업 ID와 맞춘다.

## Step Completion Rule

모든 구현 단계의 완료 조건:

```text
Feature visible/working is not enough.
Diagnostic log for that feature must also exist.
```

예:

| Step | Not Enough | Required Logs |
| --- | --- | --- |
| `tui-01-intro-scene` | intro 화면 표시 | `app_started`, `intro_rendered`, `prompt_focus_ready` |
| `tui-02-epilogue-scene` | epilogue 화면 표시 | `exit_requested`, `session_summary_created`, `epilogue_rendered` |
| `llm-01-lm-studio-connection` | response 수신 | `llm_request_started`, `llm_response_received`, `llm_request_failed` |
| `loop-01-response-envelope-parse` | parse 성공 | `raw_response_received`, `json_parse_succeeded`, `json_parse_failed` |
| `tool-01-read-file` | 파일 읽기 성공 | `tool_candidate_received`, `tool_args_validated`, `tool_execution_completed` |
| `policy-01-manual-only-gate` | 위험 요청 차단 | `permission_decision_recorded`, `manual_only_selected` |

## Redaction Policy

로그 저장 전 redaction을 수행한다.

대상:

- API keys
- tokens
- passwords
- private keys
- `.env` values
- credential files
- authorization headers
- cookies/session values
- obvious secret-like values

정책:

- redaction은 저장 전에 수행한다.
- redaction 실패가 의심되면 raw log 저장을 중단하고 `errors.jsonl`에 redaction failure summary만 남긴다.
- secret 원문은 TUI에도 보여주지 않는다.

표현 후보:

```text
[REDACTED:api_key]
[REDACTED:token]
[REDACTED:secret]
```

## Raw Content And Size Policy

로컬 LLM 분석에는 원문이 필요할 수 있다. 그러나 파일 내용과 command output은 무제한 저장하지 않는다.

정책:

- LLM request/response는 redaction 후 저장한다.
- 긴 file content, command output, web fetch body는 byte/line limit을 둔다.
- 큰 원문은 별도 artifact로 저장할 수 있지만, artifact도 `.ahreumcode/logs/` 아래에 둔다.
- JSONL에는 artifact path, hash, byte length, summary를 남긴다.
- binary content는 원문 저장하지 않는다.

초기 후보:

```text
max_inline_text_bytes = 20000
max_artifact_bytes = 1000000
```

상한값은 구현 전 실제 성능을 보고 조정한다.

## Screen Versus Log

화면과 로그는 다르다.

화면:

- 사용자가 지금 판단해야 하는 요약
- workspace 출력
- prompt-adjacent approval
- persona social message
- statusline 요약

로그:

- raw/structured diagnostic evidence
- request/response
- validation details
- tool arguments
- policy decisions
- timing/error metadata

원칙:

- 로그를 화면에 전부 뿌리지 않는다.
- persona messenger에 system log를 넣지 않는다.
- workspace에는 사용자가 이해할 수 있는 event만 표시한다.
- 상세 로그 위치는 `/status` 또는 향후 `/logs`에서 보여줄 수 있다.

## Implementation Sequence Integration

로그는 별도 마지막 단계가 아니다. 모든 단계에 관통한다.

| Area | Logging Starts |
| --- | --- |
| TUI | `tui-01-intro-scene`부터 app/scene/input log |
| Local LLM | `llm-01-*`부터 request/response/error log |
| JSON Loop | `loop-01-*`부터 parse/validation/repair log |
| Persona | `persona-01-*`부터 source event/rendered message log |
| Tool | `tool-01-*`부터 candidate/args/execution/observation log |
| Config | `config-01-*`부터 load/source/default/validation log |
| Policy | `policy-01-*`부터 permission/ManualOnly/Blocked log |
| Session | `session-01-*`부터 history/context/evidence log |

## Initial Event Names

TUI:

```text
app_started
terminal_entered
terminal_restored
intro_rendered
prompt_focus_ready
exit_requested
epilogue_rendered
scene_changed
command_surface_opened
command_selected
```

LLM:

```text
llm_request_started
llm_response_received
llm_request_failed
llm_latency_recorded
```

Loop:

```text
raw_response_received
json_parse_succeeded
json_parse_failed
schema_validation_succeeded
schema_validation_failed
repair_requested
repair_failed
```

Tool:

```text
tool_candidate_received
tool_args_validated
tool_args_rejected
tool_execution_started
tool_execution_completed
tool_execution_failed
observation_recorded
```

Policy:

```text
uncertainty_gate_decided
permission_decision_recorded
approval_requested
approval_accepted
approval_denied
manual_only_selected
blocked_selected
```

Persona:

```text
persona_panel_opened
persona_panel_closed
persona_message_created
persona_message_rendered
persona_width_rejected
```

## Non-Goals

- 로그를 원격 서버로 업로드하지 않는다.
- 로그를 git에 포함하지 않는다.
- 로그 viewer를 초기 구현 목표로 삼지 않는다.
- 모든 로그를 TUI 화면에 표시하지 않는다.
- 테스트 파일을 로그 이벤트마다 만들지 않는다.

## Open Implementation Checks

- session id 생성 규칙을 정한다.
- JSONL writer flush 주기를 정한다.
- crash/panic 시 terminal restore와 마지막 log flush 순서를 정한다.
- redaction pattern의 초기 목록을 구현 전에 구체화한다.
- `/status`에 log path를 표시할지, 별도 `/logs` command를 둘지 결정한다.

## Change History

### 2026-05-11

- Created the initial runtime logging policy.
