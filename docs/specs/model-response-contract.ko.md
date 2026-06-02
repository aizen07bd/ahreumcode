---
id: model-response-contract-ko
type: spec
status: draft
topics:
  - local-llm
  - tool-calling
  - response-contract
  - controller-loop
  - json-schema
summary: Korean specification draft for the local LLM JSON response contract and controller-driven tool loop.
last_updated: 2026-05-22
related:
  - docs/product/tool-call-benchmark.ko.md
  - docs/product/agent-tool-flow-comparison.ko.md
  - docs/specs/intent-frame-uncertainty-gate.ko.md
  - docs/product/tui-ui-command-benchmark.ko.md
  - docs/architecture/agent-operating-guardrails.md
  - docs/PROJECT-CONTEXT.md
---

# Model Response Contract Korean Draft

## 목적

아름코드는 로컬 LLM에게 도구 실행 권한을 직접 맡기지 않는다.

로컬 LLM은 매 turn마다 다음 행동 후보 하나만 JSON으로 반환한다. 아름코드 controller가 그 후보를 검증하고, 필요하면 repair/approval을 거쳐 실행한 뒤 observation을 다시 로컬 LLM에 전달해 다음 행동을 묻는다.

이 문서는 그 JSON 응답 계약과 controller-driven loop를 정의한다.

## Core Rule

```text
One local LLM response = exactly one next action candidate.
```

로컬 LLM 한 응답에서 허용되는 것은 다음 중 하나다.

```text
answer
tool
plan
clarify
blocked
```

금지:

- 한 응답에 여러 tool call을 넣기
- plan 안에 실행할 tool 목록, arguments, payload id, patch/body/command 원문을 넣기
- 자연어와 JSON을 섞기
- JSON 외 markdown/code fence 출력
- 소스코드, patch, 긴 file content를 JSON string 안에 직접 넣기
- 코드, markdown, 긴 설명이 포함된 answer 본문을 JSON string 안에 직접 넣기
- 모델 응답을 confirmed execution으로 바로 취급하기

## Controller-Driven Loop

루프는 로컬 LLM이 직접 실행하지 않는다. 아름코드 controller가 실행한다.

흐름:

```text
1. user prompt 수신
2. controller가 local LLM 호출
3. local LLM이 answer/tool/clarify/blocked 후보 하나 반환
4. controller가 JSON parse, schema validation, tool validation 수행
5. tool 후보이면 필요 시 repair/approval 수행
6. controller가 tool 실행
7. controller가 observation/evidence를 session에 저장
8. controller가 observation을 포함해 local LLM을 다시 호출
9. answer/clarify/blocked 또는 loop guard까지 반복
```

핵심 문장:

```text
The local LLM does not run the loop.
AhreumCode controller runs the loop by repeatedly calling the local LLM after each observation.
```

## Why One Tool Per Turn

사용자의 의도는 대개 복합적이다.

예:

```text
"00 파일 내용 알려줘"
```

이 요청도 실제로는 다음 단계를 요구할 수 있다.

```text
list files
find files
read matching file
search text
interpret content
answer
```

하지만 로컬 LLM에게 한 번에 여러 도구를 요청하게 하면 tool name, arguments, path, JSON 구조 오류가 늘어난다.

따라서 복합 작업은 multi-turn loop로 처리한다.

예:

```text
Turn 1:
  model -> tool candidate: list_files
  controller -> execute
  observation -> files listed

Turn 2:
  model -> tool candidate: find_files
  controller -> execute
  observation -> matching files

Turn 3:
  model -> tool candidate: read_file
  controller -> execute
  observation -> file content

Turn 4:
  model -> answer
```

로컬 LLM은 매번 "다음 행동 하나"만 제안한다.

## One Tool Candidate, Multi-Target Change Payload

`One local LLM response = exactly one next action candidate` 규칙은 유지한다.

다만 `apply_patch`의 경우 하나의 Change tool candidate가 하나의 patch document를 담고, 그 patch document 안에 여러 file target을 포함할 수 있다.

구분:

```text
One response       = one action candidate
One Change action  = one apply_patch payload
One patch payload  = one or more file targets
```

허용 예:

```text
사용자: html/css/js로 분리된 웹게임 프로젝트를 만들어줘.

model -> one tool candidate:
  tool_name: apply_patch
  payload_id: patch_001

payload patch_001:
  *** Begin Patch
  *** Add File: game/index.html
  ...
  *** Add File: game/styles.css
  ...
  *** Add File: game/game.js
  ...
  *** End Patch
```

금지 예:

```text
model -> tool candidate: apply_patch index.html
model -> tool candidate: apply_patch styles.css
model -> tool candidate: apply_patch game.js
```

즉 복수 파일 변경은 여러 tool call이 아니라 하나의 atomic change proposal로 취급한다.

복수 파일 `apply_patch` payload 계약:

- payload body는 하나의 완전한 patch document여야 한다.
- patch document는 `*** Begin Patch`로 시작하고 `*** End Patch`로 끝나야 한다.
- patch document 안의 각 target header는 `*** Add File:`, `*** Update File:`, `*** Delete File:` 중 하나여야 한다.
- 각 target path는 workspace-relative path여야 한다.
- target path는 중복될 수 없다.
- 전체 target 수는 runtime limit 안에 있어야 한다.
- 전체 additions/deletions와 target별 additions/deletions를 preview metadata로 보존해야 한다.
- 승인 화면은 전체 변경 요약과 target별 요약을 함께 보여줘야 한다.
- 승인 후 실행은 payload 전체가 성공하거나 전체가 실패해야 한다. 부분 성공을 성공 observation으로 취급하지 않는다.
- 실패 observation은 어떤 target에서 실패했는지 보존해야 한다.

