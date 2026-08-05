//! `svs` — the Smart Visual Sequencer operation CLI.
//!
//! Drives the artifact registry + pipeline API without the Dioxus UI:
//! typed operations (`svs op …`), serialized operation stacks (`svs stack
//! run` — the VLLM contract test bed), LLM stack proposals (`svs stack
//! propose`), and the operation log. `--out <dir>` drops every intermediate
//! (image, mask, composite) into a folder for the manual golden-run tier
//! (`docs/ROADMAP.md` §7).
//!
//! The project is a SQLite database (`.svs-project.db` by default, see
//! `src/persistence/` and `docs/ROADMAP.md` §10); `svs import` migrates a
//! legacy JSON project file in one step.

use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use uuid::Uuid;

use smart_visual_sequencer::app::AppConfig;
use smart_visual_sequencer::llm::{ChatMessage, ChatOptions, LmStudioClient};
use smart_visual_sequencer::persistence::ProjectDb;
use smart_visual_sequencer::registry::backend::CreativeBackend;
use smart_visual_sequencer::registry::ops::{
    self, ComposeLayer, ExecuteOptions, OpOrigin, OpStatus, Operation, OperationStack,
};
use smart_visual_sequencer::registry::pipeline::{
    ApprovalPolicy, BlockedOn, Decision, GenerationBackend, GenerationOverrides, InputPurpose,
    OutputKind, PipelineRun, RunOptions,
};
use smart_visual_sequencer::registry::{
    ArtifactKind, ArtifactRegistry, ProjectFile, Ref, VariantAxis,
};
use smart_visual_sequencer::runtime::KreaQuantization;

#[derive(Parser)]
#[command(
    name = "svs",
    version,
    about = "Drive Smart Visual Sequencer operations from the command line"
)]
struct Cli {
    /// SQLite project database (created on first use; `svs import` migrates
    /// a legacy JSON project in one step).
    #[arg(long, global = true, default_value = ".svs-project.db")]
    project: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Open (load or create) the project database.
    Project { path: PathBuf },
    /// One-time migration: import a legacy JSON project file into the
    /// SQLite database given by `--project`.
    Import { path: PathBuf },
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
    /// Show the operation log (all entries, or one artifact's).
    Log {
        /// Filter to one artifact's entries.
        target: Option<Ref>,
    },
}

