//! The typed operation set (`docs/ROADMAP.md` §7) and its compiler +
//! executor.
//!
//! Operations are the *intent* layer (semantic, logged, approval-gated,
//! user/LLM-facing); pipelines (`pipeline.rs`) are the *execution* layer.
//! Each operation compiles to a pipeline — except the model-only slice-1
//! ops (compose, and `create` of text-only kinds), which mutate the
//! registry directly. Visual `create`/`variant` are both: the registry
//! mutation plus a first-image pipeline (the variant references the base's
//! active image).
//!
//! Slice-1 operations: `create`, `variant`, `regenerate`, `compose`,
//! `draft`, and `modify` (an instruction edit against the artifact's own
//! image).
//!
//! Execution model: user-typed ops apply immediately and are logged; LLM
//! proposals (`stack propose`) are gated by approval before execution.
//! Checkpoint steps inside pipelines (draft text approval) always block per
//! the approval policy. Undo is state restore
//! (`ArtifactRegistry::snapshot`/`restore`), never re-execution.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use clipforge_inference::sd::LoraSelection;

use crate::errors::{CodedError, ErrorCode, ReportedError};

use super::{
    pipeline::{
        GenerationBackend, GenerationOverrides, Pipeline, PipelineBuildError, PipelineError,
        PromptSource, RunOptions,
    },
    Artifact, ArtifactKind, ArtifactRegistry, CreateArtifact, GenerationDefaults, Layer, LayerRole,
    Ref, RefError, RegistryError, RevisionOrigin, VariantAxis,
};

/// The kinds an operation can have. Slice 1 only ships primitives; compound
/// (named saved sequences) and pure (read-only) kinds are vocabulary for
/// later slices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationKind {
    Primitive,
    Compound,
    Pure,
}

/// The typed operation set. Serialized with an `"op"` tag so the VLLM can
/// emit stacks against a closed JSON vocabulary (see [`OperationStack`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// New artifact (story, scene, beat, character, environment, object).
    Create {
        kind: ArtifactKind,
        description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// The generation size this artifact's images default to (`None` =
        /// the kind default applies).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<(u32, u32)>,
        /// Per-run step count for the first image (user-requested only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<u32>,
        /// Per-run adapters for the first image (user-requested only).
        #[serde(default)]
        loras: Vec<LoraSelection>,
    },
    /// New visual variant of an artifact.
    Variant {
        target: Ref,
        description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        axis: Option<VariantAxis>,
        /// The size the variant's first image is generated at (`None` =
        /// inherited from the base). The artifact's stored default size is
        /// *not* changed by this.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<(u32, u32)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<u32>,
        #[serde(default)]
        loras: Vec<LoraSelection>,
    },
    /// New revision of the active image (fresh seed, or edited prompt with
    /// the current image as reference).
    Regenerate {
        target: Ref,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        /// Per-run canvas override (user-requested only; `None` keeps the
        /// artifact's size). Does not change the stored default size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<(u32, u32)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<u32>,
        #[serde(default)]
        loras: Vec<LoraSelection>,
    },
    /// Change the artifact's stored generation defaults without rendering
    /// anything. `Some(None)` clears a default; `None` leaves it alone —
    /// the distinction the inspector needs to unpin a seed.
    Set {
        target: Ref,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<Option<u32>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<Option<u64>>,
    },
    /// Change the artifact's canvas: store the new default size **and**
    /// regenerate the active image at it (the subject is kept via the
    /// reference image).
    Resize {
        target: Ref,
        width: u32,
        height: u32,
    },
    /// New beat in a scene, with a composition of layer references.
    Compose {
        scene: Ref,
        description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        background: Option<Ref>,
        #[serde(default)]
        layers: Vec<ComposeLayer>,
    },
    /// LLM proposes story text; the user approves (checkpoint).
    Draft { target: Ref, request: String },
    /// Edit the artifact's active image from a plain-language instruction,
    /// which is grounded against the artifact before it is sent: reference
    /// image = the active revision, prompt = the grounded instruction.
    Modify {
        target: Ref,
        description: String,
        /// Per-run canvas override (user-requested only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<(u32, u32)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<u32>,
        #[serde(default)]
        loras: Vec<LoraSelection>,
    },
    /// Bind a registry LoRA to an artifact; it is auto-injected into every
    /// generation referencing the artifact (and its variants).
    #[serde(rename = "attach-lora")]
    AttachLora {
        target: Ref,
        /// The adapter to bind: a file name in the configured `lora_dir`,
        /// or the name of an existing registry entry.
        lora: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        multiplier: Option<f32>,
    },
    /// Adjust a bound adapter's weight.
    #[serde(rename = "lora-scale")]
    LoraScale { lora: String, multiplier: f32 },
    /// Unbind an adapter from the registry.
    #[serde(rename = "lora-remove")]
    LoraRemove { lora: String },
    /// Replace an artifact's description — its canonical look.
    ///
    /// The description is what `modify` re-states as the traits an edit must
    /// hold constant, so it is also what every later edit re-anchors to. An
    /// edit does not change it: `/modify c:mara "dye her hair auburn"`
    /// produces an auburn revision, but the next edit still grounds against
    /// the original description and the auburn washes out. `describe` is how
    /// a change becomes canonical and survives.
    Describe { target: Ref, description: String },
}

/// One layer reference for `compose`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComposeLayer {
    pub artifact: Ref,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<Ref>,
}

/// A serialized operation stack — the VLLM contract (function calling
/// against the closed vocabulary) and the `svs stack run` input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OperationStack {
    #[serde(default)]
    pub operations: Vec<Operation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpOrigin {
    User,
    Llm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpStatus {
    Proposed,
    Applied,
    Rejected,
    Reverted,
}

/// One operation-log entry — provenance for the artifact, the basis for
/// undo (state restore, never replay).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationLogEntry {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<Uuid>,
    pub op: Operation,
    pub origin: OpOrigin,
    pub status: OpStatus,
}

impl Operation {
    pub fn kind(&self) -> OperationKind {
        OperationKind::Primitive
    }

    /// A one-line human summary for logs and proposals.
    pub fn summary(&self) -> String {
        match self {
            Operation::Create {
                kind,
                description,
                name,
                size,
                ..
            } => {
                let label = name.clone().unwrap_or_default();
                let label = if label.is_empty() {
                    String::new()
                } else {
                    format!(" ({label})")
                };
                format!(
                    "create {kind} {description:?}{label}{}",
                    size.map(|(width, height)| format!(" [{width}x{height}]"))
                        .unwrap_or_default()
                )
            }
            Operation::Variant {
                target,
                description,
                name,
                axis,
                ..
            } => format!(
                "variant {target} {description:?}{}{}",
                name.as_deref()
                    .map(|name| format!(" ({name})"))
                    .unwrap_or_default(),
                axis.map(|axis| format!(" [{axis}]")).unwrap_or_default()
            ),
            Operation::Regenerate { target, prompt, .. } => format!(
                "regenerate {target}{}",
                prompt
                    .as_deref()
                    .map(|prompt| format!(" {prompt:?}"))
                    .unwrap_or_default()
            ),
            Operation::Set {
                target,
                steps,
                seed,
            } => {
                let mut parts = Vec::new();
                if let Some(steps) = steps {
                    parts.push(match steps {
                        Some(value) => format!("--steps {value}"),
                        None => "--steps auto".to_owned(),
                    });
                }
                if let Some(seed) = seed {
                    parts.push(match seed {
                        Some(value) => format!("--seed {value}"),
                        None => "--seed random".to_owned(),
                    });
                }
                format!("set {target} {}", parts.join(" "))
            }
            Operation::Resize {
                target,
                width,
                height,
            } => format!("resize {target} {width}x{height}"),
            Operation::Compose {
                scene,
                description,
                background,
                layers,
            } => format!(
                "compose {scene} {description:?} (background: {}, layers: {})",
                background
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "fresh".into()),
                layers.len()
            ),
            Operation::Draft { target, request } => format!("draft {target} {request:?}"),
            Operation::Modify {
                target,
                description,
                ..
            } => {
                format!("modify {target} {description:?}")
            }
            Operation::AttachLora {
                target,
                lora,
                multiplier,
            } => format!(
                "attach-lora {target} {lora}{}",
                multiplier
                    .map(|multiplier| format!(" [{multiplier}]"))
                    .unwrap_or_default()
            ),
            Operation::LoraScale { lora, multiplier } => {
                format!("lora-scale {lora} [{multiplier}]")
            }
            Operation::LoraRemove { lora } => format!("lora-remove {lora}"),
            Operation::Describe {
                target,
                description,
            } => format!("describe {target} {description:?}"),
        }
    }

    /// The artifact the operation primarily touches, if statically known.
    pub fn target_ref(&self) -> Option<&Ref> {
        match self {
            Operation::Variant { target, .. }
            | Operation::Regenerate { target, .. }
            | Operation::Resize { target, .. }
            | Operation::Set { target, .. }
            | Operation::Draft { target, .. }
            | Operation::Modify { target, .. }
            | Operation::AttachLora { target, .. }
            | Operation::Describe { target, .. } => Some(target),
            Operation::Compose { scene, .. } => Some(scene),
            Operation::Create { .. }
            | Operation::LoraScale { .. }
            | Operation::LoraRemove { .. } => None,
        }
    }
}

/// The result of compiling an operation.
#[derive(Clone, Debug)]
pub enum CompiledOp {
    /// A model-only operation; no pipeline (compose, and text-only create).
    Direct,
    /// A pipeline to execute against a backend.
    Pipeline(Pipeline),
}

/// The outcome of executing one operation.
#[derive(Clone, Debug)]
pub struct OpOutcome {
    pub artifact_id: Option<Uuid>,
    pub revision_id: Option<Uuid>,
    pub log_id: Option<Uuid>,
    pub run: Option<super::pipeline::PipelineRun>,
    pub status: OpStatus,
}

/// Execution options for [`execute`].
pub struct ExecuteOptions<'a> {
    /// The backend pipelines run against. `None` for model-only ops, or for
    /// a draft supplied via `manual_text`.
    pub backend: Option<&'a dyn GenerationBackend>,
    pub run: RunOptions,
    /// Per-run generation overrides (CLI flags); applied to the compiled
    /// pipeline and re-validated before execution.
    pub generation: GenerationOverrides,
    /// The configurable sizes and LoRA bindings the compilation resolves
    /// against.
    pub defaults: &'a GenerationDefaults,
    /// Manual text for a draft, bypassing the LLM proposal (soft-dependency
    /// degradation and the `--text` CLI path).
    pub manual_text: Option<&'a str>,
    pub origin: OpOrigin,
}

