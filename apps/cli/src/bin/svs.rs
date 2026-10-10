//! `svs` — the Smart Visual Sequencer operation CLI.
//!
//! Drives the artifact registry + pipeline API without the Dioxus UI:
//! typed operations (`svs op …`), serialized operation stacks (`svs stack
//! run` — the VLLM contract test bed), LLM stack proposals (`svs stack
//! propose`), and the operation log. `--out <dir>` drops every intermediate
//! (image, text) into a folder for the manual golden-run tier
//! (`docs/ROADMAP.md` §7).
//!
//! The project is a SQLite database (`.svs-project.db` by default, see
//! `src/persistence/` and `docs/ROADMAP.md` §10).

use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use clipforge_inference::chat::ChatImage;
use clipforge_inference::control::{self, Managed, ServerState, StopOutcome};
use clipforge_inference::sd::LoraSelection;
use clipforge_inference::servers::kill_servers;
use uuid::Uuid;

use storyfella_studio_core::config::AppConfig;
use storyfella_studio_core::errors::{CodedError, ErrorCode, ReportedError};
use storyfella_studio_core::persistence::ProjectDb;
use storyfella_studio_core::registry::backend::CreativeBackend;
use storyfella_studio_core::registry::ops::{
    self, ComposeLayer, ExecuteOptions, OpOrigin, OpStatus, Operation, OperationStack,
};
use storyfella_studio_core::registry::pipeline::{
    ApprovalPolicy, BlockedOn, Decision, GenerationBackend, GenerationOverrides, InputPurpose,
    OutputKind, PipelineRun, RunOptions,
};
use storyfella_studio_core::registry::{
    ArtifactKind, ArtifactRegistry, GenerationDefaults, Ref, VariantAxis,
};
use storyfella_studio_core::vision;

#[path = "../story.rs"]
mod story;

#[derive(Parser)]
#[command(
    name = "svs",
    version,
    about = "Drive Smart Visual Sequencer operations from the command line"
)]
struct Cli {
    /// SQLite project database (created on first use).
    #[arg(long, global = true, default_value = ".svs-project.db")]
    project: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check or compile a Storyfella source file without opening a project or models.
    Story {
        #[command(subcommand)]
        story: story::StoryCommand,
    },
    /// Open (load or create) the project database.
    Project { path: PathBuf },
    /// Apply a typed operation.
    Op {
        #[command(subcommand)]
        op: OpCommand,
    },
    /// Execute a serialized operation stack (the VLLM contract test bed).
    Stack {
        #[command(subcommand)]
        stack: StackCommand,
    },
    /// Manage the resident generation server.
    Runtime {
        #[command(subcommand)]
        runtime: RuntimeCommand,
    },
    /// Ask the local LLM to judge images — the
    /// automated reviewer's eyes for evaluating generations, edits, and
    /// regressions. Attach images, ask questions, and optionally gate an
    /// exit code on the answer.
    Eval {
        #[command(flatten)]
        eval: EvalArgs,
    },
    /// Show the operation log (all entries, or one artifact's).
    Log {
        /// Filter to one artifact's entries.
        target: Option<Ref>,
    },
}

/// The image-evaluation workflow against the local vision model.
#[derive(Args)]
struct EvalArgs {
    /// Image paths to attach (repeatable; the model sees them in order and
    /// refers to them as image 1, image 2, …).
    #[arg(long = "image", short = 'i', required = true)]
    images: Vec<PathBuf>,
    /// Question(s) to ask (repeatable — one round per question).
    #[arg(
        long = "prompt",
        short = 'p',
        default_value = "Describe these images concisely, then note any visual defects, artifacts, or inconsistencies."
    )]
    prompts: Vec<String>,
    /// The LLM model id (defaults to the configured planner model,
    /// which must have vision support).
    #[arg(long = "model")]
    model: Option<String>,
    #[arg(long = "max-tokens", default_value_t = 2048)]
    max_tokens: u32,
    /// Check mode: exit 1 unless the answer contains this text
    /// (case-insensitive substring).
    #[arg(long = "expect")]
    expect: Option<String>,
    /// Check mode: exit 1 when the answer contains this text
    /// (case-insensitive substring).
    #[arg(long = "expect-not")]
    expect_not: Option<String>,
}

