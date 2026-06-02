---
id: tool-05-change-tool-preview-ko
type: spec
status: implemented
topics:
  - tool-runtime
  - change
  - apply-patch
  - diff-preview
  - implementation-spec
summary: Korean technical specification for preparing apply_patch candidates as preview and approval targets without executing them.
last_updated: 2026-05-22
related:
  - docs/specs/implementation/tool-runtime-technical-spec.ko.md
  - docs/specs/model-response-contract.ko.md
  - docs/specs/permission-mode-policy.ko.md
  - docs/tasks/tool-runtime-todo.ko.md
---

# tool-05 Change Tool Preview

## 목적

`tool-05`는 `apply_patch` 후보를 실행하지 않고 preview/diff/approval 대상으로 준비한다.

이 단계의 목표는 mutation 실행이 아니다. 목표는 raw payload가 apply_patch 형식인지 검증하고, 변경 대상과 영향도를 workspace/approval surface에 보여주는 것이다.

## 범위

포함:

- `apply_patch` payload grammar check
- patch target extraction
- addition/deletion count extraction
- workspace-relative target guard
- workspace diff summary 표시
- approval details에 patch preview metadata 연결

제외:

- patch 실제 적용
- delete/rename 실행
- multi-file patch 실행
- fuzzy patch/edit
- full inline diff 확장 UI

## 초기 Policy

```text
apply_patch payload must:
  begin with *** Begin Patch
  end with *** End Patch
  contain exactly one Add/Update/Delete File target
  use workspace-relative target path
```

Preview metadata:

```text
target_path
operation
additions
deletions
payload_id
```

## 완료 기준

- malformed patch payload는 approval로 올라가지 않는다.
- multi-file patch는 clarification으로 전환된다.
- valid single-file patch는 workspace diff summary에 표시된다.
- approval surface details에 patch target/operation/additions/deletions가 표시된다.
- `cargo fmt --check`가 통과한다.
- `cargo test`가 통과한다.
- `cargo run -- --scene main --smoke`가 통과한다.

## Multi-Target Contract Update

2026-05-22 실제 TUI 검증에서 `index.html`, `styles.css`, `game.js`를 한 번에 생성하는 분리형 웹게임 요청이 실패했다.

실패 원인:

```text
apply_patch target count must be exactly one, got 3
```

따라서 `tool-05`의 기존 완료 범위는 다음처럼 해석한다.

- `tool-05`는 single-target change preview를 안정화한 완료 작업이다.
- multi-target change는 `tool-05`의 누락 수정으로 몰래 확장하지 않는다.
- multi-target change는 `tool-11-multi-target-apply-patch`에서 별도 런타임 capability로 연다.

새 계약:

```text
One local LLM response = one next action candidate
One Change candidate = one apply_patch payload
One apply_patch payload = one or more file targets
```

즉 복수 파일 변경은 여러 tool call이 아니라 하나의 atomic change proposal이다.

`tool-11`에서 열어야 하는 preview metadata:

```text
operation_count
targets[]
  target_path
  operation
  additions
  deletions
payload_id
total_additions
total_deletions
```

승인 화면은 전체 변경 요약과 target별 요약을 함께 보여줘야 한다. target별 검증 실패가 있으면 payload 전체를 승인 후보로 올리지 않는다.

금지:

- 사용자 프롬프트에 `html`, `css`, `js`가 있는지 보고 복수 파일로 분기하기
- 특정 파일명 조합을 보고 special case 처리하기
- 실패한 multi-target patch를 여러 single-target tool call로 조용히 쪼개 실행하기
- 일부 target만 성공한 상태를 성공 observation으로 기록하기

## Change History

### 2026-05-22

- Recorded that the original `tool-05` implementation was single-target preview only.
- Added the multi-target change preview contract for the follow-up `tool-11` capability.
- Preserved the one-candidate-per-response model while allowing one patch document to contain multiple file targets.
- Recorded the real TUI failure that exposed the gap: split web game creation produced three patch targets and the runtime rejected it.

### 2026-05-17

- Created `tool-05` technical spec before implementation.
- Implemented apply_patch preview metadata extraction in `DecisionGate`.
- Added grammar and workspace-relative target guards for apply_patch payloads.
- Connected `ChangePreview` to approval details and workspace diff summary without executing patches.
- Verified with `cargo fmt --check`, `cargo test`, and `cargo run -- --scene main --smoke`.
