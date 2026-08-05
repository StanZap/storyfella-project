//! The typed operation set (`docs/ROADMAP.md` §7) and its compiler +
//! executor.
//!
//! Operations are the *intent* layer (semantic, logged, approval-gated,
//! user/LLM-facing); pipelines (`pipeline.rs`) are the *execution* layer.
//! Each operation compiles to a pipeline — except the model-only slice-1
//! ops (create, variant, compose), which mutate the registry directly, and
//! pure ops which do not exist in slice 1.
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

use super::{
    pipeline::{
        GenerationBackend, GenerationOverrides, Pipeline, PipelineBuildError, PipelineError,
        PromptSource, RunOptions,
    },
    ArtifactKind, ArtifactRegistry, CreateArtifact, Layer, LayerRole, Ref, RefError, RegistryError,
    StoredMask, VariantAxis,
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
    },
    /// New visual variant of an artifact.
    Variant {
        target: Ref,
        description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        axis: Option<VariantAxis>,
    },
    /// New revision of the active image (fresh seed, or edited prompt with
    /// the current image as reference).
    Regenerate {
        target: Ref,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
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
    },
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
                axis,
            } => format!(
                "variant {target} {description:?}{}",
                axis.map(|axis| format!(" [{axis}]")).unwrap_or_default()
            ),
            Operation::Regenerate { target, prompt } => format!(
                "regenerate {target}{}",
                prompt
                    .as_deref()
                    .map(|prompt| format!(" {prompt:?}"))
                    .unwrap_or_default()
            ),
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
        }
    }

    /// The artifact the operation primarily touches, if statically known.
    pub fn target_ref(&self) -> Option<&Ref> {
        match self {
            Operation::Variant { target, .. }
            | Operation::Regenerate { target, .. }
            | Operation::Draft { target, .. }
            | Operation::Modify { target, .. } => Some(target),
            Operation::Compose { scene, .. } => Some(scene),
            Operation::Create { .. } => None,
        }
    }
}

