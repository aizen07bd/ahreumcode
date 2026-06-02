---
id: tui-04-command-area-basic-actions-ko
type: spec
status: draft
topics:
  - tui
  - slash-command
  - implementation-spec
summary: Korean section technical specification for the AhreumCode TUI command area and basic actions.
last_updated: 2026-05-12
related:
  - docs/specs/implementation/tui-technical-spec.ko.md
  - docs/product/tui-ui-command-benchmark.ko.md
  - docs/tasks/tui-implementation-todo.ko.md
  - docs/specs/logging-policy.ko.md
---

# tui-04 Command Area Basic Actions

## 설명

prompt 아래, statusline 위에 command surface를 표시하고 기본 command를 실행한다.

## 주요 함수

| Function | Role |
| --- | --- |
| `CommandRegistry::new()` | command metadata 등록 |
| `open_command_surface(state)` | `/` 입력 시 command surface 열기 |
| `filter_commands(query, registry)` | keyword 기반 command filtering |
| `move_command_selection(delta, state)` | 방향키 선택 이동 |
| `confirm_command_selection(state)` | 선택 command 확정 |
| `dispatch_basic_command(command, state)` | `/exit`, `/quit`, `/status`, persona shell 처리 |

## 함수 연결 흐름

```mermaid
flowchart TD
  A[key '/'] --> B[open_command_surface]
  B --> C[log command_surface_opened]
  C --> D[user types keyword]
  D --> E[filter_commands]
  E --> F[log command_filter_changed]
  F --> G{arrow or enter}
  G -->|arrow| H[move_command_selection]
  G -->|enter| I[confirm_command_selection]
  I --> J[dispatch_basic_command]
  J --> K[log command_action_dispatched]
```

## 로그 이벤트

- `command_surface_opened`
- `command_filter_changed`
- `command_selected`
- `command_action_dispatched`

## 완료 기준

- `/` 입력으로 command surface가 열린다.
- keyword filtering이 동작한다.
- 방향키와 Enter로 선택/실행한다.
- 기본 command가 state 전이를 만든다.
