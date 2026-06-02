---
id: tool-04-permission-branches-ko
type: spec
status: implemented
topics:
  - tool-runtime
  - permission
  - approval
  - safety
  - implementation-spec
summary: Korean technical specification for routing tool candidates through allow, approval, and deny permission branches.
last_updated: 2026-05-17
related:
  - docs/specs/implementation/tool-runtime-technical-spec.ko.md
  - docs/specs/permission-mode-policy.ko.md
  - docs/specs/implementation/tool/tool-03-tool-loop-binding.ko.md
  - docs/tasks/tool-runtime-todo.ko.md
---

# tool-04 Permission Branches

## 목적

`tool-04`는 모델이 만든 tool candidate를 실행하기 전에 permission branch로 분리한다.

이 단계의 목적은 더 많은 도구를 실행하는 것이 아니다. 목적은 `allow`, `ask`, `deny`를 명확히 나누고, 승인 대상은 TUI approval surface로 보내는 것이다.

## 범위

포함:

- workspace-local Explore allow branch
- web/network Explore ask or deny branch
- Change/Execute/Configure ask branch
- command capability policy branch
- approval surface 연결
- permission log event

제외:

- 승인 후 실제 mutation 실행
- 승인 후 실제 command 실행
- web search/fetch 실행
- apply_patch preview
- full LLM E2E

## 초기 Branch Policy

```text
Explore local workspace read/search/list/status -> allow
Explore web_search/web_fetch with web.enabled=true -> ask
Explore web_search/web_fetch with web.enabled=false -> deny
Change apply_patch -> ask
Execute run_command safe verification/build-test -> ask
Execute run_command ManualOnly capability -> deny/guidance
Configure add_provider/update_config -> ask
ManualOnly command -> deny
```

Command capability 초기 기준:

| Capability | Initial branch |
| --- | --- |
| read-only | ask |
| build/test | ask |
| mutation | ask |
| process control | ManualOnly deny |
| destructive filesystem | ManualOnly deny |
| system-level | ManualOnly deny |
| external service | ManualOnly deny |
| high-load | ManualOnly deny |
| unknown invalid command | ManualOnly deny |

ManualOnly deny는 실행 우회가 아니다. approval surface에 올리지 않고 사용자가 직접 검토/실행해야 하는 guidance 경계다.

## Approval Surface

approval surface에는 실제 tool candidate 정보를 넣는다.

```text
title: Approval required
reason: Change/Execute/Configure or network access requires approval.
action: apply_patch (Change)
details: reason, tool name, activity
```

이 단계에서 사용자가 approve하더라도 실제 실행은 하지 않는다. 후속 `tool-05+` 단계에서 preview/execution branch를 구현한다.

## 완료 기준

- local Explore는 기존 tool loop로 계속 실행된다.
- web Explore는 approval 또는 deny branch로 간다.
- Change/Execute/Configure는 approval surface로 간다.
- ManualOnly command는 approval surface에 올리지 않고 workspace/log에 deny로 남긴다.
- `cargo fmt --check`가 통과한다.
- `cargo test`가 통과한다.
- `cargo run -- --scene main --smoke`가 통과한다.

## Change History

### 2026-05-17

- Created `tool-04` technical spec before implementation.
- Implemented `PermissionGate` with allow, ask, and deny branches.
- Connected web/network and Change/Execute/Configure candidates to the TUI approval surface.
- Added hard safety deny branch for elevated/process/destructive command candidates.
- Added permission branch log events and focused permission tests.
- Verified with `cargo fmt --check`, `cargo test`, and `cargo run -- --scene main --smoke`.
- Replaced the hard safety command-name branch with `CommandPolicy` capability classification.
- Routed destructive filesystem, system-level, process-control, external-service, high-load, and unknown invalid commands to ManualOnly deny before approval.
