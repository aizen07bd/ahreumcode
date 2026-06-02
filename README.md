# 아름코드 AhreumCode

한국어 README입니다. English README: [README.en.md](README.en.md)

아름코드는 로컬 LLM을 위한 Rust TUI 코딩 에이전트 런타임입니다.

로컬 모델이 도구 호출을 한 번에 완벽하게 만든다고 가정하지 않습니다. 대신 모델 응답을 파싱하고, 깨진 응답을 복구하고, 권한과 승인을 확인하고, 실제 도구 실행 결과를 구조화된 관측으로 기록한 뒤, 충분한 근거가 있을 때만 작업 완료로 닫습니다.

## 상태

- 현재 릴리스: `v1.0.0`
- 주 배포 바이너리: macOS Apple Silicon
- 런타임: LM Studio 같은 OpenAI-compatible 로컬 엔드포인트
- 기본 로컬 엔드포인트 예시: `http://127.0.0.1:1234/v1`
- 주요 검증 모델: `qwen3-4b-instruct-2507`

## 왜 만들었나

로컬 LLM은 비용과 프라이버시 측면에서 매력적이지만, 코딩 에이전트로 사용하면 도구 호출이 자주 깨집니다.

자주 발생하는 실패:

- JSON 또는 응답 envelope 형식 오류
- 잘못된 tool 이름, activity, 인자 구조
- 파일 patch payload 참조 누락
- workspace 근거 없이 파일 경로 추측
- 같은 실패 도구 호출 반복
- 실제 파일 상태와 맞지 않는 patch 생성
- 도구 실행이 실패했는데도 성공한 것처럼 최종 답변

아름코드는 raw tool call, repair, permission check, execution, observation, guarded completion을 분리해 로컬 모델의 실패를 제품 런타임에서 관리합니다.

## 주요 기능

- `ratatui`, `crossterm` 기반 Rust 터미널 UI
- OpenAI-compatible 로컬 LLM provider 연동
- tool manifest echo 검증이 포함된 구조화 응답 계약
- 지원 도구:
  - `read_file`
  - `search_text`
  - `list_files`
  - `apply_patch`
  - `run_command`
- 깨진 로컬 LLM 응답을 위한 parser/repair loop
- 파일 변경과 명령 실행에 대한 승인 기반 흐름
- patch 적용 전 파일 precondition snapshot
- 파일 변경 후 postcondition 검증
- 여러 파일을 한 번에 다루는 multi-target `apply_patch`
- 여러 read/create/update/delete 대상을 추적하는 plan ledger
- 내부 상태나 실제 파일 경로가 persona panel에 노출되지 않도록 하는 경계 강화

## 런타임 흐름

```text
사용자 요청
  -> schema prompt와 대화 컨텍스트 구성
  -> 로컬 LLM 응답
  -> 응답 parser
  -> decision gate
  -> permission 또는 approval gate
  -> 도구 실행
  -> 구조화된 observation
  -> repair 또는 follow-up 요청
  -> 근거 기반 최종 답변
```

모델의 첫 응답은 성공으로 보지 않습니다. 실행 가능한 tool candidate와 실제 runtime evidence가 있어야 완료로 인정합니다.

## 설치

플랫폼에 맞는 최신 Release asset을 내려받습니다.

macOS Apple Silicon:

```bash
tar -xzf ahreumcode-v1.0.0-darwin-arm64.tar.gz
cd ahreumcode-v1.0.0-darwin-arm64
./ahreumcode
```

macOS가 서명되지 않은 바이너리를 차단하면 quarantine 속성을 제거합니다.

```bash
xattr -d com.apple.quarantine ./ahreumcode
```

## 소스에서 빌드

필요 조건:

- Rust toolchain
- OpenAI-compatible API를 제공하는 로컬 LLM 서버

빌드:

```bash
cargo build --release
```

실행:

```bash
cargo run
```

검증:

```bash
cargo fmt --check
cargo test
```

## 로컬 LLM 벤치마크

아름코드는 세 가지 지표로 도구 호출을 측정합니다.

| 지표 | 의미 |
| --- | --- |
| Raw Tool Call Success | 첫 모델 응답이 기대 도구 후보로 파싱/분류된 비율 |
| Repaired Tool Call Success | raw 성공 또는 repair loop를 통해 실행 가능한 도구 후보로 복구된 비율 |
| Guarded Task Completion | 복구된 후보가 실제 실행되고 시나리오별 근거 검증까지 만족한 비율 |

벤치마크는 30개 로컬 코딩 에이전트 시나리오로 구성됩니다.

