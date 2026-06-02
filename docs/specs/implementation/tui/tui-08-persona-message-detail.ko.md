---
id: tui-08-persona-message-detail-ko
type: spec
status: draft
topics:
  - tui
  - persona
  - messenger
  - implementation-spec
summary: Korean section technical specification for the AhreumCode TUI persona message panel.
last_updated: 2026-05-12
related:
  - docs/specs/implementation/tui-technical-spec.ko.md
  - docs/product/tui-ui-command-benchmark.ko.md
  - docs/tasks/tui-implementation-todo.ko.md
  - docs/specs/logging-policy.ko.md
---

# tui-08 Persona Message Detail

## 설명

persona messenger의 메시지 행을 세부 구현한다. persona는 system log가 아니라 팀 대화처럼 읽혀야 한다.

## 주요 함수

| Function | Role |
| --- | --- |
| `PersonaPanelState::open_full()` | persona full 열기 |
| `PersonaPanelState::close()` | persona panel 닫기 |
| `check_persona_width(term_width)` | width guard |
| `PersonaMessage::from_run_event(event)` | run event에서 persona message 후보 생성 |
| `render_persona_panel(frame, area, state)` | right panel 렌더 |
| `render_persona_message(row)` | `[팀장]`, timestamp, body 렌더 |

## 함수 연결 흐름

```mermaid
flowchart TD
  A[/persona full] --> B[check_persona_width]
  B -->|fail| C[log persona_width_rejected]
  B -->|ok| D[PersonaPanelState::open_full]
  D --> E[log persona_panel_opened]
  E --> F{run event}
  F --> G[PersonaMessage::from_run_event]
  G --> H[render_persona_message]
  H --> I[log persona_message_rendered]
  A2[/persona off] --> J[PersonaPanelState::close]
  J --> K[log persona_panel_closed]
```

## 로그 이벤트

- `persona_panel_opened`
- `persona_panel_closed`
- `persona_message_rendered`
- `persona_width_rejected`

## 완료 기준

- off 상태에서는 우측 영역이 없다.
- full 상태에서만 우측 영역이 생긴다.
- `[팀장]`은 bold/cyan과 `│` accent를 가진다.
- system/tool/validation tag는 오른쪽에 표시되지 않는다.
