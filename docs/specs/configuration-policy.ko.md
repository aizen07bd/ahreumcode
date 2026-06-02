---
id: configuration-policy-ko
type: spec
status: draft
topics:
  - configuration
  - init
  - docs-init
  - local-llm
  - project-instructions
summary: Korean specification draft for AhreumCode configuration, init commands, and opt-in documentation features.
last_updated: 2026-05-11
related:
  - docs/specs/permission-mode-policy.ko.md
  - docs/specs/intent-frame-uncertainty-gate.ko.md
  - docs/specs/model-response-contract.ko.md
  - docs/product/tui-ui-command-benchmark.ko.md
  - docs/architecture/agent-operating-guardrails.md
  - docs/PROJECT-CONTEXT.md
---

# Configuration Policy Korean Draft

## 목적

이 문서는 아름코드의 초기 config 구조와 `/init`, `/docs-init` 명령이 config를 어떻게 확장하는지 정의한다.

핵심 방향:

```text
Default config stays thin.
Project instruction and documentation management are opt-in through slash commands.
Hard safety cannot be disabled by config.
```

## Config File Location

기본 project-local config 위치:

```text
.ahreumcode/config.toml
```

초기 구현은 project-local config를 우선한다.

주의:

- config는 실행 설정이다.
- `AGENTS.md`는 프로젝트 운영 지시문이다.
- 문서 템플릿/라우터는 문서 관리 기능이다.
- 이 세 가지를 기본 config에서 강제로 묶지 않는다.

## Initial Default Config

새 프로젝트에서 아름코드가 생성하거나 기대하는 기본 config는 다음처럼 얇게 유지한다.

```toml
[provider]
active = "lm-studio"

[providers.lm-studio]
type = "openai-compatible"
base_url = "http://127.0.0.1:1234/v1"
model = "google/gemma-4-e4b"
context_tokens = 32000
api_key_env = ""

[workspace]
root = "."

[mode]
default = "Crew"

[persona]
default = "off"
min_terminal_width = 140

[limits]
max_model_turns = 8
max_tool_calls = 8
max_same_tool_repeats = 2
read_max_lines = 300
search_max_results = 200
command_timeout_ms = 30000

[web]
enabled = true
```

초기 config에 넣지 않는 것:

- `[instructions]`
- `[docs]`
- statusline customization
- color theme customization
- persona 말투/성격 세부 설정
- 전체 permission matrix 사용자 편집
- 복잡한 tool allowlist
- 실험 기능 toggle 묶음

## Provider Policy

초기 기본 provider:

```toml
[provider]
active = "lm-studio"

[providers.lm-studio]
type = "openai-compatible"
base_url = "http://127.0.0.1:1234/v1"
model = "google/gemma-4-e4b"
context_tokens = 32000
api_key_env = ""
```

의미:

- LM Studio의 OpenAI-compatible endpoint를 기본값으로 둔다.
- 모델은 초기 테스트 대상인 `google/gemma-4-e4b`를 기본값으로 둔다.
- context는 `32000`으로 둔다.
- local endpoint는 API key가 없을 수 있으므로 `api_key_env`는 빈 문자열을 허용한다.

`/provider add` 또는 provider expanded form은 이 구조를 갱신한다.

## Mode Policy

기본 mode:

```toml
[mode]
default = "Crew"
```

정책:

- `Guide`, `Crew`, `Pilot`만 허용한다.
- 기본값은 `Crew`다.
- `Pilot`은 별도 explicit confirmation 없이 자동 기본값으로 설정하지 않는다.
- mode는 permission/hard safety를 우회하지 못한다.

## Persona Policy

기본 persona:

```toml
[persona]
default = "off"
min_terminal_width = 140
```

정책:

- persona messenger는 기본 off다.
- 사용자가 full 형태로 켜야 한다.
- terminal width가 `min_terminal_width` 미만이면 persona messenger를 열지 않는다.
- 좁은 terminal에서는 왼쪽 timeline에 system log로 이유를 보여준다.

## Limits Policy

초기 limit:

```toml
[limits]
max_model_turns = 8
max_tool_calls = 8
max_same_tool_repeats = 2
read_max_lines = 300
search_max_results = 200
command_timeout_ms = 30000
```

정책:

- limit은 로컬 LLM loop 폭주와 tool 반복을 막기 위한 최소 안전장치다.
- limit을 높여도 hard safety, ManualOnly, uncertainty gate는 우회하지 못한다.
- 구현 전에 각 tool의 내부 hard cap을 별도로 둘 수 있다.

## Web Policy

기본 web 설정:

```toml
[web]
enabled = true
```

의미:

- 웹 검색 기능은 기본적으로 사용 가능하다.
- web search/fetch는 `Explore` activity에 속한다.
- `enabled = true`는 모든 네트워크 요청을 무승인으로 실행한다는 뜻이 아니다.
- permission policy, network policy, hard safety, user intent gate를 계속 통과해야 한다.

## Init Command

`/init`은 프로젝트 운영 지시문 기능을 opt-in으로 설정한다.

역할:

- `AGENTS.md` 존재 여부 탐지
- 없으면 생성 후보 제안
- 있으면 config 연결 후보 제안
- 최종 반영 전 변경 preview와 approval 제공

`/init` 이후 추가될 수 있는 config:

```toml
[instructions]
project_file = "AGENTS.md"
```

정책:

- 기본 config에는 `[instructions]`를 넣지 않는다.
- `/init`을 실행한 프로젝트만 project instruction source를 config에 기록한다.
- `AGENTS.md`를 자동으로 길게 늘리지 않는다.
- nested `AGENTS.md` 처리는 구현 단계에서 별도 path discovery 정책으로 다룬다.
- config는 `AGENTS.md`를 대체하지 않는다.

slash command 분류:

```text
/init -> Configure
```

## Docs Init Command

`/docs-init`은 문서 관리 기능을 opt-in으로 설정한다.

역할:

- 문서 가이드 파일 탐지
- 템플릿 디렉토리 탐지
- 문서 라우터 탐지
- 발견 결과를 사용자에게 보여준다.
- 최종 반영 전 변경 preview와 approval 제공

`/docs-init` 이후 추가될 수 있는 config:

```toml
[docs]
guide_file = "documentation-guide.md"
template_dir = "templates"
router_file = "docs/PROJECT-CONTEXT.md"
use_templates = true
update_router_on_new_doc = "ask"
durable_docs_require_frontmatter = true
```

정책:

- 기본 config에는 `[docs]`를 넣지 않는다.
- `/docs-init`을 실행한 프로젝트만 문서 관리 기능을 config에 기록한다.
- 새 durable doc 생성 시 template 사용을 권장한다.
- 새 문서가 router에 들어갈지는 기본 `ask`다.
- 모든 markdown을 자동 재작성하지 않는다.
- 사용자가 요청하지 않은 문서 구조 변경을 하지 않는다.
- 문서 번호, 시나리오, 테스트 케이스를 자동 증식하지 않는다.

slash command 분류:

```text
/docs-init -> Configure
```

## Config Cannot Disable Safety

다음 정책은 config로 끌 수 없다.

- hard safety limits
- ManualOnly
- uncertainty gate
- path safety
- secret protection
- destructive command block/ask policy
- outside-workspace safety
- high-impact system/project-wide action refusal

즉 다음과 같은 설정은 허용하지 않는다.

```toml
[unsafe]
disable_hard_safety = true
allow_manual_only_execution = true
```

## Config Validation

controller는 config load 시 다음을 검증한다.

- TOML parse 가능 여부
- unknown top-level section 처리
- provider active target 존재 여부
- mode enum 유효성
- persona width 양수 여부
- limit 값의 허용 범위
- web enabled boolean 여부
- opt-in section path safety

초기 정책:

- unknown top-level section은 warning 후 무시할 수 있다.
- unknown key inside known section은 warning 후 무시할 수 있다.
- 안전 관련 section에서 unknown key가 발견되면 warning을 강하게 표시한다.
- 구현 시 실제 오류/경고 기준은 config loader spec에서 더 좁힌다.

## Non-Goals

- 이 문서는 모든 provider 옵션을 정의하지 않는다.
- 이 문서는 permission matrix 저장 포맷 전체를 정의하지 않는다.
- 이 문서는 color/theme 설정을 열지 않는다.
- 이 문서는 문서 시스템을 모든 프로젝트에 강제하지 않는다.
- 이 문서는 `AGENTS.md` 내용을 생성하는 템플릿 자체를 정의하지 않는다.

## Open Implementation Checks

- config 파일이 없을 때 자동 생성할지, 첫 실행에서 생성 확인을 받을지 결정한다.
- unknown config key를 warning으로 둘지 error로 둘지 구현 전에 한 번 더 결정한다.
- global config와 project config를 둘지, 초기에는 project config만 둘지 결정한다.
- `/init`에서 `AGENTS.md` 생성 템플릿을 어디까지 제공할지 결정한다.
- `/docs-init`에서 router file이 없을 때 생성 후보를 제공할지 결정한다.

## Change History

### 2026-05-11

- Created the initial configuration policy with thin default config and opt-in `/init` and `/docs-init`.