수정 요청 계약:

- 기존 파일을 `Update File`로 수정하려면 해당 target에 대한 성공한 `read_file` 또는 동등한 workspace evidence가 먼저 있어야 한다.
- 새 파일 생성은 `Add File`을 사용한다.
- 기존 파일을 덮어쓰는 생성 요청은 `Update File` 또는 명시적 overwrite approval 정책을 따라야 하며, 조용히 `Add File`로 바꾸면 안 된다.
- 여러 파일이 섞인 프로젝트 변경에서는 각 target의 operation을 파일 상태와 사용자 요청에 맞게 독립적으로 검증한다.

현재 구현 상태:

```text
2026-05-22 실제 TUI 검증에서 분리형 웹게임 생성 요청은 실패했다.
모델은 하나의 apply_patch payload 안에 index.html/styles.css/game.js 3개 target을 넣었고,
현재 runtime은 "apply_patch target count must be exactly one"으로 차단했다.
따라서 multi-target apply_patch는 계약상 필요한 다음 capability이며, 구현은 아직 완료되지 않았다.
```

## Response Envelope

상위 JSON envelope는 다음 형태로 고정한다.

```json
{
  "response_type": "tool",
  "activity": "Explore",
  "message": "I need to inspect the project context first.",
  "tool_name": "read_file",
  "arguments": {
    "path": "docs/PROJECT-CONTEXT.md",
    "start_line": 1,
    "max_lines": 120
  },
  "reason": "The project context routes current decisions."
}
```

필드:

| Field | Required | Description |
| --- | --- | --- |
| `response_type` | yes | `answer`, `tool`, `plan`, `clarify`, `blocked` |
| `activity` | yes | `None`, `Explore`, `Change`, `Execute`, `Configure`, `Ask` |
| `message` | yes | 사용자에게 보여줄 짧은 설명. 코드/markdown/긴 본문을 직접 넣지 않는다. |
| `answer_payload_id` | only for answer with payload | 코드, markdown, 긴 설명 본문을 담은 raw payload id |
| `tool_name` | only for tool | concrete tool name |
| `arguments` | only for tool | tool-specific typed arguments |
| `plan_items` | only for plan | 실행 목록이 아니라 완료 장부용 작업 단위. 각 item은 `operation`과 선택적 `target`만 가진다. |
| `reason` | required for `tool`, `plan`, `clarify`, `blocked` | 왜 이 행동이 필요한지 또는 왜 막혔는지 |

제거한 필드:

| Field | Decision | Reason |
| --- | --- | --- |
| `confidence` | removed | 로컬 LLM confidence는 신뢰 가능한 실행 근거가 아니며 잘못된 안정감을 만든다. |
| `expected_next` | removed | 다음 행동 판단은 controller loop가 담당한다. 모델이 힌트를 남기면 숨은 plan처럼 동작할 위험이 있다. |

Unknown field 정책:

```text
Unknown fields are rejected.
```

로컬 LLM이 임의 필드를 추가하면 controller는 해당 응답을 invalid schema로 처리하고 repair flow로 보낸다.

## Raw Payload Separation Rule

소스코드, patch, 긴 file content, command body, code/markdown answer body처럼 quote와 escape가 많은 원문은 JSON string 안에 직접 넣지 않는다.

이 규칙의 목적은 로컬 LLM이 `"`, `'`, `\`, newline, template literal, raw string, nested JSON 같은 문자를 잘못 escape해서 tool call 전체가 malformed JSON이 되는 실패를 줄이는 것이다.

핵심 문장:

```text
JSON carries control data.
Raw payload blocks carry source/code text.
```

정책:

- JSON envelope는 `response_type`, `activity`, `tool_name`, typed arguments, payload id 같은 제어 정보만 담는다.
- `answer`라도 코드, markdown, 긴 설명 본문을 포함하면 JSON `message`에 직접 넣지 않고 `answer_payload_id`와 raw payload block을 사용한다.
- `create_file`, `edit_file`, `apply_patch`처럼 긴 원문이 필요한 tool은 JSON arguments에 원문을 넣지 않고 `payload_id`를 넣는다.
- 원문은 response envelope 밖의 raw payload block에 둔다.
- controller는 JSON parser와 payload parser를 분리한다.
- `payload_id`가 arguments에 있으면 같은 id의 payload block이 정확히 하나 있어야 한다.
- payload block이 없거나, 중복되거나, format validation에 실패하면 tool 후보는 실행하지 않는다.
- repair loop는 JSON envelope와 payload block 중 실패한 부분을 구분해서 재요청한다.

금지:

- `arguments.content`에 전체 파일 내용 넣기
- `arguments.patch`에 patch 원문 넣기
- JSON string escape 실패를 문자열 치환으로 보정하기
- payload parser 실패를 성공 tool candidate로 취급하기

초기 block 형식 후보:

```text
<AHREUM_ACTION>
{
  "response_type": "tool",
  "activity": "Change",
  "message": "I can prepare the requested patch.",
  "tool_name": "apply_patch",
  "arguments": {
    "payload_id": "patch_001"
  },
  "reason": "The change requires a patch payload."
}
</AHREUM_ACTION>

