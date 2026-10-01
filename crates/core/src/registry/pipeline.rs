//! The pipeline execution layer (`docs/ROADMAP.md` §7).
//!
//! Two layers, kept distinct: operations (intent — `ops.rs`) and pipelines
//! (execution — this module). A pipeline is a linear, fail-fast stack of
//! steps from a closed vocabulary. Steps produce **typed intermediates**
//! (handles): `ImageHandle`, `PromptHandle`, `FeedbackHandle`. Handles are
//! checked statically by the type system — a caption cannot be fed into an
//! image slot — and the builder validates the stack at `build()`.
//!
//! Rules implemented here:
//!
//! - **Closed vocabulary.** `Step` is the complete set; the VLLM combines
//!   these kinds, it never invents them.
//! - **Static validation at `build()`** — parameter bounds (sizes are
//!   multiples of 32 within the contract bounds, step counts, LoRA count)
//!   and step ordering.
//! - **Linear, fail-fast execution.** A failed step ends the stack with the
//!   intermediates produced so far; there is no branching or retry.
//! - **Checkpoints.** `Checkpoint` steps block for human approval (draft text
//!   approval). The approval policy decides; rejection ends the stack cleanly.
//! - **LLM steps are soft dependencies.** `Llm` steps degrade to manual
//!   input when the backend has no LLM or it fails; they never hard-fail a
//!   stack.

use std::path::PathBuf;

use clipforge_inference::sd::{self, GenerateRequest, LoraSelection};
use futures_util::future::BoxFuture;
use thiserror::Error;

use crate::errors::{CodedError, ErrorCode, ReportedError};

/// The generation model.
pub const DEFAULT_MODEL: &str = sd::qwen_image_2_1().id;
/// The interactive draft profile used by the UI and the CLI defaults.
pub const DEFAULT_WIDTH: u32 = 768;
pub const DEFAULT_HEIGHT: u32 = 448;

/// Generation parameters shared by every `Generate` step in a pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationParams {
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    /// Text-guidance scale (`sample_params.guidance.txt_cfg`).
    pub cfg_scale: f32,
    /// Sampling method (`sample_params.sample_method`).
    pub sample_method: String,
    pub seed: Option<u64>,
    pub loras: Vec<LoraSelection>,
}

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_owned(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            steps: sd::DEFAULT_STEPS,
            cfg_scale: sd::DEFAULT_CFG_SCALE,
            sample_method: sd::DEFAULT_SAMPLE_METHOD.to_owned(),
            seed: None,
            loras: Vec::new(),
        }
    }
}

/// Per-run generation overrides (CLI flags like `--steps`/`--size`/`--model`
/// /`--lora`, plus the configured defaults; the operation itself carries only
/// intent). Applied to the compiled pipeline and re-validated before
/// execution. `loras` are appended after the auto-injected adapters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerationOverrides {
    pub steps: Option<u32>,
    pub size: Option<(u32, u32)>,
    pub model: Option<String>,
    pub cfg_scale: Option<f32>,
    pub sample_method: Option<String>,
    pub loras: Vec<LoraSelection>,
}

/// A typed handle to a step's output. The step index is resolved at run
/// time; the *kind* is checked by the Rust type system at build time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageHandle {
    pub(crate) step: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptHandle {
    pub(crate) step: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeedbackHandle {
    pub(crate) step: usize,
}

/// Where a prompt-typed slot gets its text from.
#[derive(Clone, Debug, PartialEq)]
pub enum PromptSource {
    Text(String),
    Prompt(PromptHandle),
}

impl From<&str> for PromptSource {
    fn from(text: &str) -> Self {
        PromptSource::Text(text.to_owned())
    }
}

impl From<String> for PromptSource {
    fn from(text: String) -> Self {
        PromptSource::Text(text)
    }
}

/// The closed step vocabulary.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Loads a file on disk as the stack's working image. -> Image
    LoadImage { source: PathBuf },
    /// Blocks for human approval. Produces no value.
    Checkpoint {
        description: String,
        on: CheckpointOn,
    },
    /// A native generation: fresh (no reference) or a reference edit of the
    /// working image. -> Image
    Generate {
        prompt: PromptSource,
        reference: Option<ImageHandle>,
    },
    /// An LLM step — a soft dependency. -> Prompt | Feedback
    Llm { kind: LlmStepKind },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CheckpointOn {
    ApproveText(PromptHandle),
}

#[derive(Clone, Debug, PartialEq)]
pub enum LlmStepKind {
    /// Propose story text for the given request. -> Prompt
    Draft { request: String },
    /// Rewrite a terse edit instruction into one grounded against the
    /// artifact it edits. -> Prompt
    ///
    /// Unlike a blocking step this never stops: `fallback` is a usable
    /// deterministic grounding, so a missing LLM degrades silently instead
    /// of asking the user. The LLM earns its place by doing what the
    /// template cannot — dropping invariants the edit contradicts.
    Ground { request: String, fallback: String },
    /// Caption an image. -> Prompt
    Describe { image: ImageHandle },
    /// Critique an image against a prompt. -> Feedback
    Critique {
        image: ImageHandle,
        prompt: PromptSource,
    },
}

/// A statically validated pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Pipeline {
    pub(crate) steps: Vec<Step>,
    pub(crate) params: GenerationParams,
}

