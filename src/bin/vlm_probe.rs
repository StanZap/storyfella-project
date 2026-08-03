use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;
use smart_visual_sequencer::{
    app::AppConfig,
    llm::{ChatMessage, ChatOptions, LmStudioClient, LmStudioConfig},
};

const ANALYSIS_PROMPT: &str = "Analyze this synthetic storyboard frame. Identify only what is visibly present. Use simple color and spatial language. Do not invent text or hidden details.";

#[derive(Debug, Parser)]
#[command(about = "Benchmark local vision-language models exposed by LM Studio")]
struct Args {
    /// Model identifiers. Repeat the flag or provide comma-separated identifiers.
    #[arg(short, long, value_delimiter = ',')]
    model: Vec<String>,

    /// Print the model identifiers visible through LM Studio and exit.
    #[arg(long)]
    list_models: bool,

    /// Override the configured OpenAI-compatible base URL.
    #[arg(long)]
    base_url: Option<String>,

    /// Directory containing the generated PNG fixtures.
    #[arg(long, default_value = "tests/fixtures/vlm")]
    fixtures: PathBuf,

    /// Directory for results.json and report.md.
    #[arg(long, default_value = "docs/poc/vlm/latest")]
    output: PathBuf,

    /// Per-request timeout, including JIT model loading.
    #[arg(long, default_value_t = 300)]
    timeout_seconds: u64,

    /// Deterministic generation seed.
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Completion budget. Reasoning models may consume tokens before emitting JSON.
    #[arg(long, default_value_t = 1400)]
    max_tokens: u32,
}

#[derive(Clone, Debug)]
struct Fixture {
    name: &'static str,
    file: &'static str,
    kind: FixtureKind,
}

#[derive(Clone, Copy, Debug)]
enum FixtureKind {
    ColorsAndShapes,
    SimpleLandscape,
    CountingGrid,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "colors-and-shapes",
        file: "colors-and-shapes.png",
        kind: FixtureKind::ColorsAndShapes,
    },
    Fixture {
        name: "simple-landscape",
        file: "simple-landscape.png",
        kind: FixtureKind::SimpleLandscape,
    },
    Fixture {
        name: "counting-grid",
        file: "counting-grid.png",
        kind: FixtureKind::CountingGrid,
    },
];

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SceneAnalysis {
    summary: String,
    objects: Vec<DetectedObject>,
    dominant_colors: Vec<String>,
    composition: String,
    confidence: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DetectedObject {
    label: String,
    color: String,
    position: String,
    count: u32,
}

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    generated_unix_seconds: u64,
    base_url: String,
    models: Vec<String>,
    fixtures: Vec<String>,
    results: Vec<ProbeResult>,
}