#[derive(Subcommand)]
enum OpCommand {
    /// /create <kind> <description> — new artifact (visual kinds generate
    /// their first image as part of the create).
    Create {
        kind: String,
        description: String,
        #[arg(long)]
        name: Option<String>,
        /// Default generation size for this artifact's images (kind
        /// defaults apply when omitted).
        #[arg(long, value_parser = parse_size)]
        size: Option<(u32, u32)>,
        /// Adapter to apply to this generation: `<file>@<multiplier>`
        /// (repeatable; files resolve against the configured lora dir).
        #[arg(long = "lora", value_parser = parse_lora)]
        loras: Vec<LoraSelection>,
    },
    /// /variant <ref> <description> [axis] — new visual variant (its first
    /// image is an edit of the base).
    Variant {
        target: Ref,
        description: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        axis: Option<String>,
        #[arg(long = "lora", value_parser = parse_lora)]
        loras: Vec<LoraSelection>,
    },
    /// /regenerate <ref> [prompt] — new revision of the active image.
    Regenerate {
        target: Ref,
        prompt: Option<String>,
        #[arg(long)]
        seed: Option<u64>,
        #[arg(long)]
        steps: Option<u32>,
        #[arg(long, value_parser = parse_size)]
        size: Option<(u32, u32)>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long = "lora", value_parser = parse_lora)]
        loras: Vec<LoraSelection>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "interactive")]
        approve: ApprovalArg,
    },
    /// /resize <ref> <WxH> — set the artifact's default size and
    /// regenerate its image at it.
    Resize {
        target: Ref,
        #[arg(value_parser = parse_size)]
        size: (u32, u32),
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "interactive")]
        approve: ApprovalArg,
    },
    /// /compose <scene> <description> [layer refs…] — new beat in a scene.
    Compose {
        scene: Ref,
        description: String,
        #[arg(long = "background")]
        background: Option<Ref>,
        #[arg(long = "layer")]
        layers: Vec<Ref>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "interactive")]
        approve: ApprovalArg,
    },
    /// /draft <ref> <request> — LLM proposes story text; user approves.
    Draft {
        target: Ref,
        request: String,
        /// Manual text; bypasses the LLM and the approval checkpoint.
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "interactive")]
        approve: ApprovalArg,
    },
    /// /modify <ref> <description> — edit the image from a plain-language
    /// instruction ("give her a bob cut").
    Modify {
        target: Ref,
        description: String,
        #[arg(long)]
        seed: Option<u64>,
        #[arg(long)]
        steps: Option<u32>,
        #[arg(long, value_parser = parse_size)]
        size: Option<(u32, u32)>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long = "lora", value_parser = parse_lora)]
        loras: Vec<LoraSelection>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "interactive")]
        approve: ApprovalArg,
    },
    /// /attach-lora <ref> <lora> [--multiplier] — bind a registry LoRA to
    /// an artifact; auto-injected into its generations.
    AttachLora {
        target: Ref,
        /// Adapter file in the lora dir, or an existing registry entry.
        lora: String,
        #[arg(long)]
        multiplier: Option<f32>,
    },
    /// /lora-scale <lora> <multiplier> — adjust a bound adapter's weight.
    LoraScale { lora: String, multiplier: f32 },
    /// /lora-remove <lora> — unbind an adapter from the registry.
    LoraRemove { lora: String },
    /// /describe <ref> <description> — replace the artifact's canonical
    /// look. Edits re-anchor to this, so writing a change back here is how
    /// it survives the next `modify`.
    Describe { target: Ref, description: String },
    /// /revert <ref> — activate the artifact's previous image. Nothing is
    /// deleted; the revision reverted from stays in the history.
    Revert { target: Ref },
}