impl Pipeline {
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn params(&self) -> &GenerationParams {
        &self.params
    }

    /// Applies per-run generation overrides and re-runs the static checks.
    pub fn apply_generation_overrides(
        &mut self,
        overrides: &GenerationOverrides,
    ) -> Result<(), PipelineBuildError> {
        if let Some(steps) = overrides.steps {
            self.params.steps = steps;
        }
        if let Some((width, height)) = overrides.size {
            self.params.width = width;
            self.params.height = height;
        }
        if let Some(model) = &overrides.model {
            self.params.model = model.clone();
        }
        if let Some(cfg_scale) = overrides.cfg_scale {
            self.params.cfg_scale = cfg_scale;
        }
        if let Some(sample_method) = &overrides.sample_method {
            self.params.sample_method = sample_method.clone();
        }
        // Per-run adapters are appended after the auto-injected ones;
        // duplicates of an already-carried path are dropped (first wins).
        for lora in &overrides.loras {
            if !self
                .params
                .loras
                .iter()
                .any(|carried| carried.path == lora.path)
            {
                self.params.loras.push(lora.clone());
            }
        }
        validate(&self.steps, &self.params)
    }

    /// Executes the stack against a backend, resolving typed intermediates
    /// at run time. See [`RunOptions`] for checkpoint handling.
    pub async fn run(
        &self,
        backend: &dyn GenerationBackend,
        options: &RunOptions,
    ) -> Result<PipelineRun, PipelineError> {
        let seed = options.seed.or(self.params.seed);
        let mut values: Vec<StepValue> = Vec::with_capacity(self.steps.len());
        let mut outputs = Vec::new();
        let mut decisions = Vec::new();

        for (index, step) in self.steps.iter().enumerate() {
            let value = match step {
                Step::LoadImage { source } => {
                    if !source.exists() {
                        return Err(PipelineError::Step {
                            step: index,
                            message: format!("reference image {} does not exist", source.display()),
                        });
                    }
                    StepValue::Image(source.clone())
                }
                Step::Checkpoint { description, on } => match on {
                    CheckpointOn::ApproveText(text_handle) => {
                        let text = values
                            .get(text_handle.step)
                            .and_then(StepValue::as_prompt)
                            .cloned()
                            .ok_or_else(|| PipelineError::Step {
                                step: index,
                                message: "checkpoint references a non-text step".to_owned(),
                            })?;
                        let blocked = BlockedOn::ApproveText {
                            description: description.clone(),
                            text: text.clone(),
                        };
                        let decision = options.approvals.decide(blocked.clone());
                        decisions.push(BlockRecord {
                            step_index: index,
                            blocked_on: blocked,
                            decision: decision.clone(),
                        });
                        match decision {
                            Decision::AcceptText => StepValue::Nothing,
                            Decision::Reject => return Err(PipelineError::Rejected),
                            other => {
                                return Err(PipelineError::Step {
                                    step: index,
                                    message: format!(
                                        "invalid decision for text checkpoint: {other:?}"
                                    ),
                                })
                            }
                        }
                    }
                },
                Step::Generate { prompt, reference } => {
                    let prompt_text = resolve_prompt(prompt, &values, index)?;
                    let reference_path = reference
                        .map(|handle| {
                            values
                                .get(handle.step)
                                .and_then(StepValue::as_image)
                                .cloned()
                                .ok_or_else(|| PipelineError::Step {
                                    step: index,
                                    message: "generate reference is not an image".to_owned(),
                                })
                        })
                        .transpose()?;
                    let request = GenerateRequest {
                        prompt: prompt_text,
                        reference_image_path: reference_path.map(|path| path.display().to_string()),
                        width: self.params.width,
                        height: self.params.height,
                        steps: self.params.steps,
                        cfg_scale: self.params.cfg_scale,
                        sample_method: self.params.sample_method.clone(),
                        seed,
                        loras: self.params.loras.clone(),
                    };
                    let response = backend.generate(&request).await?;
                    if response.status == "failed" {
                        return Err(PipelineError::Step {
                            step: index,
                            message: response.error.unwrap_or_else(|| "generation failed".into()),
                        });
                    }
                    let image_path = response.image_path.ok_or_else(|| PipelineError::Step {
                        step: index,
                        message: "generation returned no image".to_owned(),
                    })?;
                    StepValue::Image(PathBuf::from(image_path))
                }
                Step::Llm { kind } => match kind {
                    LlmStepKind::Draft { request } => {
                        // Soft dependency: an unavailable or failing LLM
                        // degrades to manual input instead of failing the
                        // stack.
                        let produced = match backend.llm_draft(request).await {
                            Ok(Some(text)) => Some(text),
                            Ok(None) | Err(_) => None,
                        };
                        match produced {
                            Some(text) => StepValue::Prompt(text),
                            None => {
                                let blocked = BlockedOn::ProvideInput {
                                    description: format!("draft story text for {request:?}"),
                                    purpose: InputPurpose::DraftText,
                                };
                                let decision = options.approvals.decide(blocked.clone());
                                decisions.push(BlockRecord {
                                    step_index: index,
                                    blocked_on: blocked,
                                    decision: decision.clone(),
                                });
                                match decision {
                                    Decision::ProvideText(text) => StepValue::Prompt(text),
                                    Decision::Reject => return Err(PipelineError::Rejected),
                                    other => {
                                        return Err(PipelineError::Step {
                                            step: index,
                                            message: format!(
                                                "invalid decision for degraded draft: {other:?}"
                                            ),
                                        })
                                    }
                                }
                            }
                        }
                    }
                    LlmStepKind::Ground { request, fallback } => {
                        // Soft dependency with a usable default: never blocks.
                        let grounded = match backend.llm_ground(request).await {
                            Ok(Some(grounded)) => grounded,
                            Ok(None) | Err(_) => fallback.clone(),
                        };
                        StepValue::Prompt(grounded)
                    }
                    LlmStepKind::Describe { image } => {
                        let image_path = value_image(values.get(image.step), index)?;
                        let blocked = BlockedOn::ProvideInput {
                            description: format!(
                                "describe image {} (LLM captioning is a soft dependency)",
                                image_path.display()
                            ),
                            purpose: InputPurpose::Caption,
                        };
                        let decision = options.approvals.decide(blocked.clone());
                        decisions.push(BlockRecord {
                            step_index: index,
                            blocked_on: blocked,
                            decision: decision.clone(),
                        });
                        match decision {
                            Decision::ProvideText(text) => StepValue::Prompt(text),
                            Decision::Reject => return Err(PipelineError::Rejected),
                            other => {
                                return Err(PipelineError::Step {
                                    step: index,
                                    message: format!(
                                        "invalid decision for caption step: {other:?}"
                                    ),
                                })
                            }
                        }
                    }
                    LlmStepKind::Critique { image, prompt } => {
                        let image_path = value_image(values.get(image.step), index)?;
                        let prompt_text = resolve_prompt(prompt, &values, index)?;
                        let blocked = BlockedOn::ProvideInput {
                            description: format!(
                                "critique image {} against {prompt_text:?} (LLM is a soft dependency)",
                                image_path.display()
                            ),
                            purpose: InputPurpose::Feedback,
                        };
                        let decision = options.approvals.decide(blocked.clone());
                        decisions.push(BlockRecord {
                            step_index: index,
                            blocked_on: blocked,
                            decision: decision.clone(),
                        });
                        match decision {
                            Decision::ProvideText(text) => StepValue::Feedback(text),
                            Decision::Reject => return Err(PipelineError::Rejected),
                            other => {
                                return Err(PipelineError::Step {
                                    step: index,
                                    message: format!(
                                        "invalid decision for critique step: {other:?}"
                                    ),
                                })
                            }
                        }
                    }
                },
            };

            let (kind, path, text) = step_output(&value, step_label(step));
            outputs.push(StepOutput {
                step_index: index,
                label: step_label(step).to_owned(),
                kind,
                path,
                text,
            });
            values.push(value);
        }

        Ok(PipelineRun { outputs, decisions })
    }
}