#[derive(Debug, Serialize)]
struct ProbeResult {
    model: String,
    fixture: String,
    duration_ms: u128,
    status: ProbeStatus,
    checks_passed: usize,
    checks_total: usize,
    checks: Vec<SemanticCheck>,
    score_percent: f32,
    analysis: Option<SceneAnalysis>,
    raw_response: Option<String>,
    error: Option<String>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SemanticCheck {
    name: &'static str,
    passed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProbeStatus {
    Passed,
    InvalidJson,
    RequestFailed,
    EmptyResponse,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let app_config = AppConfig::load().context("load application config")?;
    let base_url = args
        .base_url
        .clone()
        .unwrap_or_else(|| app_config.lm_studio.base_url.clone());

    if args.list_models {
        list_models(&app_config, &base_url, args.timeout_seconds).await?;
        return Ok(());
    }
    if args.model.is_empty() {
        bail!("provide at least one --model; use --list-models to inspect LM Studio");
    }

    validate_fixtures(&args.fixtures)?;
    let response_format = scene_response_format();
    let mut results = Vec::with_capacity(args.model.len() * FIXTURES.len());

    for model in &args.model {
        let client = client_for(
            &app_config,
            &base_url,
            model,
            Duration::from_secs(args.timeout_seconds),
        )?;
        for fixture in FIXTURES {
            println!("probing {model} with {}...", fixture.name);
            results.push(probe_fixture(&client, model, fixture, &args, &response_format).await);
        }
    }

    let report = BenchmarkReport {
        generated_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_secs(),
        base_url,
        models: args.model,
        fixtures: FIXTURES.iter().map(|item| item.name.to_owned()).collect(),
        results,
    };
    write_report(&args.output, &report)?;
    println!("wrote benchmark results to {}", args.output.display());
    Ok(())
}

async fn list_models(config: &AppConfig, base_url: &str, timeout_seconds: u64) -> Result<()> {
    let client = client_for(
        config,
        base_url,
        &config.lm_studio.model,
        Duration::from_secs(timeout_seconds),
    )?;
    let mut models = client
        .list_models()
        .await
        .context("list models from LM Studio")?
        .data;
    models.sort_by(|a, b| a.id.cmp(&b.id));
    for model in models {
        println!("{}", model.id);
    }
    Ok(())
}

fn client_for(
    config: &AppConfig,
    base_url: &str,
    model: &str,
    timeout: Duration,
) -> Result<LmStudioClient> {
    LmStudioClient::new(LmStudioConfig {
        base_url: base_url.to_owned(),
        model: model.to_owned(),
        api_key: config.lm_studio.api_key.clone(),
        timeout,
    })
    .context("construct LM Studio client")
}

async fn probe_fixture(
    client: &LmStudioClient,
    model: &str,
    fixture: &Fixture,
    args: &Args,
    response_format: &serde_json::Value,
) -> ProbeResult {
    let path = args.fixtures.join(fixture.file);
    let started = Instant::now();
    let request = (|| -> Result<Vec<ChatMessage>> {
        let image = fs::read(&path).with_context(|| format!("read fixture {}", path.display()))?;
        let data_url = format!("data:image/png;base64,{}", STANDARD.encode(image));
        Ok(vec![
            ChatMessage::text(
                "system",
                "You are a precise visual analyst. Respond using the supplied JSON schema.",
            ),
            ChatMessage::with_image("user", ANALYSIS_PROMPT, data_url),
        ])
    })();

    let messages = match request {
        Ok(messages) => messages,
        Err(error) => return failed_result(model, fixture, started, error.to_string()),
    };
    let options = ChatOptions {
        temperature: Some(0.0),
        max_tokens: Some(args.max_tokens),
        seed: Some(args.seed),
        response_format: Some(response_format.clone()),
    };

    let completion = match client.chat_with_options(&messages, &options).await {
        Ok(completion) => completion,
        Err(error) => return failed_result(model, fixture, started, error.to_string()),
    };
    let usage = completion.usage;
    let raw = completion
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content);
    let Some(raw) = raw else {
        return ProbeResult {
            model: model.to_owned(),
            fixture: fixture.name.to_owned(),
            duration_ms: started.elapsed().as_millis(),
            status: ProbeStatus::EmptyResponse,
            checks_passed: 0,
            checks_total: semantic_checks(fixture.kind, None).len(),
            checks: semantic_checks(fixture.kind, None),
            score_percent: 0.0,
            analysis: None,
            raw_response: None,
            error: Some("completion contained no message content".to_owned()),
            prompt_tokens: usage.as_ref().map(|item| item.prompt_tokens),
            completion_tokens: usage.as_ref().map(|item| item.completion_tokens),
        };
    };

    let clean = strip_json_fence(&raw);
    match serde_json::from_str::<SceneAnalysis>(clean) {
        Ok(analysis) => {
            let checks = semantic_checks(fixture.kind, Some(&analysis));
            let passed = checks.iter().filter(|check| check.passed).count();
            let total = checks.len();
            ProbeResult {
                model: model.to_owned(),
                fixture: fixture.name.to_owned(),
                duration_ms: started.elapsed().as_millis(),
                status: ProbeStatus::Passed,
                checks_passed: passed,
                checks_total: total,
                score_percent: percent(passed, total),
                checks,
                analysis: Some(analysis),
                raw_response: Some(raw),
                error: None,
                prompt_tokens: usage.as_ref().map(|item| item.prompt_tokens),
                completion_tokens: usage.as_ref().map(|item| item.completion_tokens),
            }
        }
        Err(error) => ProbeResult {
            model: model.to_owned(),
            fixture: fixture.name.to_owned(),
            duration_ms: started.elapsed().as_millis(),
            status: ProbeStatus::InvalidJson,
            checks_passed: 0,
            checks_total: semantic_checks(fixture.kind, None).len(),
            checks: semantic_checks(fixture.kind, None),
            score_percent: 0.0,
            analysis: None,
            raw_response: Some(raw),
            error: Some(error.to_string()),
            prompt_tokens: usage.as_ref().map(|item| item.prompt_tokens),
            completion_tokens: usage.as_ref().map(|item| item.completion_tokens),
        },
    }
}

fn failed_result(model: &str, fixture: &Fixture, started: Instant, error: String) -> ProbeResult {
    ProbeResult {
        model: model.to_owned(),
        fixture: fixture.name.to_owned(),
        duration_ms: started.elapsed().as_millis(),
        status: ProbeStatus::RequestFailed,
        checks_passed: 0,
        checks_total: semantic_checks(fixture.kind, None).len(),
        checks: semantic_checks(fixture.kind, None),
        score_percent: 0.0,
        analysis: None,
        raw_response: None,
        error: Some(error),
        prompt_tokens: None,
        completion_tokens: None,
    }
}

fn scene_response_format() -> serde_json::Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "storyboard_scene_analysis",
            "strict": true,
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "summary": { "type": "string" },
                    "objects": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "label": { "type": "string" },
                                "color": { "type": "string" },
                                "position": { "type": "string" },
                                "count": { "type": "integer", "minimum": 1 }
                            },
                            "required": ["label", "color", "position", "count"]
                        }
                    },
                    "dominant_colors": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "composition": { "type": "string" },
                    "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
                },
                "required": ["summary", "objects", "dominant_colors", "composition", "confidence"]
            }
        }
    })
}