#[derive(Subcommand)]
enum StackCommand {
    /// Execute a serialized operation stack.
    Run {
        path: PathBuf,
        #[arg(long, value_enum, default_value = "llm")]
        origin: OriginArg,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "interactive")]
        approve: ApprovalArg,
    },
    /// Drive the LLM to produce an operation stack from free text.
    Propose {
        message: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum RuntimeCommand {
    /// Start the generation server in the background and return; it stays
    /// resident after this command exits (its log goes to the cache directory).
    Start {
        #[arg(long, default_value = "qwen-image-2.1", value_parser = parse_model)]
        model: String,
        /// Stop whatever holds the port first, even a server this app did not
        /// start.
        #[arg(long)]
        force: bool,
    },
    /// Stop the generation server this app started (a recorded pid).
    /// `--force` also kills a server on the port that was not recorded.
    Stop {
        #[arg(long)]
        force: bool,
    },
    /// Stop then start the generation server in the background.
    Restart {
        #[arg(long, default_value = "qwen-image-2.1", value_parser = parse_model)]
        model: String,
        /// Force-stop first, even a server this app did not start.
        #[arg(long)]
        force: bool,
    },
    /// Report whether the generation server is up, and whether this app owns it.
    Status,
    /// Start (or reuse) the native generation server and keep it resident in
    /// this terminal; ops in other terminals reuse it. Ctrl-C stops it.
    Serve {
        #[arg(long, default_value = "qwen-image-2.1", value_parser = parse_model)]
        model: String,
        /// Kill stale sd-server processes holding the port first (leftovers
        /// from interrupted sessions cannot be restarted by the runtime).
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ApprovalArg {
    /// Non-interactive: accept proposed text without asking.
    Auto,
    /// Ask at every checkpoint.
    Interactive,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OriginArg {
    User,
    Llm,
}

impl From<OriginArg> for OpOrigin {
    fn from(origin: OriginArg) -> Self {
        match origin {
            OriginArg::User => OpOrigin::User,
            OriginArg::Llm => OpOrigin::Llm,
        }
    }
}

fn parse_kind(value: &str) -> Result<ArtifactKind, String> {
    match value {
        "story" => Ok(ArtifactKind::Story),
        "scene" => Ok(ArtifactKind::Scene),
        "beat" => Ok(ArtifactKind::Beat),
        "character" => Ok(ArtifactKind::Character),
        "environment" => Ok(ArtifactKind::Environment),
        "object" => Ok(ArtifactKind::Object),
        other => Err(format!(
            "unknown artifact kind {other:?}; expected story|scene|beat|character|environment|object"
        )),
    }
}

fn parse_axis(value: &str) -> Result<VariantAxis, String> {
    match value {
        "outfit" => Ok(VariantAxis::Outfit),
        "age" => Ok(VariantAxis::Age),
        "body" => Ok(VariantAxis::Body),
        "hair" => Ok(VariantAxis::Hair),
        "expression" => Ok(VariantAxis::Expression),
        "time-of-day" => Ok(VariantAxis::TimeOfDay),
        "weather" => Ok(VariantAxis::Weather),
        "season" => Ok(VariantAxis::Season),
        "mood" => Ok(VariantAxis::Mood),
        other => Err(format!(
            "unknown variant axis {other:?}; expected outfit|age|body|hair|expression|time-of-day|weather|season|mood"
        )),
    }
}

fn parse_size(value: &str) -> Result<(u32, u32), String> {
    let (width, height) = value
        .split_once('x')
        .ok_or_else(|| format!("expected WxH (e.g. 768x448), got {value:?}"))?;
    let width = width
        .parse::<u32>()
        .map_err(|_| format!("invalid width {width:?}"))?;
    let height = height
        .parse::<u32>()
        .map_err(|_| format!("invalid height {height:?}"))?;
    Ok((width, height))
}

/// Parses `<file>@<multiplier>` for `--lora` (multiplier optional, 1.0).
fn parse_lora(value: &str) -> Result<LoraSelection, String> {
    let (path, multiplier) = match value.rsplit_once('@') {
        Some((path, multiplier)) => (
            path.to_owned(),
            multiplier
                .parse::<f32>()
                .map_err(|_| format!("invalid lora multiplier {multiplier:?}"))?,
        ),
        None => (value.to_owned(), 1.0),
    };
    if !(-2.0..=2.0).contains(&multiplier) {
        return Err(format!(
            "lora multiplier {multiplier} must be within -2..=2"
        ));
    }
    if path.is_empty() {
        return Err("--lora needs an adapter file: --lora <file>@<multiplier>".to_owned());
    }
    Ok(LoraSelection { path, multiplier })
}

fn parse_model(value: &str) -> Result<String, String> {
    let architecture = AppConfig::load()
        .map(|config| config.image.architecture)
        .map_err(|error| error.to_string())?;
    if value == architecture {
        Ok(value.to_owned())
    } else {
        Err(format!(
            "unknown generation profile {value:?}; expected {architecture}"
        ))
    }
}

fn random_seed() -> u64 {
    let id = Uuid::new_v4();
    u64::from_be_bytes(id.as_bytes()[..8].try_into().expect("eight bytes"))
}

fn main() -> Result<()> {
    // Errors surface through `CodedError::report`, which logs with the
    // error code; without a subscriber those lines would be dropped.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .compact()
        .init();
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new().context("failed to start the async runtime")?;
    runtime.block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Story { story } => story::run(story),
        Command::Project { path } => {
            let registry = load_project(&path)?;
            println!(
                "project {} ready ({} artifacts, {} log entries)",
                path.display(),
                registry.artifacts.len(),
                registry.log.len()
            );
            Ok(())
        }
        Command::Op { op } => {
            let mut registry = load_project(&cli.project)?;
            let (operation, cli_options) = build_operation(op)?;
            // The config is always loaded: the LoRA ops resolve the
            // `lora_dir` through the generation defaults.
            let config = AppConfig::load().context("could not load the configuration")?;
            let needs_backend =
                operation_needs_backend(&operation, cli_options.manual_text.as_deref());
            let backend = if needs_backend {
                Some(CreativeBackend::new(&config))
            } else {
                None
            };
            // Save even on failure: a visual op that fails its pipeline keeps
            // the artifact and a failed revision (fail-fast, intermediates
            // preserved), like `stack run`.
            let result = execute_one(
                &mut registry,
                &operation,
                backend.as_ref().map(|b| b as &dyn GenerationBackend),
                &cli_options,
                &config.generation_defaults,
                OpOrigin::User,
            )
            .await;
            if let Ok(outcome) = &result {
                print_outcome(&registry, &operation, outcome);
            }
            let saved = save_project(&cli.project, &registry);
            result?;
            saved
        }
        Command::Stack { stack } => match stack {
            StackCommand::Run {
                path,
                origin,
                out,
                approve,
            } => {
                let mut registry = load_project(&cli.project)?;
                // Save even on failure: a stack that fails partway keeps the
                // ops already applied (fail-fast, intermediates preserved).
                let result = run_stack(&mut registry, &path, origin.into(), out, approve).await;
                save_project(&cli.project, &registry)?;
                result
            }
            StackCommand::Propose { message, out } => {
                propose_stack(&cli.project, &message, out).await
            }
        },
        Command::Log { target } => {
            let registry = load_project(&cli.project)?;
            print_log(&registry, target.as_ref());
            Ok(())
        }
        Command::Eval { eval } => run_eval(eval).await,
        Command::Runtime { runtime } => {
            let config = AppConfig::load().context("could not load the configuration")?;
            match runtime {
                RuntimeCommand::Serve { model, force } => {
                    if force {
                        kill_foreign_generation_servers(&config)?;
                    }
                    // The address the provider declares is the one both the
                    // runtime and the client use.
                    let runtime = config.image.runtime();
                    runtime
                        .ensure_ready(&config.image.host, config.image.port)
                        .await
                        .map_err(|error| anyhow!(error.report().log()))?;
                    println!("generation runtime ready with {model}; Ctrl-C to stop");
                    let _ = tokio::signal::ctrl_c().await;
                    runtime
                        .stop()
                        .await
                        .map_err(|error| anyhow!(error.report().log()))?;
                    println!("generation runtime stopped");
                    Ok(())
                }
                RuntimeCommand::Start { model, force } => {
                    if force {
                        stop_generation_server(&config, true)?;
                    }
                    let pid =
                        control::start(&Managed::Image(config.image.clone()), &config.state_dir)
                            .await
                            .map_err(|error| anyhow!(error.report().log()))?;
                    match pid {
                        Some(pid) => println!(
                            "generation runtime started with {model} (pid {pid}); `svs runtime stop` stops it"
                        ),
                        None => println!(
                            "generation runtime already resident with {model}; `svs runtime stop` stops it"
                        ),
                    }
                    Ok(())
                }
                RuntimeCommand::Stop { force } => stop_generation_server(&config, force),
                RuntimeCommand::Restart { model, force } => {
                    if force {
                        kill_foreign_generation_servers(&config)?;
                    }
                    control::restart(&Managed::Image(config.image.clone()), &config.state_dir)
                        .await
                        .map_err(|error| anyhow!(error.report().log()))?;
                    println!("generation runtime restarted with {model}");
                    Ok(())
                }
                RuntimeCommand::Status => generation_status(&config).await,
            }
        }
    }
}

/// Asks the local vision model to judge images — the automated
/// reviewer's eyes. One round per `--prompt`, all images attached in order;
/// `--expect`/`--expect-not` gate the exit code so evaluation can gate a
/// test script.
async fn run_eval(eval: EvalArgs) -> Result<()> {
    let config = AppConfig::load().context("could not load the configuration")?;
    let mut deployment = config
        .vision
        .clone()
        .context("no `vision` provider is configured in clipforge.toml")?;
    if let Some(model) = &eval.model {
        deployment.model = model.clone();
    }
    // A managed (`llamacpp`) vision provider is started on demand and the
    // session holds it for the whole evaluation; an `openai` provider has no
    // server and is called as-is.
    let session = vision::VisionSession::start(deployment)
        .await
        .map_err(|error| anyhow!(error.report().log()))?;

    // Read every image up front so a missing file fails before any model round
    // trips. `image_for_chat` downscales each to a fixed maximum edge.
    let images: Vec<ChatImage> = eval
        .images
        .iter()
        .map(|path| vision::image_for_chat(path).map_err(|error| anyhow!(error.report().log())))
        .collect::<Result<_>>()?;

    let mut failures = 0usize;
    for prompt in &eval.prompts {
        let question = format!(
            "{prompt}\n\nThe attached images are numbered in the order given (image 1, image 2, …). Be specific and concise."
        );
        let answer = session
            .assess(images.clone(), &question, eval.max_tokens)
            .await
            .map_err(|error| anyhow!(error.report().log()))?;

        println!("model: {}", session.model());
        println!("question: {prompt}");
        println!("{answer}");
        println!("----");

        if let Err(reason) =
            vision::check(&answer, eval.expect.as_deref(), eval.expect_not.as_deref())
        {
            failures += 1;
            eprintln!("expectation failed: {reason}");
        }
    }

    if failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Stops the generation server on the provider's port. A server the shared
/// owner started is found by its recorded marker; `force` also stops one it did
/// not start, identified precisely by the served executable and listen port.
fn stop_generation_server(config: &AppConfig, force: bool) -> Result<()> {
    let port = config.image.port;
    match control::stop(&config.state_dir, port).map_err(|error| anyhow!(error.report().log()))? {
        StopOutcome::Stopped(pid) => {
            println!("stopped the generation server on :{port} (pid {pid})");
            return Ok(());
        }
        // A marker whose process is gone: cleared so nothing keeps pointing at a
        // dead pid, and fall through to the force/kill path.
        StopOutcome::ClearedStale(pid) => {
            println!("cleared a stale marker for :{port} (pid {pid} is gone)");
        }
        StopOutcome::NotFound => {}
    }
    if force {
        kill_foreign_generation_servers(config)?;
    } else {
        println!(
            "no generation server recorded on :{port} — a server the shared owner did not start needs `--force`"
        );
    }
    Ok(())
}

/// Reports whether the generation server is up, and whether the shared owner
/// manages it.
async fn generation_status(config: &AppConfig) -> Result<()> {
    let port = config.image.port;
    match control::status(&Managed::Image(config.image.clone()), &config.state_dir).await {
        ServerState::Up { owned: Some(pid) } => {
            println!(":{port} up · managed by the shared owner (pid {pid})")
        }
        ServerState::Up { owned: None } => println!(
            ":{port} up · not managed by the shared owner — `svs runtime stop --force` stops it"
        ),
        ServerState::Down => println!(":{port} down"),
    }
    Ok(())
}

/// Stops every server holding the provider's port, including one this app did
/// not start. Identified by the served executable and listen port — not a
/// `pkill` pattern, so another model's server is left alone.
fn kill_foreign_generation_servers(config: &AppConfig) -> Result<()> {
    let port = config.image.port;
    let killed =
        kill_servers(&config.image.executable, port).map_err(|error| anyhow!(error.to_string()))?;
    if killed.is_empty() {
        println!("no generation server is running on :{port}");
    } else {
        println!("stopped the generation server on :{port} (pid {killed:?})");
    }
    Ok(())
}

// ------------------------------------------------------------------ project store

/// Loads the registry snapshot from the SQLite database, creating the
/// database (and schema) on first use.
fn load_project(path: &Path) -> Result<ArtifactRegistry> {
    let db = ProjectDb::open(path).map_err(|error| anyhow!(error.report().log()))?;
    let stored = db.load().map_err(|error| anyhow!(error.report().log()))?;
    Ok(stored.registry)
}

/// Replaces the stored snapshot with the in-memory registry (the database is
/// a snapshot, not an event log).
fn save_project(path: &Path, registry: &ArtifactRegistry) -> Result<()> {
    let db = ProjectDb::open(path).map_err(|error| anyhow!(error.report().log()))?;
    db.save_registry(registry)
        .map_err(|error| anyhow!(error.report().log()))
}

// ------------------------------------------------------------------ operations

/// Per-invocation execution settings extracted from the CLI args.
struct CliRunOptions {
    work_dir: PathBuf,
    /// The `--out` golden-run folder; intermediates are copied here.
    golden_dir: Option<PathBuf>,
    approvals: ApprovalPolicy,
    seed: Option<u64>,
    generation: GenerationOverrides,
    manual_text: Option<String>,
}

/// Whether executing the operation needs the live backend. Text-only
/// model ops (create of story/scene/beat, compose) never do; visual
/// `create`/`variant` generate the first image as part of the op, so they
/// do; draft only needs it for the LLM step, which `--text` bypasses.
/// The LoRA ops are registry mutations — no backend, but they do need the
/// loaded config (the `lora_dir` existence check).
fn operation_needs_backend(operation: &Operation, manual_text: Option<&str>) -> bool {
    match operation {
        Operation::Create { kind, .. } => kind.is_visual(),
        Operation::Variant { .. } => true,
        Operation::Compose { .. } => false,
        Operation::Draft { .. } => manual_text.is_none(),
        Operation::Regenerate { .. } | Operation::Resize { .. } | Operation::Modify { .. } => true,
        Operation::AttachLora { .. }
        | Operation::LoraScale { .. }
        | Operation::LoraRemove { .. }
        | Operation::Set { .. }
        | Operation::Describe { .. }
        | Operation::Revert { .. } => false,
    }
}

impl CliRunOptions {
    fn to_execute_options<'a>(
        &'a self,
        backend: Option<&'a dyn GenerationBackend>,
        defaults: &'a GenerationDefaults,
        origin: OpOrigin,
    ) -> ExecuteOptions<'a> {
        // Adapters are sent to the native server under their registered
        // (top-level) name; missing files are dropped with a warning.
        let loras: Vec<LoraSelection> = self
            .generation
            .loras
            .iter()
            .filter_map(|selection| {
                defaults
                    .loras
                    .registered(&selection.path)
                    .map(|path| LoraSelection {
                        path,
                        multiplier: selection.multiplier,
                    })
            })
            .collect();
        ExecuteOptions {
            backend,
            run: RunOptions {
                work_dir: self.work_dir.clone(),
                approvals: match &self.approvals {
                    ApprovalPolicy::Auto { .. } => ApprovalPolicy::auto(),
                    ApprovalPolicy::Interactive(_) => interactive_policy(),
                },
                seed: self.seed,
            },
            generation: GenerationOverrides {
                // The configured default step count (Settings → Models)
                // shapes every CLI visual op too, mirroring the
                // canvas path; an explicit per-op `--steps` wins last.
                steps: Some(defaults.steps),
                loras,
                ..self.generation.clone()
            },
            defaults,
            manual_text: self.manual_text.as_deref(),
            origin,
        }
    }
}

/// Builds the typed operation from CLI args plus the execution settings.
fn build_operation(op: OpCommand) -> Result<(Operation, CliRunOptions)> {
    Ok(match op {
        OpCommand::Create {
            kind,
            description,
            name,
            size,
            loras,
        } => (
            Operation::Create {
                kind: parse_kind(&kind).map_err(anyhow::Error::msg)?,
                description,
                name,
                size,
                steps: None,
                loras: Vec::new(),
            },
            CliRunOptions {
                work_dir: std::env::temp_dir(),
                golden_dir: None,
                approvals: ApprovalPolicy::auto(),
                seed: None,
                generation: GenerationOverrides {
                    loras,
                    ..Default::default()
                },
                manual_text: None,
            },
        ),
        OpCommand::Variant {
            target,
            description,
            name,
            axis,
            loras,
        } => (
            Operation::Variant {
                target,
                description,
                name,
                axis: axis
                    .map(|axis| parse_axis(&axis).map_err(anyhow::Error::msg))
                    .transpose()?,
                size: None,
                steps: None,
                loras: Vec::new(),
            },
            CliRunOptions {
                work_dir: std::env::temp_dir(),
                golden_dir: None,
                approvals: ApprovalPolicy::auto(),
                seed: None,
                generation: GenerationOverrides {
                    loras,
                    ..Default::default()
                },
                manual_text: None,
            },
        ),
        OpCommand::Regenerate {
            target,
            prompt,
            seed,
            steps,
            size,
            model,
            loras,
            out,
            approve,
        } => (
            Operation::Regenerate {
                target,
                prompt,
                size: None,
                steps: None,
                loras: Vec::new(),
            },
            CliRunOptions {
                work_dir: work_dir(out.as_deref()),
                golden_dir: out,
                approvals: approval_policy(approve),
                // A fresh seed by default so "regenerate" is not identical to
                // the previous revision; pass --seed for golden replay.
                seed: Some(seed.unwrap_or_else(random_seed)),
                generation: GenerationOverrides {
                    steps,
                    size,
                    model,
                    loras,
                    ..Default::default()
                },
                manual_text: None,
            },
        ),
        OpCommand::Resize {
            target,
            size: (width, height),
            out,
            approve,
        } => (
            Operation::Resize {
                target,
                width,
                height,
            },
            CliRunOptions {
                work_dir: work_dir(out.as_deref()),
                golden_dir: out,
                approvals: approval_policy(approve),
                seed: Some(random_seed()),
                generation: GenerationOverrides::default(),
                manual_text: None,
            },
        ),
        OpCommand::Compose {
            scene,
            description,
            background,
            layers,
            out,
            approve,
        } => (
            Operation::Compose {
                scene,
                description,
                background,
                layers: layers
                    .into_iter()
                    .map(|artifact| ComposeLayer {
                        artifact,
                        variant: None,
                    })
                    .collect(),
            },
            CliRunOptions {
                work_dir: work_dir(out.as_deref()),
                golden_dir: out,
                approvals: approval_policy(approve),
                seed: None,
                generation: GenerationOverrides::default(),
                manual_text: None,
            },
        ),
        OpCommand::Draft {
            target,
            request,
            text,
            out,
            approve,
        } => (
            Operation::Draft { target, request },
            CliRunOptions {
                work_dir: work_dir(out.as_deref()),
                golden_dir: out,
                approvals: approval_policy(approve),
                seed: None,
                generation: GenerationOverrides::default(),
                manual_text: text,
            },
        ),
        OpCommand::Modify {
            target,
            description,
            seed,
            steps,
            size,
            model,
            loras,
            out,
            approve,
        } => (
            Operation::Modify {
                target,
                description,
                size: None,
                steps: None,
                loras: Vec::new(),
            },
            CliRunOptions {
                work_dir: work_dir(out.as_deref()),
                golden_dir: out,
                approvals: approval_policy(approve),
                seed: Some(seed.unwrap_or_else(random_seed)),
                generation: GenerationOverrides {
                    steps,
                    size,
                    model,
                    loras,
                    ..Default::default()
                },
                manual_text: None,
            },
        ),
        OpCommand::AttachLora {
            target,
            lora,
            multiplier,
        } => (
            Operation::AttachLora {
                target,
                lora,
                multiplier,
            },
            CliRunOptions {
                work_dir: std::env::temp_dir(),
                golden_dir: None,
                approvals: ApprovalPolicy::auto(),
                seed: None,
                generation: GenerationOverrides::default(),
                manual_text: None,
            },
        ),
        OpCommand::LoraScale { lora, multiplier } => (
            Operation::LoraScale { lora, multiplier },
            CliRunOptions {
                work_dir: std::env::temp_dir(),
                golden_dir: None,
                approvals: ApprovalPolicy::auto(),
                seed: None,
                generation: GenerationOverrides::default(),
                manual_text: None,
            },
        ),
        OpCommand::Describe {
            target,
            description,
        } => (
            Operation::Describe {
                target,
                description,
            },
            CliRunOptions {
                work_dir: std::env::temp_dir(),
                golden_dir: None,
                approvals: ApprovalPolicy::auto(),
                seed: None,
                generation: GenerationOverrides::default(),
                manual_text: None,
            },
        ),
        OpCommand::Revert { target } => (
            Operation::Revert { target },
            CliRunOptions {
                work_dir: std::env::temp_dir(),
                golden_dir: None,
                approvals: ApprovalPolicy::auto(),
                seed: None,
                generation: GenerationOverrides::default(),
                manual_text: None,
            },
        ),
        OpCommand::LoraRemove { lora } => (
            Operation::LoraRemove { lora },
            CliRunOptions {
                work_dir: std::env::temp_dir(),
                golden_dir: None,
                approvals: ApprovalPolicy::auto(),
                seed: None,
                generation: GenerationOverrides::default(),
                manual_text: None,
            },
        ),
    })
}

async fn execute_one(
    registry: &mut ArtifactRegistry,
    operation: &Operation,
    backend: Option<&dyn GenerationBackend>,
    cli: &CliRunOptions,
    defaults: &GenerationDefaults,
    origin: OpOrigin,
) -> Result<ops::OpOutcome> {
    let options = cli.to_execute_options(backend, defaults, origin);
    let outcome = ops::execute(registry, operation, &options)
        .await
        .map_err(|error| anyhow!(error.report().log()))?;
    if let Some(run) = &outcome.run {
        copy_golden_outputs(cli.golden_dir.as_deref(), run)?;
    }
    Ok(outcome)
}

/// `--out <dir>` golden runs: drop every intermediate (image, text) into the
/// folder for human review.
fn copy_golden_outputs(golden_dir: Option<&Path>, run: &PipelineRun) -> Result<()> {
    let Some(dir) = golden_dir else {
        return Ok(());
    };
    std::fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    for output in &run.outputs {
        let Some(path) = &output.path else {
            if let Some(text) = &output.text {
                println!("golden {}: {text}", output.label);
            }
            continue;
        };
        if output.kind == OutputKind::Void {
            continue;
        }
        let mut destination = dir.join(format!("{:02}-{}.png", output.step_index, output.label));
        if destination.exists() {
            let mut counter = 1;
            while destination.exists() {
                destination = dir.join(format!(
                    "{:02}-{}-{counter}.png",
                    output.step_index, output.label
                ));
                counter += 1;
            }
        }
        if path == &destination {
            continue; // already written into the golden folder
        }
        std::fs::copy(path, &destination).with_context(|| {
            format!(
                "could not copy {} to {}",
                path.display(),
                destination.display()
            )
        })?;
        println!("golden {}: {}", output.label, destination.display());
    }
    Ok(())
}

fn work_dir(out: Option<&Path>) -> PathBuf {
    out.map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("svs-work-{}", Uuid::new_v4())))
}