fn value_image(value: Option<&StepValue>, step: usize) -> Result<PathBuf, PipelineError> {
    value
        .and_then(StepValue::as_image)
        .cloned()
        .ok_or_else(|| PipelineError::Step {
            step,
            message: "expected an image intermediate".to_owned(),
        })
}

fn resolve_prompt(
    source: &PromptSource,
    values: &[StepValue],
    step: usize,
) -> Result<String, PipelineError> {
    match source {
        PromptSource::Text(text) => Ok(text.clone()),
        PromptSource::Prompt(handle) => values
            .get(handle.step)
            .and_then(StepValue::as_prompt)
            .cloned()
            .ok_or_else(|| PipelineError::Step {
                step,
                message: "prompt handle does not reference a text step".to_owned(),
            }),
    }
}

fn step_label(step: &Step) -> &'static str {
    match step {
        Step::LoadImage { .. } => "load_image",
        Step::Checkpoint { .. } => "checkpoint",
        Step::Generate { .. } => "generate",
        Step::Llm { .. } => "llm",
    }
}

fn step_output(value: &StepValue, label: &str) -> (OutputKind, Option<PathBuf>, Option<String>) {
    match value {
        StepValue::Image(path) => (OutputKind::Image, Some(path.clone()), None),
        StepValue::Prompt(text) => (OutputKind::Text, None, Some(text.clone())),
        StepValue::Feedback(text) => (OutputKind::Text, None, Some(text.clone())),
        StepValue::Nothing => (OutputKind::Void, None, Some(label.to_owned())),
    }
}

