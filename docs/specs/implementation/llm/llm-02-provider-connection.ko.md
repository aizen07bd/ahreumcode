---
id: llm-02-provider-connection-ko
type: spec
status: draft
topics:
  - local-llm
  - lm-studio
  - provider
  - health-check
summary: Korean section technical specification for llm-02 provider connection.
last_updated: 2026-05-14
related:
  - docs/specs/implementation/local-llm-runtime-technical-spec.ko.md
  - docs/specs/configuration-policy.ko.md
  - docs/tasks/local-llm-runtime-todo.ko.md
  - docs/specs/logging-policy.ko.md
---

# llm-02 Provider Connection

## 설명

LM Studio OpenAI-compatible endpoint에 연결 가능한지 확인한다. `/health`는 실제 긴 응답 생성을 요구하지 않고, endpoint와 모델 접근 가능 여부를 진단한다.

## 주요 함수

| Function | Role |
| --- | --- |
| `LlmProviderFactory::from_config(config)` | config에 맞는 provider를 만든다. |
| `LmStudioProvider::health_check()` | endpoint/model 상태를 확인한다. |
| `build_models_request(config)` | 모델 목록 또는 모델 확인 요청을 만든다. |
| `measure_latency(operation)` | health check 응답 시간을 측정한다. |
| `map_provider_error(error)` | connection/timeout/model error를 구분한다. |

## 함수 연결 흐름

```mermaid
flowchart TD
  A[/health command] --> B[LlmProviderFactory::from_config]
  B --> C[LmStudioProvider::health_check]
  C --> D[build_models_request]
  D --> E[send request]
  E --> F{success?}
  F -->|yes| G[measure_latency]
  F -->|no| H[map_provider_error]
  G --> I[render health success]
  H --> J[render health failure]
```

## 로그 이벤트

- `llm_health_check_started`
- `llm_health_check_succeeded`
- `llm_health_check_failed`
- `llm_latency_recorded`

## 완료 기준

- LM Studio가 실행 중이면 `/health`에 성공 상태와 응답 시간이 보인다.
- LM Studio가 꺼져 있으면 연결 실패가 recoverable error로 보인다.
- endpoint 실패, timeout, 모델 없음이 같은 메시지로 뭉개지지 않는다.
- scope id `llm-02-provider-connection` 로그가 남는다.