fn approval_policy(approve: ApprovalArg) -> ApprovalPolicy {
    match approve {
        ApprovalArg::Auto => ApprovalPolicy::auto(),
        ApprovalArg::Interactive => interactive_policy(),
    }
}

fn interactive_policy() -> ApprovalPolicy {
    ApprovalPolicy::Interactive(Box::new(|blocked| match blocked {
        BlockedOn::ApproveText { text, description } => {
            println!("\ncheckpoint: {description}\n\n{text}\n");
            if prompt("approve? [y/N]: ").trim().eq_ignore_ascii_case("y") {
                Decision::AcceptText
            } else {
                Decision::Reject
            }
        }
        BlockedOn::ProvideInput {
            description,
            purpose,
        } => {
            println!("\nmanual input needed: {description} (LLM unavailable)");
            match purpose {
                InputPurpose::DraftText | InputPurpose::Caption => {
                    let text = prompt("text: ");
                    if text.trim().is_empty() {
                        Decision::Reject
                    } else {
                        Decision::ProvideText(text)
                    }
                }
                InputPurpose::Feedback => {
                    let text = prompt("feedback: ");
                    if text.trim().is_empty() {
                        Decision::Reject
                    } else {
                        Decision::ProvideText(text)
                    }
                }
            }
        }
    }))
}