/// The runtime value of a step.
#[derive(Clone, Debug, PartialEq)]
pub enum StepValue {
    Image(PathBuf),
    Prompt(String),
    Feedback(String),
    Nothing,
}

impl StepValue {
    fn as_image(&self) -> Option<&PathBuf> {
        match self {
            StepValue::Image(path) => Some(path),
            _ => None,
        }
    }

    fn as_prompt(&self) -> Option<&String> {
        match self {
            StepValue::Prompt(text) => Some(text),
            _ => None,
        }
    }
}

/// What a checkpoint blocks on.
#[derive(Clone, Debug, PartialEq)]
pub enum BlockedOn {
    ApproveText {
        description: String,
        text: String,
    },
    /// A degraded LLM step asking for manual input.
    ProvideInput {
        description: String,
        purpose: InputPurpose,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputPurpose {
    DraftText,
    Caption,
    Feedback,
}

/// What the approval policy answers with.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    AcceptText,
    Reject,
    ProvideText(String),
}

/// How checkpoints are resolved.
pub enum ApprovalPolicy {
    /// Non-interactive: accepts/rejects text per `accept_text`, and rejects
    /// degraded LLM steps.
    Auto { accept_text: bool },
    /// Interactive: the closure is called for every block.
    Interactive(Box<dyn Fn(BlockedOn) -> Decision + Send>),
}

impl ApprovalPolicy {
    pub fn auto() -> Self {
        Self::Auto { accept_text: true }
    }

    fn decide(&self, blocked: BlockedOn) -> Decision {
        match self {
            ApprovalPolicy::Auto { accept_text } => match blocked {
                BlockedOn::ApproveText { .. } => {
                    if *accept_text {
                        Decision::AcceptText
                    } else {
                        Decision::Reject
                    }
                }
                BlockedOn::ProvideInput { .. } => Decision::Reject,
            },
            ApprovalPolicy::Interactive(callback) => callback(blocked),
        }
    }
}

/// The non-interactive checkpoint resolution used as the fallback for
/// block types a caller does not surface (draft-text approval and degraded
/// LLM input): accept text, reject degraded steps — the `auto()` defaults.
pub fn auto_decision(blocked: BlockedOn) -> Decision {
    match blocked {
        BlockedOn::ApproveText { .. } => Decision::AcceptText,
        BlockedOn::ProvideInput { .. } => Decision::Reject,
    }
}

/// Execution options for [`Pipeline::run`].
pub struct RunOptions {
    /// Where pure image steps write their outputs.
    pub work_dir: PathBuf,
    /// Checkpoint resolution.
    pub approvals: ApprovalPolicy,
    /// Overrides the pipeline's seed for this run (golden-run replay).
    pub seed: Option<u64>,
}

impl RunOptions {
    pub fn new(work_dir: impl Into<PathBuf>) -> Self {
        Self {
            work_dir: work_dir.into(),
            approvals: ApprovalPolicy::auto(),
            seed: None,
        }
    }
}

/// One recorded checkpoint resolution (provenance for logs and golden runs).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockRecord {
    pub step_index: usize,
    pub blocked_on: BlockedOn,
    pub decision: Decision,
}

/// The result of a successful (non-rejected) pipeline run.
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineRun {
    pub outputs: Vec<StepOutput>,
    pub decisions: Vec<BlockRecord>,
}

impl PipelineRun {
    /// The last image produced by the stack, if any.
    pub fn final_image(&self) -> Option<&PathBuf> {
        self.outputs.iter().rev().find_map(|output| {
            if output.kind == OutputKind::Image {
                output.path.as_ref()
            } else {
                None
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    Image,
    Text,
    Void,
}

/// One step's observable output — the golden-run surface.
#[derive(Clone, Debug, PartialEq)]
pub struct StepOutput {
    pub step_index: usize,
    pub label: String,
    pub kind: OutputKind,
    pub path: Option<PathBuf>,
    pub text: Option<String>,
}

/// The result of a generation step: the stored image path, or the failure
/// reason. The live backend fills it from the shared runtime's job result;
/// the pipeline reads `status`, `image_path`, and `error`.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerateResponse {
    pub status: String,
    pub image_path: Option<String>,
    pub seed: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub error: Option<String>,
}

/// The backend the pipeline executes against. The live implementation wraps
/// `CreativeRuntime` + `LmStudioClient` (see `backend.rs`); tests inject a
/// fake to assert request bodies and state transitions.
pub trait GenerationBackend: Send + Sync {
    fn generate(
        &self,
        request: &GenerateRequest,
    ) -> BoxFuture<'_, Result<GenerateResponse, PipelineError>>;
    /// Returns `None` when no LLM is available (soft dependency).
    fn llm_draft(&self, request: &str) -> BoxFuture<'_, Result<Option<String>, PipelineError>>;
    /// Rewrites a terse edit instruction into a grounded one. Returns
    /// `None` when no LLM is available; the caller holds a deterministic
    /// fallback, so this never blocks. The default declines, which makes
    /// every backend without an LLM use that fallback.
    fn llm_ground(&self, _request: &str) -> BoxFuture<'_, Result<Option<String>, PipelineError>> {
        Box::pin(async { Ok(None) })
    }
    /// Moves a locally-produced final image into the persistent asset store,
    /// returning its stored path. The default keeps the source path —
    /// backends without a store (the test doubles) leave results in place.
    fn import_asset(
        &self,
        source: std::path::PathBuf,
    ) -> BoxFuture<'_, Result<String, PipelineError>> {
        Box::pin(async move { Ok(source.display().to_string()) })
    }
}

#[derive(Debug, Error)]
pub enum PipelineError {
    /// A backend failure. `context` carries the request details (prompt,
    /// model, params, seed, job id) from the failure site so the final
    /// log line says what was being generated — the flat string alone is
    /// not enough to debug a failed generation.
    #[error("backend error: {message}")]
    Backend {
        message: String,
        context: Vec<(&'static str, String)>,
    },
    #[error("step {step} failed: {message}")]
    Step { step: usize, message: String },
    #[error("the stack was rejected at a checkpoint")]
    Rejected,
}

impl PipelineError {
    pub fn backend(error: impl std::fmt::Display) -> Self {
        Self::Backend {
            message: error.to_string(),
            context: Vec::new(),
        }
    }

