---
id: tool-02-observation-and-truncation-ko
type: spec
status: implemented
topics:
  - tool-runtime
  - observation
  - truncation
  - artifact
  - implementation-spec
summary: Korean technical specification for separating Tool Runtime preview output from preserved artifact output.
last_updated: 2026-05-17
related:
  - docs/specs/implementation/tool-runtime-technical-spec.ko.md
  - docs/specs/implementation/tool/tool-01-explore-tool-runtime.ko.md
  - docs/tasks/tool-runtime-todo.ko.md
  - docs/specs/logging-policy.ko.md
---

# tool-02 Observation And Truncation

## 목적

`tool-02`는 도구 실행 결과를 화면 preview와 보존 artifact로 분리한다.

도구가 긴 결과를 만들었을 때 TUI workspace에는 짧은 preview만 보여주고, 전체 결과는 log bucket 아래 artifact로 저장한다. observation에는 전체 line/byte 수, preview truncation 여부, source truncation 여부, 다음 범위 힌트를 남긴다.

## 범위

포함:

- observation preview line limit
- full output artifact path
- total lines
- total bytes
- preview truncated 여부
- source truncated 여부
- next range hint
- tool log metadata 확장

제외:

- LLM tool loop 재요청
- artifact 장기 보관 정책
- web/network artifact
- mutation artifact
- config.toml에 preview limit 추가

## 출력 정책

초기 정책:

```text
tool observation preview = first 40 lines
```

이 값은 TUI workspace를 읽을 수 있는 상태로 유지하기 위한 product policy이다. 모델이 요청한 `max_lines`, `max_results`, `max_entries`는 도구별 source limit으로 유지하고, preview limit은 화면/LLM observation 전달을 위한 별도 표시 제한이다.

## artifact 위치

```text
.ahreumcode/logs/sessions/YYYY-MM-DD/artifacts/tool/
```

파일명 형식:

```text
{run_id}_{turn_id}_{tool_name}.txt
```

현재 runtime은 한 LLM turn에 하나의 tool call만 허용하므로 이 이름은 충돌하지 않는다. 후속 multi-tool 구조가 열리면 call index를 추가한다.

## 데이터 구조

`ToolObservation`은 다음 metadata를 가진다.

```rust
struct ToolObservation {
    preview: Vec<String>,
    total_lines: usize,
    total_bytes: usize,
    truncated: bool,
    source_truncated: bool,
    preview_truncated: bool,
    artifact_path: Option<String>,
    next_range_hint: Option<String>,
}
```

의미:

- `total_lines`: 도구가 runtime에 전달한 전체 output line 수
- `total_bytes`: 전체 output을 newline으로 합쳤을 때의 byte 수
- `source_truncated`: 도구 자체 limit 때문에 더 가져오지 않은 결과가 있음
- `preview_truncated`: preview line limit 때문에 화면에서 줄인 결과가 있음
- `truncated`: `source_truncated || preview_truncated`
- `artifact_path`: preview 밖 전체 output을 저장한 artifact
- `next_range_hint`: 다음에 이어서 요청할 수 있는 범위 힌트

## 완료 기준

- 긴 observation은 preview와 artifact로 분리된다.
- 짧은 observation은 artifact를 만들지 않는다.
- `total_lines`, `total_bytes`, `truncated` metadata가 log에 남는다.
- `read_file` source truncation은 다음 `start_line` hint를 남긴다.
- `cargo fmt --check`가 통과한다.
- `cargo test`가 통과한다.
- `cargo run -- --scene main --smoke`가 통과한다.

## Change History

### 2026-05-17

- Created `tool-02` technical spec before implementation.
- Implemented observation output policy with preview limit, artifact writing, total line/byte metadata, truncation flags, and next range hints.
- Extended tool log metadata for total output size, artifact path, and truncation source.
- Verified with `cargo fmt --check`, `cargo test`, and `cargo run -- --scene main --smoke`.