fn prompt(message: &str) -> String {
    print!("{message}");
    io::stdout().flush().expect("stdout should flush");
    io::stdin()
        .lock()
        .lines()
        .next()
        .transpose()
        .expect("stdin should read")
        .unwrap_or_default()
}

fn print_outcome(registry: &ArtifactRegistry, operation: &Operation, outcome: &ops::OpOutcome) {
    match operation {
        Operation::Create { .. } => {
            let id = outcome.artifact_id.expect("create has an artifact");
            println!(
                "created {} {}",
                registry.artifact(id).expect("created artifact exists").kind,
                registry.ref_of(id)
            );
        }
        Operation::Variant { .. } => {
            let id = outcome.artifact_id.expect("variant has an artifact");
            let artifact = registry.artifact(id).expect("variant exists");
            let base = artifact
                .variant_of
                .map(|base| registry.ref_of(base))
                .unwrap_or_else(|| "?".into());
            println!("created variant {} of {base}", registry.ref_of(id));
        }
        Operation::Compose { .. } => {
            let id = outcome.artifact_id.expect("compose has a beat");
            println!("composed beat {}", registry.ref_of(id));
        }
        Operation::AttachLora { .. } => {
            let id = outcome.artifact_id.expect("attach-lora has an artifact");
            println!("attached lora to {}", registry.ref_of(id));
        }
        Operation::LoraScale { lora, .. } => {
            println!("scaled lora {lora}");
        }
        Operation::LoraRemove { lora } => {
            println!("removed lora {lora}");
        }
        Operation::Describe { .. } => {
            let id = outcome.artifact_id.expect("describe has an artifact");
            println!("described {}", registry.ref_of(id));
        }
        Operation::Set { .. } => {
            let id = outcome.artifact_id.expect("set has an artifact");
            let artifact = registry.artifact(id).expect("set artifact exists");
            let steps = artifact
                .default_steps
                .map_or_else(|| "auto".to_owned(), |steps| steps.to_string());
            let seed = artifact
                .default_seed
                .map_or_else(|| "random".to_owned(), |seed| seed.to_string());
            println!("set {} steps={steps} seed={seed}", registry.ref_of(id));
        }
        Operation::Regenerate { .. } | Operation::Resize { .. } | Operation::Modify { .. } => {
            let id = outcome.artifact_id.expect("visual ops have an artifact");
            let revision_id = outcome.revision_id.expect("visual ops start a revision");
            match outcome.status {
                OpStatus::Applied => {
                    let asset = registry
                        .artifact(id)
                        .and_then(|artifact| {
                            artifact.revisions.iter().find(|r| r.id == revision_id)
                        })
                        .and_then(|revision| revision.asset_path.clone())
                        .unwrap_or_else(|| "?".into());
                    println!(
                        "{} {} → revision {} (completed, {})",
                        operation_name(operation),
                        registry.ref_of(id),
                        revision_id,
                        asset
                    );
                }
                OpStatus::Rejected => {
                    println!(
                        "{} {} → rejected at a checkpoint",
                        operation_name(operation),
                        registry.ref_of(id)
                    );
                }
                _ => {}
            }
        }
        Operation::Draft { .. } => {
            let id = outcome.artifact_id.expect("draft has an artifact");
            match outcome.status {
                OpStatus::Applied => println!("drafted {} (approved)", registry.ref_of(id)),
                OpStatus::Rejected => {
                    println!("drafted {} (proposal rejected)", registry.ref_of(id))
                }
                _ => {}
            }
        }
        Operation::Revert { .. } => {
            let id = outcome.artifact_id.expect("revert has an artifact");
            let revision_id = outcome.revision_id.expect("revert activates a revision");
            println!(
                "reverted {} → revision {}",
                registry.ref_of(id),
                revision_id
            );
        }
    }

    if let Some(run) = &outcome.run {
        for record in &run.decisions {
            match &record.decision {
                Decision::AcceptText => {
                    println!("checkpoint at step {}: text approved", record.step_index);
                }
                Decision::ProvideText(_) => {
                    println!(
                        "checkpoint at step {}: manual input used",
                        record.step_index
                    );
                }
                Decision::Reject => {}
            }
        }
        if let Some(image) = run.final_image() {
            println!("final image: {}", image.display());
        }
    }
}