    /// A backend failure annotated with the request context (prompt,
    /// model, size, steps, seed, job id, …) that explains what was being
    /// generated when it failed.
    pub fn backend_with_context(
        error: impl std::fmt::Display,
        context: Vec<(&'static str, String)>,
    ) -> Self {
        Self::Backend {
            message: error.to_string(),
            context,
        }
    }
}

impl CodedError for PipelineError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Backend { .. } => ErrorCode::OpBackendFailed,
            Self::Step { .. } => ErrorCode::OpStepFailed,
            Self::Rejected => ErrorCode::OpRejected,
        }
    }

    /// The report keeps the request context the failure site attached —
    /// it must survive the chain up to the surfacing point. The fallback
    /// builds inline (calling `CodedError::report(self)` here would
    /// resolve to this override and recurse).
    fn report(&self) -> ReportedError {
        match self {
            Self::Backend { message, context } => ReportedError {
                code: ErrorCode::OpBackendFailed,
                details: format!("backend error: {message}"),
                context: context.clone(),
            },
            _ => ReportedError {
                code: self.code(),
                details: self.to_string(),
                context: Vec::new(),
            },
        }
    }
}

#[derive(Debug, Error)]
pub enum PipelineBuildError {
    #[error("a pipeline must contain at least one step")]
    Empty,
    #[error("step {step} references the output of a later step ({reference})")]
    Ordering { step: usize, reference: usize },
    #[error("unsupported model {0}; expected the configured Qwen-Image-2.1 profile")]
    BadModel(String),
    #[error("invalid size {width}x{height}: dimensions must be multiples of 32 within 256..=2048")]
    BadSize { width: u32, height: u32 },
    #[error("invalid step count {0}: expected 1..=50")]
    BadSteps(u32),
    #[error("too many LoRAs ({0}); the backend accepts at most 8")]
    TooManyLoras(usize),
    #[error("LoRA multiplier {0} is outside the -2.0..=2.0 contract bounds")]
    BadLoraMultiplier(f32),
}

impl CodedError for PipelineBuildError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Empty => ErrorCode::OpBuildEmpty,
            Self::Ordering { .. } => ErrorCode::OpBuildOrdering,
            Self::BadModel(_) => ErrorCode::OpBuildBadModel,
            Self::BadSize { .. } => ErrorCode::OpBuildBadSize,
            Self::BadSteps(_) => ErrorCode::OpBuildBadSteps,
            Self::TooManyLoras(_) => ErrorCode::OpBuildTooManyLoras,
            Self::BadLoraMultiplier(_) => ErrorCode::OpBuildBadLoraMultiplier,
        }
    }
}

/// Builds a linear pipeline. Handles returned by the methods are typed by
/// construction, so ordering and kind compatibility cannot be violated
/// through this API; `build()` runs the remaining static checks.
pub struct PipelineBuilder {
    steps: Vec<Step>,
    params: GenerationParams,
}