#[derive(Debug, Error)]
pub enum CompileError {
    #[error(transparent)]
    Ref(#[from] RefError),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error("op requires a backend (CreativeRuntime) but none was provided")]
    BackendRequired,
    #[error("create requires a non-empty description")]
    EmptyDescription,
    #[error("invalid size {width}x{height}: dimensions must be multiples of 32 within 256..=2048")]
    InvalidSize { width: u32, height: u32 },
    #[error("variant requires a character, environment, or object; got {0}")]
    NotVariantable(ArtifactKind),
    #[error("compose requires a scene; {target} is a {kind}")]
    NotAScene { target: Uuid, kind: ArtifactKind },
    #[error("compose background must be an environment; {target} is a {kind}")]
    NotAnEnvironment { target: Uuid, kind: ArtifactKind },
    #[error(
        "compose layers must reference characters, objects, or environments; {target} is a {kind}"
    )]
    BadLayerKind { target: Uuid, kind: ArtifactKind },
    #[error("artifact {0} has no completed image to edit")]
    NoActiveImage(Uuid),
    #[error("regenerate has no prompt and the artifact has neither an active revision nor a description")]
    NoPrompt(Uuid),
    #[error("lora multiplier {0} must be within -2..=2")]
    BadLoraMultiplier(f32),
    #[error("steps {0} must be within 1..=50")]
    BadSteps(u32),
    #[error("lora file {file:?} does not exist in the configured lora dir {dir:?}")]
    LoraFileMissing { file: String, dir: String },
    #[error("pipeline build failed: {0}")]
    Build(#[from] PipelineBuildError),
}

impl CodedError for CompileError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Ref(error) => error.code(),
            Self::Registry(error) => error.code(),
            Self::BackendRequired => ErrorCode::OpBackendRequired,
            Self::EmptyDescription => ErrorCode::OpEmptyDescription,
            Self::InvalidSize { .. } => ErrorCode::OpInvalidSize,
            Self::NotVariantable(_) => ErrorCode::OpNotVariantable,
            Self::NotAScene { .. } => ErrorCode::OpNotAScene,
            Self::NotAnEnvironment { .. } => ErrorCode::OpNotAnEnvironment,
            Self::BadLayerKind { .. } => ErrorCode::OpBadLayerKind,
            Self::NoActiveImage(_) => ErrorCode::OpNoActiveImage,
            Self::NoPrompt(_) => ErrorCode::OpNoPrompt,
            Self::BadLoraMultiplier(_) => ErrorCode::OpBadLoraMultiplier,
            Self::BadSteps(_) => ErrorCode::OpBadSteps,
            Self::LoraFileMissing { .. } => ErrorCode::OpLoraFileMissing,
            Self::Build(error) => error.code(),
        }
    }
}

#[derive(Debug, Error)]
pub enum OpError {
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error("registry error: {0}")]
    Registry(#[from] RegistryError),
    #[error("ref error: {0}")]
    Ref(#[from] RefError),
    #[error("operation failed while applying results: {0}")]
    Apply(String),
}

impl CodedError for OpError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Compile(error) => error.code(),
            Self::Pipeline(error) => error.code(),
            Self::Registry(error) => error.code(),
            Self::Ref(error) => error.code(),
            Self::Apply(_) => ErrorCode::OpApplyFailed,
        }
    }

    /// Delegates to the wrapped error so request context attached deep in
    /// the pipeline (prompt, model, seed, job id) survives to the surface.
    /// Note: the fallback must build the report inline — calling
    /// `CodedError::report(self)` here would resolve to this override
    /// (trait-qualified calls pick the impl for the concrete type).
    fn report(&self) -> ReportedError {
        match self {
            Self::Pipeline(error) => error.report(),
            _ => ReportedError {
                code: self.code(),
                details: self.to_string(),
                context: Vec::new(),
            },
        }
    }
}

/// The default cfg scale and sampling method applied to every compiled
/// pipeline come from the configurable [`GenerationDefaults`]; the builder
/// carries the model's own defaults until then.
///
/// Statically validates the operation against the registry and compiles it
/// to a pipeline. No IO; this is the static-validation surface. `defaults`
/// carries the configurable sizes, sampling, and LoRA bindings that shape the
/// generation (the canvas passes its live signal; the CLI passes the
/// loaded config).
///
/// Finalizes a compiled pipeline: the configurable sampling controls apply to
/// every visual op, then the static checks run once more.
fn finish(
    builder: super::pipeline::PipelineBuilder,
    defaults: &GenerationDefaults,
) -> Result<CompiledOp, CompileError> {
    let mut pipeline = builder.build()?;
    pipeline.params.cfg_scale = defaults.cfg_scale;
    pipeline.params.sample_method = defaults.sample_method.clone();
    Ok(CompiledOp::Pipeline(pipeline))
}