fn operation_name(operation: &Operation) -> &'static str {
    match operation {
        Operation::Create { .. } => "create",
        Operation::Variant { .. } => "variant",
        Operation::Regenerate { .. } => "regenerate",
        Operation::Resize { .. } => "resize",
        Operation::Set { .. } => "set",
        Operation::Compose { .. } => "compose",
        Operation::Draft { .. } => "draft",
        Operation::Modify { .. } => "modify",
        Operation::AttachLora { .. } => "attach-lora",
        Operation::LoraScale { .. } => "lora-scale",
        Operation::LoraRemove { .. } => "lora-remove",
        Operation::Describe { .. } => "describe",
        Operation::Revert { .. } => "revert",
    }
}

// ------------------------------------------------------------------ stacks

async fn run_stack(
    registry: &mut ArtifactRegistry,
    path: &Path,
    origin: OpOrigin,
    out: Option<PathBuf>,
    approve: ApprovalArg,
) -> Result<()> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read stack file {}", path.display()))?;
    let stack: OperationStack = serde_json::from_str(&contents)
        .with_context(|| format!("could not parse stack file {}", path.display()))?;
    if stack.operations.is_empty() {
        bail!("stack contains no operations");
    }
    println!(
        "stack {} ({} operations):",
        path.display(),
        stack.operations.len()
    );
    for (index, operation) in stack.operations.iter().enumerate() {
        println!("  {index}: {}", operation.summary());
    }

    if approve == ApprovalArg::Interactive
        && !prompt("execute this stack? [y/N]: ")
            .trim()
            .eq_ignore_ascii_case("y")
    {
        println!("stack rejected");
        return Ok(());
    }

    let needs_backend = stack
        .operations
        .iter()
        .any(|op| operation_needs_backend(op, None));
    let config = AppConfig::load().context("could not load the configuration")?;
    let backend = if needs_backend {
        Some(CreativeBackend::new(&config))
    } else {
        None
    };

    let cli = CliRunOptions {
        work_dir: work_dir(out.as_deref()),
        golden_dir: out,
        approvals: approval_policy(approve),
        seed: None, // stacks replay deterministically unless ops carry prompts
        generation: GenerationOverrides::default(),
        manual_text: None,
    };
    for operation in &stack.operations {
        match execute_one(
            registry,
            operation,
            backend.as_ref().map(|b| b as &dyn GenerationBackend),
            &cli,
            &config.generation_defaults,
            origin,
        )
        .await
        {
            Ok(outcome) => print_outcome(registry, operation, &outcome),
            Err(error) => {
                // The error was already reported (coded log) inside
                // `execute_one`; its text carries the code.
                bail!("operation {} failed: {error}", operation.summary());
            }
        }
    }
    Ok(())
}

