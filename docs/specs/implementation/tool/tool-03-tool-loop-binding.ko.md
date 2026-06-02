---
id: tool-03-tool-loop-binding-ko
type: spec
status: implemented
topics:
  - tool-runtime
  - tool-loop
  - local-llm
  - implementation-spec
summary: Korean technical specification for binding Tool Runtime observations back into the local LLM loop.
last_updated: 2026-05-17
related:
  - docs/specs/implementation/tool-runtime-technical-spec.ko.md
  - docs/specs/implementation/tool/tool-01-explore-tool-runtime.ko.md
  - docs/specs/implementation/tool/tool-02-observation-and-truncation.ko.md
  - docs/tasks/tool-runtime-todo.ko.md
  - docs/specs/model-response-contract.ko.md
---

# tool-03 Tool Loop Binding

## 목적

`tool-03`은 모델이 낸 Explore tool 후보를 실행한 뒤, 그 observation을 다시 LLM에게 전달하는 루프를 연결한다.

이 단계부터 아름코드는 단일 응답 처리기가 아니라 다음 흐름을 가진다.

```text
user prompt -> LLM candidate -> DecisionGate -> ToolRuntime -> observation -> LLM next turn
```

## 범위

포함:

- `tool_candidate_pending` 이후 다음 LLM request spawn
- observation을 message history에 internal system message로 기록
- tool loop count 추적
- max tool calls circuit breaker
- same tool signature repeat circuit breaker
- tool loop log event

제외:

- mutation tool 실행
- approval branch
- web/network branch
- full LLM E2E
- context compaction

## Observation Message

LLM에게 전달하는 observation은 자유 대화가 아니라 구획화된 텍스트로 남긴다.

```text
<AHREUM_TOOL_OBSERVATION>
tool_name: read_file
status: succeeded
target_raw: src/main.rs
total_lines: 40
total_bytes: 1200
truncated: true
source_truncated: true
preview_truncated: false
next_range_hint: read_file path=src/main.rs start_line=41 max_lines=40
preview:
...
</AHREUM_TOOL_OBSERVATION>
```

모델은 이 observation을 근거로 다음 응답을 만든다. 충분하면 `answer`, 더 필요하면 다시 `tool`을 하나만 반환한다.

tool loop의 다음 LLM request에는 직전 assistant tool candidate 원문을 다시 포함하지 않는다. 다음 요청은 schema/context/user prompt/internal observation 중심으로 새로 구성한다. 로컬 LLM이 이미 실행된 도구 후보 JSON을 다시 보고 빈 응답이나 반복 tool candidate를 만들 위험을 줄이기 위한 정책이다.

repair를 거쳐 tool candidate가 만들어진 경우에도 이전 repair system prompt를 다음 tool loop 요청에 포함하지 않는다. repair prompt는 이전 실패 응답을 보정하기 위한 지시일 뿐이며, tool observation 이후 최종 답변 요청의 근거가 아니다.

tool loop 요청은 다음 메시지로 구성한다.

- schema prompt
- project runtime context
- original user prompt
- latest `AHREUM_TOOL_OBSERVATION`
- observation 기반으로 답변하거나, 증거가 부족할 때만 다음 tool을 요청하라는 짧은 system instruction

## Circuit Breaker

초기 정책:

- `max_tool_calls`는 config `[limits]` 값을 따른다.
- 같은 `tool_name + arguments` signature가 `max_same_tool_repeats`를 초과하면 루프를 중단한다.
- 중단은 failure가 아니라 runtime blocked report로 workspace에 표시한다.

## 완료 기준

- Explore observation 이후 다음 LLM request가 생성된다.
- observation message가 history에 기록된다.
- 다음 LLM request에서 직전 assistant tool candidate 원문은 제외된다.
- 다음 LLM request에서 이전 repair system prompt는 제외된다.
- tool loop count와 same tool repeat guard가 동작한다.
- loop 중단 시 workspace와 log에 이유가 남는다.
- `cargo fmt --check`가 통과한다.
- `cargo test`가 통과한다.
- `cargo run -- --scene main --smoke`가 통과한다.

## Change History

### 2026-05-17

- Created `tool-03` technical spec before implementation.
- Implemented observation-to-history binding and next LLM request spawning after Explore tool execution.
- Updated tool loop request construction to exclude the previous assistant tool candidate from the next provider request.
- Updated tool loop request construction to rebuild the request from schema/context/user/observation and exclude stale repair prompts.
- Added tool loop circuit breakers for `max_tool_calls` and `max_same_tool_repeats`.
- Added structured observation message formatting and tool loop log events.
- Verified with `cargo fmt --check`, `cargo test`, and `cargo run -- --scene main --smoke`.