pub fn compile(
    registry: &ArtifactRegistry,
    op: &Operation,
    defaults: &GenerationDefaults,
) -> Result<CompiledOp, CompileError> {
    match op {
        Operation::Create {
            kind,
            description,
            size,
            steps,
            loras,
            ..
        } => {
            if description.trim().is_empty() {
                return Err(CompileError::EmptyDescription);
            }
            if let Some((width, height)) = size {
                if !super::pipeline::is_valid_size(*width, *height) {
                    return Err(CompileError::InvalidSize {
                        width: *width,
                        height: *height,
                    });
                }
            }
            if let Some(steps) = steps {
                validate_steps(*steps)?;
            }
            if !kind.is_visual() {
                return Ok(CompiledOp::Direct);
            }
            // Visual kinds generate their first image as part of create: the
            // description is the prompt, at the artifact's size (explicit
            // `--size`, the config override, or the kind default).
            let mut builder = super::pipeline::PipelineBuilder::new();
            if let Some((width, height)) = size.or_else(|| defaults.size_for(*kind)) {
                builder.size(width, height);
            }
            // No stored defaults to consult: the artifact does not exist
            // yet, so only the op's own values apply.
            if let Some(steps) = steps {
                builder.steps(*steps);
            }
            // The fresh artifact has no bindings yet; the kind-default
            // adapter (config) applies, then the per-run adapters.
            for lora in auto_loras(registry, None, *kind, defaults) {
                builder.lora(lora.path, lora.multiplier);
            }
            for lora in loras {
                // Adapters are sent to the native server under their
                // registered (top-level) name.
                let Some(registered) = defaults.loras.registered(&lora.path) else {
                    tracing::warn!(
                        file = %lora.path,
                        dir = %defaults.loras.dir.display(),
                        "per-run lora missing on disk; skipping",
                    );
                    continue;
                };
                builder.lora(registered, lora.multiplier);
            }
            builder.generate(PromptSource::Text(description.clone()), None);
            finish(builder, defaults)
        }
        Operation::Variant {
            target,
            description,
            size,
            steps,
            loras,
            ..
        } => {
            let base = registry.resolve(target)?;
            let base_artifact = registry
                .artifact(base)
                .ok_or(RegistryError::NoArtifact(base))?;
            if !matches!(
                base_artifact.kind,
                ArtifactKind::Character | ArtifactKind::Environment | ArtifactKind::Object
            ) {
                return Err(CompileError::NotVariantable(base_artifact.kind));
            }
            if description.trim().is_empty() {
                return Err(CompileError::EmptyDescription);
            }
            if let Some((width, height)) = size {
                if !super::pipeline::is_valid_size(*width, *height) {
                    return Err(CompileError::InvalidSize {
                        width: *width,
                        height: *height,
                    });
                }
            }
            if let Some(steps) = steps {
                validate_steps(*steps)?;
            }
            // The variant's first image is an edit of the base: the
            // description as prompt with the base's active image as the
            // reference, at the size the variant inherits (or the explicit
            // per-op size, which does not change the stored default).
            let mut builder = super::pipeline::PipelineBuilder::new();
            if let Some((width, height)) = size.or_else(|| {
                base_artifact
                    .default_size
                    .or_else(|| defaults.size_for(base_artifact.kind))
            }) {
                builder.size(width, height);
            }
            // Steps and seed are inherited from the base the same way size
            // is; the variant gets its own stored defaults only if set later.
            if let Some(steps) = steps.or(base_artifact.default_steps) {
                builder.steps(steps);
            }
            if let Some(seed) = base_artifact.default_seed {
                builder.seed(seed);
            }
            // The variant inherits the base's bound adapters and the kind
            // default (the base artifact stands in for the not-yet-created
            // variant — same bindings by construction), then the per-run
            // adapters.
            for lora in auto_loras(registry, Some(base_artifact), base_artifact.kind, defaults) {
                builder.lora(lora.path, lora.multiplier);
            }
            for lora in loras {
                // Adapters are sent to the native server under their
                // registered (top-level) name.
                let Some(registered) = defaults.loras.registered(&lora.path) else {
                    tracing::warn!(
                        file = %lora.path,
                        dir = %defaults.loras.dir.display(),
                        "per-run lora missing on disk; skipping",
                    );
                    continue;
                };
                builder.lora(registered, lora.multiplier);
            }
            let reference = registry
                .latest_image(base)
                .and_then(|revision| revision.asset_path.clone())
                .map(|path| builder.reference_image(path));
            // A variant description is a *delta* ("in heavy yellow
            // oilskins"), which is exactly the terse instruction that
            // collapses toward base-model priors — measured, an ungrounded
            // variant of a woman in her forties came back as a different
            // person of a different sex, with a scene the base never had.
            // Ground it against the base artifact, like `modify`.
            let fallback = ground_instruction(description, base_artifact);
            let grounded = builder.llm_ground(
                format!(
                    "Edit: {description}\nDescription of the image: {}",
                    base_artifact.description
                ),
                fallback,
            );
            builder.generate(PromptSource::Prompt(grounded), reference);
            finish(builder, defaults)
        }
        Operation::Compose {
            scene,
            description,
            background,
            layers,
        } => {
            let scene_id = registry.resolve(scene)?;
            let scene_artifact = registry
                .artifact(scene_id)
                .ok_or(RegistryError::NoArtifact(scene_id))?;
            if scene_artifact.kind != ArtifactKind::Scene {
                return Err(CompileError::NotAScene {
                    target: scene_id,
                    kind: scene_artifact.kind,
                });
            }
            if description.trim().is_empty() {
                return Err(CompileError::EmptyDescription);
            }
            if let Some(background_ref) = background {
                let background_id = registry.resolve(background_ref)?;
                let artifact = registry
                    .artifact(background_id)
                    .ok_or(RegistryError::NoArtifact(background_id))?;
                if artifact.kind != ArtifactKind::Environment {
                    return Err(CompileError::NotAnEnvironment {
                        target: background_id,
                        kind: artifact.kind,
                    });
                }
            }
            for layer in layers {
                let layer_id = registry.resolve(&layer.artifact)?;
                let artifact = registry
                    .artifact(layer_id)
                    .ok_or(RegistryError::NoArtifact(layer_id))?;
                if !matches!(
                    artifact.kind,
                    ArtifactKind::Character | ArtifactKind::Object | ArtifactKind::Environment
                ) {
                    return Err(CompileError::BadLayerKind {
                        target: layer_id,
                        kind: artifact.kind,
                    });
                }
                if let Some(variant_ref) = &layer.variant {
                    let _ = registry.resolve(variant_ref)?;
                }
            }
            Ok(CompiledOp::Direct)
        }
        Operation::Regenerate {
            target,
            prompt,
            size,
            steps,
            loras,
        } => {
            let id = registry.resolve(target)?;
            let artifact = registry.artifact(id).ok_or(RegistryError::NoArtifact(id))?;
            let active = registry.latest_image(id);
            let resolved_prompt = prompt.clone().or_else(|| {
                active.map(|revision| revision.prompt.clone()).or_else(|| {
                    if artifact.description.trim().is_empty() {
                        None
                    } else {
                        Some(artifact.description.clone())
                    }
                })
            });
            let Some(resolved_prompt) = resolved_prompt else {
                return Err(CompileError::NoPrompt(id));
            };
            if let Some((width, height)) = size {
                if !super::pipeline::is_valid_size(*width, *height) {
                    return Err(CompileError::InvalidSize {
                        width: *width,
                        height: *height,
                    });
                }
            }
            if let Some(steps) = steps {
                validate_steps(*steps)?;
            }

            let mut builder = super::pipeline::PipelineBuilder::new();
            // The artifact's default size (explicit `--size` at creation,
            // else the configurable kind default) drives its images; an
            // explicit per-op size (user-requested resize of the canvas)
            // wins without changing the stored default; per-run CLI
            // overrides still win last.
            if let Some((width, height)) = size
                .or(artifact.default_size)
                .or_else(|| defaults.size_for(artifact.kind))
            {
                builder.size(width, height);
            }
            // Same precedence for steps and seed: the artifact's stored
            // default applies unless the op names one. A pinned seed is what
            // makes an edit show only what the edit changed.
            if let Some(steps) = steps.or(artifact.default_steps) {
                builder.steps(steps);
            }
            if let Some(seed) = artifact.default_seed {
                builder.seed(seed);
            }
            for lora in auto_loras(registry, Some(artifact), artifact.kind, defaults) {
                builder.lora(lora.path, lora.multiplier);
            }
            for lora in loras {
                // Adapters are sent to the native server under their
                // registered (top-level) name.
                let Some(registered) = defaults.loras.registered(&lora.path) else {
                    tracing::warn!(
                        file = %lora.path,
                        dir = %defaults.loras.dir.display(),
                        "per-run lora missing on disk; skipping",
                    );
                    continue;
                };
                builder.lora(registered, lora.multiplier);
            }
            let reference = active
                .and_then(|revision| revision.asset_path.clone())
                .map(|path| builder.reference_image(path));
            // An explicit prompt is a delta ("make it warmer") and needs the
            // same grounding as a variant or a modify. A regenerate *without*
            // one replays the previous revision's prompt, which was already
            // grounded when it was made — grounding it twice would nest the
            // description inside itself.
            let prompt_source = match prompt {
                Some(delta) => {
                    let fallback = ground_instruction(delta, artifact);
                    PromptSource::Prompt(builder.llm_ground(
                        format!(
                            "Edit: {delta}\nDescription of the image: {}",
                            artifact.description
                        ),
                        fallback,
                    ))
                }
                None => PromptSource::Text(resolved_prompt),
            };
            builder.generate(prompt_source, reference);
            finish(builder, defaults)
        }
        Operation::Resize {
            target,
            width,
            height,
        } => {
            let id = registry.resolve(target)?;
            let artifact = registry.artifact(id).ok_or(RegistryError::NoArtifact(id))?;
            if !artifact.kind.is_visual() {
                return Err(CompileError::NotVariantable(artifact.kind));
            }
            if !super::pipeline::is_valid_size(*width, *height) {
                return Err(CompileError::InvalidSize {
                    width: *width,
                    height: *height,
                });
            }
            let active = registry
                .latest_image(id)
                .ok_or(CompileError::NoActiveImage(id))?;
            let asset = active
                .asset_path
                .clone()
                .ok_or(CompileError::NoActiveImage(id))?;

            // The subject is kept via the reference image; the prompt is the
            // last one used (the new canvas is the only intent).
            let mut builder = super::pipeline::PipelineBuilder::new();
            builder.size(*width, *height);
            for lora in auto_loras(registry, Some(artifact), artifact.kind, defaults) {
                builder.lora(lora.path, lora.multiplier);
            }
            let image = builder.reference_image(asset);
            builder.generate(PromptSource::Text(active.prompt.clone()), Some(image));
            finish(builder, defaults)
        }
        Operation::Modify {
            target,
            description,
            size,
            steps,
            loras,
        } => {
            let id = registry.resolve(target)?;
            let artifact = registry.artifact(id).ok_or(RegistryError::NoArtifact(id))?;
            let active = registry
                .latest_image(id)
                .ok_or(CompileError::NoActiveImage(id))?;
            let asset = active
                .asset_path
                .clone()
                .ok_or(CompileError::NoActiveImage(id))?;
            if description.trim().is_empty() {
                return Err(CompileError::EmptyDescription);
            }
            if let Some((width, height)) = size {
                if !super::pipeline::is_valid_size(*width, *height) {
                    return Err(CompileError::InvalidSize {
                        width: *width,
                        height: *height,
                    });
                }
            }
            if let Some(steps) = steps {
                validate_steps(*steps)?;
            }

            let mut builder = super::pipeline::PipelineBuilder::new();
            // Same default-size rule as regenerate: the artifact's own size
            // wins over the configurable kind default, an explicit per-op
            // size wins over both, per-run overrides still win last.
            if let Some((width, height)) = size
                .or(artifact.default_size)
                .or_else(|| defaults.size_for(artifact.kind))
            {
                builder.size(width, height);
            }
            // Same precedence for steps and seed: the artifact's stored
            // default applies unless the op names one. A pinned seed is what
            // makes an edit show only what the edit changed.
            if let Some(steps) = steps.or(artifact.default_steps) {
                builder.steps(steps);
            }
            if let Some(seed) = artifact.default_seed {
                builder.seed(seed);
            }
            for lora in auto_loras(registry, Some(artifact), artifact.kind, defaults) {
                builder.lora(lora.path, lora.multiplier);
            }
            for lora in loras {
                // Adapters are sent to the native server under their
                // registered (top-level) name.
                let Some(registered) = defaults.loras.registered(&lora.path) else {
                    tracing::warn!(
                        file = %lora.path,
                        dir = %defaults.loras.dir.display(),
                        "per-run lora missing on disk; skipping",
                    );
                    continue;
                };
                builder.lora(registered, lora.multiplier);
            }
            // A `modify` is a single instruction edit: the artifact's active
            // revision is the reference image and the grounded instruction is
            // the prompt. Localization comes from the instruction's semantics
            // against the in-context reference; there is no segmentation, no
            // mask and no composite.
            //
            // The instruction is grounded before it is sent — see
            // `ground_instruction`. A bare instruction collapses the edit into
            // base-model priors.
            let fallback = ground_instruction(description, artifact);
            // The artifact's name is an internal handle, not a visual trait:
            // including it gets it echoed back into the instruction.
            let grounded = builder.llm_ground(
                format!(
                    "Edit: {description}\nDescription of the image: {}",
                    artifact.description
                ),
                fallback,
            );
            let image = builder.reference_image(asset);
            builder.generate(PromptSource::Prompt(grounded), Some(image));
            finish(builder, defaults)
        }
        Operation::Draft { target, request } => {
            registry.resolve(target)?;
            if request.trim().is_empty() {
                return Err(CompileError::EmptyDescription);
            }
            let mut builder = super::pipeline::PipelineBuilder::new();
            let text = builder.llm_draft(request.clone());
            builder.confirm_text(text);
            finish(builder, defaults)
        }
        Operation::AttachLora {
            target, multiplier, ..
        } => {
            let id = registry.resolve(target)?;
            let artifact = registry.artifact(id).ok_or(RegistryError::NoArtifact(id))?;
            if !matches!(
                artifact.kind,
                ArtifactKind::Character | ArtifactKind::Environment | ArtifactKind::Object
            ) {
                return Err(CompileError::NotVariantable(artifact.kind));
            }
            if let Some(multiplier) = multiplier {
                validate_lora_multiplier(*multiplier)?;
            }
            Ok(CompiledOp::Direct)
        }
        Operation::LoraScale { lora, multiplier } => {
            registry.resolve_lora(lora)?;
            validate_lora_multiplier(*multiplier)?;
            Ok(CompiledOp::Direct)
        }
        Operation::LoraRemove { lora } => {
            registry.resolve_lora(lora)?;
            Ok(CompiledOp::Direct)
        }
        Operation::Describe {
            target,
            description,
        } => {
            registry.resolve(target)?;
            if description.trim().is_empty() {
                return Err(CompileError::EmptyDescription);
            }
            Ok(CompiledOp::Direct)
        }
        Operation::Set { target, .. } => {
            registry.resolve(target)?;
            Ok(CompiledOp::Direct)
        }
    }
}

fn validate_lora_multiplier(multiplier: f32) -> Result<(), CompileError> {
    if (-2.0..=2.0).contains(&multiplier) {
        Ok(())
    } else {
        Err(CompileError::BadLoraMultiplier(multiplier))
    }
}

/// Step counts share the pipeline bounds (1..=50).
fn validate_steps(steps: u32) -> Result<(), CompileError> {
    if (1..=50).contains(&steps) {
        Ok(())
    } else {
        Err(CompileError::BadSteps(steps))
    }
}

/// Anchors an edit instruction so the model edits *this* image instead of
/// inventing a new one.
///
/// A terse instruction carries almost no signal, and the reference
/// conditioning alone is too weak to hold the subject: measured 2026-08-10,
/// `"make him smile"` against a photo of a man returned an *anime
/// illustration of a different person*, twice, at two aspect ratios. The
/// identity-edit card names the failure — unusual or under-specified input
/// "may drift toward base model priors".
///
/// Two things pull it back, and both are needed. Naming the medium ("a
/// photograph") recovers the medium but not the subject — the same prompt
/// with only generic grounding returned a photograph of a *different* man in
/// a different setting. Naming the specific invariants is what preserves
/// identity, and the artifact's own description is exactly that list, so it
/// is restated as the thing to hold constant.
///
/// Caveat: a description that contradicts the edit ("long dark hair" +
/// "give her a bob cut") asks the model for both and it splits the
/// difference. Keep traits the edit will override out of the description.
fn ground_instruction(instruction: &str, artifact: &Artifact) -> String {
    let subject = match artifact.kind {
        ArtifactKind::Character => "the same person",
        _ => "the same subject",
    };
    let instruction = instruction.trim().trim_end_matches('.');
    let description = artifact.description.trim();
    if description.is_empty() {
        return format!(
            "{instruction}. A photograph of {subject}. Keep everything else unchanged."
        );
    }
    format!(
        "{instruction}. A photograph of {subject}: {description}. Keep everything else unchanged."
    )
}

/// The LoRAs an auto-injected generation must carry, first-wins by
/// resolved path: project entries bound to the artifact (or its variant
/// base chain), then the config kind binding. A configured adapter
/// that is missing on disk is skipped with a warning rather than failing
/// the generation.
fn auto_loras(
    registry: &ArtifactRegistry,
    artifact: Option<&Artifact>,
    kind: ArtifactKind,
    defaults: &GenerationDefaults,
) -> Vec<LoraSelection> {
    let mut selections: Vec<LoraSelection> = Vec::new();
    let mut push = |file: &str, multiplier: f32| {
        // The request must name the adapter as the native server registers
        // it (top-level file of the lora directory — see
        // `LoraBindings::registered`); existence is validated here.
        let Some(path) = defaults.loras.registered(file) else {
            tracing::warn!(
                file = %file,
                dir = %defaults.loras.dir.display(),
                "lora missing on disk; skipping",
            );
            return;
        };
        if !selections.iter().any(|selection| selection.path == path) {
            selections.push(LoraSelection { path, multiplier });
        }
    };
    if let Some(artifact) = artifact {
        for entry in registry.loras_bound_to(artifact.id) {
            push(&entry.path, entry.multiplier);
        }
    }
    if let Some(binding) = defaults.loras.kinds.binding_for(kind) {
        push(&binding.file, defaults.loras.multiplier);
    }
    selections
}