/// The VLLM contract test bed: free text → an operation stack the model
/// emits as JSON against the closed vocabulary Rust defines.
async fn propose_stack(project_path: &Path, message: &str, out: Option<PathBuf>) -> Result<()> {
    let registry = load_project(project_path)?;
    let config = AppConfig::load().context("could not load the configuration")?;
    let backend = CreativeBackend::new(&config);

    let stack = backend
        .propose_stack(&registry, &config.generation_defaults, None, message)
        .await
        .map_err(
            |error| anyhow!(ReportedError::plain(ErrorCode::AppProposalFailed, error).log()),
        )?;
    let pretty = serde_json::to_string_pretty(&stack).context("could not serialize stack")?;
    println!("{pretty}");

    if let Some(out) = out {
        std::fs::write(&out, pretty)
            .with_context(|| format!("could not write stack to {}", out.display()))?;
        println!("stack written to {}", out.display());
    }
    Ok(())
}

// ------------------------------------------------------------------ log

fn print_log(registry: &ArtifactRegistry, target: Option<&Ref>) {
    let filter = target
        .map(|target| registry.resolve(target))
        .transpose()
        .ok()
        .flatten();
    let entries: Vec<_> = match filter {
        Some(id) => registry.log_for(id).collect(),
        None => registry.log.iter().collect(),
    };
    if entries.is_empty() {
        println!("no log entries");
        return;
    }
    for entry in entries {
        let artifact = entry
            .artifact_id
            .map(|id| registry.ref_of(id))
            .unwrap_or_else(|| "-".into());
        println!(
            "  {} {} {:>8} artifact {} — {}",
            status_str(entry.status),
            origin_str(entry.origin),
            entry.id,
            artifact,
            entry.op.summary()
        );
    }
}