impl PipelineBuilder {
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            params: GenerationParams::default(),
        }
    }

    pub fn model(&mut self, model: impl Into<String>) -> &mut Self {
        self.params.model = model.into();
        self
    }

    pub fn size(&mut self, width: u32, height: u32) -> &mut Self {
        self.params.width = width;
        self.params.height = height;
        self
    }

    pub fn steps(&mut self, steps: u32) -> &mut Self {
        self.params.steps = steps;
        self
    }

    pub fn cfg_scale(&mut self, cfg_scale: f32) -> &mut Self {
        self.params.cfg_scale = cfg_scale;
        self
    }

    pub fn sample_method(&mut self, method: impl Into<String>) -> &mut Self {
        self.params.sample_method = method.into();
        self
    }

    pub fn seed(&mut self, seed: u64) -> &mut Self {
        self.params.seed = Some(seed);
        self
    }

    pub fn lora(&mut self, path: impl Into<String>, multiplier: f32) -> &mut Self {
        self.params.loras.push(LoraSelection {
            path: path.into(),
            multiplier,
        });
        self
    }

    /// Loads a file on disk as the working image. -> Image
    pub fn reference_image(&mut self, source: impl Into<PathBuf>) -> ImageHandle {
        let step = self.steps.len();
        self.steps.push(Step::LoadImage {
            source: source.into(),
        });
        ImageHandle { step }
    }

    /// A generation step: fresh (no reference) or an edit (reference). -> Image
    pub fn generate(
        &mut self,
        prompt: impl Into<PromptSource>,
        reference: Option<ImageHandle>,
    ) -> ImageHandle {
        let step = self.steps.len();
        self.steps.push(Step::Generate {
            prompt: prompt.into(),
            reference,
        });
        ImageHandle { step }
    }

    /// An LLM story-text proposal (soft dependency). -> Prompt
    pub fn llm_draft(&mut self, request: impl Into<String>) -> PromptHandle {
        let step = self.steps.len();
        self.steps.push(Step::Llm {
            kind: LlmStepKind::Draft {
                request: request.into(),
            },
        });
        PromptHandle { step }
    }

    /// Grounds a terse edit instruction against its artifact, falling back
    /// to `fallback` when no LLM is configured. -> Prompt
    pub fn llm_ground(
        &mut self,
        request: impl Into<String>,
        fallback: impl Into<String>,
    ) -> PromptHandle {
        let step = self.steps.len();
        self.steps.push(Step::Llm {
            kind: LlmStepKind::Ground {
                request: request.into(),
                fallback: fallback.into(),
            },
        });
        PromptHandle { step }
    }

    pub fn llm_describe(&mut self, image: ImageHandle) -> PromptHandle {
        let step = self.steps.len();
        self.steps.push(Step::Llm {
            kind: LlmStepKind::Describe { image },
        });
        PromptHandle { step }
    }

    pub fn llm_critique(
        &mut self,
        image: ImageHandle,
        prompt: impl Into<PromptSource>,
    ) -> FeedbackHandle {
        let step = self.steps.len();
        self.steps.push(Step::Llm {
            kind: LlmStepKind::Critique {
                image,
                prompt: prompt.into(),
            },
        });
        FeedbackHandle { step }
    }

    /// Appends the text-approval checkpoint (draft proposals are gated).
    pub fn confirm_text(&mut self, text: PromptHandle) {
        self.steps.push(Step::Checkpoint {
            description: "approve the proposed story text".to_owned(),
            on: CheckpointOn::ApproveText(text),
        });
    }

    /// Static validation happens here (see [`PipelineBuildError`]).
    pub fn build(self) -> Result<Pipeline, PipelineBuildError> {
        validate(&self.steps, &self.params)?;
        Ok(Pipeline {
            steps: self.steps,
            params: self.params,
        })
    }
}

impl Default for PipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a generation size is within contract bounds: multiples of 32
/// within 256..=2048 (the same rule `build()` enforces). Shared by the
/// registry (artifact default sizes) and the pipeline builder.
pub fn is_valid_size(width: u32, height: u32) -> bool {
    width.is_multiple_of(32)
        && height.is_multiple_of(32)
        && (256..=2048).contains(&width)
        && (256..=2048).contains(&height)
}

fn validate(steps: &[Step], params: &GenerationParams) -> Result<(), PipelineBuildError> {
    if steps.is_empty() {
        return Err(PipelineBuildError::Empty);
    }
    if !is_valid_size(params.width, params.height) {
        return Err(PipelineBuildError::BadSize {
            width: params.width,
            height: params.height,
        });
    }
    if !(1..=50).contains(&params.steps) {
        return Err(PipelineBuildError::BadSteps(params.steps));
    }
    if params.loras.len() > 8 {
        return Err(PipelineBuildError::TooManyLoras(params.loras.len()));
    }
    if params
        .loras
        .iter()
        .any(|lora| !(-2.0..=2.0).contains(&lora.multiplier))
    {
        return Err(PipelineBuildError::BadLoraMultiplier(
            params
                .loras
                .iter()
                .find(|lora| !(-2.0..=2.0).contains(&lora.multiplier))
                .map(|lora| lora.multiplier)
                .unwrap_or(0.0),
        ));
    }
    if params.model != DEFAULT_MODEL {
        return Err(PipelineBuildError::BadModel(params.model.clone()));
    }

    for (index, step) in steps.iter().enumerate() {
        for reference in step_references(step) {
            if reference >= index {
                return Err(PipelineBuildError::Ordering {
                    step: index,
                    reference,
                });
            }
        }
    }
    Ok(())
}