#[derive(Subcommand)]
enum OpCommand {
    /// /create <kind> <description> — new artifact.
    Create {
        kind: String,
        description: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// /variant <ref> <description> [axis] — new visual variant.
    Variant {
        target: Ref,
        description: String,
        #[arg(long)]
        axis: Option<String>,
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
    /// /modify <ref> <description> — mask-guided regional edit
    /// (segment → confirm mask → inpaint, composite fallback).
    Modify {
        target: Ref,
        description: String,
        #[arg(long = "mask-prompt")]
        mask_prompt: Option<String>,
        #[arg(long = "inpaint-prompt")]
        inpaint_prompt: Option<String>,
        #[arg(long)]
        seed: Option<u64>,
        #[arg(long)]
        steps: Option<u32>,
        #[arg(long, value_parser = parse_size)]
        size: Option<(u32, u32)>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "interactive")]
        approve: ApprovalArg,
    },
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
    /// Drive LM Studio to produce an operation stack from free text.
    Propose {
        message: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum RuntimeCommand {
    /// Start (or restart) the native generation server with a Krea profile
    /// and keep it resident in this terminal; ops in other terminals reuse
    /// it. Ctrl-C stops it.
    Serve {
        #[arg(long, default_value = "krea-2-turbo-q2", value_parser = parse_model)]
        model: String,
        /// Kill stale sd-server processes holding the port first (leftovers
        /// from interrupted sessions cannot be restarted by the runtime).
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ApprovalArg {
    /// Non-interactive: best mask candidate, accept text.
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

fn parse_model(value: &str) -> Result<KreaQuantization, String> {
    match value {
        "krea-2-turbo-q2" => Ok(KreaQuantization::Q2),
        "krea-2-turbo-q4" => Ok(KreaQuantization::Q4),
        other => Err(format!(
            "unknown generation profile {other:?}; expected krea-2-turbo-q2 or krea-2-turbo-q4"
        )),
    }
}

fn random_seed() -> u64 {
    let id = Uuid::new_v4();
    u64::from_be_bytes(id.as_bytes()[..8].try_into().expect("eight bytes"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new().context("failed to start the async runtime")?;
    runtime.block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Project { path } => {
            let file = load_project(&path)?;
            println!(
                "project {} ready ({} artifacts, {} log entries)",
                path.display(),
                file.registry.artifacts.len(),
                file.registry.log.len()
            );
            Ok(())
        }
        Command::Import { path } => import_project(&cli.project, &path),
        Command::Op { op } => {
            let mut file = load_project(&cli.project)?;
            let (operation, cli_options) = build_operation(op)?;
            let needs_backend =
                operation_needs_backend(&operation, cli_options.manual_text.as_deref());
            let backend = if needs_backend {
                let config = AppConfig::load().context("could not load config/app.toml")?;
                Some(CreativeBackend::new(&config))
            } else {
                None
            };
            let outcome = execute_one(
                &mut file.registry,
                &operation,
                backend.as_ref().map(|b| b as &dyn GenerationBackend),
                &cli_options,
                OpOrigin::User,
            )
            .await?;
            print_outcome(&file.registry, &operation, &outcome);
            save_project(&cli.project, &file)
        }
        Command::Stack { stack } => match stack {
            StackCommand::Run {
                path,
                origin,
                out,
                approve,
            } => {
                let mut file = load_project(&cli.project)?;
                // Save even on failure: a stack that fails partway keeps the
                // ops already applied (fail-fast, intermediates preserved).
                let result =
                    run_stack(&mut file.registry, &path, origin.into(), out, approve).await;
                save_project(&cli.project, &file)?;
                result
            }
            StackCommand::Propose { message, out } => {
                propose_stack(&cli.project, &message, out).await
            }
        },
        Command::Log { target } => {
            let file = load_project(&cli.project)?;
            print_log(&file.registry, target.as_ref());
            Ok(())
        }
        Command::Runtime { runtime } => match runtime {
            RuntimeCommand::Serve { model, force } => {
                if force {
                    kill_resident_generation_servers()?;
                }
                let profile = parse_model(&model).map_err(anyhow::Error::msg)?;
                let config = AppConfig::load().context("could not load config/app.toml")?;
                let backend = CreativeBackend::with_profile(&config, profile);
                backend
                    .ensure_profile_ready(profile)
                    .await
                    .map_err(|error| anyhow!(error))?;
                println!(
                    "generation runtime ready with {model} ({}); Ctrl-C to stop",
                    backend.profile().profile_id()
                );
                let _ = tokio::signal::ctrl_c().await;
                backend
                    .stop_generation()
                    .await
                    .map_err(|error| anyhow!(error))?;
                println!("generation runtime stopped");
                Ok(())
            }
        },
    }
}

/// Kills sd-server processes this CLI does not own (stale instances from
/// interrupted sessions hold the port and cannot be restarted by the
/// runtime). `pkill` exits 1 when nothing matched, which is fine.
fn kill_resident_generation_servers() -> Result<()> {
    let status = std::process::Command::new("pkill")
        .args(["-f", "sd-server --diffusion"])
        .status()
        .context("could not run pkill")?;
    println!("killed stale generation servers (pkill exit {status})");
    Ok(())
}

// ------------------------------------------------------------------ project store

/// Loads the registry snapshot from the SQLite database, creating the
/// database (and schema) on first use.
fn load_project(path: &Path) -> Result<ProjectFile> {
    let db = ProjectDb::open(path).context("could not open project database")?;
    let stored = db
        .load()
        .context("could not load registry from project database")?;
    Ok(ProjectFile {
        version: 1,
        registry: stored.registry,
    })
}

/// Replaces the stored snapshot with the in-memory registry (same semantics
/// as the old stopgap JSON write: the database is a snapshot, not an event
/// log).
fn save_project(path: &Path, file: &ProjectFile) -> Result<()> {
    let db = ProjectDb::open(path).context("could not open project database")?;
    db.save_registry(&file.registry)
        .context("could not save registry to project database")
}

/// One-time migration from the legacy JSON `ProjectFile` (`.svs-project.json`)
/// into the SQLite database at `db_path`. Refuses to clobber a database that
/// already holds artifacts.
fn import_project(db_path: &Path, legacy_path: &Path) -> Result<()> {
    let db = ProjectDb::open(db_path).context("could not open project database")?;
    let (artifacts, log) = db
        .import_json(legacy_path)
        .context("could not import legacy project")?;
    println!(
        "imported {artifacts} artifacts and {log} log entries from {} into {}",
        legacy_path.display(),
        db_path.display()
    );
    Ok(())
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

/// Whether executing the operation needs the live backend. Model-only ops
/// (create, variant, compose) never do; draft only needs it for the LLM
/// step, which `--text` bypasses.
fn operation_needs_backend(operation: &Operation, manual_text: Option<&str>) -> bool {
    match operation {
        Operation::Create { .. } | Operation::Variant { .. } | Operation::Compose { .. } => false,
        Operation::Draft { .. } => manual_text.is_none(),
        Operation::Regenerate { .. } | Operation::Modify { .. } => true,
    }
}

impl CliRunOptions {
    fn to_execute_options<'a>(
        &'a self,
        backend: Option<&'a dyn GenerationBackend>,
        origin: OpOrigin,
    ) -> ExecuteOptions<'a> {
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
            generation: self.generation.clone(),
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
        } => (
            Operation::Create {
                kind: parse_kind(&kind).map_err(anyhow::Error::msg)?,
                description,
                name,
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
        OpCommand::Variant {
            target,
            description,
            axis,
        } => (
            Operation::Variant {
                target,
                description,
                axis: axis
                    .map(|axis| parse_axis(&axis).map_err(anyhow::Error::msg))
                    .transpose()?,
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
        OpCommand::Regenerate {
            target,
            prompt,
            seed,
            steps,
            size,
            model,
            out,
            approve,
        } => (
            Operation::Regenerate { target, prompt },
            CliRunOptions {
                work_dir: work_dir(out.as_deref()),
                golden_dir: out,
                approvals: approval_policy(approve),
                // A fresh seed by default so "regenerate" is not identical to
                // the previous revision; pass --seed for golden replay.
                seed: Some(seed.unwrap_or_else(random_seed)),
                generation: GenerationOverrides { steps, size, model },
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
            mask_prompt,
            inpaint_prompt,
            seed,
            steps,
            size,
            model,
            out,
            approve,
        } => (
            Operation::Modify {
                target,
                description,
                mask_prompt,
                inpaint_prompt,
            },
            CliRunOptions {
                work_dir: work_dir(out.as_deref()),
                golden_dir: out,
                approvals: approval_policy(approve),
                seed: Some(seed.unwrap_or_else(random_seed)),
                generation: GenerationOverrides { steps, size, model },
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
    origin: OpOrigin,
) -> Result<ops::OpOutcome> {
    let options = cli.to_execute_options(backend, origin);
    let outcome = ops::execute(registry, operation, &options)
        .await
        .map_err(|error| anyhow!(error))?;
    if let Some(run) = &outcome.run {
        copy_golden_outputs(cli.golden_dir.as_deref(), run)?;
    }
    Ok(outcome)
}

/// `--out <dir>` golden runs: drop every intermediate (image, mask,
/// composite) into the folder for human review.
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
            continue; // already written into the golden folder (composite etc.)
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
        BlockedOn::SelectMask {
            candidates,
            description,
        } => {
            println!("\ncheckpoint: {description}");
            for (index, candidate) in candidates.iter().enumerate() {
                println!(
                    "  [{index}] {} (score {:.3}, {} px)",
                    candidate.path.display(),
                    candidate.score,
                    candidate.area_pixels
                );
            }
            let choice = prompt("select mask [0]: ").trim().to_owned();
            let choice = choice.parse::<usize>().unwrap_or(0);
            Decision::SelectMask(choice.min(candidates.len().saturating_sub(1)))
        }
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
                InputPurpose::PlannedEdit => {
                    let mask_prompt = prompt("mask prompt: ");
                    let inpaint_prompt = prompt("inpaint prompt: ");
                    if mask_prompt.trim().is_empty() || inpaint_prompt.trim().is_empty() {
                        Decision::Reject
                    } else {
                        Decision::ProvidePlan(
                            smart_visual_sequencer::registry::pipeline::PlannedEdit {
                                mask_prompt,
                                inpaint_prompt,
                            },
                        )
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
        Operation::Regenerate { .. } | Operation::Modify { .. } => {
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
    }

    if let Some(run) = &outcome.run {
        for record in &run.decisions {
            match &record.decision {
                Decision::SelectMask(choice) => {
                    if let BlockedOn::SelectMask { candidates, .. } = &record.blocked_on {
                        if let Some(candidate) = candidates.get(*choice) {
                            println!(
                                "checkpoint at step {}: mask {choice} selected (score {:.3}, {})",
                                record.step_index,
                                candidate.score,
                                candidate.path.display()
                            );
                        }
                    }
                }
                Decision::AcceptText => {
                    println!("checkpoint at step {}: text approved", record.step_index);
                }
                Decision::ProvideText(_) | Decision::ProvidePlan(_) => {
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
        Operation::Compose { .. } => "compose",
        Operation::Draft { .. } => "draft",
        Operation::Modify { .. } => "modify",
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

    let needs_backend = stack.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Regenerate { .. } | Operation::Modify { .. } | Operation::Draft { .. }
        )
    });
    let backend = if needs_backend {
        let config = AppConfig::load().context("could not load config/app.toml")?;
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
            origin,
        )
        .await
        {
            Ok(outcome) => print_outcome(registry, operation, &outcome),
            Err(error) => {
                bail!("operation {} failed: {error}", operation.summary());
            }
        }
    }
    Ok(())
}

/// The VLLM contract test bed: free text → an operation stack the model
/// emits as JSON against the closed vocabulary Rust defines.
async fn propose_stack(project_path: &Path, message: &str, out: Option<PathBuf>) -> Result<()> {
    let file = load_project(project_path)?;
    let config = AppConfig::load().context("could not load config/app.toml")?;
    let client = LmStudioClient::new(config.lm_studio.clone())
        .map_err(|error| anyhow!("could not build the LM Studio client: {error}"))?;

    let mut registry_listing = String::new();
    for artifact in file.registry.iter() {
        registry_listing.push_str(&format!(
            "  {} {} {:?}",
            file.registry.ref_of(artifact.id),
            artifact.kind,
            artifact.name
        ));
        if !artifact.description.is_empty() {
            registry_listing.push_str(&format!(" — {}", artifact.description));
        }
        registry_listing.push('\n');
    }

    let system = format!(
        "You plan operations for a visual story tool. Respond with JSON only, exactly \
         this shape: {{\"operations\": [ ... ]}} where every operation is one of:\n\
         {{\"op\": \"create\", \"kind\": \"story|scene|beat|character|environment|object\", \"description\": \"...\", \"name\": \"...\"}}\n\
         {{\"op\": \"variant\", \"target\": \"c:<name>\", \"description\": \"...\", \"axis\": \"outfit|age|body|hair|expression|time-of-day|weather|season|mood\"}}\n\
         {{\"op\": \"regenerate\", \"target\": \"c:<name>\", \"prompt\": \"...\"}}\n\
         {{\"op\": \"compose\", \"scene\": \"c:<name>\", \"description\": \"...\", \"background\": \"c:<name>\", \"layers\": [{{\"artifact\": \"c:<name>\", \"variant\": \"c:<name>\"}}]}}\n\
         {{\"op\": \"draft\", \"target\": \"c:<name>\", \"request\": \"...\"}}\n\
         {{\"op\": \"modify\", \"target\": \"c:<name>\", \"description\": \"...\", \"mask_prompt\": \"...\", \"inpaint_prompt\": \"...\"}}\n\
         This vocabulary is closed — never invent an op name. References are the \
         c:<name> keys listed below (case-insensitive); use only those.\n\n\
         Registry:\n{registry_listing}"
    );
    let options = ChatOptions {
        response_format: Some(serde_json::json!({"type": "json_object"})),
        ..ChatOptions::default()
    };
    let completion = client
        .chat_with_options(
            &[
                ChatMessage::text("system", system),
                ChatMessage::text("user", message),
            ],
            &options,
        )
        .await
        .map_err(|error| anyhow!("LM Studio request failed: {error}"))?;
    let content = completion
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .ok_or_else(|| anyhow!("LM Studio returned no content"))?;

    let stack: OperationStack = serde_json::from_str(&content)
        .with_context(|| format!("LLM output was not a valid operation stack:\n{content}"))?;
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
    use smart_visual_sequencer::registry::pipeline::{OutputKind, PipelineRun, StepOutput};

    #[test]
    fn import_migrates_a_legacy_json_project_and_refuses_to_clobber() {
        let dir = std::env::temp_dir().join(format!("svs-import-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = dir.join("legacy.svs-project.json");
        let db_path = dir.join("project.db");

        let mut file = ProjectFile::default();
        file.registry
            .artifacts
            .push(smart_visual_sequencer::registry::Artifact {
                id: Uuid::new_v4(),
                kind: smart_visual_sequencer::registry::ArtifactKind::Story,
                name: "The Lighthouse".to_owned(),
                description: "A quiet lighthouse above a silver sea".to_owned(),
                variant_of: None,
                variant_axis: None,
                parent_id: None,
                composition: None,
                active_revision_id: None,
                revisions: Vec::new(),
                drafts: Vec::new(),
            });
        std::fs::write(
            &legacy,
            serde_json::to_string_pretty(&file).expect("legacy file should serialize"),
        )
        .unwrap();

        import_project(&db_path, &legacy).expect("import should succeed");
        let db = ProjectDb::open(&db_path).expect("database should open");
        let stored = db.load().expect("database should load");
        assert_eq!(stored.registry.artifacts.len(), 1);

        let error = import_project(&db_path, &legacy).expect_err("import must refuse to clobber");
        assert!(
            error
                .chain()
                .any(|cause| cause.to_string().contains("refusing to import over")),
            "unexpected error: {error:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

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
                    label: "segment".into(),
                    kind: OutputKind::Mask,
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

        assert!(golden.join("00-segment.png").is_file());
        assert!(golden.join("01-generate.png").is_file());
        assert!(
            !golden.join("03-llm.png").exists(),
            "text outputs are not copied"
        );
    }
}