fn status_str(status: OpStatus) -> &'static str {
    match status {
        OpStatus::Proposed => "proposed",
        OpStatus::Applied => "applied",
        OpStatus::Rejected => "rejected",
        OpStatus::Reverted => "reverted",
    }
}

fn origin_str(origin: OpOrigin) -> &'static str {
    match origin {
        OpOrigin::User => "user",
        OpOrigin::Llm => "llm",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyfella_studio_core::registry::pipeline::{OutputKind, PipelineRun, StepOutput};

    #[test]
    fn golden_outputs_are_copied_with_step_prefixed_names() {
        let dir = std::env::temp_dir().join(format!("svs-golden-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("generated.png");
        std::fs::write(&source, b"png").unwrap();
        let golden = dir.join("golden");

        let run = PipelineRun {
            outputs: vec![
                StepOutput {
                    step_index: 0,
                    label: "load_image".into(),
                    kind: OutputKind::Image,
                    path: Some(source.clone()),
                    text: None,
                },
                StepOutput {
                    step_index: 1,
                    label: "generate".into(),
                    kind: OutputKind::Image,
                    path: Some(source.clone()),
                    text: None,
                },
                StepOutput {
                    step_index: 3,
                    label: "llm".into(),
                    kind: OutputKind::Text,
                    path: None,
                    text: Some("draft text".into()),
                },
            ],
            decisions: vec![],
        };

        copy_golden_outputs(Some(&golden), &run).unwrap();

        assert!(golden.join("00-load_image.png").is_file());
        assert!(golden.join("01-generate.png").is_file());
        assert!(
            !golden.join("03-llm.png").exists(),
            "text outputs are not copied"
        );
    }
}
