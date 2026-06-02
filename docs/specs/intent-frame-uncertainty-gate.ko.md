---
id: intent-frame-uncertainty-gate-ko
type: spec
status: draft
topics:
  - intent-frame
  - uncertainty-gate
  - local-llm
  - permissions
  - tool-calling
summary: Korean specification draft for user intent framing and uncertainty handling by activity group.
last_updated: 2026-05-11
related:
  - docs/product/tool-call-benchmark.ko.md
  - docs/specs/model-response-contract.ko.md
  - docs/specs/permission-mode-policy.ko.md
  - docs/product/tui-ui-command-benchmark.ko.md
  - docs/architecture/agent-operating-guardrails.md
  - docs/PROJECT-CONTEXT.md
---

# Intent Frame And Uncertainty Gate Korean Draft

## 목적

이 문서는 아름코드가 사용자 의중을 어떻게 구조화하고, 불확실한 요청을 activity group별로 어떻게 처리할지 정의한다.

핵심 전제:

```text
Local LLM output is a next-action candidate, not trusted intent truth.
AhreumCode controller owns the intent frame, uncertainty gate, and final action decision.
```

로컬 LLM이 사용자의 의중, 대상 파일, 실행 범위, 위험도를 항상 올바르게 판단한다고 가정하지 않는다. 로컬 LLM은 후보를 제안하고, 아름코드 controller가 검증하고, 필요한 경우 검색/읽기/질문/차단으로 흐름을 정한다.

## Position In The Loop

`Intent Frame`과 `Uncertainty Gate`는 다음 위치에서 동작한다.

```text
User prompt
-> Intent Frame 생성/갱신
-> Local LLM next-action candidate
-> Schema validation
-> Uncertainty Gate
-> Permission/Hard Safety evaluation
-> Tool execution or Ask/Blocked/ManualOnly
-> Observation
-> Local LLM continuation
```

순서상 `Uncertainty Gate`는 permission approval보다 앞선다. 대상과 범위가 불명확한 변경/실행/설정은 approval 화면까지 보내지 않는다.

## Intent Frame

`Intent Frame`은 한 사용자 요청을 controller가 다루기 위한 구조화된 작업 의도다.

초기 필드 후보:

| Field | Meaning |
| --- | --- |
| `user_goal` | 사용자가 얻고 싶은 최종 결과 |
| `activity` | `None`, `Explore`, `Change`, `Execute`, `Configure`, `Ask` 중 현재 후보 활동 |
| `scope` | workspace, path, file set, command cwd, config target 등 작업 범위 |
| `constraints` | 사용자가 명시한 제한, 금지, 선호 |
| `known_targets` | 확인된 파일, 디렉토리, 명령, 설정 키 |
| `unknowns` | 아직 모르는 대상/범위/값/영향 |
| `allowed_next` | 지금 안전하게 가능한 다음 행동 |
| `blocked_next` | 지금 하면 안 되는 다음 행동 |
| `evidence` | 읽은 파일, 검색 결과, 사용자 승인, tool observation |

주의:

- `Intent Frame`은 로컬 LLM이 마음대로 확정하지 않는다.
- controller가 사용자 입력, 검색/읽기 결과, 승인 결과, tool observation을 바탕으로 갱신한다.
- 불확실성을 숨기지 않고 `unknowns`로 유지한다.

## Gate Outcomes

`Uncertainty Gate`의 결과는 다음 다섯 가지로 고정한다.

| Outcome | Meaning |
| --- | --- |
| `Proceed` | 안전하고 범위가 명확해서 바로 다음 단계로 진행 |
| `ExploreFirst` | 읽기/검색으로 불확실성을 줄인 뒤 다시 판단 |
| `AskUser` | 수행 가능성이 있지만 사용자 확인/선택/승인이 필요 |
| `Blocked` | 정책상 수행 불가 |
| `ManualOnly` | 위험도가 높아 아름코드는 직접 실행하지 않고 절차만 안내 |

`ManualOnly`는 `AskUser`보다 강하다. 사용자가 강하게 요청하거나 승인해도 아름코드는 직접 실행하지 않는다.

## Activity Policy

### None

목적:

```text
도구 호출 없는 일반 답변.
```

정책:

- 단순 설명, 의견, 문서화 방향 논의는 `None`이 될 수 있다.
- 파일/상태 근거가 필요한 답변이면 `Explore`로 전환한다.
- 확실하지 않은 프로젝트 사실을 추측해서 답하지 않는다.

#### Evidence Required Answer Gate

`None` answer는 항상 최종 성공이 아니다. controller는 `answer/None` 후보가 들어와도
현재 요청이 workspace evidence를 요구하면 `Proceed`하지 않고 `ExploreFirst`로 전환해야 한다.

근거가 필요한 요청:

- 현재 workspace의 파일, 디렉토리, 코드 위치, 구현 내용, dependency, git 상태를 묻는 요청
- 프로젝트 내부 구조, registry, 설정, 함수/타입 정의 위치를 묻는 요청
- 사용자가 정확한 파일을 모르지만 repository 안의 사실을 알고 싶어 하는 요청

