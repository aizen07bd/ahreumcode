# AhreumCode

English README. Korean version: [README.md](README.md)

AhreumCode is a Rust TUI coding-agent runtime for local LLMs.

It makes weak local model tool calling usable by validating, repairing, approving, executing, and checking tool calls at runtime. The goal is not to assume that a local model will produce a perfect tool call on the first try, but to recover safe failures and block unsafe ones.

## Status

- Current release: `v1.0.0`
- Primary binary target: macOS Apple Silicon
- Runtime: local OpenAI-compatible endpoint, such as LM Studio at `http://127.0.0.1:1234/v1`
- Main validation model: `qwen3-4b-instruct-2507`

## Why

Local LLMs are attractive for privacy and cost, but weak tool calling makes them brittle as coding agents.

Common failure modes:

- malformed JSON or broken response envelopes
- wrong tool names, activities, or argument shapes
- missing payload references for file patches
- guessed file paths without workspace evidence
- repeated failed tool calls
- patch generation that does not match actual file state
- final answers that claim success without successful tool observations

AhreumCode separates first-pass tool selection, repair, permission checks, execution, observation, and guarded completion.

## Features

- Rust terminal UI built with `ratatui` and `crossterm`
- Local OpenAI-compatible LLM provider integration
- Structured response contract with manifest echo validation
- Tool support for `read_file`, `search_text`, `list_files`, `apply_patch`, and `run_command`
- Parser and repair loop for malformed local LLM responses
- Approval-based mutation and command execution
- File precondition snapshots before applying patches
- Postcondition checks after file changes
- Multi-target `apply_patch` support for related multi-file changes
- Plan ledger for multi-target read/create/update/delete goals
- Persona panel with strict boundaries to avoid leaking internal state or literal paths

## Runtime Flow

```text
user prompt
  -> schema prompt and conversation context
  -> local LLM response
  -> response parser
  -> decision gate
  -> permission or approval gate
  -> tool execution
  -> structured observation
  -> repair or follow-up request
  -> guarded final answer
```

A raw model response is not treated as success. Success requires an executable tool candidate and enough runtime evidence to complete the user request.

## Install

Download the latest release asset for your platform.

For macOS Apple Silicon:

```bash
tar -xzf ahreumcode-v1.0.0-darwin-arm64.tar.gz
cd ahreumcode-v1.0.0-darwin-arm64
./ahreumcode
```

If macOS blocks the unsigned binary after download, remove the quarantine attribute:

```bash
xattr -d com.apple.quarantine ./ahreumcode
```

## Build From Source

Requirements:

- Rust toolchain
- Local LLM server with an OpenAI-compatible API

Build:

```bash
cargo build --release
```

Run:

```bash
cargo run
```

Useful checks:

```bash
cargo fmt --check
cargo test
```

## Local LLM Benchmark

AhreumCode tracks three benchmark metrics:

| Metric | Meaning |
| --- | --- |
| Raw Tool Call Success | The first model response parsed and classified into an expected tool candidate. |
| Repaired Tool Call Success | Raw success or repair loop recovered an expected executable tool candidate. |
| Guarded Task Completion | The recovered candidate executed and satisfied scenario-specific runtime evidence. |

The benchmark covers 30 local coding-agent scenarios:

- file reads with exact evidence checks
- text search
- directory listing
- file creation with `apply_patch`
- file update with read-before-change requirements
- command execution through `run_command`

### v1.0.0 Release Validation

Latest release validation was run against `qwen3-4b-instruct-2507` via LM Studio at `http://127.0.0.1:1234/v1`.

| Model | Raw Tool Call | Repaired Tool Call | Guarded Completion |
| --- | ---: | ---: | ---: |
| `qwen3-4b-instruct-2507` | 24/30, 80.0% | 30/30, 100.0% | 30/30, 100.0% |

Scenario coverage:

| Group | Scenarios | Guarded Result |
| --- | ---: | ---: |
| `read_file` | 5 | 5/5 |
| `search_text` | 5 | 5/5 |
| `list_files` | 5 | 5/5 |
| `apply_patch` create | 5 | 5/5 |
| `apply_patch` update | 5 | 5/5 |
| `run_command` | 5 | 5/5 |

Release validation commands:

```bash
cargo fmt --check
cargo test
cargo run --bin local_llm_bench -- --model qwen3-4b-instruct-2507 --base-url http://127.0.0.1:1234/v1
```

### Comparative Local Model Runs

Earlier comparative benchmark runs used the same 30-scenario shape across several local models. These results are useful for model selection, but the `v1.0.0` release validation above is the current release gate.

| Model | Script Raw | Script Repaired | Script Guarded | TUI Raw | TUI Repaired | TUI Guarded | Notes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `qwen3-4b-instruct-2507` | 70.0% | 100.0% | 93.3% | 86.7% | 100.0% | 80.0% | Best practical local candidate in earlier runs. |
| `qwen2.5-coder-7b-instruct` | 93.3% | 100.0% | 80.0% | 90.0% | 96.7% | 40.0% | Strong raw tool choice, weaker TUI completion closure. |
| `google/gemma-4-e4b` | 63.3% | 93.3% | 76.7% | 63.3% | 76.7% | 43.3% | Repair helps, but contract stability is inconsistent. |
| `hermes-3-llama-3.1-8b` | 66.7% | 70.0% | 53.3% | 80.0% | 86.7% | 20.0% | Tool selection is usable, but completion evidence is weak. |

Benchmark interpretation:

- Raw success is not enough for a coding agent.
- The practical value is in the gap between raw output and guarded completion.
- Local LLMs can choose plausible tools but still fail after observations.
- AhreumCode makes these failures explicit, recoverable when safe, and blocked when unsafe.

## Safety Model

AhreumCode uses runtime boundaries rather than trusting the model.

- Explore tools are bounded and workspace-scoped.
- File changes require an `apply_patch` payload and approval.
- Updates and deletes require current file evidence.
- Commands are split by capability and require approval.
- External paths, sensitive paths, network access, and destructive commands are not silently normalized into executable actions.
- Failed tool observations are fed back to the model as structured evidence instead of being treated as success.

## Release Notes

`v1.0.0` includes:

- local LLM response parser and repair loop
- typed tool schemas and manifest echo checks
- guarded file read/search/list flows
- approval-based `apply_patch` and `run_command`
- multi-target patch support
- plan ledger support for multi-file tasks
- Persona panel boundary hardening
- local qwen3 benchmark validation at 30/30 guarded completion

## Project State

AhreumCode is usable as a local LLM coding-agent runtime, but it is still early software. The current release is focused on macOS Apple Silicon and local LM Studio-style providers.