fn validate_fixtures(directory: &Path) -> Result<()> {
    for fixture in FIXTURES {
        let path = directory.join(fixture.file);
        if !path.is_file() {
            bail!(
                "missing fixture {}; run `cargo run --example generate_vlm_fixtures`",
                path.display()
            );
        }
    }
    Ok(())
}

fn write_report(output: &Path, report: &BenchmarkReport) -> Result<()> {
    fs::create_dir_all(output)
        .with_context(|| format!("create output directory {}", output.display()))?;
    let json = serde_json::to_string_pretty(report).context("serialize benchmark report")?;
    fs::write(output.join("results.json"), json).context("write results.json")?;
    fs::write(output.join("report.md"), markdown_report(report)).context("write report.md")?;
    Ok(())
}

fn markdown_report(report: &BenchmarkReport) -> String {
    let mut output = format!(
        "# Visual LLM comparison\n\nGenerated at Unix time `{}` against `{}`. Scores are deterministic semantic checks over schema-valid responses, not perceptual-quality judgments.\n\n| Model | Fixture | Status | Score | Latency | Tokens |\n|---|---|---:|---:|---:|---:|\n",
        report.generated_unix_seconds, report.base_url
    );
    for result in &report.results {
        let tokens = match (result.prompt_tokens, result.completion_tokens) {
            (Some(prompt), Some(completion)) => format!("{prompt}+{completion}"),
            _ => "—".to_owned(),
        };
        output.push_str(&format!(
            "| `{}` | {} | `{:?}` | {:.0}% ({}/{}) | {} ms | {} |\n",
            result.model,
            result.fixture,
            result.status,
            result.score_percent,
            result.checks_passed,
            result.checks_total,
            result.duration_ms,
            tokens
        ));
    }

    output.push_str("\n## Failures\n\n");
    let mut failures = 0;
    for result in &report.results {
        if let Some(error) = &result.error {
            failures += 1;
            output.push_str(&format!(
                "- `{}` / `{}`: {}\n",
                result.model,
                result.fixture,
                error.replace('\n', " ")
            ));
        }
    }
    if failures == 0 {
        output.push_str("None.\n");
    }
    output
}