근거로 인정하는 것:

- `project_context` system message가 제공한 고정 프로젝트 정보
- Tool Runtime이 실행해 남긴 `tool observation`
- 사용자가 프롬프트에 직접 제공한 코드, 로그, 텍스트

근거로 인정하지 않는 것:

- 로컬 LLM이 일반 지식으로 추측한 파일명, 경로, 함수명, registry 정보
- repair 후 만들어진 자연어 설명
- TUI workspace에 출력된 answer 자체

금지:

- 특정 테스트 프롬프트 문자열에 맞춘 분기
- 모델 문구를 사후 보정해 evidence가 있는 것처럼 처리
- observation 없는 프로젝트 내부 사실 답변을 부분 성공으로 표시

### Explore

목적:

```text
읽기, 검색, 조사로 근거를 확보한다.
```

정책:

- workspace-local, bounded read/search는 불확실성이 있어도 진행할 수 있다.
- 사용자가 파일명을 모르는 경우에도 파일 목록, 텍스트 검색, 관련 파일 읽기로 의중을 좁힐 수 있다.
- 검색/읽기는 사용자의 의중을 파악하기 위한 안전한 선행 작업으로 사용할 수 있다.
- workspace 밖, 대량 파일 읽기, 민감 파일, 네트워크 탐색은 별도 permission/hard safety 평가를 거친다.

예:

```text
"권한 처리 부분이 어디 있는지 알려줘"
-> ExploreFirst
-> find/search/read
-> evidence 기반 답변
```

### Change

목적:

```text
파일 생성, 수정, 삭제, rename, patch 적용.
```

정책:

- target/scope가 명확하지 않으면 진행하지 않는다.
- 후보 파일이 여러 개이면 `AskUser`로 전환한다.
- 변경 전에는 가능한 diff/preview를 만들고 approval을 받는다.
- 삭제/rename/대량 변경은 더 엄격하게 다룬다.
- 사용자의 말이 애매한데 임의로 파일을 골라 수정하지 않는다.

예:

```text
"그 파일 수정해줘"
-> 대상이 명확하지 않음
-> AskUser 또는 ExploreFirst 후 AskUser
```

### Execute

목적:

```text
shell command, build, test, dev server, process 실행.
```

정책:

- command, cwd, 목적, 예상 영향이 명확해야 한다.
- 초기 구현에서는 test/build도 approval 대상이다.
- 장시간 실행, 고부하, 외부 시스템 영향, 파일시스템 전체 영향 가능성이 있으면 `ManualOnly` 또는 `Blocked`로 처리한다.
- 명령어 문자열을 로컬 LLM 후보 그대로 실행하지 않는다. controller가 command policy를 평가한다.

예:

```text
"전체 캐시 싹 지워"
-> 시스템/프로젝트 전체 영향 가능
-> ManualOnly 또는 Blocked
```

### Configure

목적:

```text
provider, model, permission, project config 변경.
```

정책:

- 설정 대상과 값이 명확해야 한다.
- 사용자 정의 provider/model 추가는 expanded form 또는 명시적 approval을 거친다.
- 기본값을 임의로 추측해서 저장하지 않는다.
- 보안/권한/네트워크 관련 설정은 변경 이유와 영향 범위를 보여준다.

### Ask

목적:

```text
사용자 확인, 선택, 승인, clarification.
```

정책:

- `Ask`는 진행을 미루는 실패가 아니라 안전한 판단 지점이다.
- 질문은 사용자가 결정해야 하는 최소 정보만 묻는다.
- 사용자가 모른다고 답한 경우, read/search로 좁힐 수 있으면 `ExploreFirst`로 전환한다.
- read/search로도 좁힐 수 없고 위험 작업이면 `Blocked` 또는 `ManualOnly`로 처리한다.

## ManualOnly Policy

`ManualOnly`는 아름코드가 직접 실행하지 않는 안내 전용 상태다.

적용 대상:

- 전체 삭제, 대량 삭제, workspace 밖 파일시스템 변경
- `rm -rf`, 대량 `find -delete`, 대량 권한 변경, 대량 소유권 변경
- 시스템 디렉토리, 홈 전체, 숨김 설정, 키체인, credential 저장소 접근
- CPU/메모리/디스크 부하가 큰 작업
- 장시간 백그라운드 작업, 무한 루프 가능 작업
- 외부 서비스, 배포, 결제, 계정, 인증, 보안 정책에 영향 있는 작업
- 프로젝트 전체 구조를 한 번에 바꾸는 대규모 자동 변환
- 복구가 어렵거나 영향 범위를 controller가 제한할 수 없는 작업

응답 원칙:

```text
This action can affect the system, filesystem, project-wide state, or external services.
AhreumCode will not run it directly.
The following command/procedure is guidance only. Review scope, backup, and risk before running it yourself.
```

한국어 UI/응답 후보:

```text
이 작업은 시스템, 파일시스템, 프로젝트 전체 상태, 또는 외부 서비스에 큰 영향을 줄 수 있어 아름코드가 직접 실행하지 않습니다.
아래 명령/절차는 참고용입니다. 실행 전 범위, 백업, 복구 가능성을 직접 확인해야 합니다.
```

`ManualOnly`에서 명령어를 안내할 때는 다음을 함께 제공한다.

- 왜 직접 실행하지 않는지
- 영향 범위
- 실행 전 확인할 항목
- 가능한 백업/되돌리기 기준
- 더 안전한 제한 범위가 있는 대안

## User Does Not Know

사용자가 대상 파일, 경로, 정확한 구현 위치를 모를 수 있다. 이 경우를 실패로 처리하지 않는다.

정책:

- 읽기/검색으로 좁힐 수 있으면 `ExploreFirst`.
- 여러 후보를 근거와 함께 찾은 뒤 변경이 필요하면 `AskUser`.
- 사용자가 모르는 상태에서 변경/삭제/실행/설정을 추측해서 진행하지 않는다.
- 단순 설명/요약 요청은 여러 파일을 읽고 evidence 기반으로 답할 수 있다.

예:

```text
"전에 말한 권한 관련 문서가 뭐였지?"
-> ExploreFirst
-> 관련 문서 검색/읽기
-> 근거와 함께 답변
```

```text
"그 권한 코드 고쳐줘. 어디 있는지는 몰라."
-> ExploreFirst로 후보 탐색
-> target 후보와 변경 방향 제시
-> AskUser/approval 후 Change
```

## Outcome Rules By Activity

| Activity | Proceed | ExploreFirst | AskUser | Blocked | ManualOnly |
| --- | --- | --- | --- | --- | --- |
| `None` | 일반 답변 가능 | 근거 필요 시 | 질문 자체가 답변에 필요할 때 | 정책상 답변 불가 | 해당 없음 |
| `Explore` | bounded read/search | 대상 탐색 필요 시 | workspace 밖, web, 민감 범위 | 민감/금지 대상 | 대량/고부하 조사 |
| `Change` | 명확한 target + 승인 후 | 후보 탐색만 가능 | target/scope 선택 필요 | 금지 대상 변경 | 대량/시스템 영향 변경 |
| `Execute` | 명확한 safe command + 승인 후 | command 생성을 위한 조사 | cwd/영향 확인 필요 | 금지 명령 | 고부하/파괴/외부 영향 명령 |
| `Configure` | 명확한 값 + 승인 후 | 관련 config 탐색 | 값/대상 확인 필요 | 보안상 금지 설정 | 시스템/계정 영향 설정 |
| `Ask` | 사용자 결정 요청 | 사용자가 모르면 조사 전환 | 기본 상태 | 질문해도 안전해질 수 없음 | 사용자 직접 수행 안내 |

## Controller Requirements

초기 구현 요구:

- activity group과 concrete tool을 분리한다.
- 로컬 LLM 응답을 바로 실행하지 않는다.
- schema validation 이후 `Intent Frame`과 `Uncertainty Gate`를 통과해야 한다.
- `ExploreFirst`는 observation을 만든 뒤 로컬 LLM에 다시 요청한다.
- `AskUser`, `Blocked`, `ManualOnly`는 사용자에게 명확한 이유를 보여준다.
- 동일한 불확실성으로 반복 루프가 발생하면 loop guard가 개입한다.
- `ManualOnly`는 permission approval로 승격되지 않는다.

## UI Mapping

왼쪽 timeline tag 후보:

| Outcome | Timeline tag |
| --- | --- |
| `Proceed` | `[valid]` |
| `ExploreFirst` | `[evidence]` or `[manager]` |
| `AskUser` | `[approve]` |
| `Blocked` | `[danger]` |
| `ManualOnly` | `[danger]` |

오른쪽 persona messenger에는 system outcome tag를 넣지 않는다. 필요한 경우 팀장이 자연어로 상황을 설명할 수 있지만, tool/policy/system log는 왼쪽 timeline에만 남긴다.

## Non-Goals

- 이 문서는 최종 tool schema를 정의하지 않는다.
- 이 문서는 permission matrix 전체를 대체하지 않는다.
- 이 문서는 모드 이름이나 persona 대화 스타일을 정의하지 않는다.
- `ManualOnly`는 사용자를 막는 UX가 아니라, 에이전트가 직접 실행하지 않는 안전 경계다.

## Open Implementation Checks

- `Explore` bounded 범위의 초기 파일 수/바이트 제한을 정한다.
- `ManualOnly` 명령 안내 포맷을 TUI에서 어떻게 접고 펼칠지 정한다.
- `AskUser`와 permission approval surface를 같은 컴포넌트로 처리할지 분리할지 정한다.
- `Intent Frame`을 session state에 저장할지 run-local state로 둘지 정한다.
- `unknowns`가 일정 횟수 이상 줄지 않을 때 loop guard 문구를 정한다.

## Change History

### 2026-05-11

- Created the initial Intent Frame and Uncertainty Gate policy.
