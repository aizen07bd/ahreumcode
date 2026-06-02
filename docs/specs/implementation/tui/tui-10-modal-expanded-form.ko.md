---
id: tui-10-modal-expanded-form-ko
type: spec
status: draft
topics:
  - tui
  - modal
  - expanded-form
  - implementation-spec
summary: Korean section technical specification for AhreumCode TUI modal-like expanded forms.
last_updated: 2026-05-12
related:
  - docs/specs/implementation/tui-technical-spec.ko.md
  - docs/product/tui-ui-command-benchmark.ko.md
  - docs/specs/configuration-policy.ko.md
  - docs/tasks/tui-implementation-todo.ko.md
  - docs/specs/logging-policy.ko.md
---

# tui-10 Modal Expanded Form

## 설명

사용자 정의 항목 추가나 긴 입력이 필요한 expanded form을 구현한다. command list를 대체하지 않고, 필요한 경우에만 열린다.

## 주요 함수

| Function | Role |
| --- | --- |
| `open_expanded_form(form_kind, state)` | form 열기 |
| `ExpandedFormState::focus_next()` | field focus 이동 |
| `update_form_field(field, value)` | field 값 갱신 |
| `validate_form(state)` | field validation |
| `submit_form(state)` | form 결과 제출 |
| `cancel_form(state)` | form 취소 |
| `render_expanded_form(frame, area, state)` | form 렌더 |

## 함수 연결 흐름

```mermaid
flowchart TD
  A[form command action] --> B[open_expanded_form]
  B --> C[log expanded_form_opened]
  C --> D[render_expanded_form]
  D --> E{user input}
  E -->|type| F[update_form_field]
  E -->|tab| G[focus_next]
  E -->|enter| H[validate_form]
  E -->|esc| I[cancel_form]
  F --> J[log expanded_form_field_changed]
  H -->|invalid| D
  H -->|valid| K[submit_form]
  K --> L[log expanded_form_submitted]
  I --> M[log expanded_form_cancelled]
```

## 로그 이벤트

- `expanded_form_opened`
- `expanded_form_field_changed`
- `expanded_form_submitted`
- `expanded_form_cancelled`

## 완료 기준

- expanded form은 command list를 대체하지 않는다.
- local LLM provider/model form을 담을 수 있다.
- 입력/취소/확정 흐름이 명확하다.