/// All step indices a step consumes.
fn step_references(step: &Step) -> Vec<usize> {
    match step {
        Step::LoadImage { .. } => Vec::new(),
        Step::Checkpoint { on, .. } => match on {
            CheckpointOn::ApproveText(text) => vec![text.step],
        },
        Step::Generate { reference, .. } => reference.iter().map(|image| image.step).collect(),
        Step::Llm { kind } => match kind {
            LlmStepKind::Draft { .. } | LlmStepKind::Ground { .. } => Vec::new(),
            LlmStepKind::Describe { image } => vec![image.step],
            LlmStepKind::Critique { image, .. } => vec![image.step],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use uuid::Uuid;

    // ---------------------------------------------------------------- helpers

    fn work_dir() -> PathBuf {
        std::env::temp_dir().join(format!("svs-pipeline-{}", Uuid::new_v4()))
    }

    fn write_rgba(path: &Path, width: u32, height: u32, color: [u8; 4]) {
        image::ImageBuffer::from_pixel(width, height, image::Rgba(color))
            .save(path)
            .expect("test image should save");
    }

    #[derive(Clone, Debug)]
    enum CapturedRequest {
        Generate(GenerateRequest),
    }

    #[derive(Default)]
    struct FakeBackend {
        generated_image: std::sync::Mutex<Option<PathBuf>>,
        draft_text: std::sync::Mutex<Option<String>>,
        captured: std::sync::Mutex<Vec<CapturedRequest>>,
    }

    impl FakeBackend {
        fn with_generated(path: PathBuf) -> Self {
            let fake = Self::default();
            *fake.generated_image.lock().unwrap() = Some(path);
            fake
        }

        fn requests(&self) -> Vec<CapturedRequest> {
            self.captured.lock().unwrap().clone()
        }
    }

    impl GenerationBackend for FakeBackend {
        fn generate(
            &self,
            request: &GenerateRequest,
        ) -> BoxFuture<'_, Result<GenerateResponse, PipelineError>> {
            let request = request.clone();
            let image = self.generated_image.lock().unwrap().clone();
            Box::pin(async move {
                self.captured
                    .lock()
                    .unwrap()
                    .push(CapturedRequest::Generate(request.clone()));
                match image {
                    Some(path) => Ok(GenerateResponse {
                        status: "completed".to_owned(),
                        image_path: Some(path.display().to_string()),
                        seed: request.seed,
                        width: Some(request.width),
                        height: Some(request.height),
                        error: None,
                    }),
                    None => Ok(GenerateResponse {
                        status: "failed".to_owned(),
                        image_path: None,
                        seed: None,
                        width: None,
                        height: None,
                        error: Some("fake generation failed".to_owned()),
                    }),
                }
            })
        }

        fn llm_draft(
            &self,
            _request: &str,
        ) -> BoxFuture<'_, Result<Option<String>, PipelineError>> {
            let text = self.draft_text.lock().unwrap().clone();
            Box::pin(async move { Ok(text) })
        }
    }

    // ------------------------------------------------------- builder validation

    #[test]
    fn build_rejects_an_empty_pipeline() {
        let error = PipelineBuilder::new().build().unwrap_err();
        assert!(matches!(error, PipelineBuildError::Empty));
    }

    #[test]
    fn build_rejects_out_of_bounds_parameters() {
        let mut builder = PipelineBuilder::new();
        builder.size(1000, 1000);
        builder.generate("a frame", None);
        assert!(matches!(
            builder.build().unwrap_err(),
            PipelineBuildError::BadSize { .. }
        ));

        let mut builder = PipelineBuilder::new();
        builder.steps(0);
        builder.generate("a frame", None);
        assert!(matches!(
            builder.build().unwrap_err(),
            PipelineBuildError::BadSteps(0)
        ));

        let mut builder = PipelineBuilder::new();
        builder.model("sd-xl");
        builder.generate("a frame", None);
        assert!(matches!(
            builder.build().unwrap_err(),
            PipelineBuildError::BadModel(_)
        ));
    }

    #[test]
    fn build_rejects_out_of_order_handles() {
        let steps = vec![Step::Generate {
            prompt: PromptSource::Text("a frame".into()),
            reference: Some(ImageHandle { step: 1 }), // references a later step
        }];
        let pipeline = Pipeline {
            steps,
            params: GenerationParams::default(),
        };
        let error = validate(&pipeline.steps, &pipeline.params).unwrap_err();
        assert!(matches!(error, PipelineBuildError::Ordering { .. }));
    }

    #[test]
    fn build_accepts_the_regenerate_shape() {
        let mut builder = PipelineBuilder::new();
        builder.size(768, 448).steps(25).seed(42);
        let image = builder.reference_image("frame.png");
        builder.generate("make it warmer", Some(image));
        let pipeline = builder.build().expect("valid pipeline should build");
        assert_eq!(pipeline.params.model, DEFAULT_MODEL);
        assert_eq!(pipeline.params.seed, Some(42));
    }

    // ------------------------------------------------------- execution

    #[test]
    fn regenerate_pipeline_passes_the_right_request_and_reference() {
        let dir = work_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let generated = dir.join("generated.png");
        write_rgba(&original, 64, 64, [10, 20, 30, 255]);
        write_rgba(&generated, 64, 64, [40, 50, 60, 255]);

        let backend = FakeBackend::with_generated(generated.clone());
        let mut builder = PipelineBuilder::new();
        builder.size(768, 448).steps(25).seed(7);
        let image = builder.reference_image(&original);
        builder.generate("a lighthouse at dusk", Some(image));
        let pipeline = builder.build().unwrap();

        let run = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run(&backend, &RunOptions::new(dir.join("work"))))
            .unwrap();

        assert_eq!(run.final_image(), Some(&generated));
        let requests = backend.requests();
        assert!(
            matches!(requests.as_slice(), [CapturedRequest::Generate(request)] if
                request.prompt == "a lighthouse at dusk"
                && request.reference_image_path.as_deref() == Some(original.to_str().unwrap())
                && request.width == 768
                && request.height == 448
                && request.steps == 25
                && request.seed == Some(7)
                && request.cfg_scale == sd::DEFAULT_CFG_SCALE
                && request.sample_method == sd::DEFAULT_SAMPLE_METHOD
                && request.loras.is_empty()
            )
        );
    }

    #[test]
    fn auto_decision_matches_the_auto_policy_defaults() {
        assert_eq!(
            auto_decision(BlockedOn::ApproveText {
                description: String::new(),
                text: String::new()
            }),
            Decision::AcceptText
        );
        assert_eq!(
            auto_decision(BlockedOn::ProvideInput {
                description: String::new(),
                purpose: InputPurpose::DraftText
            }),
            Decision::Reject
        );
    }

    #[test]
    fn draft_pipeline_gates_llm_text_behind_approval() {
        let backend = {
            let fake = FakeBackend::default();
            *fake.draft_text.lock().unwrap() = Some("The keeper winds the lamp at dusk.".into());
            fake
        };
        let mut builder = PipelineBuilder::new();
        let text = builder.llm_draft("write the opening beat");
        builder.confirm_text(text);
        let pipeline = builder.build().unwrap();

        let run = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run(&backend, &RunOptions::new(work_dir())))
            .unwrap();

        let decision = &run.decisions[0];
        assert_eq!(decision.decision, Decision::AcceptText);
        let prompt = run
            .outputs
            .iter()
            .find(|output| output.label == "llm")
            .expect("llm output should exist");
        assert_eq!(
            prompt.text.as_deref(),
            Some("The keeper winds the lamp at dusk.")
        );
    }

    #[test]
    fn degraded_llm_steps_accept_manual_input() {
        let backend = FakeBackend::default(); // no LLM configured
        let mut builder = PipelineBuilder::new();
        let text = builder.llm_draft("write the opening beat");
        builder.confirm_text(text);
        let pipeline = builder.build().unwrap();

        let options = RunOptions {
            work_dir: work_dir(),
            approvals: ApprovalPolicy::Interactive(Box::new(|blocked| match blocked {
                BlockedOn::ProvideInput { .. } => {
                    Decision::ProvideText("A storm gathers over the bay.".into())
                }
                BlockedOn::ApproveText { .. } => Decision::AcceptText,
            })),
            seed: None,
        };
        let run = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run(&backend, &options))
            .unwrap();

        let prompt = run
            .outputs
            .iter()
            .find(|output| output.label == "llm")
            .expect("llm output should exist");
        assert_eq!(
            prompt.text.as_deref(),
            Some("A storm gathers over the bay.")
        );
    }

    #[test]
    fn failed_generation_marks_the_run_failed_with_partial_outputs() {
        let dir = work_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        write_rgba(&original, 64, 48, [1, 2, 3, 255]);
        let backend = FakeBackend::default(); // no generated image → failure

        let mut builder = PipelineBuilder::new();
        let image = builder.reference_image(&original);
        builder.generate("doomed", Some(image));
        let pipeline = builder.build().unwrap();

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run(&backend, &RunOptions::new(dir.join("work"))));
        let error = result.expect_err("generation should fail");
        assert!(matches!(error, PipelineError::Step { .. }));
    }

    #[test]
    fn run_options_seed_overrides_the_pipeline_seed() {
        let dir = work_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let generated = dir.join("generated.png");
        write_rgba(&original, 32, 32, [0, 0, 0, 255]);
        write_rgba(&generated, 32, 32, [0, 0, 0, 255]);
        let backend = FakeBackend::with_generated(generated);

        let mut builder = PipelineBuilder::new();
        builder.seed(11);
        let image = builder.reference_image(&original);
        builder.generate("frame", Some(image));
        let pipeline = builder.build().unwrap();

        let options = RunOptions {
            work_dir: dir.join("work"),
            approvals: ApprovalPolicy::auto(),
            seed: Some(99),
        };
        let _ = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run(&backend, &options))
            .unwrap();

        let requests = backend.requests();
        assert!(
            matches!(&requests[0], CapturedRequest::Generate(request) if request.seed == Some(99))
        );
    }
}
