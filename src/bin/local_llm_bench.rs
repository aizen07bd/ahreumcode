#[allow(dead_code, unused_imports)]
#[path = "../config/mod.rs"]
mod config;
#[allow(dead_code, unused_imports)]
#[path = "../llm/mod.rs"]
mod llm;
#[allow(dead_code, unused_imports)]
#[path = "../tool/mod.rs"]
mod tool;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use config::RuntimeConfig;
use llm::{
    parse_runtime_response, DecisionGate, LlmChatRequest, LlmChatStatus, LlmMessage,
    LlmMessageRole, LlmMessageVisibility, LlmProviderFactory, RepairLoop, RuntimeDecision,
    SchemaPromptBuilder,
};
use tool::{
    apply_approved_change, capture_change_precondition, ApprovedChange, ObservationStatus,
    PermissionDecision, PermissionGate, ToolCall, ToolObservation, ToolRuntime,
};

const MAX_BENCH_STEPS: usize = 4;

#[derive(Default)]
struct BenchOptions {
    limit: Option<usize>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("benchmark failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let project_root = env::current_dir().map_err(|error| error.to_string())?;
    let mut config = RuntimeConfig::load(&project_root)
        .map_err(|error| error.to_string())?
        .config;
    let options = apply_overrides(&mut config, env::args().skip(1))?;

    let run_id = run_id();
    let bench_dir = PathBuf::from(".ahreumcode").join("bench").join(&run_id);
    fs::create_dir_all(project_root.join(&bench_dir)).map_err(|error| error.to_string())?;
    seed_workspace(&project_root, &bench_dir)?;

    let mut scenarios = scenarios(&bench_dir);
    if let Some(limit) = options.limit {
        scenarios.truncate(limit);
    }
    let schema = SchemaPromptBuilder::build()
        .map_err(|error| format!("schema prompt build failed: {}", error.missing_rule))?;
    let provider = LlmProviderFactory::from_config(&config);
    let send_chat = |messages: Vec<LlmMessage>| {
        let report = provider.send_chat(LlmChatRequest { messages });
        match report.status {
            LlmChatStatus::Succeeded { answer } => Ok(answer),
            LlmChatStatus::Failed(error) => Err(format!(
                "chat failed: {} {}",
                error.kind.as_str(),
                error.message
            )),
        }
    };
    let runtime = ToolRuntime::new(
        project_root.clone(),
        project_root
            .join(".ahreumcode")
            .join("logs")
            .join("bench")
            .join(&run_id),
    );

    let mut results = Vec::new();
    for (index, scenario) in scenarios.iter().enumerate() {
        let result = run_scenario(
            index + 1,
            scenario,
            &schema.content,
            &send_chat,
            &runtime,
            &config,
            &project_root,
        );
        print_result(scenario, &result);
        results.push(result);
    }

    let report = render_report(&config, &run_id, &bench_dir, &scenarios, &results);
    let report_path = project_root.join(&bench_dir).join("report.md");
    fs::write(&report_path, report).map_err(|error| error.to_string())?;
    println!();
    println!("report: {}", report_path.display());
    Ok(())
}

fn apply_overrides(
    config: &mut RuntimeConfig,
    mut args: impl Iterator<Item = String>,
) -> Result<BenchOptions, String> {
    let mut options = BenchOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => {
                config.provider.model = args
                    .next()
                    .ok_or_else(|| "--model requires a value".to_owned())?;
            }
            "--base-url" => {
                config.provider.base_url = args
                    .next()
                    .ok_or_else(|| "--base-url requires a value".to_owned())?;
            }
            "--limit" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--limit requires a value".to_owned())?;
                options.limit = Some(
                    value
                        .parse::<usize>()
                        .map_err(|error| format!("invalid --limit value: {error}"))?,
                );
            }
            "--help" | "-h" => {
                println!(
                    "usage: cargo run --bin local_llm_bench -- [--model MODEL] [--base-url URL] [--limit N]"
                );
                std::process::exit(0);
            }
            value => return Err(format!("unknown argument: {value}")),
        }
    }
    Ok(options)
}