/// The result of compiling an operation.
#[derive(Clone, Debug)]
pub enum CompiledOp {
    /// A model-only operation; no pipeline (create, variant, compose).
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
    #[error("pipeline build failed: {0}")]
    Build(#[from] PipelineBuildError),
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

/// Statically validates the operation against the registry and compiles it
/// to a pipeline. No IO; this is the static-validation surface.
pub fn compile(registry: &ArtifactRegistry, op: &Operation) -> Result<CompiledOp, CompileError> {
    match op {
        Operation::Create {
            description, size, ..
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
            Ok(CompiledOp::Direct)
        }
        Operation::Variant {
            target,
            description,
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
            Ok(CompiledOp::Direct)
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
        Operation::Regenerate { target, prompt } => {
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

            let mut builder = super::pipeline::PipelineBuilder::new();
            // The artifact's default size (explicit `--size` at creation, or
            // the kind default) drives its images; per-run overrides still win.
            if let Some((width, height)) = artifact
                .default_size
                .or_else(|| artifact.kind.default_size())
            {
                builder.size(width, height);
            }
            let reference = active
                .and_then(|revision| revision.asset_path.clone())
                .map(|path| builder.reference_image(path));
            builder.generate(PromptSource::Text(resolved_prompt), reference, None);
            Ok(CompiledOp::Pipeline(builder.build()?))
        }
        Operation::Modify {
            target,
            description,
            mask_prompt,
            inpaint_prompt,
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

            let mut builder = super::pipeline::PipelineBuilder::new();
            // Same default-size rule as regenerate: the artifact's own size
            // wins over the pipeline default, per-run overrides still win.
            if let Some((width, height)) = artifact
                .default_size
                .or_else(|| artifact.kind.default_size())
            {
                builder.size(width, height);
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
    }
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
        } => {
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
            axis,
        } => {
            let base = registry.resolve(target)?;
            let base_artifact = registry
                .artifact(base)
                .ok_or(RegistryError::NoArtifact(base))?;
            // Variants are auto-named from the base so every system-created
            // key stays unique: `mia-outfit`, `mia-rain-gear`.
            let name = if base_artifact.name.trim().is_empty() {
                derive_name(description)
            } else if let Some(axis) = axis {
                format!("{}-{}", base_artifact.name, axis.as_str())
            } else {
                format!("{}-{}", base_artifact.name, derive_name(description))
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
            let log_id = registry.push_log(op.clone(), options.origin, OpStatus::Applied, Some(id));
            Ok(OpOutcome {
                artifact_id: Some(id),
                revision_id: None,
                log_id: Some(log_id),
                run: None,
                status: OpStatus::Applied,
            })
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
            let CompiledOp::Pipeline(pipeline) = compile(registry, op)? else {
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
            let mut pipeline = match compile(registry, op)? {
                CompiledOp::Pipeline(pipeline) => pipeline,
                CompiledOp::Direct => unreachable!("visual ops always compile to pipelines"),
            };
            pipeline
                .apply_generation_overrides(&options.generation)
                .map_err(CompileError::Build)?;
            let backend = options.backend.ok_or(CompileError::BackendRequired)?;
            let seed = options.run.seed.or(pipeline.params().seed);
            let model = Some(pipeline.params().model.clone());
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
            let masks = match op {
                Operation::Modify { mask_prompt, .. } => {
                    let mask = selected_mask_from_run(&run)
                        .ok_or_else(|| OpError::Apply("modify recorded no mask".to_owned()))?;
                    vec![StoredMask {
                        id: Uuid::new_v4(),
                        asset_path: mask.path.display().to_string(),
                        source: super::MaskSource::Auto,
                        prompt: mask_prompt
                            .clone()
                            .or_else(|| segment_prompt_from_run(&run)),
                        score: Some(mask.score),
                    }]
                }
                _ => Vec::new(),
            };
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
    }
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
    use super::*;
    use crate::registry::{
        pipeline::{BlockedOn, Decision, MaskCandidate, PipelineRun, StepOutput},
        CreateArtifact,
    };

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
            compile(&registry, &op),
            Err(CompileError::NotAScene { .. })
        ));

        // Variant on a story is a compile error.
        let op = Operation::Variant {
            target: Ref::new(story.to_string()),
            description: "a variant".into(),
            axis: None,
        };
        assert!(matches!(
            compile(&registry, &op),
            Err(CompileError::NotVariantable(ArtifactKind::Story))
        ));

        // Modify without an active image is a compile error.
        let op = Operation::Modify {
            target: Ref::new(story.to_string()),
            description: "change it".into(),
            mask_prompt: Some("everything".into()),
            inpaint_prompt: Some("different".into()),
        };
        assert!(matches!(
            compile(&registry, &op),
            Err(CompileError::NoActiveImage(_))
        ));

        // Unknown refs surface as RefError.
        let op = Operation::Draft {
            target: Ref::new("c:00000000"),
            request: "write something".into(),
        };
        assert!(matches!(compile(&registry, &op), Err(CompileError::Ref(_))));
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
        };
        let CompiledOp::Pipeline(pipeline) = compile(&registry, &op).unwrap() else {
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
        };
        let CompiledOp::Pipeline(pipeline) = compile(&registry, &op).unwrap() else {
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
        };
        let CompiledOp::Pipeline(pipeline) = compile(&registry, &op).unwrap() else {
            panic!("regenerate must compile to a pipeline");
        };
        assert_eq!(pipeline.params().width, 768);
        assert_eq!(pipeline.params().height, 768);
    }

    #[test]
    fn create_stores_the_size_and_variant_inherits_it() {
        let mut registry = ArtifactRegistry::new();
        let create = Operation::Create {
            kind: ArtifactKind::Character,
            description: "Mia, a lighthouse keeper".into(),
            name: Some("mia".into()),
            size: Some((512, 768)),
        };
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
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
            axis: Some(VariantAxis::Outfit),
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
    fn create_and_variant_apply_directly_and_log() {
        let mut registry = ArtifactRegistry::new();
        let create = Operation::Create {
            kind: ArtifactKind::Character,
            description: "Mia, a lighthouse keeper".into(),
            name: Some("mia".into()),
            size: None,
        };
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
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
        assert_eq!(registry.log.len(), 1);
        assert_eq!(registry.log[0].status, OpStatus::Applied);

        let variant = Operation::Variant {
            target: Ref::new(registry.short_id(mia)),
            description: "in rain gear".into(),
            axis: Some(VariantAxis::Outfit),
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
        assert_eq!(registry.log.len(), 2);
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
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            manual_text: None,
            origin: OpOrigin::User,
        };
        let variant = Operation::Variant {
            target: Ref::new("c:mia"),
            description: "in rain gear".into(),
            axis: Some(VariantAxis::Outfit),
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
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
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

    #[test]
    fn draft_applies_approved_llm_text_to_the_artifact() {
        let mut registry = registry_with(&[(ArtifactKind::Story, "a story")]);
        let id = registry.artifacts[0].id;
        let op = Operation::Draft {
            target: Ref::new(registry.short_id(id)),
            request: "write the opening".into(),
        };
        let options = ExecuteOptions {
            backend: Some(&DraftBackend),
            run: RunOptions {
                work_dir: std::env::temp_dir(),
                approvals: crate::registry::pipeline::ApprovalPolicy::auto(),
                seed: None,
            },
            generation: Default::default(),
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
        let options = ExecuteOptions {
            backend: None, // no backend needed
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
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
