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
//! `draft`, and `modify` (the mask-edit path: segment → confirm mask →
//! inpaint with the composite fallback).
//!
//! Execution model: user-typed ops apply immediately and are logged; LLM
//! proposals (`stack propose`) are gated by approval before execution.
//! Checkpoint steps inside pipelines (mask confirmation, draft text
//! approval) always block per the approval policy. Undo is state restore
//! (`ArtifactRegistry::snapshot`/`restore`), never re-execution.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::errors::{CodedError, ErrorCode, ReportedError};
use crate::vision::LoraSelection;

use super::{
    pipeline::{
        GenerationBackend, GenerationOverrides, Pipeline, PipelineBuildError, PipelineError,
        PromptSource, RunOptions,
    },
    Artifact, ArtifactKind, ArtifactRegistry, CreateArtifact, GenerationDefaults, Layer, LayerRole,
    Ref, RefError, RegistryError, StoredMask, VariantAxis,
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
    /// Mask-guided regional edit: segment → confirm mask → inpaint
    /// (composite fallback). Explicit prompts skip the LLM plan step.
    Modify {
        target: Ref,
        description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mask_prompt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inpaint_prompt: Option<String>,
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
        }
    }

    /// The artifact the operation primarily touches, if statically known.
    pub fn target_ref(&self) -> Option<&Ref> {
        match self {
            Operation::Variant { target, .. }
            | Operation::Regenerate { target, .. }
            | Operation::Resize { target, .. }
            | Operation::Draft { target, .. }
            | Operation::Modify { target, .. }
            | Operation::AttachLora { target, .. } => Some(target),
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
    #[error("modify requires both --mask-prompt and --inpaint-prompt (or an LLM to plan them)")]
    IncompleteModify,
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
            Self::IncompleteModify => ErrorCode::OpIncompleteModify,
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

/// Statically validates the operation against the registry and compiles it
/// to a pipeline. No IO; this is the static-validation surface. `defaults`
/// carries the configurable sizes and LoRA bindings that shape the
/// generation (the canvas passes its live signal; the CLI passes the
/// loaded config).
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
            if let Some(steps) = steps {
                builder.steps(*steps);
            }
            // The fresh artifact has no bindings yet; the kind-default
            // adapter (config) applies, then the per-run adapters.
            for lora in auto_loras(registry, None, *kind, defaults, false) {
                builder.lora(lora.path, lora.multiplier);
            }
            for lora in loras {
                builder.lora(lora.path.clone(), lora.multiplier);
            }
            builder.generate(PromptSource::Text(description.clone()), None, None);
            Ok(CompiledOp::Pipeline(builder.build()?))
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
            if let Some(steps) = steps {
                builder.steps(*steps);
            }
            // The variant inherits the base's bound adapters and the kind
            // default (the base artifact stands in for the not-yet-created
            // variant — same bindings by construction), then the per-run
            // adapters.
            for lora in auto_loras(
                registry,
                Some(base_artifact),
                base_artifact.kind,
                defaults,
                false,
            ) {
                builder.lora(lora.path, lora.multiplier);
            }
            for lora in loras {
                builder.lora(lora.path.clone(), lora.multiplier);
            }
            let reference = registry
                .latest_image(base)
                .and_then(|revision| revision.asset_path.clone())
                .map(|path| builder.reference_image(path));
            builder.generate(PromptSource::Text(description.clone()), reference, None);
            Ok(CompiledOp::Pipeline(builder.build()?))
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
            if let Some(steps) = steps {
                builder.steps(*steps);
            }
            for lora in auto_loras(registry, Some(artifact), artifact.kind, defaults, false) {
                builder.lora(lora.path, lora.multiplier);
            }
            for lora in loras {
                builder.lora(lora.path.clone(), lora.multiplier);
            }
            let reference = active
                .and_then(|revision| revision.asset_path.clone())
                .map(|path| builder.reference_image(path));
            builder.generate(PromptSource::Text(resolved_prompt), reference, None);
            Ok(CompiledOp::Pipeline(builder.build()?))
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
            for lora in auto_loras(registry, Some(artifact), artifact.kind, defaults, false) {
                builder.lora(lora.path, lora.multiplier);
            }
            let image = builder.reference_image(asset);
            builder.generate(PromptSource::Text(active.prompt.clone()), Some(image), None);
            Ok(CompiledOp::Pipeline(builder.build()?))
        }
        Operation::Modify {
            target,
            description,
            mask_prompt,
            inpaint_prompt,
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
            if let Some(steps) = steps {
                builder.steps(*steps);
            }
            // Mask-guided edits additionally carry the configured inpaint
            // adapter (e.g. INPAINTKREA), after the artifact's bindings,
            // then the per-run adapters.
            for lora in auto_loras(registry, Some(artifact), artifact.kind, defaults, true) {
                builder.lora(lora.path, lora.multiplier);
            }
            for lora in loras {
                builder.lora(lora.path.clone(), lora.multiplier);
            }
            let image = builder.reference_image(asset);
            let (mask_source, inpaint_source) = match (mask_prompt, inpaint_prompt) {
                (Some(mask), Some(inpaint)) => (
                    PromptSource::Text(mask.clone()),
                    PromptSource::Text(inpaint.clone()),
                ),
                (None, None) => {
                    let plan = builder.llm_plan(format!(
                        "{description}\n\nArtifact: {} — {}",
                        artifact.name, artifact.description
                    ));
                    (
                        PromptSource::PlanPart(plan, super::pipeline::PlanPart::MaskPrompt),
                        PromptSource::PlanPart(plan, super::pipeline::PlanPart::InpaintPrompt),
                    )
                }
                _ => return Err(CompileError::IncompleteModify),
            };
            let candidates = builder.segment(image, mask_source);
            let mask = builder.confirm_mask(candidates);
            builder.inpaint(inpaint_source, image, mask);
            Ok(CompiledOp::Pipeline(builder.build()?))
        }
        Operation::Draft { target, request } => {
            registry.resolve(target)?;
            if request.trim().is_empty() {
                return Err(CompileError::EmptyDescription);
            }
            let mut builder = super::pipeline::PipelineBuilder::new();
            let text = builder.llm_draft(request.clone());
            builder.confirm_text(text);
            Ok(CompiledOp::Pipeline(builder.build()?))
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

/// The LoRAs an auto-injected generation must carry, first-wins by
/// resolved path: project entries bound to the artifact (or its variant
/// base chain), then the config kind binding, then the inpaint use-case
/// adapter for mask-guided edits. A configured adapter that is missing on
/// disk is skipped with a warning rather than failing the generation.
fn auto_loras(
    registry: &ArtifactRegistry,
    artifact: Option<&Artifact>,
    kind: ArtifactKind,
    defaults: &GenerationDefaults,
    inpaint: bool,
) -> Vec<crate::vision::LoraSelection> {
    let mut selections: Vec<crate::vision::LoraSelection> = Vec::new();
    let mut push = |path: std::path::PathBuf, multiplier: f32| {
        let path_string = path.display().to_string();
        if !selections
            .iter()
            .any(|selection| selection.path == path_string)
        {
            selections.push(crate::vision::LoraSelection {
                path: path_string,
                multiplier,
            });
        }
    };
    if let Some(artifact) = artifact {
        for entry in registry.loras_bound_to(artifact.id) {
            match defaults.loras.resolve(&entry.path) {
                Some(path) => push(path, entry.multiplier),
                None => tracing::warn!(
                    file = %entry.path,
                    dir = %defaults.loras.dir.display(),
                    "bound lora missing on disk; skipping",
                ),
            }
        }
    }
    if let Some(binding) = defaults.loras.kinds.binding_for(kind) {
        match defaults.loras.resolve(&binding.file) {
            Some(path) => push(path, defaults.loras.multiplier),
            None => tracing::warn!(
                file = %binding.file,
                dir = %defaults.loras.dir.display(),
                "kind lora missing on disk; skipping",
            ),
        }
    }
    if inpaint {
        if let Some(binding) = &defaults.loras.inpaint {
            match defaults.loras.resolve(&binding.file) {
                Some(path) => push(path, defaults.loras.multiplier),
                None => tracing::warn!(
                    file = %binding.file,
                    dir = %defaults.loras.dir.display(),
                    "inpaint lora missing on disk; skipping",
                ),
            }
        }
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
                return run_generation(
                    registry,
                    op,
                    id,
                    pipeline,
                    description.clone(),
                    |_| Ok(Vec::new()),
                    options,
                )
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
            return run_generation(
                registry,
                op,
                id,
                pipeline,
                description.clone(),
                |_| Ok(Vec::new()),
                options,
            )
            .await;
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
            run_generation(
                registry,
                op,
                id,
                pipeline,
                revision_prompt,
                |_| Ok(Vec::new()),
                options,
            )
            .await
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
            let masks = |run: &super::pipeline::PipelineRun| -> Result<Vec<StoredMask>, OpError> {
                let Operation::Modify { mask_prompt, .. } = op else {
                    return Ok(Vec::new());
                };
                let mask = selected_mask_from_run(run)
                    .ok_or_else(|| OpError::Apply("modify recorded no mask".to_owned()))?;
                Ok(vec![StoredMask {
                    id: Uuid::new_v4(),
                    asset_path: mask.path.display().to_string(),
                    source: super::MaskSource::Auto,
                    prompt: mask_prompt.clone().or_else(|| segment_prompt_from_run(run)),
                    score: Some(mask.score),
                }])
            };
            run_generation(registry, op, id, pipeline, revision_prompt, masks, options).await
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
    masks: impl FnOnce(&super::pipeline::PipelineRun) -> Result<Vec<StoredMask>, OpError>,
    options: &ExecuteOptions<'_>,
) -> Result<OpOutcome, OpError> {
    pipeline
        .apply_generation_overrides(&options.generation)
        .map_err(CompileError::Build)?;
    let backend = options.backend.ok_or(CompileError::BackendRequired)?;
    let seed = options.run.seed.or(pipeline.params().seed);
    let model = Some(pipeline.params().model.clone());
    let revision_id = registry.start_revision(id, revision_prompt, seed, model)?;

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
    let masks = masks(&run)?;
    registry.finish_revision(id, revision_id, asset.display().to_string(), masks)?;
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

/// The mask the run confirmed, if the stack included a mask checkpoint.
fn selected_mask_from_run(
    run: &super::pipeline::PipelineRun,
) -> Option<super::pipeline::MaskCandidate> {
    for record in &run.decisions {
        if let super::pipeline::BlockedOn::SelectMask { candidates, .. } = &record.blocked_on {
            if let super::pipeline::Decision::SelectMask(index) = record.decision {
                return candidates.get(index).cloned();
            }
        }
    }
    None
}

/// The mask prompt that fed the confirmed mask, from the segment step's own
/// output (the stored mask keeps its grounding prompt as provenance).
fn segment_prompt_from_run(run: &super::pipeline::PipelineRun) -> Option<String> {
    run.outputs
        .iter()
        .find(|output| output.label == "segment")
        .and_then(|output| output.text.clone())
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
    };

    use super::*;
    use crate::registry::{
        pipeline::{BlockedOn, Decision, MaskCandidate, PipelineRun, StepOutput},
        CreateArtifact,
    };

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
            steps: 4,
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
                {"op": "modify", "target": "c:1234abcd", "description": "change her hair", "mask_prompt": "hair", "inpaint_prompt": "bob cut"}
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
            Operation::Modify {
                mask_prompt: Some(_),
                inpaint_prompt: Some(_),
                ..
            }
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
            mask_prompt: Some("everything".into()),
            inpaint_prompt: Some("different".into()),
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
            .start_revision(id, "Mia on the cliff".into(), Some(1), None)
            .unwrap();
        registry
            .finish_revision(id, revision, "assets/mia.png".into(), Vec::new())
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
            .start_revision(id, "Mia on the cliff".into(), Some(1), None)
            .unwrap();
        registry
            .finish_revision(id, revision, "assets/mia.png".into(), Vec::new())
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
            .start_revision(base, "Mia on the cliff".into(), Some(1), None)
            .unwrap();
        registry
            .finish_revision(
                base,
                base_revision,
                mia_asset.display().to_string(),
                Vec::new(),
            )
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
        let backend = VisualBackend::new(test_assets_dir().join("mia.png"));
        let create = Operation::Create {
            kind: ArtifactKind::Character,
            description: "Mia, a lighthouse keeper".into(),
            name: Some("mia".into()),
            size: Some((1024, 1024)),
            steps: Some(8),
            loras: vec![LoraSelection {
                path: "krea/char.safetensors".into(),
                multiplier: 0.8,
            }],
        };
        let test_defaults = test_defaults();
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
        assert_eq!((request.width, request.height), (Some(1024), Some(1024)));
        assert_eq!(request.steps, Some(8));
        assert_eq!(request.loras.len(), 1);
        assert_eq!(request.loras[0].path, "krea/char.safetensors");
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
            .start_revision(mia, "Mia on the cliff".into(), Some(1), None)
            .unwrap();
        registry
            .finish_revision(mia, revision, mia_asset.display().to_string(), Vec::new())
            .unwrap();

        let backend = VisualBackend::new(assets.join("mia-regen.png"));
        let op = Operation::Regenerate {
            target: Ref::new(registry.short_id(mia)),
            prompt: Some("warmer".into()),
            size: Some((768, 768)),
            steps: Some(12),
            loras: vec![LoraSelection {
                path: "krea/style.safetensors".into(),
                multiplier: 1.0,
            }],
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
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute(&mut registry, &op, &options))
            .unwrap();
        let request = &backend.requests()[0];
        assert_eq!((request.width, request.height), (Some(768), Some(768)));
        assert_eq!(request.steps, Some(12));
        assert!(request
            .loras
            .iter()
            .any(|lora| lora.path == "krea/style.safetensors"));
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
            .start_revision(mia, "Mia on the cliff".into(), Some(1), None)
            .unwrap();
        registry
            .finish_revision(mia, revision, mia_asset.display().to_string(), Vec::new())
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
        assert_eq!((request.width, request.height), (Some(1024), Some(1024)));
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
        let inpaint_lora = lora_dir.join("inpaintkrea.safetensors");
        std::fs::write(&inpaint_lora, b"lora").expect("dummy inpaint lora should write");

        let defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 4,
            loras: crate::registry::LoraBindings {
                dir: lora_dir.clone(),
                inpaint: Some(crate::registry::LoraBinding {
                    file: "inpaintkrea.safetensors".into(),
                }),
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
            .start_revision(mia, "Mia on the cliff".into(), Some(1), None)
            .unwrap();
        registry
            .finish_revision(mia, revision, mia_asset.display().to_string(), Vec::new())
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
        assert_eq!(
            request.loras[0].path,
            lora_dir.join("mia-v2.safetensors").display().to_string()
        );
        assert_eq!(request.loras[0].multiplier, 0.8);
        assert!(request
            .loras
            .iter()
            .any(|lora| lora.path.ends_with("char-style.safetensors")));

        // A mask-guided edit additionally carries the inpaint adapter
        // (checked at compile: the modify pipeline's generation params).
        let modify = Operation::Modify {
            target: Ref::new(registry.short_id(mia)),
            description: "change her hair".into(),
            mask_prompt: Some("her hair".into()),
            inpaint_prompt: Some("a bob cut".into()),
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
            3,
            "both bound adapters + the inpaint binding"
        );
        assert!(
            modify_pipeline
                .params()
                .loras
                .iter()
                .any(|lora| lora.path.ends_with("inpaintkrea.safetensors")),
            "the inpaint binding rides mask-guided edits"
        );
    }

    #[test]
    fn attach_lora_requires_the_file_to_exist_and_a_visual_target() {
        let assets = test_assets_dir();
        let lora_dir = assets.join("loras");
        std::fs::create_dir_all(&lora_dir).expect("lora dir should create");
        let defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 4,
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
        assert_eq!((request.width, request.height), (Some(512), Some(768)));
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
            .start_revision(base, "Mia on the cliff".into(), Some(1), None)
            .unwrap();
        registry
            .finish_revision(
                base,
                base_revision,
                mia_asset.display().to_string(),
                Vec::new(),
            )
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
        // reference, the variant description as the prompt, the base's size.
        let request = &backend.requests()[0];
        assert_eq!(request.prompt, "in yellow rain gear");
        assert_eq!(
            request.reference_image_path.as_deref(),
            Some(mia_asset.to_str().unwrap())
        );
        assert_eq!((request.width, request.height), (Some(512), Some(768)));

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

    /// A minimal fake backend returning Ok(None) for LLM steps.
    struct DraftBackend;
    impl GenerationBackend for DraftBackend {
        fn segment(
            &self,
            _request: &crate::vision::SegmentRequest,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<crate::vision::SegmentResponse, PipelineError>,
        > {
            unreachable!()
        }
        fn generate(
            &self,
            _request: &crate::vision::GenerateRequest,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<crate::vision::GenerateResponse, PipelineError>,
        > {
            unreachable!()
        }
        fn llm_draft(
            &self,
            _request: &str,
        ) -> futures_util::future::BoxFuture<'_, Result<Option<String>, PipelineError>> {
            Box::pin(async move { Ok(Some("The keeper winds the lamp.".to_owned())) })
        }
        fn llm_plan(
            &self,
            _request: &str,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<Option<super::super::pipeline::PlannedEdit>, PipelineError>,
        > {
            unreachable!()
        }
    }

    /// A fake generation backend: captures requests and returns a canned
    /// image path for every generate call.
    struct VisualBackend {
        generated: PathBuf,
        requests: Mutex<Vec<crate::vision::GenerateRequest>>,
    }

    impl VisualBackend {
        fn new(generated: PathBuf) -> Self {
            Self {
                generated,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<crate::vision::GenerateRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl GenerationBackend for VisualBackend {
        fn segment(
            &self,
            _request: &crate::vision::SegmentRequest,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<crate::vision::SegmentResponse, PipelineError>,
        > {
            unreachable!()
        }
        fn generate(
            &self,
            request: &crate::vision::GenerateRequest,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<crate::vision::GenerateResponse, PipelineError>,
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
                Ok(crate::vision::GenerateResponse {
                    status: "completed".to_owned(),
                    image_path: Some(generated.display().to_string()),
                    model: request.model.clone(),
                    device: None,
                    dtype: None,
                    seed: request.seed,
                    width: request.width,
                    height: request.height,
                    duration_ms: None,
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
        fn llm_plan(
            &self,
            _request: &str,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<Option<super::super::pipeline::PlannedEdit>, PipelineError>,
        > {
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

    #[test]
    fn selected_mask_is_derived_from_the_run_decisions() {
        let run = PipelineRun {
            outputs: vec![StepOutput {
                step_index: 1,
                label: "segment".into(),
                kind: crate::registry::pipeline::OutputKind::Mask,
                path: Some("masks/hair.png".into()),
                text: Some("her hair".into()),
            }],
            decisions: vec![crate::registry::pipeline::BlockRecord {
                step_index: 2,
                blocked_on: BlockedOn::SelectMask {
                    description: "confirm the segmentation mask".into(),
                    candidates: vec![MaskCandidate {
                        path: "masks/hair.png".into(),
                        score: 0.98,
                        area_pixels: 900,
                        bounding_box: crate::vision::SegmentBox {
                            x_min: 0.0,
                            y_min: 0.0,
                            x_max: 10.0,
                            y_max: 20.0,
                        },
                    }],
                },
                decision: Decision::SelectMask(0),
            }],
        };
        let mask = selected_mask_from_run(&run).expect("mask should be derived");
        assert_eq!(mask.path.to_str(), Some("masks/hair.png"));
        assert_eq!(mask.score, 0.98);
        assert_eq!(segment_prompt_from_run(&run).as_deref(), Some("her hair"));
    }
}