- 정확한 근거 확인이 필요한 파일 읽기
- 텍스트 검색
- 디렉터리 목록 조회
- `apply_patch` 기반 파일 생성
- read-before-change가 필요한 파일 수정
- `run_command` 기반 명령 실행

### v1.0.0 릴리스 검증

최신 릴리스 검증은 LM Studio의 `http://127.0.0.1:1234/v1` 엔드포인트에서 `qwen3-4b-instruct-2507` 모델로 수행했습니다.

| 모델 | Raw Tool Call | Repaired Tool Call | Guarded Completion |
| --- | ---: | ---: | ---: |
| `qwen3-4b-instruct-2507` | 24/30, 80.0% | 30/30, 100.0% | 30/30, 100.0% |

시나리오별 결과:

| 그룹 | 시나리오 수 | Guarded Result |
| --- | ---: | ---: |
| `read_file` | 5 | 5/5 |
| `search_text` | 5 | 5/5 |
| `list_files` | 5 | 5/5 |
| `apply_patch` create | 5 | 5/5 |
| `apply_patch` update | 5 | 5/5 |
| `run_command` | 5 | 5/5 |

릴리스 검증 명령:

```bash
cargo fmt --check
cargo test
cargo run --bin local_llm_bench -- --model qwen3-4b-instruct-2507 --base-url http://127.0.0.1:1234/v1
```

### 로컬 모델 비교 벤치마크

이전 비교 벤치는 같은 30개 시나리오 형태를 여러 로컬 모델에 적용했습니다. 아래 결과는 모델 선택 참고용이며, 현재 릴리스 기준은 위의 `v1.0.0` 검증 결과입니다.

| 모델 | Script Raw | Script Repaired | Script Guarded | TUI Raw | TUI Repaired | TUI Guarded | 메모 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `qwen3-4b-instruct-2507` | 70.0% | 100.0% | 93.3% | 86.7% | 100.0% | 80.0% | 이전 비교 실행에서 가장 안정적인 실사용 후보 |
| `qwen2.5-coder-7b-instruct` | 93.3% | 100.0% | 80.0% | 90.0% | 96.7% | 40.0% | raw tool 선택은 강하지만 TUI completion closure가 약함 |
| `google/gemma-4-e4b` | 63.3% | 93.3% | 76.7% | 63.3% | 76.7% | 43.3% | repair 효과는 있으나 계약 유지가 불안정함 |
| `hermes-3-llama-3.1-8b` | 66.7% | 70.0% | 53.3% | 80.0% | 86.7% | 20.0% | tool 후보 선택 대비 완료 근거가 약함 |

해석:

- raw 성공률만으로는 코딩 에이전트 품질을 판단할 수 없습니다.
- 제품 가치는 raw output과 guarded completion 사이의 간격에서 드러납니다.
- 로컬 LLM은 그럴듯한 도구를 선택해도 observation 이후 완료 단계에서 흔들릴 수 있습니다.
- 아름코드는 실패를 숨기지 않고 관측 가능하게 만들며, 안전하게 복구 가능한 실패만 복구합니다.

## 안전 모델

아름코드는 모델을 신뢰하기보다 runtime boundary를 둡니다.

- Explore 도구는 workspace 내부와 bounded range로 제한합니다.
- 파일 변경은 `apply_patch` payload와 사용자 승인을 요구합니다.
- update/delete는 현재 파일 근거를 요구합니다.
- 명령은 capability별로 분리하고 승인을 요구합니다.
- 외부 경로, 민감 경로, 네트워크 접근, 파괴적 명령은 조용히 보정해 실행하지 않습니다.
- 실패한 도구 결과는 성공으로 처리하지 않고 구조화된 observation으로 모델에게 다시 전달합니다.

## 릴리스 노트

`v1.0.0` 포함 내용:

- 로컬 LLM 응답 parser와 repair loop
- typed tool schema와 manifest echo check
- 파일 읽기/검색/목록 조회 guarded flow
- 승인 기반 `apply_patch`, `run_command`
- multi-target patch 지원
- multi-file task를 위한 plan ledger
- Persona panel 경계 강화
- qwen3 로컬 벤치 기준 30/30 guarded completion 검증

## 프로젝트 상태

아름코드는 로컬 LLM 코딩 에이전트 런타임으로 사용할 수 있는 상태지만 아직 초기 소프트웨어입니다. 현재 릴리스는 macOS Apple Silicon과 LM Studio 계열 로컬 provider를 중심으로 검증했습니다.

## 라이선스

MIT License. 자세한 내용은 [LICENSE](LICENSE)를 참고하세요.