<AHREUM_PAYLOAD id="patch_001" format="apply_patch">
*** Begin Patch
*** Update File: src/main.rs
@@
-println!("old");
+println!("new");
*** End Patch
</AHREUM_PAYLOAD>
```

주의:

- 위 block tag는 response framing을 위한 후보이며, tool 실행 권한을 의미하지 않는다.
- XML parser 전체를 도입한다는 뜻이 아니다. action JSON과 raw payload 영역을 안정적으로 분리하기 위한 framing이다.
- payload 내부는 XML attribute/body escaping 규칙이 아니라 `format`별 parser가 검증한다.
- payload block 내부에서 종료 tag 문자열이 필요한 경우에는 해당 `format`별 escape 또는 artifact 방식으로 별도 처리한다.

## Response Type Shapes

### answer

```json
{
  "response_type": "answer",
  "activity": "None",
  "message": "..."
}
```

코드, markdown, 긴 설명 본문이 필요한 `answer`는 다음 형태를 사용한다.

````text
<AHREUM_ACTION>
{
  "response_type": "answer",
  "activity": "None",
  "message": "Prepared a TypeScript example.",
  "answer_payload_id": "answer_001"
}
</AHREUM_ACTION>

<AHREUM_PAYLOAD id="answer_001" format="markdown">
TypeScript example:

```typescript
const greeting: string = "Hello, World!";
console.log(greeting);
```
</AHREUM_PAYLOAD>
````

허용 필드:

```text
response_type
activity
message
answer_payload_id
```

`answer`에는 `tool_name`, `arguments`, `reason`을 넣지 않는다.

`answer_payload_id`는 선택 필드다. 짧은 일반 답변은 `message`만 사용해도 된다. 코드 block, markdown fence, 긴 설명, 따옴표가 많은 본문이 있으면 반드시 `answer_payload_id`와 `AHREUM_PAYLOAD format="markdown"`을 사용한다.

### tool

```json
{
  "response_type": "tool",
  "activity": "Explore",
  "message": "I need to read the project context first.",
  "tool_name": "read_file",
  "arguments": {
    "path": "docs/PROJECT-CONTEXT.md",
    "start_line": 1,
    "max_lines": 120
  },
  "reason": "Current project context is needed before answering."
}
```

허용 필드:

```text
response_type
activity
message
tool_name
arguments
reason
```

`tool`의 `activity`는 `Explore`, `Change`, `Execute`, `Configure` 중 하나여야 한다.

### plan

```json
{
  "response_type": "plan",
  "activity": "None",
  "message": "I will track the requested targets before changing files.",
  "plan_items": [
    { "operation": "create", "target": "web/index.html" },
    { "operation": "create", "target": "web/app.js" },
    { "operation": "verify" }
  ],
  "reason": "The request has multiple concrete targets."
}
```

허용 필드:

```text
response_type
activity
message
plan_items
reason
```

`plan`의 `activity`는 `None`이다.

`plan_items`는 실행할 tool 목록이 아니다. 각 item은 `operation`과 선택적 `target`만 가진다. 허용 operation은 `read`, `create`, `update`, `delete`, `execute`, `verify`, `answer`이다.

runtime은 `plan_items`를 완료 장부로만 사용하고, 다음 turn에서 실제 `tool` 후보를 다시 요구한다. 따라서 `plan`에는 `tool_name`, `arguments`, `payload_id`, patch text, file body, command argv를 넣을 수 없다.

`read`, `create`, `update`, `delete` item은 target이 알려져 있을 때만 사용한다. target이 사용자 요청이나 observation에 없으면 conventional filename을 추측하지 않고 `clarify` 또는 bounded `Explore`를 사용한다.

### clarify

```json
{
  "response_type": "clarify",
  "activity": "Ask",
  "message": "Which file should I update?",
  "reason": "The target file is unclear."
}
```

허용 필드:

```text
response_type
activity
message
reason
```

`clarify`의 `activity`는 `Ask`여야 한다.

#### Clarify Boundary Rule

`clarify`는 사용자만 확정할 수 있는 정보가 부족할 때 사용한다.

사용하면 안 되는 경우:

- runtime system context가 이미 제공한 프로젝트 정체성으로 답할 수 있는 경우
- 현재 workspace/config/known project goal 같은 기본 맥락으로 답할 수 있는 경우
- 일반 설명, 요약, 개념 답변처럼 도구 없이 답변 가능한 경우

이 경우 모델은 다음 형태를 사용해야 한다.

```json
{
  "response_type": "answer",
  "activity": "None",
  "message": "..."
}
```

실제 실패 반영:

```text
2026-05-17 e2e-01에서 "아름코드가 지금 어떤 프로젝트인지 한 문단으로 설명해줘." 요청이
clarify로 반환되어 실패했다. 이 요청은 runtime project context가 있으면 answer로 처리되어야 한다.
```

### blocked

```json
{
  "response_type": "blocked",
  "activity": "None",
  "message": "I cannot run that directly.",
  "reason": "The command may affect the whole filesystem."
}
```

허용 필드:

```text
response_type
activity
message
reason
```

`blocked`의 `activity`는 `None` 또는 `Ask`다. controller의 `Blocked`/`ManualOnly` gate outcome은 이 모델 응답과 별도로 최종 판단할 수 있다.

## Activity Enum

```text
None
Explore
Change
Execute
Configure
Ask
```

`None`이 필요하다.

단순 채팅, 도구 없는 답변, 일반 설명, 일부 blocked 응답은 tool activity가 없기 때문이다.

권장 매핑:

| response_type | activity |
| --- | --- |
| `answer` | `None` by default |
| `tool` | `Explore`, `Change`, `Execute`, or `Configure` |
| `plan` | `None` |
| `clarify` | `Ask` |
| `blocked` | `None` or `Ask` depending on whether user input is needed |

`Ask`는 모든 자연어 답변을 뜻하지 않는다. clarification, approval, selection처럼 사용자 판단이 필요한 경우에만 사용한다.

초기 구현에서 `Ask`는 model-facing tool이 아니라 `clarify` 또는 `blocked` response type으로 표현한다.

## Response Examples

### Simple Answer

```json
{
  "response_type": "answer",
  "activity": "None",
  "message": "The UI policy is ready enough to move into tool-call schema design."
}
```

### Answer With Markdown Payload

코드 예시나 markdown 설명은 JSON 문자열 escape에 기대지 않는다.

````text
<AHREUM_ACTION>
{
  "response_type": "answer",
  "activity": "None",
  "message": "Prepared a TypeScript hello-world example.",
  "answer_payload_id": "answer_001"
}
</AHREUM_ACTION>

<AHREUM_PAYLOAD id="answer_001" format="markdown">
```typescript
const greeting: string = "Hello, World!";
console.log(greeting);
```
</AHREUM_PAYLOAD>
````

### Clarification

```json
{
  "response_type": "clarify",
  "activity": "Ask",
  "message": "Should web search be included in the first implementation?",
  "reason": "The implementation scope is not yet decided."
}
```

### Tool Candidate

```json
{
  "response_type": "tool",
  "activity": "Explore",
  "message": "I need to read the project context first.",
  "tool_name": "read_file",
  "arguments": {
    "path": "docs/PROJECT-CONTEXT.md",
    "start_line": 1,
    "max_lines": 120
  },
  "reason": "Current project context is needed before answering."
}
```

### Tool Candidate With Raw Payload

긴 code/patch 원문이 필요한 tool은 JSON에 payload id만 둔다.

```text
<AHREUM_ACTION>
{
  "response_type": "tool",
  "activity": "Change",
  "message": "I prepared one patch candidate.",
  "tool_name": "apply_patch",
  "arguments": {
    "payload_id": "patch_001"
  },
  "reason": "The requested code change must be represented as a patch payload."
}
</AHREUM_ACTION>