/// Executes an operation: compile → (run pipeline) → apply results →
/// log. User-typed ops apply immediately; LLM proposals are gated before
/// this function is called. A rejected checkpoint logs the op as rejected;
/// a failed pipeline fails the revision and surfaces the error.
pub async fn execute(
    registry: &mut ArtifactRegistry,
    op: &Operation,
    options: &ExecuteOptions<'_>,
) -> Result<OpOutcome, OpError> {
    match op {
        Operation::Create {
            kind,
            description,
            name,
            size,
            ..
        } => {
            // Sizes are honored as given (the planner prompt tells the LLM
            // to include one only when the user asks for a specific size).
            let id = registry.create_artifact(
                *kind,
                CreateArtifact {
                    name: unique_name(
                        registry,
                        &name.clone().unwrap_or_else(|| derive_name(description)),
                    ),
                    description: description.clone(),
                    default_size: *size,
                    ..Default::default()
                },
            )?;
            if kind.is_visual() {
                // The first image is part of create: compile the pipeline
                // against the fresh artifact and run it like regenerate.
                let CompiledOp::Pipeline(pipeline) = compile(registry, op, options.defaults)?
                else {
                    unreachable!("visual create compiles to a pipeline");
                };
                return run_generation(registry, op, id, pipeline, description.clone(), options)
                    .await;
            }
            let log_id = registry.push_log(op.clone(), options.origin, OpStatus::Applied, Some(id));
            Ok(OpOutcome {
                artifact_id: Some(id),
                revision_id: None,
                log_id: Some(log_id),
                run: None,
                status: OpStatus::Applied,
            })
        }
        Operation::Variant {
            target,
            description,
            name,
            axis,
            ..
        } => {
            let base = registry.resolve(target)?;
            let base_artifact = registry
                .artifact(base)
                .ok_or(RegistryError::NoArtifact(base))?;
            // An explicit `--name` wins; otherwise variants are auto-named
            // from the base so every system-created key stays unique:
            // `mia-outfit`, `mia-rain-gear`.
            let name = match name {
                Some(explicit) => explicit.clone(),
                None if base_artifact.name.trim().is_empty() => derive_name(description),
                None => {
                    if let Some(axis) = axis {
                        format!("{}-{}", base_artifact.name, axis.as_str())
                    } else {
                        format!("{}-{}", base_artifact.name, derive_name(description))
                    }
                }
            };
            let id = registry.create_artifact(
                base_artifact.kind,
                CreateArtifact {
                    name: unique_name(registry, &name),
                    description: description.clone(),
                    variant_of: Some(base),
                    axis: *axis,
                    // Variants inherit the base's size so regenerating a
                    // variant keeps the same canvas (a base with no explicit
                    // size follows the kind default).
                    default_size: base_artifact.default_size,
                    ..Default::default()
                },
            )?;
            // The first image is part of variant: an edit of the base (its
            // active image as reference), run like regenerate.
            let CompiledOp::Pipeline(pipeline) = compile(registry, op, options.defaults)? else {
                unreachable!("variant compiles to a pipeline");
            };
            return run_generation(registry, op, id, pipeline, description.clone(), options).await;
        }
        Operation::Resize {
            target,
            width,
            height,
        } => {
            let id = registry.resolve(target)?;
            let description = registry
                .artifact(id)
                .ok_or(RegistryError::NoArtifact(id))?
                .description
                .clone();
            // The new canvas becomes the artifact's default (future
            // generations inherit it)…
            registry
                .artifact_mut(id)
                .ok_or(RegistryError::NoArtifact(id))?
                .default_size = Some((*width, *height));
            let CompiledOp::Pipeline(pipeline) = compile(registry, op, options.defaults)? else {
                unreachable!("resize compiles to a pipeline");
            };
            // …and the active image is regenerated at it right away.
            let revision_prompt = registry
                .latest_image(id)
                .map(|revision| revision.prompt.clone())
                .unwrap_or(description);
            run_generation(registry, op, id, pipeline, revision_prompt, options).await
        }
        Operation::Compose {
            scene,
            description,
            background,
            layers,
        } => {
            let scene_id = registry.resolve(scene)?;
            let beat_id = registry.create_artifact(
                ArtifactKind::Beat,
                CreateArtifact {
                    name: unique_name(registry, &derive_name(description)),
                    description: description.clone(),
                    parent: Some(scene_id),
                    ..Default::default()
                },
            )?;
            let mut composition_layers: Vec<Layer> = Vec::new();
            let mut position = 0u32;
            if let Some(background_ref) = background {
                let background_id = registry.resolve(background_ref)?;
                composition_layers.push(Layer {
                    id: Uuid::new_v4(),
                    position,
                    artifact_ref: background_id,
                    variant_ref: None,
                    role: LayerRole::Backdrop,
                    anchor: None,
                });
                position += 1;
            }
            for layer in layers {
                let layer_id = registry.resolve(&layer.artifact)?;
                let variant_id = layer
                    .variant
                    .as_ref()
                    .map(|variant| registry.resolve(variant))
                    .transpose()?;
                composition_layers.push(Layer {
                    id: Uuid::new_v4(),
                    position,
                    artifact_ref: layer_id,
                    variant_ref: variant_id,
                    role: LayerRole::Dynamic,
                    anchor: None,
                });
                position += 1;
            }
            registry.set_composition(
                beat_id,
                super::BeatComposition {
                    mode: super::CompositionMode::Baked,
                    layers: composition_layers,
                },
            )?;
            let log_id =
                registry.push_log(op.clone(), options.origin, OpStatus::Applied, Some(beat_id));
            Ok(OpOutcome {
                artifact_id: Some(beat_id),
                revision_id: None,
                log_id: Some(log_id),
                run: None,
                status: OpStatus::Applied,
            })
        }
        Operation::Draft { target, request } => {
            if let Some(text) = options.manual_text {
                let id = registry.resolve(target)?;
                registry.add_draft(id, request.clone(), text.to_owned(), true)?;
                let log_id =
                    registry.push_log(op.clone(), options.origin, OpStatus::Applied, Some(id));
                return Ok(OpOutcome {
                    artifact_id: Some(id),
                    revision_id: None,
                    log_id: Some(log_id),
                    run: None,
                    status: OpStatus::Applied,
                });
            }
            let id = registry.resolve(target)?;
            let CompiledOp::Pipeline(pipeline) = compile(registry, op, options.defaults)? else {
                unreachable!("draft always compiles to a pipeline");
            };
            let backend = options.backend.ok_or(CompileError::BackendRequired)?;
            let run = match pipeline.run(backend, &options.run).await {
                Ok(run) => run,
                Err(PipelineError::Rejected) => {
                    let log_id =
                        registry.push_log(op.clone(), options.origin, OpStatus::Rejected, Some(id));
                    return Ok(OpOutcome {
                        artifact_id: Some(id),
                        revision_id: None,
                        log_id: Some(log_id),
                        run: None,
                        status: OpStatus::Rejected,
                    });
                }
                Err(error) => return Err(OpError::Pipeline(error)),
            };
            let text = run
                .outputs
                .iter()
                .find(|output| output.label == "llm")
                .and_then(|output| output.text.clone())
                .ok_or_else(|| OpError::Apply("draft produced no text".to_owned()))?;
            registry.add_draft(id, request.clone(), text, true)?;
            let log_id = registry.push_log(op.clone(), options.origin, OpStatus::Applied, Some(id));
            Ok(OpOutcome {
                artifact_id: Some(id),
                revision_id: None,
                log_id: Some(log_id),
                run: Some(run),
                status: OpStatus::Applied,
            })
        }
        Operation::Regenerate { .. } | Operation::Modify { .. } => {
            let id = registry.resolve(op.target_ref().expect("visual ops have a target"))?;
            let CompiledOp::Pipeline(pipeline) = compile(registry, op, options.defaults)? else {
                unreachable!("visual ops always compile to pipelines");
            };
            let revision_prompt = match op {
                Operation::Regenerate { prompt, .. } => prompt
                    .clone()
                    .or_else(|| {
                        registry
                            .latest_image(id)
                            .map(|revision| revision.prompt.clone())
                    })
                    .unwrap_or_default(),
                Operation::Modify { description, .. } => description.clone(),
                _ => unreachable!(),
            };
            run_generation(registry, op, id, pipeline, revision_prompt, options).await
        }
        Operation::AttachLora {
            target,
            lora,
            multiplier,
        } => {
            let id = registry.resolve(target)?;
            let artifact = registry.artifact(id).ok_or(RegistryError::NoArtifact(id))?;
            if !matches!(
                artifact.kind,
                ArtifactKind::Character | ArtifactKind::Environment | ArtifactKind::Object
            ) {
                return Err(CompileError::NotVariantable(artifact.kind).into());
            }
            // The arg is a file name in the lora dir, or the name/path of an
            // existing registry entry (re-attaching reuses its file).
            let file = registry
                .resolve_lora(lora)
                .ok()
                .and_then(|entry_id| {
                    registry
                        .loras
                        .iter()
                        .find(|entry| entry.id == entry_id)
                        .map(|entry| entry.path.clone())
                })
                .unwrap_or_else(|| lora.clone());
            let Some(resolved) = options.defaults.loras.resolve(&file) else {
                return Err(CompileError::LoraFileMissing {
                    file,
                    dir: options.defaults.loras.dir.display().to_string(),
                }
                .into());
            };
            // The display name carries the relative path without the
            // extension (`external-models/char-model`) so two same-stem
            // adapters in different subfolders stay distinct.
            let name = resolved
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| {
                    resolved
                        .parent()
                        .and_then(|parent| parent.strip_prefix(&options.defaults.loras.dir).ok())
                        .map(|parent| parent.join(stem).to_string_lossy().into_owned())
                        .unwrap_or_else(|| stem.to_owned())
                })
                .unwrap_or_else(|| file.clone());
            let multiplier = multiplier.unwrap_or(1.0);
            registry.attach_lora(id, name, file, None, multiplier)?;
            let log_id = registry.push_log(op.clone(), options.origin, OpStatus::Applied, Some(id));
            Ok(OpOutcome {
                artifact_id: Some(id),
                revision_id: None,
                log_id: Some(log_id),
                run: None,
                status: OpStatus::Applied,
            })
        }
        Operation::LoraScale { lora, multiplier } => {
            let entry_id = registry.resolve_lora(lora)?;
            let artifact_id = registry
                .loras
                .iter()
                .find(|entry| entry.id == entry_id)
                .and_then(|entry| entry.artifact_id);
            registry.scale_lora(entry_id, *multiplier)?;
            let log_id =
                registry.push_log(op.clone(), options.origin, OpStatus::Applied, artifact_id);
            Ok(OpOutcome {
                artifact_id,
                revision_id: None,
                log_id: Some(log_id),
                run: None,
                status: OpStatus::Applied,
            })
        }
        Operation::LoraRemove { lora } => {
            let entry_id = registry.resolve_lora(lora)?;
            let artifact_id = registry
                .loras
                .iter()
                .find(|entry| entry.id == entry_id)
                .and_then(|entry| entry.artifact_id);
            registry.remove_lora(entry_id)?;
            let log_id =
                registry.push_log(op.clone(), options.origin, OpStatus::Applied, artifact_id);
            Ok(OpOutcome {
                artifact_id,
                revision_id: None,
                log_id: Some(log_id),
                run: None,
                status: OpStatus::Applied,
            })
        }
        Operation::Set {
            target,
            steps,
            seed,
        } => {
            let id = registry.resolve(target)?;
            if let Some(steps) = steps {
                registry.set_default_steps(id, *steps)?;
            }
            if let Some(seed) = seed {
                registry.set_default_seed(id, *seed)?;
            }
            let log_id = registry.push_log(op.clone(), options.origin, OpStatus::Applied, Some(id));
            Ok(OpOutcome {
                artifact_id: Some(id),
                revision_id: None,
                log_id: Some(log_id),
                run: None,
                status: OpStatus::Applied,
            })
        }
        Operation::Describe {
            target,
            description,
        } => {
            let id = registry.resolve(target)?;
            if description.trim().is_empty() {
                return Err(CompileError::EmptyDescription.into());
            }
            registry.set_description(id, description.trim())?;
            let log_id = registry.push_log(op.clone(), options.origin, OpStatus::Applied, Some(id));
            Ok(OpOutcome {
                artifact_id: Some(id),
                revision_id: None,
                log_id: Some(log_id),
                run: None,
                status: OpStatus::Applied,
            })
        }
    }
}