fn semantic_checks(kind: FixtureKind, analysis: Option<&SceneAnalysis>) -> Vec<SemanticCheck> {
    let Some(analysis) = analysis else {
        return check_names(kind)
            .into_iter()
            .map(|name| SemanticCheck {
                name,
                passed: false,
            })
            .collect();
    };
    let text = serde_json::to_string(analysis)
        .unwrap_or_default()
        .to_lowercase();

    match kind {
        FixtureKind::ColorsAndShapes => vec![
            SemanticCheck {
                name: "red circle",
                passed: object_matches(analysis, "circle", "red", Some("left")),
            },
            SemanticCheck {
                name: "blue square",
                passed: object_matches_any_label(
                    analysis,
                    &["square", "rectangle"],
                    "blue",
                    Some("right"),
                ),
            },
            SemanticCheck {
                name: "one circle",
                passed: object_count(analysis, &["circle"]) == 1,
            },
            SemanticCheck {
                name: "one square",
                passed: object_count(analysis, &["square", "rectangle"]) == 1,
            },
        ],
        FixtureKind::SimpleLandscape => vec![
            SemanticCheck {
                name: "house",
                passed: text.contains("house"),
            },
            SemanticCheck {
                name: "sun",
                passed: text.contains("sun"),
            },
            SemanticCheck {
                name: "blue sky",
                passed: text.contains("sky") && text.contains("blue"),
            },
            SemanticCheck {
                name: "green ground",
                passed: text.contains("green")
                    && ["ground", "grass", "field"]
                        .iter()
                        .any(|term| text.contains(term)),
            },
        ],
        FixtureKind::CountingGrid => vec![
            SemanticCheck {
                name: "three circles",
                passed: object_count(analysis, &["circle"]) == 3,
            },
            SemanticCheck {
                name: "two squares",
                passed: object_count(analysis, &["square", "rectangle"]) == 2,
            },
            SemanticCheck {
                name: "red green blue circles",
                passed: ["red", "green", "blue"]
                    .iter()
                    .all(|color| object_matches_any_label(analysis, &["circle"], color, None)),
            },
            SemanticCheck {
                name: "black squares",
                passed: analysis.objects.iter().any(|object| {
                    ["square", "rectangle"]
                        .iter()
                        .any(|label| object.label.to_lowercase().contains(label))
                        && object.color.to_lowercase().contains("black")
                }),
            },
        ],
    }
}

fn check_names(kind: FixtureKind) -> Vec<&'static str> {
    match kind {
        FixtureKind::ColorsAndShapes => {
            vec!["red circle", "blue square", "one circle", "one square"]
        }
        FixtureKind::SimpleLandscape => vec!["house", "sun", "blue sky", "green ground"],
        FixtureKind::CountingGrid => vec![
            "three circles",
            "two squares",
            "red green blue circles",
            "black squares",
        ],
    }
}

fn object_matches(
    analysis: &SceneAnalysis,
    label: &str,
    color: &str,
    position: Option<&str>,
) -> bool {
    object_matches_any_label(analysis, &[label], color, position)
}

fn object_matches_any_label(
    analysis: &SceneAnalysis,
    labels: &[&str],
    color: &str,
    position: Option<&str>,
) -> bool {
    analysis.objects.iter().any(|object| {
        let object_label = object.label.to_lowercase();
        let object_color = object.color.to_lowercase();
        let object_position = object.position.to_lowercase();
        labels.iter().any(|label| object_label.contains(label))
            && object_color.contains(color)
            && position.is_none_or(|expected| object_position.contains(expected))
    })
}

fn object_count(analysis: &SceneAnalysis, labels: &[&str]) -> u32 {
    analysis
        .objects
        .iter()
        .filter(|object| {
            let label = object.label.to_lowercase();
            labels.iter().any(|expected| label.contains(expected))
        })
        .map(|object| object.count)
        .sum()
}

fn strip_json_fence(value: &str) -> &str {
    let trimmed = value.trim();
    let without_start = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    without_start
        .strip_suffix("```")
        .unwrap_or(without_start)
        .trim()
}

fn percent(found: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        found as f32 / total as f32 * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_common_json_fences() {
        assert_eq!(
            strip_json_fence("```json\n{\"ok\":true}\n```"),
            "{\"ok\":true}"
        );
        assert_eq!(strip_json_fence(" {\"ok\":true} "), "{\"ok\":true}");
    }

    #[test]
    fn schema_contains_strict_object_contracts() {
        let schema = scene_response_format();
        assert_eq!(
            schema["json_schema"]["schema"]["additionalProperties"],
            false
        );
        assert_eq!(
            schema["json_schema"]["schema"]["properties"]["objects"]["items"]
                ["additionalProperties"],
            false
        );
    }
}
