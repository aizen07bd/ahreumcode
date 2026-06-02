---
id: llm-10-diagnostics-and-status-ko
type: spec
status: draft
topics:
  - local-llm
  - diagnostics
  - status
  - logging
summary: Korean section technical specification for llm-10 diagnostics and status.
last_updated: 2026-05-14
related:
  - docs/specs/implementation/local-llm-runtime-technical-spec.ko.md
  - docs/tasks/local-llm-runtime-todo.ko.md
  - docs/specs/logging-policy.ko.md
---

# llm-10 Diagnostics And Status

## 설명

Local LLM Runtime의 최근 상태를 `/status`, `/health`, 로그에서 확인할 수 있게 마무리한다. 이 단계는 다음 큰 범위인 tool runtime으로 넘어가기 전 진단 가능성을 확보한다.

## 주요 함수

| Function | Role |
| --- | --- |
| `LlmDiagnostics::snapshot(runtime)` | 최근 runtime 상태 요약을 만든다. |
| `collect_latency_summary(logs)` | 최근 latency 정보를 요약한다. |
| `collect_failure_summary(logs)` | 최근 실패 원인을 요약한다. |
| `render_llm_status(snapshot)` | `/status`에 LLM 상태를 표시한다. |
| `mark_runtime_ready_for_tools(snapshot)` | tool 단계 진입 가능 여부를 기록한다. |
| `LlmDiagnosticsState::record_health(report)` | `/health` 결과를 최근 LLM 진단 상태에 반영한다. |
| `LlmDiagnosticsState::record_parse_*()` | parser 성공/실패 상태를 진단 상태에 반영한다. |
| `LlmDiagnosticsState::record_decision*()` | decision gate 결과를 진단 상태에 반영한다. |

## 함수 연결 흐름

```mermaid
flowchart TD
  A[/status or /health] --> B[LlmDiagnostics::snapshot]
  B --> C[collect_latency_summary]
  B --> D[collect_failure_summary]
  C --> E[render_llm_status]
  D --> E
  E --> F[mark_runtime_ready_for_tools]
  F --> G[log status snapshot]
```

## 로그 이벤트

- `llm_diagnostics_requested`
- `llm_diagnostics_rendered`
- `llm_status_snapshot_recorded`
- `llm_runtime_ready_for_tool_stage`

## 구현 정책

- `/status`는 기존 runtime status에 더해 LLM diagnostics snapshot을 workspace에 표시한다.
- `/health` 결과는 `LlmDiagnosticsState`에 기록되어 이후 `/status`와 모순되지 않게 한다.
- 최근 request, parse, repair, decision, failure 요약을 메모리 상태로 유지한다.
- raw prompt, raw response, schema prompt 원문은 diagnostics snapshot에 넣지 않는다.
- `llm_runtime_ready_for_tool_stage`는 구조 연결 상태를 의미한다.
- 실제 LM Studio E2E 검증 완료를 의미하지 않는다. 로그에는 `e2e_verified: false`를 남긴다.

현재 `/status` 추가 출력 항목:

| Item | Meaning |
| --- | --- |
| `llm provider/model` | 현재 provider와 model |
| `llm endpoint/context/mode/web` | endpoint, context, mode, web 상태 |
| `llm health` | 최근 `/health` 결과 또는 `not_checked` |
| `llm request` | 최근 LLM request 상태와 latency |
| `llm parse` | 최근 parser 결과 |
| `llm repair` | 최근 repair 결과 |
| `llm decision` | 최근 decision gate 결과 |
| `llm failure` | 최근 실패 요약 |
| `tool stage` | 다음 tool stage 진입 구조 준비 여부 |

## 완료 기준

- `/status`에서 provider/model/base url/context와 최근 LLM 상태를 볼 수 있다.
- `/health` 결과와 `/status` 요약이 모순되지 않는다.
- 로그로 실패 원인을 재구성할 수 있다.
- tool stage readiness는 구조 준비와 E2E 검증을 구분해 기록한다.
- scope id `llm-10-diagnostics-and-status` 로그가 남는다.