/// The shared visual-op execution: start a revision on the artifact, run
/// the pipeline against the backend, then finish (promote the image) or
/// fail (record the error; the artifact keeps its last good image). A
/// rejected checkpoint cancels the revision and logs the op as rejected; a
/// failed pipeline fails the revision and surfaces the error. One op-log
/// entry is written with `op` on success and rejection.
async fn run_generation(
    registry: &mut ArtifactRegistry,
    op: &Operation,
    id: Uuid,
    mut pipeline: Pipeline,
    revision_prompt: String,
    options: &ExecuteOptions<'_>,
) -> Result<OpOutcome, OpError> {
    pipeline
        .apply_generation_overrides(&options.generation)
        .map_err(CompileError::Build)?;
    let backend = options.backend.ok_or(CompileError::BackendRequired)?;
    let seed = options.run.seed.or(pipeline.params().seed);
    let model = Some(pipeline.params().model.clone());
    let revision_id =
        registry.start_revision(id, revision_prompt, seed, model, RevisionOrigin::of(op))?;

    let run = match pipeline.run(backend, &options.run).await {
        Ok(run) => run,
        Err(PipelineError::Rejected) => {
            registry.cancel_revision(id, revision_id)?;
            let log_id =
                registry.push_log(op.clone(), options.origin, OpStatus::Rejected, Some(id));
            return Ok(OpOutcome {
                artifact_id: Some(id),
                revision_id: Some(revision_id),
                log_id: Some(log_id),
                run: None,
                status: OpStatus::Rejected,
            });
        }
        Err(error) => {
            registry.fail_revision(id, revision_id, error.to_string())?;
            return Err(OpError::Pipeline(error));
        }
    };

    let asset = run
        .final_image()
        .ok_or_else(|| OpError::Apply("pipeline produced no final image".to_owned()))?;
    // Import the final into the persistent asset store so the revision
    // survives the work dir's cleanup.
    let stored = backend.import_asset(asset.clone()).await?;
    registry.finish_revision(id, revision_id, stored)?;
    let log_id = registry.push_log(op.clone(), options.origin, OpStatus::Applied, Some(id));
    Ok(OpOutcome {
        artifact_id: Some(id),
        revision_id: Some(revision_id),
        log_id: Some(log_id),
        run: Some(run),
        status: OpStatus::Applied,
    })
}