fn run_id() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("bench-{seconds}")
}

fn seed_workspace(project_root: &Path, bench_dir: &Path) -> Result<(), String> {
    let absolute = project_root.join(bench_dir);
    fs::create_dir_all(absolute.join("out")).map_err(|error| error.to_string())?;
    write_seed(
        &absolute.join("seed.md"),
        "title: AhreumCode benchmark\nkeyword: BENCHMARK_KEYWORD\nstatus: old\n",
    )?;
    write_seed(
        &absolute.join("requirements.md"),
        "owner: OWNER_ALPHA\nphase: draft\nneed: reusable local LLM benchmark\n",
    )?;
    write_seed(
        &absolute.join("metrics.txt"),
        "metric=raw_tool_call\nthreshold=THRESHOLD_42\nscore=12\n",
    )?;
    write_seed(
        &absolute.join("config-sample.toml"),
        "name = \"bench-config\"\nport = 45123\nenabled = false\n",
    )?;
    write_seed(
        &absolute.join("changelog.md"),
        "# Change Log\nmarker: CHANGELOG_MARKER\nstate: pending\n",
    )
}

fn write_seed(path: &Path, body: &str) -> Result<(), String> {
    fs::write(path, body).map_err(|error| error.to_string())
}

#[derive(Clone, Copy)]
enum CompletionCheck {
    ToolSucceeded,
    PreviewContains(&'static str),
    FileContains(&'static str),
}

struct Scenario {
    id: &'static str,
    prompt: String,
    expected_tools: &'static [&'static str],
    completion: CompletionCheck,
}

fn scenarios(bench_dir: &Path) -> Vec<Scenario> {
    let seed = bench_dir.join("seed.md").display().to_string();
    let requirements = bench_dir.join("requirements.md").display().to_string();
    let metrics = bench_dir.join("metrics.txt").display().to_string();
    let config_sample = bench_dir.join("config-sample.toml").display().to_string();
    let changelog = bench_dir.join("changelog.md").display().to_string();
    let created_plan = bench_dir.join("plan-created.md").display().to_string();
    let created_result = bench_dir.join("result-created.txt").display().to_string();
    let created_log = bench_dir
        .join("out")
        .join("generated.log")
        .display()
        .to_string();
    let created_json = bench_dir.join("created-data.json").display().to_string();
    let created_toml = bench_dir
        .join("created-settings.toml")
        .display()
        .to_string();
    vec![
        Scenario {
            id: "read_seed_keyword",
            prompt: format!("{seed} 파일을 read_file로 읽고 keyword 값을 확인해줘."),
            expected_tools: &["read_file"],
            completion: CompletionCheck::PreviewContains("BENCHMARK_KEYWORD"),
        },
        Scenario {
            id: "read_requirements_owner",
            prompt: format!("{requirements} 파일을 read_file로 열고 owner 값을 확인해줘."),
            expected_tools: &["read_file"],
            completion: CompletionCheck::PreviewContains("OWNER_ALPHA"),
        },
        Scenario {
            id: "read_metrics_threshold",
            prompt: format!("{metrics} 파일을 read_file로 읽고 threshold 값을 알려줘."),
            expected_tools: &["read_file"],
            completion: CompletionCheck::PreviewContains("THRESHOLD_42"),
        },
        Scenario {
            id: "read_config_sample_port",
            prompt: format!("{config_sample} 파일에서 port 값을 read_file로 확인해줘."),
            expected_tools: &["read_file"],
            completion: CompletionCheck::PreviewContains("45123"),
        },
        Scenario {
            id: "read_changelog_marker",
            prompt: format!("{changelog} 파일을 읽고 marker 항목을 확인해줘."),
            expected_tools: &["read_file"],
            completion: CompletionCheck::PreviewContains("CHANGELOG_MARKER"),
        },
        Scenario {
            id: "search_text_symbol",
            prompt: "src 디렉터리에서 parse_runtime_response 문자열을 search_text로 찾아줘."
                .to_owned(),
            expected_tools: &["search_text"],
            completion: CompletionCheck::PreviewContains("parse_runtime_response"),
        },
        Scenario {
            id: "search_runtime_decision",
            prompt: "src 디렉터리에서 RuntimeDecision 심볼을 search_text로 찾아줘.".to_owned(),
            expected_tools: &["search_text"],
            completion: CompletionCheck::PreviewContains("RuntimeDecision"),
        },
        Scenario {
            id: "search_permission_gate",
            prompt: "src 디렉터리에서 PermissionGate 문자열을 search_text로 검색해줘.".to_owned(),
            expected_tools: &["search_text"],
            completion: CompletionCheck::PreviewContains("PermissionGate"),
        },
        Scenario {
            id: "search_tool_observation",
            prompt: "src 디렉터리에서 ToolObservation 정의나 사용 위치를 search_text로 찾아줘."
                .to_owned(),
            expected_tools: &["search_text"],
            completion: CompletionCheck::PreviewContains("ToolObservation"),
        },
        Scenario {
            id: "search_persona_speaker",
            prompt: "src 디렉터리에서 PersonaSpeaker 문자열을 search_text로 찾아줘.".to_owned(),
            expected_tools: &["search_text"],
            completion: CompletionCheck::PreviewContains("PersonaSpeaker"),
        },
        Scenario {
            id: "list_files_root",
            prompt: "워크스페이스 최상위 파일 목록을 list_files로 확인해줘.".to_owned(),
            expected_tools: &["list_files"],
            completion: CompletionCheck::PreviewContains("Cargo.toml"),
        },
        Scenario {
            id: "list_files_src",
            prompt: "src 디렉터리의 파일 목록을 list_files로 확인해줘.".to_owned(),
            expected_tools: &["list_files"],
            completion: CompletionCheck::PreviewContains("main.rs"),
        },
        Scenario {
            id: "list_files_tool",
            prompt: "src/tool 디렉터리의 파일 목록을 list_files로 확인해줘.".to_owned(),
            expected_tools: &["list_files"],
            completion: CompletionCheck::PreviewContains("runtime.rs"),
        },
        Scenario {
            id: "list_files_docs",
            prompt: "docs 디렉터리의 파일 목록을 list_files로 확인해줘.".to_owned(),
            expected_tools: &["list_files"],
            completion: CompletionCheck::PreviewContains("tasks"),
        },
        Scenario {
            id: "list_files_bench_dir",
            prompt: format!(
                "{bench} 디렉터리의 파일 목록을 list_files로 확인해줘.",
                bench = bench_dir.display()
            ),
            expected_tools: &["list_files"],
            completion: CompletionCheck::PreviewContains("seed.md"),
        },
        Scenario {
            id: "apply_patch_create_plan",
            prompt: format!("{created_plan} 파일을 만들고 내용은 PLAN_CREATED_ALPHA 한 줄로 해줘."),
            expected_tools: &["apply_patch"],
            completion: CompletionCheck::FileContains("PLAN_CREATED_ALPHA"),
        },
        Scenario {
            id: "apply_patch_create_result",
            prompt: format!(
                "{created_result} 파일을 만들고 내용은 RESULT_CREATED_BETA 한 줄로 해줘."
            ),
            expected_tools: &["apply_patch"],
            completion: CompletionCheck::FileContains("RESULT_CREATED_BETA"),
        },
        Scenario {
            id: "apply_patch_create_log",
            prompt: format!(
                "{created_log} 파일을 만들고 내용은 NESTED_CREATED_GAMMA 한 줄로 해줘."
            ),
            expected_tools: &["apply_patch"],
            completion: CompletionCheck::FileContains("NESTED_CREATED_GAMMA"),
        },
        Scenario {
            id: "apply_patch_create_json",
            prompt: format!(
                "{created_json} 파일을 만들고 JSON에 marker 값을 CREATED_JSON_DELTA로 넣어줘."
            ),
            expected_tools: &["apply_patch"],
            completion: CompletionCheck::FileContains("CREATED_JSON_DELTA"),
        },
        Scenario {
            id: "apply_patch_create_toml",
            prompt: format!(
                "{created_toml} 파일을 만들고 marker = \"CREATED_TOML_EPSILON\" 값을 넣어줘."
            ),
            expected_tools: &["apply_patch"],
            completion: CompletionCheck::FileContains("CREATED_TOML_EPSILON"),
        },
        Scenario {
            id: "apply_patch_update_seed",
            prompt: format!("{seed} 파일을 읽고 status: old를 status: new로 수정해줘."),
            expected_tools: &["read_file", "apply_patch"],
            completion: CompletionCheck::FileContains("status: new"),
        },
        Scenario {
            id: "apply_patch_update_requirements",
            prompt: format!(
                "{requirements} 파일을 읽고 phase: draft를 phase: verified로 수정해줘."
            ),
            expected_tools: &["read_file", "apply_patch"],
            completion: CompletionCheck::FileContains("phase: verified"),
        },
        Scenario {
            id: "apply_patch_update_metrics",
            prompt: format!("{metrics} 파일의 score=12를 score=99로 수정해줘."),
            expected_tools: &["read_file", "apply_patch"],
            completion: CompletionCheck::FileContains("score=99"),
        },
        Scenario {
            id: "apply_patch_update_config",
            prompt: format!(
                "{config_sample} 파일을 읽고 enabled = false를 enabled = true로 바꿔줘."
            ),
            expected_tools: &["read_file", "apply_patch"],
            completion: CompletionCheck::FileContains("enabled = true"),
        },
        Scenario {
            id: "apply_patch_update_changelog",
            prompt: format!("{changelog} 파일에서 state: pending을 state: shipped로 수정해줘."),
            expected_tools: &["read_file", "apply_patch"],
            completion: CompletionCheck::FileContains("state: shipped"),
        },
        Scenario {
            id: "run_command_pwd",
            prompt: "run_command로 pwd를 실행해서 현재 작업 경로를 확인해줘.".to_owned(),
            expected_tools: &["run_command"],
            completion: CompletionCheck::ToolSucceeded,
        },
        Scenario {
            id: "run_command_cargo_version",
            prompt: "run_command로 cargo --version을 실행해줘.".to_owned(),
            expected_tools: &["run_command"],
            completion: CompletionCheck::PreviewContains("cargo"),
        },
        Scenario {
            id: "run_command_git_status_short",
            prompt: "run_command로 git status --short를 실행해줘.".to_owned(),
            expected_tools: &["run_command"],
            completion: CompletionCheck::ToolSucceeded,
        },
        Scenario {
            id: "run_command_git_diff_stat",
            prompt: "run_command로 git diff --stat을 실행해줘.".to_owned(),
            expected_tools: &["run_command"],
            completion: CompletionCheck::ToolSucceeded,
        },
        Scenario {
            id: "run_command_cargo_fmt_check",
            prompt: "run_command로 cargo fmt --check를 실행해줘.".to_owned(),
            expected_tools: &["run_command"],
            completion: CompletionCheck::ToolSucceeded,
        },
    ]
}

#[derive(Clone)]
struct BenchResult {
    raw_tool_success: bool,
    repaired_tool_success: bool,
    guarded_completion: bool,
    first_tool: Option<String>,
    final_tool: Option<String>,
    repair_attempts: u16,
    steps: usize,
    failure: Option<String>,
}

fn run_scenario(
    number: usize,
    scenario: &Scenario,
    schema_content: &str,
    send_chat: &impl Fn(Vec<LlmMessage>) -> Result<String, String>,
    runtime: &ToolRuntime,
    config: &RuntimeConfig,
    project_root: &Path,
) -> BenchResult {
    let mut messages = vec![
        message(
            &format!("bench-turn-{number:04}-00"),
            LlmMessageRole::System,
            schema_content,
        ),
        message(
            &format!("bench-turn-{number:04}-00"),
            LlmMessageRole::User,
            &scenario.prompt,
        ),
    ];
    let mut raw_tool_success = false;
    let mut repaired_tool_success = false;
    let mut first_tool = None;
    let mut final_tool = None;
    let mut total_repairs = 0;
    let mut last_failure = None;

    for step in 1..=MAX_BENCH_STEPS {
        let turn_id = format!("bench-turn-{number:04}-{step:02}");
        let raw = match send_chat(messages.clone()) {
            Ok(raw) => raw,
            Err(error) => return failed(error),
        };

        let raw_decision = parse_and_classify(&raw);
        if step == 1 {
            raw_tool_success = raw_decision
                .as_ref()
                .ok()
                .and_then(tool_name)
                .is_some_and(|tool| scenario.expected_tools.contains(&tool));
        }

        let (decision, repair_attempts) = match raw_decision {
            Ok(decision) => (decision, 0),
            Err(error) => match repair_parse_error(
                send_chat,
                &turn_id,
                schema_content,
                &scenario.prompt,
                &raw,
                &error,
            ) {
                Ok((decision, attempts)) => (decision, attempts),
                Err(error) => {
                    return BenchResult {
                        raw_tool_success,
                        repaired_tool_success,
                        guarded_completion: false,
                        first_tool,
                        final_tool,
                        repair_attempts: total_repairs,
                        steps: step,
                        failure: Some(error),
                    }
                }
            },
        };
        total_repairs += repair_attempts;

        if first_tool.is_none() {
            first_tool = tool_name(&decision).map(str::to_owned);
        }
        if let Some(tool) = tool_name(&decision) {
            final_tool = Some(tool.to_owned());
            if scenario.expected_tools.contains(&tool) {
                repaired_tool_success = true;
            }
        }

        match execute_and_check(
            &decision,
            scenario.completion,
            runtime,
            config,
            project_root,
        ) {
            Ok(ExecutionCheck {
                observation: _,
                completed: true,
            }) => {
                return BenchResult {
                    raw_tool_success,
                    repaired_tool_success,
                    guarded_completion: true,
                    first_tool,
                    final_tool,
                    repair_attempts: total_repairs,
                    steps: step,
                    failure: None,
                };
            }
            Ok(ExecutionCheck {
                observation,
                completed: false,
            }) => {
                last_failure = Some(format!(
                    "step {step} did not satisfy completion check: {}",
                    observation.summary()
                ));
                messages.push(message(
                    &turn_id,
                    LlmMessageRole::System,
                    &observation.history_message(),
                ));
                messages.push(message(
                    &turn_id,
                    LlmMessageRole::System,
                    "Continue from the latest AHREUM_TOOL_OBSERVATION. If the user goal is not complete, request exactly one different next tool candidate. Do not repeat a spent tool candidate. If the goal is complete, return exactly one answer response.",
                ));
            }
            Err(error) => {
                last_failure = Some(format!("step {step} execution failed: {error}"));
                break;
            }
        }
    }

    BenchResult {
        raw_tool_success,
        repaired_tool_success,
        guarded_completion: false,
        first_tool,
        final_tool,
        repair_attempts: total_repairs,
        steps: MAX_BENCH_STEPS,
        failure: last_failure,
    }
}

fn failed(error: String) -> BenchResult {
    BenchResult {
        raw_tool_success: false,
        repaired_tool_success: false,
        guarded_completion: false,
        first_tool: None,
        final_tool: None,
        repair_attempts: 0,
        steps: 0,
        failure: Some(error),
    }
}

fn message(turn_id: &str, role: LlmMessageRole, content: &str) -> LlmMessage {
    LlmMessage {
        turn_id: turn_id.to_owned(),
        role,
        visibility: LlmMessageVisibility::Internal,
        content: content.to_owned(),
    }
}

fn parse_and_classify(raw: &str) -> Result<RuntimeDecision, String> {
    let parsed = parse_runtime_response(raw)
        .map_err(|error| format!("{}:{}", error.kind.as_str(), error.message))?;
    DecisionGate::classify(&parsed)
        .map_err(|error| format!("{}:{}", error.kind.as_str(), error.message))
}

fn repair_parse_error(
    send_chat: &impl Fn(Vec<LlmMessage>) -> Result<String, String>,
    turn_id: &str,
    schema_content: &str,
    user_prompt: &str,
    raw: &str,
    parse_error: &str,
) -> Result<(RuntimeDecision, u16), String> {
    let repair_loop = RepairLoop::default_local();
    let mut attempts = 0;
    let mut last_raw = raw.to_owned();
    let mut last_error = parse_error.to_owned();
    while attempts < repair_loop.max_attempts() {
        let synthetic = llm::RuntimeResponseParseError {
            kind: llm::RuntimeResponseParseErrorKind::SchemaValidationFailed,
            message: last_error.clone(),
        };
        let repair = repair_loop
            .next_request_with_raw(attempts, &synthetic, Some(&last_raw))
            .map_err(|limit| format!("repair limit: {}", limit.reason.as_str()))?;
        let messages = vec![
            message(turn_id, LlmMessageRole::System, schema_content),
            message(turn_id, LlmMessageRole::User, user_prompt),
            message(turn_id, LlmMessageRole::System, &repair.prompt),
        ];
        let repaired_raw = send_chat(messages)?;
        attempts = repair.attempt;
        match parse_and_classify(&repaired_raw) {
            Ok(decision) => return Ok((decision, attempts)),
            Err(error) => {
                last_raw = repaired_raw;
                last_error = error;
            }
        }
    }

    Err(format!(
        "repair failed after {attempts} attempts: {last_error}"
    ))
}

fn tool_name(decision: &RuntimeDecision) -> Option<&str> {
    match decision {
        RuntimeDecision::ToolCandidatePending { tool_name, .. }
        | RuntimeDecision::ApprovalNeeded { tool_name, .. } => Some(tool_name.as_str()),
        _ => None,
    }
}

struct ExecutionCheck {
    observation: ToolObservation,
    completed: bool,
}

fn execute_and_check(
    decision: &RuntimeDecision,
    check: CompletionCheck,
    runtime: &ToolRuntime,
    config: &RuntimeConfig,
    project_root: &Path,
) -> Result<ExecutionCheck, String> {
    let observation = match decision {
        RuntimeDecision::ToolCandidatePending {
            activity,
            tool_name,
            arguments,
            ..
        } => runtime.execute(ToolCall::new(
            "bench-run".to_owned(),
            "bench-turn".to_owned(),
            *activity,
            tool_name.clone(),
            arguments.clone(),
        )),
        RuntimeDecision::ApprovalNeeded {
            tool_name,
            arguments,
            change_preview,
            ..
        } if tool_name == "apply_patch" => {
            let preview = change_preview
                .clone()
                .ok_or_else(|| "apply_patch candidate has no change preview".to_owned())?;
            let precondition = capture_change_precondition(project_root, &preview)
                .map_err(|observation| observation.summary())?;
            apply_approved_change(
                project_root,
                ApprovedChange {
                    preview,
                    precondition,
                },
            )
        }
        RuntimeDecision::ApprovalNeeded {
            tool_name,
            arguments,
            ..
        } if tool_name == "run_command" => match PermissionGate::evaluate(config, decision) {
            PermissionDecision::Ask(_) => runtime.execute_approved_command(
                "bench-run",
                "bench-turn",
                arguments.clone(),
                config.limits.command_timeout_ms,
            ),
            PermissionDecision::Deny(denial) => {
                return Err(format!("permission denied: {}", denial.reason));
            }
            PermissionDecision::Allow => {
                return Err("run_command unexpectedly allowed without approval".to_owned());
            }
        },
        _ => return Err("decision is not executable for benchmark".to_owned()),
    };

    let completed = check_observation(&observation, check, project_root);
    Ok(ExecutionCheck {
        observation,
        completed,
    })
}

fn check_observation(
    observation: &ToolObservation,
    check: CompletionCheck,
    project_root: &Path,
) -> bool {
    match check {
        CompletionCheck::ToolSucceeded => observation.status == ObservationStatus::Succeeded,
        CompletionCheck::PreviewContains(needle) => {
            observation.status == ObservationStatus::Succeeded
                && observation.preview.iter().any(|line| line.contains(needle))
        }
        CompletionCheck::FileContains(needle) => {
            let Some(target) = observation.target_raw.as_deref() else {
                return false;
            };
            observation.status == ObservationStatus::Succeeded
                && fs::read_to_string(project_root.join(target))
                    .map(|content| content.contains(needle))
                    .unwrap_or(false)
        }
    }
}

fn print_result(scenario: &Scenario, result: &BenchResult) {
    println!(
        "{:<32} raw={} repaired={} guarded={} tool={} repairs={} steps={}{}",
        scenario.id,
        yn(result.raw_tool_success),
        yn(result.repaired_tool_success),
        yn(result.guarded_completion),
        result.final_tool.as_deref().unwrap_or("-"),
        result.repair_attempts,
        result.steps,
        result
            .failure
            .as_ref()
            .map(|failure| format!(" failure={failure}"))
            .unwrap_or_default()
    );
}

fn render_report(
    config: &RuntimeConfig,
    run_id: &str,
    bench_dir: &Path,
    scenarios: &[Scenario],
    results: &[BenchResult],
) -> String {
    let total = results.len();
    let raw = count(results, |result| result.raw_tool_success);
    let repaired = count(results, |result| result.repaired_tool_success);
    let guarded = count(results, |result| result.guarded_completion);
    let mut lines = vec![
        format!("# Local LLM Tool Calling Benchmark - {run_id}"),
        String::new(),
        format!("- model: {}", config.provider.model),
        format!("- base_url: {}", config.provider.base_url),
        format!("- sample_count: {total}"),
        format!("- workspace: {}", bench_dir.display()),
        String::new(),
        "| metric | success | rate |".to_owned(),
        "| --- | ---: | ---: |".to_owned(),
        format!(
            "| Raw Tool Call Success | {raw}/{total} | {} |",
            pct(raw, total)
        ),
        format!(
            "| Repaired Tool Call Success | {repaired}/{total} | {} |",
            pct(repaired, total)
        ),
        format!(
            "| Guarded Task Completion | {guarded}/{total} | {} |",
            pct(guarded, total)
        ),
        String::new(),
        "| scenario | expected | raw | repaired | guarded | first_tool | final_tool | repairs | steps | failure |"
            .to_owned(),
        "| --- | --- | --- | --- | --- | --- | --- | ---: | ---: | --- |".to_owned(),
    ];
    for (scenario, result) in scenarios.iter().zip(results) {
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            scenario.id,
            scenario.expected_tools.join(","),
            yn(result.raw_tool_success),
            yn(result.repaired_tool_success),
            yn(result.guarded_completion),
            result.first_tool.as_deref().unwrap_or("-"),
            result.final_tool.as_deref().unwrap_or("-"),
            result.repair_attempts,
            result.steps,
            result.failure.as_deref().unwrap_or("-").replace('|', "\\|")
        ));
    }
    lines.push(String::new());
    lines.push("Definitions:".to_owned());
    lines.push("- Raw Tool Call Success: first model response parsed and classified into an expected tool candidate.".to_owned());
    lines.push("- Repaired Tool Call Success: raw success or repair loop recovered an expected executable tool candidate.".to_owned());
    lines.push("- Guarded Task Completion: the recovered candidate passed runtime execution and scenario-specific evidence check.".to_owned());
    lines.join("\n")
}

fn count(results: &[BenchResult], predicate: impl Fn(&BenchResult) -> bool) -> usize {
    results.iter().filter(|result| predicate(result)).count()
}

fn pct(value: usize, total: usize) -> String {
    if total == 0 {
        return "0.0%".to_owned();
    }
    format!("{:.1}%", (value as f64 / total as f64) * 100.0)
}

fn yn(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}