<AHREUM_PAYLOAD id="patch_001" format="apply_patch">
*** Begin Patch
*** Update File: src/example.rs
@@
-let label = "old";
+let label = "new";
*** End Patch
</AHREUM_PAYLOAD>
```

### Blocked

```json
{
  "response_type": "blocked",
  "activity": "None",
  "message": "I cannot choose a file safely because the target path is ambiguous.",
  "reason": "The request requires a file change, but no target file or safe search boundary is available."
}
```

## Prompt Sent To Local LLM

Controller가 로컬 LLM에 보낼 prompt는 매 turn 다음 요소를 포함한다.

```text
System contract:
  Return exactly one JSON object.
  Return one next action only.
  Use the allowed response_type/activity/tool_name enums.

User goal:
  Original user request.

Session context:
  Relevant conversation summary and project constraints.

Observations:
  Tool results/evidence collected so far.

Instruction:
  If enough evidence exists, answer.
  If not, request exactly one tool.
  If user input is needed, clarify.
  If impossible or unsafe, blocked.
```

예:

```text
Previous observations:
1. list_files returned docs/PROJECT-CONTEXT.md and AGENTS.md.
2. read_file returned project routing information.

User goal:
"현재 프로젝트 UI 결정 상태 알려줘"

Return exactly one next action as JSON.
```

## Candidate Validation

모델 응답은 candidate일 뿐이다.

Controller는 다음을 검증한다.

- JSON parse 가능 여부
- unknown field 부재 여부
- required field 존재 여부
- enum 값 유효성
- `response_type`과 필드 조합 유효성
- `activity`와 `tool_name` 조합 유효성
- `answer_payload_id`와 payload block의 존재/중복/format 유효성
- path safety
- command safety
- network policy
- permission requirement
- loop guard 상태

검증 실패 시:

```text
malformed JSON -> repair prompt or blocked
invalid schema -> repair prompt
unknown tool -> repair prompt
unsafe target -> blocked or approval path
ambiguous target -> clarify
```

## Tool Call Defense Rules

로컬 LLM 도구 호출 방어 정책은 다음 원칙을 따른다.

```text
모델 출력은 관대하게 읽는다.
워크스페이스 변경은 엄격하게 막는다.
```

즉, parser는 모델이 자주 만드는 형식 오류를 진단하고 repair에 필요한 근거를 남길 수 있다. 하지만 파일 생성/수정/삭제, command 실행, config 변경 같은 workspace mutation은 추측 보정으로 강행하지 않는다.

아름코드 방어코드는 현재 총 24개로 둔다.

구성:

```text
1~15: 초기 상위 방어 원칙
16~24: Codex/opencode/Cline 벤치마킹 후 추가된 방어코드
```

| No | Defense | Policy |
| --- | --- | --- |
| 1 | Tool Manifest Echo Check | schema prompt에 tool manifest id/version을 넣고, 모델 응답에도 같은 id를 요구한다. runtime이 보낸 tool 계약과 모델이 사용한 계약이 다르면 invalid response로 처리한다. |
| 2 | Two-Phase Mutation | 변경 작업은 `prepare_change -> preview_diff -> approval/policy decision -> apply_change` 순서로 나눈다. 모델 응답 하나가 곧바로 파일 변경으로 이어지면 안 된다. |
| 3 | Precondition Snapshot | 변경 전 대상 파일의 path, size, mtime, content hash 같은 precondition을 기록한다. 적용 시점에 파일 상태가 달라졌으면 재검토한다. |
| 4 | Patch Impact Guard | patch가 건드리는 파일 수, line 수, delete 비율, binary 여부, docs/test/source 범위를 계산한다. 영향이 크면 자동 진행하지 않고 사용자 승인 또는 blocked로 보낸다. |
| 5 | Unique Target Requirement | mutation 대상은 정확히 하나로 확정되어야 한다. 유사한 후보가 여러 개면 모델이나 runtime이 임의 선택하지 않고 사용자에게 묻는다. |
| 6 | Observation Schema | tool 실행 결과를 자유문장으로만 넣지 않고 typed observation으로 저장한다. success/failure, target, evidence, truncated 여부를 구분한다. |
| 7 | Truncation Contract | 긴 파일/검색 결과를 줄일 때는 잘렸다는 사실, 원래 길이, 남은 범위를 observation에 반드시 표시한다. 모델이 전체를 읽은 것처럼 판단하게 두지 않는다. |
| 8 | Repeat Failure Circuit Breaker | 같은 tool, 같은 target, 같은 failure가 반복되면 repair loop를 멈추고 사용자에게 보고한다. 무한 재시도와 비용 증가를 막는다. |
| 9 | Command Capability Split | shell 실행은 capability별로 분리한다. read-only command, build/test command, process start, destructive/system command를 같은 권한으로 취급하지 않는다. |
| 10 | Shell-Free Command Schema | 가능한 command는 raw shell string이 아니라 argv 배열과 command kind로 표현한다. shell expansion, pipe, redirect, substitution은 별도 승인 또는 blocked 대상이다. |
| 11 | Dry-Run First | 지원되는 도구는 실제 변경 전에 dry-run 또는 preview를 먼저 수행한다. 특히 format/build/test가 아닌 mutation 계열 command는 preview 없는 실행을 피한다. |
| 12 | Postcondition Verification | 변경 후에는 기대한 파일/라인/상태가 실제로 반영됐는지 확인한다. 적용 성공 메시지만으로 완료 처리하지 않는다. |
| 13 | No Silent Normalization | path, command, payload를 runtime이 조용히 바꿔서 실행하지 않는다. normalize가 필요하면 observation 또는 approval 화면에 원본과 결과를 남긴다. |
| 14 | Tool Error Taxonomy | 실패를 하나의 error string으로 뭉치지 않는다. parse_error, schema_error, path_error, permission_error, execution_error, timeout, model_error를 구분한다. |
| 15 | Human Boundary Rule | 시스템 전체, 파일시스템 외부, 과도한 CPU/메모리, 대량 삭제/이동, 보안/권한 변경처럼 영향이 큰 작업은 모델이 요청해도 실행하지 않는다. 명령 가이드만 제공하고 사용자가 직접 수행하게 한다. |
| 16 | Approval Persistence Broad Prefix Deny List | 영구 허용 저장 시 `bash`, `sh`, `zsh`, `python`, `node -e`, `git`처럼 너무 넓은 prefix는 금지한다. |
| 17 | Command Original Vs Parsed Display | 승인 화면에 모델이 제안한 원문 command와 runtime이 파싱한 실행 segment를 함께 보여준다. |
| 18 | Hidden/Unicode Character Marker | command, path, branch에 보이지 않는 문자나 혼동 가능한 문자가 있으면 위험 표시를 남긴다. |
| 19 | External Path Permission Branch | workspace 밖 read/write는 workspace 내부 read/write와 다른 permission branch로 처리한다. |
| 20 | Network/Web Permission Branch | webfetch, websearch, network access는 local read와 분리된 permission branch로 처리한다. |
| 21 | Tool Argument Schema-First Gate | tool handler 실행 전에 typed argument schema를 반드시 통과해야 한다. schema failure는 실행 실패가 아니라 model response failure다. |
| 22 | Partial Tool Block State | streaming 중 덜 닫힌 tool block은 UI 진행 상태로만 표시하고 실행 가능 상태로 승격하지 않는다. |
| 23 | Full Output Artifact | 긴 출력은 history에 preview만 넣고 전체 출력은 session artifact로 저장한다. observation에는 artifact path, total size, truncation metadata를 남긴다. |
| 24 | Post-Edit Diagnostics Hook | 변경 후 formatter/LSP/build diagnostic 결과를 observation으로 남길 수 있다. 진단 결과가 있다고 자동 추가 수정하지 않는다. |

### Observed Runtime Failure Case

2026-05-15 LM Studio `google/gemma-4-e4b` 수동 검증에서 다음 실패가 확인됐다.

```text
answer 응답의 message 안에 TypeScript code block과 markdown 설명이 직접 들어감
-> JSON string escape가 깨짐
-> parser가 json_parse_failed 처리
-> repair request가 발생
-> repair 응답 content가 비어 최종 invalid_response로 표시
```

이 사례는 목업 테스트나 특정 프롬프트용 예외가 아니다. 로컬 LLM이 코드/markdown 답변을 만들 때 반복적으로 발생할 수 있는 응답 계약 실패 유형이다.

구현 범위에 포함할 방어:

- `answer` payload separation: 코드/markdown/긴 본문은 `message`가 아니라 `answer_payload_id`와 `AHREUM_PAYLOAD format="markdown"`에 둔다.
- `compact repair context`: malformed assistant 원문 전체를 다음 request에 그대로 재투입하지 않는다. 실패 종류, 위치, 길이, payload 필요 여부 같은 compact diagnostic만 repair instruction에 포함한다.
- `empty repair response taxonomy`: repair 응답이 비어 있으면 단순 provider 실패로 뭉개지 않고 `model_empty_response` 또는 동등한 runtime failure로 기록한다.
- `real transcript verification`: 완료 검증은 샘플 문자열이나 mock happy path만으로 처리하지 않는다. 실제 LM Studio 응답 또는 저장된 provider transcript를 기준으로 parser/repair/log/TUI 결과가 일관되는지 확인한다.

매핑:

| Runtime Failure | Defense Mapping |
| --- | --- |
| answer 본문 JSON escape 실패 | `13. No Silent Normalization`, `14. Tool Error Taxonomy`, `21. Tool Argument Schema-First Gate` |
| malformed 응답 원문 재투입으로 repair 불안정 | `6. Observation Schema`, `8. Repeat Failure Circuit Breaker`, `14. Tool Error Taxonomy` |
| 빈 repair 응답이 provider error로만 표시 | `14. Tool Error Taxonomy`, `6. Observation Schema` |
| mock만 통과하고 실제 로컬 LLM에서 실패 | `1. Tool Manifest Echo Check`, `6. Observation Schema`, `14. Tool Error Taxonomy` |

### Parser-Side Tolerance

다음 항목은 모델 응답을 "읽기 쉽게" 만들기 위한 parser-side tolerance다. 성공 실행을 보장하는 기능이 아니다.

- Markdown fence unwrapping은 응답 전체가 하나의 code fence로 감싸진 경우에만 허용한다.
- fence unwrap은 JSON action envelope 또는 raw payload block을 추출하기 위한 전처리일 뿐이다.
- 파일 내부에 있는 의도적인 markdown fence를 제거하지 않는다.
- XML처럼 보이는 framing tag를 쓰더라도 XML parser에 실행 의미를 위임하지 않는다.
- raw payload block의 내용은 `format`별 parser가 별도로 검증한다.

금지:

- 깨진 JSON을 임의 문자열 치환으로 성공 처리하기
- 누락된 field를 runtime이 추측해서 채우기
- payload 종료 tag 충돌을 조용히 보정하기

### Mutation-Side Strictness

변경 작업에서는 관대한 보정이 아니라 명시적 결정이 필요하다.

- fuzzy match는 자동 수정에 사용하지 않는다.
- fuzzy match 결과는 diagnostic, repair request, 사용자 선택 후보로만 사용한다.
- flexible path resolution은 `./path`, workspace-relative path, workspace 내부 absolute path의 안전한 정규화까지만 허용한다.
- 비슷한 파일명, 첫 번째 검색 결과, 가장 그럴듯한 후보를 runtime이 임의 선택하지 않는다.
- delete, rename, write, patch, config update는 대상과 범위가 확정되지 않으면 clarify/approval/blocked로 보낸다.

### Error Feedback Loop

도구 실행 실패는 사용자에게 숨기지 않고, 모델에게도 구조화된 observation으로 되돌린다.

정책:

- 실패 observation에는 `tool_name`, `target`, `error_kind`, `message`, `recoverable`, `attempt_count`를 포함한다.
- recoverable failure는 repair loop로 보낼 수 있다.
- non-recoverable 또는 반복 failure는 blocked/report로 종료한다.
- repair loop가 새로운 근거 없이 같은 요청을 반복하면 circuit breaker가 중단한다.

## Benchmark Reinforcement

2026-05-14 기준 공개 소스와 문서를 확인한 벤치마킹 대상:

| Product | Source |
| --- | --- |
| Codex | `openai/codex`, `codex-rs/core/src/exec_policy.rs`, `codex-rs/core/src/tools/handlers/mod.rs` |
| opencode | `sst/opencode`, `packages/opencode/src/tool/*`, `packages/opencode/src/permission/*` |
| Cline | `cline/cline`, `src/core/assistant-message/*`, `src/core/permissions/*`, `src/core/task/index.ts` |

주의:

```text
벤치마킹 보강 항목은 고정 개수 목록이 아니다.
제품별 구현에서 확인한 방어축을 먼저 적고, 그 뒤 기존 15개와 매핑한다.
기존 15개에 이미 포함되는 방어축은 새 항목으로 세지 않는다.
기존 15개에 문장 그대로 없고, 벤치마킹에서 구현 필요성이 확인된 항목만 추가 항목으로 세다.
```

### Product Defense Axes

| Product | Confirmed Defense Axes |
| --- | --- |
| Codex | sandbox policy, approval policy, command parse/evaluate, dangerous command detection, writable root/protected path, network limitation, approval persistence guard, patch/apply separation, hidden-character risk surface |
| opencode | allow/ask/deny permission, tool-name permission, modification group, external directory guard, repeated-call guard, web permission split, approval rejection feedback, output truncation, tool input comparison |
| Cline | tool approval, auto approve category, command deny-first, command segment parsing, partial assistant message, edit/diff workflow, edit failure feedback, retry risk, SDK tool policy, fallback matching |

### Covered By Existing 15 Rules

다음 벤치마킹 결과는 기존 15개 방어 정책과 겹친다. 새 번호를 만들지 않고 기존 항목의 근거로 둔다.

| Existing Rule | Benchmark Axes |
| --- | --- |
| `Tool Manifest Echo Check` | Cline SDK tool policy, opencode tool schema/permission split |
| `Two-Phase Mutation` | Codex patch/apply separation, Cline edit/diff workflow, opencode modification group |
| `Patch Impact Guard` | Cline edit/diff workflow, opencode edit/write/patch group |
| `Unique Target Requirement` | Codex writable root/protected path, opencode external directory guard |
| `Observation Schema` | opencode approval rejection feedback, Cline edit failure feedback |
| `Truncation Contract` | opencode output truncation |
| `Repeat Failure Circuit Breaker` | opencode repeated-call guard, Cline retry risk |
| `Command Capability Split` | Codex command policy, opencode tool permission split, Cline auto approve category |
| `Shell-Free Command Schema` | Codex command parse/evaluate, Cline segment/subshell parsing |
| `No Silent Normalization` | Codex path/command risk, opencode external directory guard |
| `Tool Error Taxonomy` | Cline edit failure feedback, Codex command evaluate result |
| `Human Boundary Rule` | Codex dangerous command/sandbox, Cline deny-first command permission |

### Existing 15 Rules To Strengthen

다음은 벤치마킹에서 확인됐지만 기존 15개에 이미 포함되는 항목이다. 새 번호를 만들지 않고 기존 정책의 구현 조건을 강화한다.

| Benchmark Detail | Existing Rule To Strengthen |
| --- | --- |
| repeated tool+input comparison | `8. Repeat Failure Circuit Breaker` |
| rejection reason as structured observation | `6. Observation Schema` |
| edit failure feedback taxonomy | `14. Tool Error Taxonomy` |
| approval before mutation/shell | `2. Two-Phase Mutation`, `15. Human Boundary Rule` |
| diff/preview before write | `2. Two-Phase Mutation`, `4. Patch Impact Guard` |

### Added Defense Codes From Benchmarking

기존 15개와 비교한 뒤 벤치마킹 기반으로 추가 확정한 방어코드는 9개다. 이 9개는 상단 `Tool Call Defense Rules` 표의 16~24번에 반영한다.

```text
추가 n = 9
총 방어코드 = 24
```

| Detail | Derived From | Policy |
| --- | --- | --- |
| 16. Approval Persistence Broad Prefix Deny List | Codex | `bash`, `sh`, `zsh`, `python`, `node -e`, `git`처럼 너무 넓은 prefix는 영구 허용 후보에서 제외한다. |
| 17. Command Original Vs Parsed Display | Codex, Cline | 승인 화면에 원문 command와 runtime이 파싱한 실행 segment를 같이 보여준다. |
| 18. Hidden/Unicode Character Marker | Codex risk surface | command/path/branch에 보이지 않는 문자나 혼동 가능한 문자가 있으면 위험 표시한다. |
| 19. External Path Permission Branch | opencode | workspace 밖 read/write는 workspace 내부 read/write와 다른 permission branch로 다룬다. |
| 20. Network/Web Permission Branch | Codex, opencode | webfetch/websearch/network는 local read와 분리된 permission branch로 다룬다. |
| 21. Tool Argument Schema-First Gate | opencode, Cline | tool handler 실행 전에 typed argument schema를 통과해야 한다. |
| 22. Partial Tool Block State | Cline | streaming partial tool block은 UI 진행 상태로만 표시하고 실행 가능 상태로 승격하지 않는다. |
| 23. Full Output Artifact | opencode truncation pattern | 긴 출력은 preview와 session artifact로 분리하고 truncation metadata를 남긴다. |
| 24. Post-Edit Diagnostics Hook | Cline edit workflow | 변경 후 진단은 observation으로 남기되 자동 추가 수정은 하지 않는다. |

주의:

- `n = 9`는 현재 Codex/opencode/Cline 벤치마킹과 기존 15개 대조 결과다.
- 현재 아름코드 방어코드는 `15 + 9 = 24`개다.
- 앞으로 다른 오픈소스나 추가 소스코드를 조사하면 n은 바뀔 수 있다.
- `file mutation lock`은 필요한 내부 설계 후보지만, 이번 오픈소스 공통 방어코드 대조 결과에는 넣지 않는다. `Precondition Snapshot` 구현 단계에서 별도 검토한다.

### Intentional Difference From Benchmarks

Cline/opencode 계열의 edit fallback은 모델의 원본 문자열 오류를 흡수해 성공률을 높이는 장점이 있다. 하지만 아름코드는 로컬 LLM의 잘못된 target/search/path를 자동 mutation으로 연결하지 않는다.

정책:

- fallback/fuzzy match는 diagnostic, repair request, approval candidate까지만 허용한다.
- 자동 적용은 exact match, precondition snapshot, preview diff, approval/policy decision, postcondition verification을 통과해야 한다.
- 전체 파일 rewrite fallback은 대형 파일 truncation과 잘못된 전체 치환 위험이 있으므로 기본 정책으로 사용하지 않는다.

### Benchmark Source Links

- Codex exec policy: `https://github.com/openai/codex/blob/main/codex-rs/core/src/exec_policy.rs`
- Codex tool handlers: `https://github.com/openai/codex/blob/main/codex-rs/core/src/tools/handlers/mod.rs`
- opencode permission service: `https://github.com/sst/opencode/blob/dev/packages/opencode/src/permission/index.ts`
- opencode bash tool: `https://github.com/sst/opencode/blob/dev/packages/opencode/src/tool/bash.ts`
- opencode truncation tool: `https://github.com/sst/opencode/blob/dev/packages/opencode/src/tool/truncate.ts`
- Cline command permission controller: `https://github.com/cline/cline/blob/main/src/core/permissions/CommandPermissionController.ts`
- Cline assistant message parser: `https://github.com/cline/cline/blob/main/src/core/assistant-message/parse-assistant-message.ts`
- Cline task runtime: `https://github.com/cline/cline/blob/main/src/core/task/index.ts`

### Priority

구현 우선순위는 다음과 같이 둔다.

1. JSON action envelope + raw payload block
2. strict path resolution and unique target requirement
3. no fuzzy mutation
4. two-phase mutation
5. observation schema
6. repeat failure circuit breaker
7. postcondition verification

주의:

```text
우선순위 7개만 문서화한다는 뜻이 아니다.
15개 전체가 정책이고, 위 7개는 먼저 구현해야 할 항목이다.
```

## Loop Guards

무한 루프와 같은 도구 반복을 막기 위해 controller guard가 필요하다.

초기 후보:

```text
max_model_turns
max_tool_calls
max_same_tool_repeats
max_elapsed_time
require_progress
```

예:

```text
max_tool_calls = 8
max_same_tool_repeats = 2
```

loop guard에 걸리면 `blocked` 또는 사용자 clarification으로 전환한다.

## Initial Tool Names

초기 concrete tool 후보는 `docs/product/tool-call-benchmark.ko.md`를 따른다.

```text
Explore:
  list_files
  find_files
  search_text
  read_file
  inspect_git
  web_search
  web_fetch

Change:
  apply_patch

Execute:
  run_command

Configure:
  add_provider
  update_config
```

주의:

- `request_approval`은 일반 모델 tool로 남용하지 않는다.
- approval은 controller permission flow의 결과로 발생하는 UI 상태일 수 있다.
- 초기 구현에서 model-facing Ask tools는 열지 않는다.
- 사용자 질문/확인은 `clarify` response type으로 처리한다.

## Tool Argument Schemas

`arguments`는 generic free-form object가 아니다. 각 `tool_name`별 typed schema로 검증한다.

공통 정책:

- unknown argument field는 거부한다.
- 긴 원문은 arguments에 직접 넣지 않는다.
- code/patch/file body가 필요한 tool은 `payload_id`를 사용한다.
- path는 workspace-relative를 기본으로 한다.
- absolute path, `..`, symlink escape, sensitive path는 controller path safety에서 별도 평가한다.
- optional field는 문서에 명시된 경우만 허용한다.
- large output을 만들 수 있는 도구는 limit field를 가진다.

### Explore Tools

#### list_files

```json
{
  "path": ".",
  "max_depth": 2,
  "max_entries": 200
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `path` | yes | string | workspace-relative directory |
| `max_depth` | yes | integer | initial upper bound candidate: 1-5 |
| `max_entries` | yes | integer | initial upper bound candidate: 1-500 |

#### find_files

```json
{
  "path": ".",
  "pattern": "*.md",
  "max_results": 50
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `path` | yes | string | workspace-relative directory |
| `pattern` | yes | string | glob-style file/path pattern |
| `max_results` | yes | integer | initial upper bound candidate: 1-200 |

#### search_text

```json
{
  "path": ".",
  "query": "permission",
  "use_regex": false,
  "max_results": 50
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `path` | yes | string | workspace-relative directory or file |
| `query` | yes | string | literal text by default |
| `use_regex` | yes | boolean | regex must be explicit |
| `max_results` | yes | integer | initial upper bound candidate: 1-200 |

#### read_file

```json
{
  "path": "docs/PROJECT-CONTEXT.md",
  "start_line": 1,
  "max_lines": 120
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `path` | yes | string | workspace-relative file |
| `start_line` | yes | integer | 1-based |
| `max_lines` | yes | integer | initial upper bound candidate: 1-300 |

#### inspect_git

```json
{
  "scope": "status"
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `scope` | yes | string enum | `status`, `diff_summary`, `recent_commits` |

#### web_search

```json
{
  "query": "official documentation query",
  "max_results": 5
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `query` | yes | string | search query |
| `max_results` | yes | integer | initial upper bound candidate: 1-10 |

#### web_fetch

```json
{
  "url": "https://example.com/docs",
  "max_bytes": 200000
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `url` | yes | string | http/https only |
| `max_bytes` | yes | integer | bounded fetch size |

### Change Tools

#### apply_patch

```json
{
  "payload_id": "patch_001"
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `payload_id` | yes | string | raw payload block id |

`apply_patch`는 controller가 path safety, diff preview, permission, uncertainty gate를 통과시킨 뒤에만 실행한다.

`apply_patch` payload block:

```text
<AHREUM_PAYLOAD id="patch_001" format="apply_patch">
*** Begin Patch
*** Update File: src/main.rs
@@
-old
+new
*** End Patch
</AHREUM_PAYLOAD>
```

정책:

- `apply_patch` 원문은 JSON string에 넣지 않는다.
- controller는 payload id, payload format, apply_patch grammar를 모두 검증한다.
- payload format이 `apply_patch`가 아니면 `apply_patch` tool candidate로 실행하지 않는다.

### Execute Tools

#### run_command

```json
{
  "argv": ["rg", "-n", "permission", "docs"],
  "cwd": ".",
  "timeout_ms": 30000
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `argv` | yes | array of string | program and args; no shell string by default |
| `cwd` | yes | string | workspace-relative directory |
| `timeout_ms` | yes | integer | bounded execution time |

초기 모델 스키마는 shell command string을 받지 않는다. shell feature가 필요한 경우 controller가 별도 정책과 approval을 통해 다룬다.

### Configure Tools

#### add_provider

```json
{
  "provider_id": "lm-studio",
  "base_url": "http://127.0.0.1:1234/v1",
  "model": "google/gemma-4-e4b",
  "context_tokens": 32000
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `provider_id` | yes | string | config key |
| `base_url` | yes | string | provider endpoint |
| `model` | yes | string | model id |
| `context_tokens` | yes | integer | model context setting |

#### update_config

```json
{
  "key_path": "providers.lm-studio.context_tokens",
  "value": 32000
}
```

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `key_path` | yes | string | known config key path |
| `value` | yes | string, number, boolean, array, or object | schema-validated by config target |

## Open Design Checks

- 초기 구현은 Rust serde validation으로 시작한다. JSON Schema 문서는 필요해질 때 생성/추가한다.
- malformed JSON repair는 1회만 허용한다. 2회 연속 실패하면 `blocked` 또는 사용자 보고로 전환한다.
- `answer`에 evidence reference 필드를 추가할지 결정한다.
- tool별 limit 상한값을 구현 전에 최종 숫자로 확정한다.
- model-facing Ask tools는 초기 구현에서 열지 않는다. 필요해질 때 별도 spec으로 추가한다.

## Change History

### 2026-05-11

- Created model response contract draft with one-next-action rule and controller-driven loop.
- Fixed the response envelope by removing `confidence` and `expected_next`.
- Fixed unknown field rejection and tool-specific argument schema direction.
- Closed initial model-facing Ask tools; clarification uses the `clarify` response type.