/// Derives a memorable key/display name from a description: lowercase
/// alphanumeric words joined by hyphens (e.g. "Mia, a lighthouse keeper" →
/// "mia-a-lighthouse-keeper"). These are the `c:` refs users type.
pub fn derive_name(description: &str) -> String {
    let mut slug = String::new();
    for word in description.split_whitespace().take(4) {
        if !slug.is_empty() {
            slug.push('-');
        }
        for character in word.chars() {
            if character.is_ascii_alphanumeric() {
                slug.push(character.to_ascii_lowercase());
            } else if !slug.ends_with('-') {
                slug.push('-');
            }
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(40);
    if slug.is_empty() {
        "untitled".to_owned()
    } else {
        slug
    }
}

/// Ensures a system-derived name stays unique (case-insensitive): appends
/// `-2`, `-3`, … until the key is free, so every auto-created `c:` ref
/// resolves. Explicit `--name` choices are respected as-is (duplicates then
/// surface as ambiguous refs).
pub fn unique_name(registry: &ArtifactRegistry, base: &str) -> String {
    let taken = |candidate: &str| {
        registry
            .artifacts
            .iter()
            .any(|artifact| artifact.name.eq_ignore_ascii_case(candidate))
    };
    if !taken(base) {
        return base.to_owned();
    }
    let mut counter = 2;
    loop {
        let candidate = format!("{base}-{counter}");
        if !taken(&candidate) {
            return candidate;
        }
        counter += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
    };

    use super::*;
    use crate::registry::pipeline::Decision;
    use clipforge_inference::sd::{DEFAULT_CFG_SCALE, DEFAULT_SAMPLE_METHOD};

    /// A fresh temp folder for generated asset paths in tests (the pipeline
    /// checks reference images exist on disk).
    fn test_assets_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("svs-ops-assets-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("assets dir should create");
        dir
    }

    /// Empty generation defaults (built-in sizes, no bindings).
    fn test_defaults() -> GenerationDefaults {
        GenerationDefaults {
            sizes: Default::default(),
            steps: 25,
            cfg_scale: DEFAULT_CFG_SCALE,
            sample_method: DEFAULT_SAMPLE_METHOD.to_owned(),
            loras: Default::default(),
        }
    }

    fn registry_with(artifacts: &[(ArtifactKind, &str)]) -> ArtifactRegistry {
        let mut registry = ArtifactRegistry::new();
        for (kind, description) in artifacts {
            registry
                .create_artifact(
                    *kind,
                    CreateArtifact {
                        name: String::new(),
                        description: (*description).to_owned(),
                        ..Default::default()
                    },
                )
                .expect("test artifact should create");
        }
        registry
    }

    #[test]
    fn operation_stacks_deserialize_from_the_closed_json_vocabulary() {
        let json = r#"{
            "operations": [
                {"op": "create", "kind": "character", "description": "Mia, a lighthouse keeper", "name": "mia"},
                {"op": "variant", "target": "c:1234abcd", "description": "rain gear", "axis": "outfit"},
                {"op": "regenerate", "target": "c:1234abcd", "prompt": "make it warmer"},
                {"op": "compose", "scene": "c:1234abcd", "description": "Mia at the lantern", "background": "c:abcd1234", "layers": [{"artifact": "c:1234abcd", "variant": "c:abcd1234"}]},
                {"op": "draft", "target": "c:1234abcd", "request": "write the opening beat"},
                {"op": "modify", "target": "c:1234abcd", "description": "change her hair"}
            ]
        }"#;
        let stack: OperationStack = serde_json::from_str(json).expect("stack should parse");
        assert_eq!(stack.operations.len(), 6);
        assert!(
            matches!(&stack.operations[0], Operation::Create { kind: ArtifactKind::Character, name: Some(name), .. } if name == "mia")
        );
        assert!(matches!(
            &stack.operations[1],
            Operation::Variant {
                axis: Some(VariantAxis::Outfit),
                ..
            }
        ));
        assert!(
            matches!(&stack.operations[3], Operation::Compose { layers, .. } if layers.len() == 1)
        );
        assert!(matches!(
            &stack.operations[5],
            Operation::Modify { description, .. } if description == "change her hair"
        ));
    }

    #[test]
    fn unknown_operations_and_axes_are_rejected() {
        let unknown: Result<OperationStack, _> =
            serde_json::from_str(r#"{"operations": [{"op": "explode", "description": "x"}]}"#);
        assert!(unknown.is_err());

        let bad_axis: Result<OperationStack, _> = serde_json::from_str(
            r#"{"operations": [{"op": "variant", "target": "c:1234abcd", "description": "x", "axis": "glamour"}]}"#,
        );
        assert!(bad_axis.is_err());
    }

    #[test]
    fn compile_validates_op_kinds_and_resolves_refs() {
        let registry = registry_with(&[(ArtifactKind::Story, "a story")]);
        let story = registry.artifacts[0].id;

        // Compose against a story (not a scene) is a compile error.
        let op = Operation::Compose {
            scene: Ref::new(story.to_string()),
            description: "a beat".into(),
            background: None,
            layers: Vec::new(),
        };
        assert!(matches!(
            compile(&registry, &op, &test_defaults()),
            Err(CompileError::NotAScene { .. })
        ));

        // Variant on a story is a compile error.
        let op = Operation::Variant {
            target: Ref::new(story.to_string()),
            description: "a variant".into(),
            name: None,
            axis: None,
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        assert!(matches!(
            compile(&registry, &op, &test_defaults()),
            Err(CompileError::NotVariantable(ArtifactKind::Story))
        ));

        // Modify without an active image is a compile error.
        let op = Operation::Modify {
            target: Ref::new(story.to_string()),
            description: "change it".into(),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        assert!(matches!(
            compile(&registry, &op, &test_defaults()),
            Err(CompileError::NoActiveImage(_))
        ));

        // Unknown refs surface as RefError.
        let op = Operation::Draft {
            target: Ref::new("c:00000000"),
            request: "write something".into(),
        };
        assert!(matches!(
            compile(&registry, &op, &test_defaults()),
            Err(CompileError::Ref(_))
        ));
    }

    #[test]
    fn regenerate_without_a_prompt_uses_the_active_revision_prompt() {
        let mut registry = registry_with(&[(ArtifactKind::Character, "Mia")]);
        let id = registry.artifacts[0].id;
        let revision = registry
            .start_revision(
                id,
                "Mia on the cliff".into(),
                Some(1),
                None,
                RevisionOrigin::Canon,
            )
            .unwrap();
        registry
            .finish_revision(id, revision, "assets/mia.png".into())
            .unwrap();

        let op = Operation::Regenerate {
            target: Ref::new(registry.short_id(id)),
            prompt: None,
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let CompiledOp::Pipeline(pipeline) = compile(&registry, &op, &test_defaults()).unwrap()
        else {
            panic!("regenerate must compile to a pipeline");
        };
        assert!(matches!(
            pipeline.steps()[1],
            crate::registry::pipeline::Step::Generate { .. }
        ));
        assert!(pipeline.params().seed.is_none());
    }

    #[test]
    fn regenerate_uses_the_artifact_default_size() {
        let mut registry = registry_with(&[(ArtifactKind::Character, "Mia")]);
        let id = registry.artifacts[0].id;
        registry.artifact_mut(id).unwrap().default_size = Some((512, 768));
        let revision = registry
            .start_revision(
                id,
                "Mia on the cliff".into(),
                Some(1),
                None,
                RevisionOrigin::Canon,
            )
            .unwrap();
        registry
            .finish_revision(id, revision, "assets/mia.png".into())
            .unwrap();

        let op = Operation::Regenerate {
            target: Ref::new(registry.short_id(id)),
            prompt: None,
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let CompiledOp::Pipeline(pipeline) = compile(&registry, &op, &test_defaults()).unwrap()
        else {
            panic!("regenerate must compile to a pipeline");
        };
        assert_eq!(pipeline.params().width, 512);
        assert_eq!(pipeline.params().height, 768);

        // An artifact without an explicit size follows its kind default.
        let registry = registry_with(&[(ArtifactKind::Object, "Lantern")]);
        let lantern = registry.artifacts[0].id;
        let op = Operation::Regenerate {
            target: Ref::new(registry.short_id(lantern)),
            prompt: Some("the brass lantern".into()),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let CompiledOp::Pipeline(pipeline) = compile(&registry, &op, &test_defaults()).unwrap()
        else {
            panic!("regenerate must compile to a pipeline");
        };
        assert_eq!(pipeline.params().width, 768);
        assert_eq!(pipeline.params().height, 768);
    }

    #[test]
    fn create_stores_the_size_and_variant_inherits_it() {
        let mut registry = ArtifactRegistry::new();
        let assets = test_assets_dir();
        let backend = VisualBackend::new(assets.join("mia.png"));
        let create = Operation::Create {
            kind: ArtifactKind::Character,
            description: "Mia, a lighthouse keeper".into(),
            name: Some("mia".into()),
            size: Some((512, 768)),
            steps: None,
            loras: Vec::new(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &create, &options))
            .unwrap();
        let mia = outcome.artifact_id.unwrap();
        assert_eq!(
            registry.artifact(mia).unwrap().default_size,
            Some((512, 768))
        );

        let variant = Operation::Variant {
            target: Ref::new(registry.short_id(mia)),
            description: "in rain gear".into(),
            name: None,
            axis: Some(VariantAxis::Outfit),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &variant, &options))
            .unwrap();
        let variant_id = outcome.artifact_id.unwrap();
        assert_eq!(
            registry.artifact(variant_id).unwrap().default_size,
            Some((512, 768)),
            "variants inherit the base's size"
        );
    }

    #[test]
    fn create_and_variant_apply_and_log() {
        let mut registry = ArtifactRegistry::new();
        let assets = test_assets_dir();
        let backend = VisualBackend::new(assets.join("mia.png"));
        let create = Operation::Create {
            kind: ArtifactKind::Character,
            description: "Mia, a lighthouse keeper".into(),
            name: Some("mia".into()),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &create, &options))
            .unwrap();
        let mia = outcome.artifact_id.unwrap();
        assert_eq!(
            registry.artifact(mia).unwrap().kind,
            ArtifactKind::Character
        );
        // The first image came with the create.
        assert!(registry.latest_image(mia).is_some());
        assert_eq!(registry.log.len(), 1);
        assert_eq!(registry.log[0].status, OpStatus::Applied);

        let variant = Operation::Variant {
            target: Ref::new(registry.short_id(mia)),
            description: "in rain gear".into(),
            name: None,
            axis: Some(VariantAxis::Outfit),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &variant, &options))
            .unwrap();
        let variant_id = outcome.artifact_id.unwrap();
        let artifact = registry.artifact(variant_id).unwrap();
        assert_eq!(artifact.variant_of, Some(mia));
        assert_eq!(artifact.variant_axis, Some(VariantAxis::Outfit));
        // Variants are auto-named so their c: ref stays unique.
        assert_eq!(artifact.name, "mia-outfit");
        assert_eq!(registry.ref_of(variant_id), "c:mia-outfit");
        // The variant's first image came with the variant.
        assert!(registry.latest_image(variant_id).is_some());
        assert_eq!(registry.log.len(), 2);
    }

    #[test]
    fn variant_honors_an_explicit_name() {
        let mut registry = ArtifactRegistry::new();
        let assets = test_assets_dir();
        let mia_asset = assets.join("mia.png");
        std::fs::write(&mia_asset, b"png").expect("dummy image should write");
        let base = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mia".into(),
                    description: "Mia, a lighthouse keeper".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let base_revision = registry
            .start_revision(
                base,
                "Mia on the cliff".into(),
                Some(1),
                None,
                RevisionOrigin::Canon,
            )
            .unwrap();
        registry
            .finish_revision(base, base_revision, mia_asset.display().to_string())
            .unwrap();

        let backend = VisualBackend::new(assets.join("mia-rain.png"));
        let variant = Operation::Variant {
            target: Ref::new(registry.short_id(base)),
            description: "in yellow rain gear".into(),
            name: Some("mia-rain-gear".into()),
            axis: Some(VariantAxis::Outfit),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &variant, &options))
            .unwrap();
        let variant_id = outcome.artifact_id.unwrap();
        let artifact = registry.artifact(variant_id).unwrap();
        assert_eq!(artifact.name, "mia-rain-gear");
        assert_eq!(registry.ref_of(variant_id), "c:mia-rain-gear");
    }

    #[test]
    fn llm_proposed_ops_carry_the_requested_size_steps_and_loras() {
        let mut registry = ArtifactRegistry::new();
        let assets = test_assets_dir();
        let backend = VisualBackend::new(assets.join("mia.png"));
        let lora_dir = assets.join("loras");
        std::fs::create_dir_all(lora_dir.join("characters")).expect("lora dir should create");
        std::fs::write(lora_dir.join("characters/char.safetensors"), b"lora")
            .expect("dummy lora should write");
        let create = Operation::Create {
            kind: ArtifactKind::Character,
            description: "Mia, a lighthouse keeper".into(),
            name: Some("mia".into()),
            size: Some((1024, 1024)),
            steps: Some(8),
            loras: vec![LoraSelection {
                path: "characters/char.safetensors".into(),
                multiplier: 0.8,
            }],
        };
        let mut test_defaults = test_defaults();
        test_defaults.loras.dir = lora_dir;
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::Llm,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &create, &options))
            .unwrap();
        let mia = outcome.artifact_id.unwrap();
        // A user-requested size on an LLM op is honored (stored + generated).
        assert_eq!(
            registry.artifact(mia).unwrap().default_size,
            Some((1024, 1024))
        );
        let request = &backend.requests()[0];
        assert_eq!((request.width, request.height), (1024, 1024));
        assert_eq!(request.steps, 8);
        assert_eq!(request.loras.len(), 1);
        assert_eq!(request.loras[0].path, "characters-char.safetensors");
        assert_eq!(request.loras[0].multiplier, 0.8);
    }

    #[test]
    fn regenerate_honors_per_run_size_steps_and_loras() {
        let assets = test_assets_dir();
        let mia_asset = assets.join("mia.png");
        std::fs::write(&mia_asset, b"png").expect("dummy image should write");
        let mut registry = ArtifactRegistry::new();
        let mia = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mia".into(),
                    description: "Mia".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let revision = registry
            .start_revision(
                mia,
                "Mia on the cliff".into(),
                Some(1),
                None,
                RevisionOrigin::Canon,
            )
            .unwrap();
        registry
            .finish_revision(mia, revision, mia_asset.display().to_string())
            .unwrap();

        let backend = VisualBackend::new(assets.join("mia-regen.png"));
        let lora_dir = assets.join("loras");
        std::fs::create_dir_all(lora_dir.join("styles")).expect("lora dir should create");
        std::fs::write(lora_dir.join("styles/style.safetensors"), b"lora")
            .expect("dummy lora should write");
        let op = Operation::Regenerate {
            target: Ref::new(registry.short_id(mia)),
            prompt: Some("warmer".into()),
            size: Some((768, 768)),
            steps: Some(12),
            loras: vec![LoraSelection {
                path: "styles/style.safetensors".into(),
                multiplier: 1.0,
            }],
        };
        let mut test_defaults = test_defaults();
        test_defaults.loras.dir = lora_dir;
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &op, &options))
            .unwrap();
        let request = &backend.requests()[0];
        assert_eq!((request.width, request.height), (768, 768));
        assert_eq!(request.steps, 12);
        assert!(request
            .loras
            .iter()
            .any(|lora| lora.path == "styles-style.safetensors"));
        // The artifact's stored default size is untouched (per-run only).
        assert_eq!(registry.artifact(mia).unwrap().default_size, None);
    }

    #[test]
    fn resize_sets_the_default_size_and_regenerates_at_it() {
        let assets = test_assets_dir();
        let mia_asset = assets.join("mia.png");
        std::fs::write(&mia_asset, b"png").expect("dummy image should write");
        let mut registry = ArtifactRegistry::new();
        let mia = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mia".into(),
                    description: "Mia, the keeper".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let revision = registry
            .start_revision(
                mia,
                "Mia on the cliff".into(),
                Some(1),
                None,
                RevisionOrigin::Canon,
            )
            .unwrap();
        registry
            .finish_revision(mia, revision, mia_asset.display().to_string())
            .unwrap();

        let backend = VisualBackend::new(assets.join("mia-1024.png"));
        let op = Operation::Resize {
            target: Ref::new(registry.short_id(mia)),
            width: 1024,
            height: 1024,
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &op, &options))
            .unwrap();
        assert_eq!(outcome.status, OpStatus::Applied);
        assert_eq!(
            registry.artifact(mia).unwrap().default_size,
            Some((1024, 1024)),
            "resize stores the new default size"
        );
        // The image regenerates at the new canvas, subject kept via the
        // reference image and the last prompt.
        let request = &backend.requests()[0];
        assert_eq!((request.width, request.height), (1024, 1024));
        assert_eq!(request.prompt, "Mia on the cliff");
        assert!(request.reference_image_path.is_some());

        // A resize on a text artifact is rejected.
        let story = registry
            .create_artifact(
                ArtifactKind::Story,
                CreateArtifact {
                    name: "tale".into(),
                    description: "a story".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let bad = Operation::Resize {
            target: Ref::new(registry.short_id(story)),
            width: 512,
            height: 512,
        };
        assert!(matches!(
            compile(&registry, &bad, &test_defaults),
            Err(CompileError::NotVariantable(ArtifactKind::Story))
        ));
    }

    #[test]
    fn attach_lora_binds_and_is_auto_injected_into_generations() {
        let assets = test_assets_dir();
        let lora_dir = assets.join("loras");
        std::fs::create_dir_all(&lora_dir).expect("lora dir should create");
        let mia_lora = lora_dir.join("mia-v2.safetensors");
        std::fs::write(&mia_lora, b"lora").expect("dummy lora should write");

        let defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 25,
            cfg_scale: DEFAULT_CFG_SCALE,
            sample_method: DEFAULT_SAMPLE_METHOD.to_owned(),
            loras: crate::registry::LoraBindings {
                dir: lora_dir.clone(),
                kinds: Default::default(),
                multiplier: 1.0,
            },
        };

        let mut registry = ArtifactRegistry::new();
        let mia = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mia".into(),
                    description: "Mia, the keeper".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let mia_asset = assets.join("mia.png");
        std::fs::write(&mia_asset, b"png").expect("dummy image should write");
        let revision = registry
            .start_revision(
                mia,
                "Mia on the cliff".into(),
                Some(1),
                None,
                RevisionOrigin::Canon,
            )
            .unwrap();
        registry
            .finish_revision(mia, revision, mia_asset.display().to_string())
            .unwrap();

        let attach = Operation::AttachLora {
            target: Ref::new(registry.short_id(mia)),
            lora: "mia-v2.safetensors".into(),
            multiplier: Some(0.8),
        };
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &attach, &options))
            .unwrap();
        assert_eq!(outcome.status, OpStatus::Applied);
        let entry = &registry.loras[0];
        assert_eq!(entry.path, "mia-v2.safetensors");
        assert_eq!(entry.multiplier, 0.8);
        assert_eq!(entry.artifact_id, Some(mia));

        // A nested adapter (e.g. through a symlinked folder) attaches too,
        // and its name carries the relative path so same-stem files in
        // different folders stay distinct.
        std::fs::create_dir_all(lora_dir.join("external-models")).unwrap();
        std::fs::write(
            lora_dir.join("external-models/char-style.safetensors"),
            b"lora",
        )
        .unwrap();
        let nested = Operation::AttachLora {
            target: Ref::new(registry.short_id(mia)),
            lora: "external-models/char-style.safetensors".into(),
            multiplier: None,
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &nested, &options))
            .unwrap();
        let nested_entry = registry.loras.last().unwrap();
        assert_eq!(nested_entry.path, "external-models/char-style.safetensors");
        assert_eq!(nested_entry.name, "external-models/char-style");
        assert_eq!(nested_entry.multiplier, 1.0);

        // A plain regenerate carries the bound adapter.
        let backend = VisualBackend::new(assets.join("mia-regen.png"));
        let regenerate = Operation::Regenerate {
            target: Ref::new(registry.short_id(mia)),
            prompt: Some("warmer".into()),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &regenerate, &options))
            .unwrap();
        let request = &backend.requests()[0];
        assert_eq!(request.loras.len(), 2, "both bound adapters");
        assert_eq!(request.loras[0].path, "mia-v2.safetensors");
        assert_eq!(request.loras[0].multiplier, 0.8);
        assert!(request
            .loras
            .iter()
            .any(|lora| lora.path == "external-models-char-style.safetensors"));

        // A `modify` is a single instruction edit: it carries the bound
        // adapters, references the active revision, and neither segments nor
        // composites.
        let modify = Operation::Modify {
            target: Ref::new(registry.short_id(mia)),
            description: "change her hair".into(),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let CompiledOp::Pipeline(modify_pipeline) = compile(&registry, &modify, &defaults).unwrap()
        else {
            panic!("modify must compile to a pipeline");
        };
        assert_eq!(
            modify_pipeline.params().loras.len(),
            2,
            "both bound adapters ride the instruction edit"
        );
        assert!(
            modify_pipeline.steps().iter().any(|step| matches!(
                step,
                crate::registry::pipeline::Step::Llm {
                    kind: crate::registry::pipeline::LlmStepKind::Ground { .. }
                }
            )),
            "the instruction is grounded before it is sent"
        );
        assert!(
            modify_pipeline.steps().iter().any(|step| matches!(
                step,
                crate::registry::pipeline::Step::Generate {
                    reference: Some(_),
                    ..
                }
            )),
            "the edit references the active image"
        );
        assert_eq!(modify_pipeline.params().cfg_scale, DEFAULT_CFG_SCALE);
        assert_eq!(
            modify_pipeline.params().sample_method,
            DEFAULT_SAMPLE_METHOD
        );
    }

    #[test]
    fn describe_rewrites_the_anchor_that_edits_ground_against() {
        let defaults = test_defaults();
        let mut registry = ArtifactRegistry::new();
        let id = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mara".into(),
                    description: "Mara, long dark hair, grey sweater".into(),
                    ..Default::default()
                },
            )
            .unwrap();

        // Editing the image never moves the description, so a second edit
        // still grounds against "long dark hair" and the first one washes
        // out. `describe` is the only way to make a change canonical.
        let grounded = ground_instruction("dye her hair auburn", registry.artifact(id).unwrap());
        assert!(grounded.contains("long dark hair"), "{grounded}");

        let op = Operation::Describe {
            target: Ref::new(registry.short_id(id)),
            description: "  Mara, long auburn hair, grey sweater  ".into(),
        };
        assert!(matches!(
            compile(&registry, &op, &defaults).unwrap(),
            CompiledOp::Direct
        ));
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &op, &options))
            .unwrap();
        assert_eq!(outcome.artifact_id, Some(id));
        assert_eq!(outcome.status, OpStatus::Applied);
        assert!(outcome.revision_id.is_none(), "describe makes no image");

        let artifact = registry.artifact(id).unwrap();
        assert_eq!(artifact.description, "Mara, long auburn hair, grey sweater");
        let grounded = ground_instruction("give her a bob cut", artifact);
        assert!(grounded.contains("auburn"), "{grounded}");
        assert!(!grounded.contains("dark"), "{grounded}");

        // An empty description would silently destroy the anchor.
        let empty = Operation::Describe {
            target: Ref::new(registry.short_id(id)),
            description: "   ".into(),
        };
        assert!(matches!(
            compile(&registry, &empty, &defaults),
            Err(CompileError::EmptyDescription)
        ));
    }

    #[test]
    fn grounding_anchors_a_terse_instruction_to_its_artifact() {
        let mut registry = ArtifactRegistry::new();
        let id = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "marcus".into(),
                    description: "Marcus, a young man at dusk, black turtleneck".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let artifact = registry.artifact(id).unwrap();

        // A bare instruction under-specifies the render and it collapses
        // into base-model priors (measured: "make him smile" returned an
        // anime illustration of a different person). The grounding states
        // the medium and restates the artifact's own traits as invariants.
        let grounded = ground_instruction("make him smile", artifact);
        assert!(grounded.starts_with("make him smile."), "{grounded}");
        assert!(
            grounded.contains("A photograph of the same person"),
            "{grounded}"
        );
        assert!(grounded.contains("black turtleneck"), "{grounded}");
        assert!(
            grounded.ends_with("Keep everything else unchanged."),
            "{grounded}"
        );

        // A description-less artifact still gets the medium anchor, which
        // on its own recovers "a photograph" but not the likeness.
        let bare = registry
            .create_artifact(
                ArtifactKind::Object,
                CreateArtifact {
                    name: "lantern".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let grounded = ground_instruction("make it brighter.", registry.artifact(bare).unwrap());
        assert_eq!(
            grounded,
            "make it brighter. A photograph of the same subject. Keep everything else unchanged."
        );
    }

    #[test]
    fn revision_origin_follows_the_operation() {
        use crate::registry::Ref;
        let target = Ref::new("c:mia");
        // Renders the description.
        assert_eq!(
            RevisionOrigin::of(&Operation::Regenerate {
                target: target.clone(),
                prompt: None,
                size: None,
                steps: None,
                loras: Vec::new(),
            }),
            RevisionOrigin::Canon
        );
        // Renders a delta against it.
        assert_eq!(
            RevisionOrigin::of(&Operation::Regenerate {
                target: target.clone(),
                prompt: Some("make it warmer".into()),
                size: None,
                steps: None,
                loras: Vec::new(),
            }),
            RevisionOrigin::Experiment
        );
        assert_eq!(
            RevisionOrigin::of(&Operation::Modify {
                target: target.clone(),
                description: "give her a bob cut".into(),
                size: None,
                steps: None,
                loras: Vec::new(),
            }),
            RevisionOrigin::Experiment
        );
        // A variant's first image is canon for the artifact it creates.
        assert_eq!(
            RevisionOrigin::of(&Operation::Variant {
                target,
                description: "in oilskins".into(),
                name: None,
                axis: None,
                size: None,
                steps: None,
                loras: Vec::new(),
            }),
            RevisionOrigin::Canon
        );
    }

    #[test]
    fn attach_lora_requires_the_file_to_exist_and_a_visual_target() {
        let assets = test_assets_dir();
        let lora_dir = assets.join("loras");
        std::fs::create_dir_all(&lora_dir).expect("lora dir should create");
        let defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 25,
            cfg_scale: DEFAULT_CFG_SCALE,
            sample_method: DEFAULT_SAMPLE_METHOD.to_owned(),
            loras: crate::registry::LoraBindings {
                dir: lora_dir,
                ..Default::default()
            },
        };
        let mut registry = ArtifactRegistry::new();
        let story = registry
            .create_artifact(
                ArtifactKind::Story,
                CreateArtifact {
                    name: "tale".into(),
                    description: "a story".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let character = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mia".into(),
                    description: "Mia".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        // Missing file.
        let missing = Operation::AttachLora {
            target: Ref::new(registry.short_id(character)),
            lora: "nobody.safetensors".into(),
            multiplier: None,
        };
        assert!(matches!(
            tokio::runtime::Runtime::new().unwrap().block_on(execute(
                &mut registry,
                &missing,
                &options
            )),
            Err(OpError::Compile(CompileError::LoraFileMissing { .. }))
        ));
        // Non-visual target is a compile error before the file check.
        let story_attach = Operation::AttachLora {
            target: Ref::new(registry.short_id(story)),
            lora: "nobody.safetensors".into(),
            multiplier: None,
        };
        assert!(matches!(
            tokio::runtime::Runtime::new().unwrap().block_on(execute(
                &mut registry,
                &story_attach,
                &options
            )),
            Err(OpError::Compile(CompileError::NotVariantable(
                ArtifactKind::Story
            )))
        ));
        // Non-visual target.
        let entry_id = registry
            .attach_lora(
                story,
                "nobody".into(),
                "nobody.safetensors".into(),
                None,
                1.0,
            )
            .expect("registry attach is not gated on the file");
        let scale = Operation::LoraScale {
            lora: "nobody".into(),
            multiplier: 0.5,
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &scale, &options))
            .unwrap();
        assert_eq!(
            registry
                .loras
                .iter()
                .find(|entry| entry.id == entry_id)
                .unwrap()
                .multiplier,
            0.5
        );
        let remove = Operation::LoraRemove {
            lora: "nobody".into(),
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &remove, &options))
            .unwrap();
        assert!(registry.loras.is_empty());
    }

    #[test]
    fn text_only_create_needs_no_backend() {
        let mut registry = ArtifactRegistry::new();
        let create = Operation::Create {
            kind: ArtifactKind::Story,
            description: "a lighthouse story".into(),
            name: Some("tale".into()),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &create, &options))
            .unwrap();
        assert_eq!(outcome.status, OpStatus::Applied);
        assert_eq!(outcome.revision_id, None, "text-only kinds carry no image");
        assert!(outcome.run.is_none());
        assert_eq!(registry.log.len(), 1);
    }

    #[test]
    fn visual_create_generates_the_first_image() {
        let mut registry = ArtifactRegistry::new();
        let backend = VisualBackend::new(test_assets_dir().join("mia.png"));
        let create = Operation::Create {
            kind: ArtifactKind::Character,
            description: "Mia, a lighthouse keeper".into(),
            name: Some("mia".into()),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &create, &options))
            .unwrap();
        let mia = outcome.artifact_id.unwrap();
        let asset = registry
            .latest_image(mia)
            .and_then(|revision| revision.asset_path.clone());
        assert!(asset.is_some(), "create generates the first image");
        assert!(Path::new(asset.as_deref().unwrap()).is_file());
        // One log entry, the create itself.
        assert_eq!(registry.log.len(), 1);
        assert_eq!(registry.log[0].status, OpStatus::Applied);
        // The request: description as prompt, kind default size, no reference.
        let request = &backend.requests()[0];
        assert_eq!(request.prompt, "Mia, a lighthouse keeper");
        assert_eq!((request.width, request.height), (512, 768));
        assert!(request.reference_image_path.is_none());
    }

    #[test]
    fn variant_first_image_references_the_base() {
        let mut registry = ArtifactRegistry::new();
        let assets = test_assets_dir();
        let mia_asset = assets.join("mia.png");
        std::fs::write(&mia_asset, b"png").expect("dummy image should write");
        let base = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mia".into(),
                    description: "Mia, a lighthouse keeper".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let base_revision = registry
            .start_revision(
                base,
                "Mia on the cliff".into(),
                Some(1),
                None,
                RevisionOrigin::Canon,
            )
            .unwrap();
        registry
            .finish_revision(base, base_revision, mia_asset.display().to_string())
            .unwrap();

        let backend = VisualBackend::new(assets.join("mia-outfit.png"));
        let variant = Operation::Variant {
            target: Ref::new(registry.short_id(base)),
            description: "in yellow rain gear".into(),
            name: None,
            axis: Some(VariantAxis::Outfit),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &variant, &options))
            .unwrap();
        let variant_id = outcome.artifact_id.unwrap();
        assert!(
            registry.latest_image(variant_id).is_some(),
            "variant generates its first image"
        );
        // The first image is an edit of the base: its active image as the
        // reference, the base's size, and the variant description *grounded*
        // against the base. Sending the bare delta lets the render drift off
        // the base entirely — measured, an ungrounded variant of a woman in
        // her forties returned a different person of a different sex.
        let request = &backend.requests()[0];
        assert_eq!(
            request.prompt,
            "in yellow rain gear. A photograph of the same person: Mia, a lighthouse keeper. \
             Keep everything else unchanged."
        );
        assert_eq!(
            request.reference_image_path.as_deref(),
            Some(mia_asset.to_str().unwrap())
        );
        assert_eq!((request.width, request.height), (512, 768));

        // A base without an image still generates, just without a reference.
        let mut registry = ArtifactRegistry::new();
        let base = registry
            .create_artifact(
                ArtifactKind::Object,
                CreateArtifact {
                    name: "lantern".into(),
                    description: "an antique brass lantern".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let backend = VisualBackend::new(test_assets_dir().join("lantern-rusted.png"));
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let variant = Operation::Variant {
            target: Ref::new(registry.short_id(base)),
            description: "rusted".into(),
            name: None,
            axis: Some(VariantAxis::Weather),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &variant, &options))
            .unwrap();
        let variant_id = outcome.artifact_id.unwrap();
        assert!(registry.latest_image(variant_id).is_some());
        assert!(backend.requests()[0].reference_image_path.is_none());
    }

    #[test]
    fn visual_create_without_a_backend_is_a_compile_error() {
        let mut registry = ArtifactRegistry::new();
        let create = Operation::Create {
            kind: ArtifactKind::Character,
            description: "Mia".into(),
            name: None,
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &create, &options))
            .unwrap_err();
        assert!(matches!(
            error,
            OpError::Compile(CompileError::BackendRequired)
        ));
        // The artifact is still created before the generation step.
        assert_eq!(registry.artifacts.len(), 1);
    }

    #[test]
    fn duplicate_system_names_are_disambiguated() {
        let mut registry = ArtifactRegistry::new();
        let mia = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mia".into(),
                    description: "Mia".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let backend = VisualBackend::new(test_assets_dir().join("mia-outfit.png"));
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let variant = Operation::Variant {
            target: Ref::new("c:mia"),
            description: "in rain gear".into(),
            name: None,
            axis: Some(VariantAxis::Outfit),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let first = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &variant, &options))
            .unwrap()
            .artifact_id
            .unwrap();
        let second = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &variant, &options))
            .unwrap()
            .artifact_id
            .unwrap();

        assert_eq!(registry.artifact(first).unwrap().name, "mia-outfit");
        assert_eq!(registry.artifact(second).unwrap().name, "mia-outfit-2");
        // Both keys resolve without ambiguity.
        assert_eq!(registry.resolve(&Ref::new("c:mia-outfit")).unwrap(), first);
        assert_eq!(
            registry.resolve(&Ref::new("c:mia-outfit-2")).unwrap(),
            second
        );
        let _ = mia;
    }

    #[test]
    fn compose_creates_a_beat_with_a_composition() {
        let mut registry = ArtifactRegistry::new();
        let story = registry
            .create_artifact(
                ArtifactKind::Story,
                CreateArtifact {
                    name: "S".into(),
                    description: "S".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let scene = registry
            .create_artifact(
                ArtifactKind::Scene,
                CreateArtifact {
                    name: "Kitchen".into(),
                    description: "The kitchen".into(),
                    parent: Some(story),
                    ..Default::default()
                },
            )
            .unwrap();
        let mia = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mia".into(),
                    description: "Mia".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let house = registry
            .create_artifact(
                ArtifactKind::Environment,
                CreateArtifact {
                    name: "House".into(),
                    description: "A house".into(),
                    ..Default::default()
                },
            )
            .unwrap();

        let op = Operation::Compose {
            scene: Ref::new(registry.short_id(scene)),
            description: "Mia lights the lantern".into(),
            background: Some(Ref::new(registry.short_id(house))),
            layers: vec![ComposeLayer {
                artifact: Ref::new(registry.short_id(mia)),
                variant: None,
            }],
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &op, &options))
            .unwrap();
        let beat_id = outcome.artifact_id.unwrap();
        let beat = registry.artifact(beat_id).unwrap();
        assert_eq!(beat.kind, ArtifactKind::Beat);
        assert_eq!(beat.parent_id, Some(scene));
        let composition = beat.composition.as_ref().expect("beat has a composition");
        assert_eq!(composition.layers.len(), 2);
        assert_eq!(composition.layers[0].role, LayerRole::Backdrop);
        assert_eq!(composition.layers[0].artifact_ref, house);
        assert_eq!(composition.layers[1].role, LayerRole::Dynamic);
        assert_eq!(composition.layers[1].artifact_ref, mia);
    }

    // ------------------------------------------------------- pipeline-backed ops

    /// A minimal fake backend returning canned text for the draft LLM step.
    struct DraftBackend;
    impl GenerationBackend for DraftBackend {
        fn generate(
            &self,
            _request: &clipforge_inference::sd::GenerateRequest,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<crate::registry::pipeline::GenerateResponse, PipelineError>,
        > {
            unreachable!()
        }
        fn llm_draft(
            &self,
            _request: &str,
        ) -> futures_util::future::BoxFuture<'_, Result<Option<String>, PipelineError>> {
            Box::pin(async move { Ok(Some("The keeper winds the lamp.".to_owned())) })
        }
    }

    /// A fake generation backend: captures requests and returns a canned
    /// image path for every generate call.
    struct VisualBackend {
        generated: PathBuf,
        requests: Mutex<Vec<clipforge_inference::sd::GenerateRequest>>,
    }

    impl VisualBackend {
        fn new(generated: PathBuf) -> Self {
            Self {
                generated,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<clipforge_inference::sd::GenerateRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl GenerationBackend for VisualBackend {
        fn generate(
            &self,
            request: &clipforge_inference::sd::GenerateRequest,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<crate::registry::pipeline::GenerateResponse, PipelineError>,
        > {
            let request = request.clone();
            let generated = self.generated.clone();
            Box::pin(async move {
                self.requests.lock().unwrap().push(request.clone());
                // Write a real file so a later op using this revision as a
                // reference image passes the pipeline's disk check.
                if let Some(parent) = generated.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&generated, b"png").expect("dummy image should write");
                Ok(crate::registry::pipeline::GenerateResponse {
                    status: "completed".to_owned(),
                    image_path: Some(generated.display().to_string()),
                    seed: request.seed,
                    width: Some(request.width),
                    height: Some(request.height),
                    error: None,
                })
            })
        }
        fn llm_draft(
            &self,
            _request: &str,
        ) -> futures_util::future::BoxFuture<'_, Result<Option<String>, PipelineError>> {
            unreachable!()
        }
    }

    #[test]
    fn draft_applies_approved_llm_text_to_the_artifact() {
        let mut registry = registry_with(&[(ArtifactKind::Story, "a story")]);
        let id = registry.artifacts[0].id;
        let op = Operation::Draft {
            target: Ref::new(registry.short_id(id)),
            request: "write the opening".into(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: Some(&DraftBackend),
            run: RunOptions {
                work_dir: std::env::temp_dir(),
                approvals: crate::registry::pipeline::ApprovalPolicy::auto(),
                seed: None,
            },
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &op, &options))
            .unwrap();
        assert_eq!(outcome.status, OpStatus::Applied);
        let drafts = &registry.artifact(id).unwrap().drafts;
        assert_eq!(drafts.len(), 1);
        assert!(drafts[0].approved);
        assert_eq!(drafts[0].text, "The keeper winds the lamp.");
        assert_eq!(registry.log.len(), 1);
    }

    #[test]
    fn draft_with_manual_text_bypasses_the_llm_and_the_checkpoint() {
        let mut registry = registry_with(&[(ArtifactKind::Story, "a story")]);
        let id = registry.artifacts[0].id;
        let op = Operation::Draft {
            target: Ref::new(registry.short_id(id)),
            request: "write the opening".into(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: None, // no backend needed
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: Some("Hand-written opening."),
            origin: OpOrigin::User,
        };
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &op, &options))
            .unwrap();
        assert_eq!(outcome.status, OpStatus::Applied);
        assert_eq!(
            registry.artifact(id).unwrap().drafts[0].text,
            "Hand-written opening."
        );
    }

    #[test]
    fn rejected_draft_logs_as_rejected_without_applying() {
        let mut registry = registry_with(&[(ArtifactKind::Story, "a story")]);
        let id = registry.artifacts[0].id;
        let op = Operation::Draft {
            target: Ref::new(registry.short_id(id)),
            request: "write the opening".into(),
        };
        let test_defaults = test_defaults();
        let options = ExecuteOptions {
            backend: Some(&DraftBackend),
            run: RunOptions {
                work_dir: std::env::temp_dir(),
                approvals: crate::registry::pipeline::ApprovalPolicy::Interactive(Box::new(|_| {
                    Decision::Reject
                })),
                seed: None,
            },
            generation: Default::default(),
            defaults: &test_defaults,
            manual_text: None,
            origin: OpOrigin::Llm,
        };
        let result =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(execute(&mut registry, &op, &options));
        let outcome = result.expect("rejection is a first-class outcome");
        assert_eq!(outcome.status, OpStatus::Rejected);
        assert!(registry.artifact(id).unwrap().drafts.is_empty());
        assert_eq!(registry.log.len(), 1);
        assert_eq!(registry.log[0].status, OpStatus::Rejected);
        assert_eq!(registry.log[0].origin, OpOrigin::Llm);
    }
}
