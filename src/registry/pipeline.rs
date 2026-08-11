//! The pipeline execution layer (`docs/ROADMAP.md` §7).
//!
//! Two layers, kept distinct: operations (intent — `ops.rs`) and pipelines
//! (execution — this module). A pipeline is a linear, fail-fast stack of
//! steps from a closed vocabulary. Steps produce **typed intermediates**
//! (handles): `ImageHandle`, `MaskHandle`, `SelectedMaskHandle`,
//! `PromptHandle`, `PlanHandle`, `FeedbackHandle`. Handles are checked
//! statically by the type system — a caption cannot be fed into a mask
//! slot — and the builder validates the stack at `build()`.
//!
//! Rules implemented here:
//!
//! - **Closed vocabulary.** `Step` is the complete set; the VLLM combines
//!   these kinds, it never invents them.
//! - **Static validation at `build()`** — parameter bounds (sizes are
//!   multiples of 32 within the contract bounds, step counts, LoRA count),
//!   step ordering, and the "a mask requires a reference image" rule.
//! - **Linear, fail-fast execution.** A failed step ends the stack with the
//!   intermediates produced so far; there is no branching or retry.
//! - **Checkpoints.** `Checkpoint` steps block for human approval
//!   (mask confirmation, draft text approval). The approval policy decides;
//!   rejection ends the stack cleanly.
//! - **LLM steps are soft dependencies.** `Llm` steps degrade to manual
//!   input when the backend has no LLM or it fails; they never hard-fail a
//!   stack.
//! - **Pure image primitives.** `Composite`/`Invert`/`Feather`/`Union`
//!   are deterministic functions over files (`image_ops.rs`), so golden runs
//!   are replayable and the composite fallback's "outside the mask is
//!   bit-identical" guarantee is assertable.

use std::path::PathBuf;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::errors::{CodedError, ErrorCode, ReportedError};
use crate::vision::{
    ComputeDevice, GenerateRequest, LoraSelection, SegmentBox, SegmentPoint, SegmentRequest,
    SegmentResponse,
};

use super::image_ops::{self, ImageOpsError};

/// The focus generation model (Krea 2 via stable-diffusion.cpp).
pub const DEFAULT_MODEL: &str = "krea-2-turbo-q4";
/// The interactive draft profile used by the UI and the CLI defaults.
pub const DEFAULT_WIDTH: u32 = 768;
pub const DEFAULT_HEIGHT: u32 = 448;
pub const DEFAULT_STEPS: u32 = 4;
/// Seam-blending radius for the composite fallback.
pub const COMPOSITE_FEATHER_RADIUS: u32 = 8;
/// How far `inpaint`'s composite grows the mask before feathering.
///
/// A segmentation mask runs tighter than a soft silhouette — measured
/// 2026-08-11, a hair mask scoring 0.977 still missed the flyaway strands,
/// and the composite preserved them as a dark rim tracing the old outline
/// over an otherwise correct edit. Growing the mask trades the
/// bit-identical-outside guarantee for covering that rim; INPAINTKREA's own
/// restore node makes the same trade at `mask_grow` 12 on a ~1MP canvas.
pub const COMPOSITE_MASK_GROW: u32 = 12;
/// The neutral colour `inpaint` pre-fills the masked init region with.
///
/// Chroma green: the point is a hue that appears nowhere in skin, hair or a
/// studio wall, so it reads as "nothing here" rather than as content the
/// generation should preserve. See
/// [`crate::registry::image_ops::fill_masked`] for why a *predictive* fill
/// (diffusing the surroundings inward) measured worse than none at all.
pub const MASKED_INIT_FILL: [u8; 3] = [128, 128, 128];
/// How far `inpaint` widens the mask *before generating*, as opposed to
/// [`COMPOSITE_MASK_GROW`] which only widens the blend afterwards.
///
/// SAM resolves a silhouette to the edge it can see; flyaway hair, fur and
/// motion blur fall outside that and are never handed to the backend, so
/// they survive an edit that replaced everything around them. Growing here
/// is the only place that can fix it — a composite cannot remove content the
/// model was never allowed to re-render.
pub const MASK_GENERATION_GROW: u32 = 12;

/// Generation parameters shared by every `Generate` step in a pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationParams {
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: Option<u64>,
    pub loras: Vec<LoraSelection>,
    /// The reference-conditioning strength for reference-based generations
    /// (see [`GenerateRequest::reference_strength`]); `None` lets the
    /// backend default.
    pub reference_strength: Option<f32>,
    /// The img2img denoise strength applied when a `Generate` step sends an
    /// init image (see [`GenerateRequest::denoise_strength`]); `None` lets
    /// the backend default. Masked-latent inpaint sets
    /// [`MASKED_LATENT_DENOISE`].
    pub denoise_strength: Option<f32>,
}

/// Denoise strength for masked-latent inpaint: run the whole sigma
/// schedule. The per-step denoise-mask blend is what preserves the unmasked
/// latents, so truncating the schedule (the backend's 0.75 default) only
/// weakens the edit inside the mask while buying no protection outside it.
pub const MASKED_LATENT_DENOISE: f32 = 1.0;

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_owned(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            steps: DEFAULT_STEPS,
            seed: None,
            loras: Vec::new(),
            reference_strength: None,
            denoise_strength: None,
        }
    }
}

/// Per-run generation overrides (CLI flags like `--steps`/`--size`/`--model`
/// /`--lora`; the operation itself carries only intent). Applied to the
/// compiled pipeline and re-validated before execution. `loras` are
/// appended after the auto-injected adapters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerationOverrides {
    pub steps: Option<u32>,
    pub size: Option<(u32, u32)>,
    pub model: Option<String>,
    pub loras: Vec<LoraSelection>,
}

/// A typed handle to a step's output. The step index is resolved at run
/// time; the *kind* is checked by the Rust type system at build time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageHandle {
    pub(crate) step: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaskHandle {
    pub(crate) step: usize,
}

/// A mask that has passed the confirmation checkpoint (or was derived from
/// one). Only these can feed inpaint/composite/mask transforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedMaskHandle {
    pub(crate) step: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptHandle {
    pub(crate) step: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanHandle {
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
    PlanPart(PlanHandle, PlanPart),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanPart {
    MaskPrompt,
    InpaintPrompt,
}

/// The output of an LLM plan step: the edit description split into a
/// segmentation prompt and an inpainting prompt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedEdit {
    pub mask_prompt: String,
    pub inpaint_prompt: String,
}

/// The closed step vocabulary.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Loads a file on disk as the stack's working image. -> Image
    LoadImage {
        source: PathBuf,
    },
    /// Loads a mask PNG on disk as an already-confirmed mask. -> SelectedMask
    ///
    /// This is the mask path's second source, beside `Segment`. A mask file
    /// needs no confirmation checkpoint: `Segment` proposes *candidates* and
    /// generating against the wrong one wastes a generation, but a file was
    /// chosen deliberately by whoever wrote it. It is also the seam every
    /// other source reduces to — SAM writes a PNG, a canvas brush would
    /// write a PNG, and a test fixture *is* a PNG.
    LoadMask {
        source: PathBuf,
    },
    /// Segments the working image; produces a *candidate set*. -> Mask
    Segment {
        image: ImageHandle,
        prompt: PromptSource,
        points: Vec<SegmentPoint>,
        boxes: Vec<SegmentBox>,
    },
    /// Blocks for human approval. Produces no value.
    Checkpoint {
        description: String,
        on: CheckpointOn,
    },
    /// A native generation (fresh, with a reference image, or with a native
    /// mask — the composite fallback path leaves `mask` as `None` and
    /// composites afterwards). -> Image
    Generate {
        prompt: PromptSource,
        reference: Option<ImageHandle>,
        mask: Option<SelectedMaskHandle>,
        /// The img2img starting image, when it differs from `reference`.
        /// `None` keeps the reference in both roles. They separate on the
        /// mask path: the reference must stay the untouched original (it is
        /// what the edit adapter reads identity from), while the init is
        /// pre-filled inside the mask so the model starts from the right
        /// palette instead of pure noise. See [`Step::FillMasked`].
        init: Option<ImageHandle>,
    },
    /// Replaces the masked region with colour diffused in from outside it,
    /// producing the pre-filled init image for a masked-latent inpaint.
    /// -> Image
    FillMasked {
        image: ImageHandle,
        mask: SelectedMaskHandle,
    },
    /// The guaranteed mask-edit primitive: blends `replacement` into
    /// `original` everywhere the confirmed mask is active. -> Image
    Composite {
        original: ImageHandle,
        mask: SelectedMaskHandle,
        replacement: ImageHandle,
        feather_radius: u32,
        /// Pixels to grow the mask by before feathering. `0` keeps the
        /// erode-and-clamp behaviour and its bit-identical-outside
        /// guarantee; see [`crate::registry::image_ops::composite`].
        mask_grow: u32,
    },
    Invert {
        mask: SelectedMaskHandle,
    }, // -> SelectedMask
    Feather {
        mask: SelectedMaskHandle,
        radius: u32,
    }, // -> SelectedMask
    /// Widens the region the backend may re-render (not the blend).
    /// -> SelectedMask
    GrowMask {
        mask: SelectedMaskHandle,
        radius: u32,
    },
    Union {
        masks: Vec<SelectedMaskHandle>,
    }, // -> SelectedMask
    /// Painted strokes as a mask source (interactive surface in the canvas;
    /// executable by future slices). -> SelectedMask
    PaintStrokes {
        image: ImageHandle,
        strokes: Vec<Stroke>,
    },
    /// An LLM step — a soft dependency. -> Prompt | Plan | Feedback
    Llm {
        kind: LlmStepKind,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CheckpointOn {
    SelectMask(MaskHandle),
    ApproveText(PromptHandle),
}

#[derive(Clone, Debug, PartialEq)]
pub enum LlmStepKind {
    /// Propose story text for the given request. -> Prompt
    Draft { request: String },
    /// Split an edit description into mask/inpaint prompts. -> Plan
    Plan { request: String },
    /// Rewrite a terse edit instruction into one grounded against the
    /// artifact it edits. -> Prompt
    ///
    /// Unlike [`Self::Plan`] this never blocks: `fallback` is a usable
    /// deterministic grounding, so a missing LLM degrades silently instead
    /// of stopping to ask the user. The LLM earns its place by doing what
    /// the template cannot — dropping invariants the edit contradicts.
    Ground { request: String, fallback: String },
    /// Caption an image. -> Prompt
    Describe { image: ImageHandle },
    /// Critique an image against a prompt. -> Feedback
    Critique {
        image: ImageHandle,
        prompt: PromptSource,
    },
}

/// A painted stroke (ellipse) for `PaintStrokes`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub center_x: f64,
    pub center_y: f64,
    pub radius_x: f64,
    pub radius_y: f64,
}

/// One segmentation candidate (mirrors the Python `SegmentMask` contract).
#[derive(Clone, Debug, PartialEq)]
pub struct MaskCandidate {
    pub path: PathBuf,
    pub score: f64,
    pub area_pixels: u64,
    pub bounding_box: SegmentBox,
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
        std::fs::create_dir_all(&options.work_dir).map_err(|source| PipelineError::Step {
            step: 0,
            message: format!(
                "could not create work directory {}: {source}",
                options.work_dir.display()
            ),
        })?;
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
                Step::FillMasked { image, mask } => {
                    let image_path = value_image(values.get(image.step), index)?;
                    let mask_path = value_selected_mask(values.get(mask.step), index)?;
                    let output = options
                        .work_dir
                        .join(format!("fill-masked-{}.png", Uuid::new_v4()));
                    image_ops::fill_masked(&image_path, &mask_path, MASKED_INIT_FILL, &output)?;
                    StepValue::Image(output)
                }
                Step::LoadMask { source } => {
                    if !source.exists() {
                        return Err(PipelineError::Step {
                            step: index,
                            message: format!("mask image {} does not exist", source.display()),
                        });
                    }
                    StepValue::MaskConfirmed(source.clone())
                }
                Step::Segment {
                    image,
                    prompt,
                    points,
                    boxes,
                } => {
                    let image_path = values
                        .get(image.step)
                        .and_then(StepValue::as_image)
                        .ok_or_else(|| PipelineError::Step {
                            step: index,
                            message: "segment input is not an image".to_owned(),
                        })?;
                    let prompt_text = resolve_prompt(prompt, &values, index)?;
                    let response = backend
                        .segment(&SegmentRequest {
                            image_path: image_path.display().to_string(),
                            prompt: Some(prompt_text.clone()),
                            points: points.clone(),
                            boxes: boxes.clone(),
                            model: None,
                            device: None,
                        })
                        .await?;
                    if response.status == "failed" {
                        return Err(PipelineError::Step {
                            step: index,
                            message: response
                                .error
                                .unwrap_or_else(|| "segmentation failed".into()),
                        });
                    }
                    let candidates: Vec<MaskCandidate> = response
                        .masks
                        .into_iter()
                        .map(|mask| MaskCandidate {
                            path: PathBuf::from(mask.path),
                            score: mask.score,
                            area_pixels: mask.area_pixels,
                            bounding_box: mask.bounding_box,
                        })
                        .collect();
                    if candidates.is_empty() {
                        return Err(PipelineError::Step {
                            step: index,
                            message: format!("segmentation for {prompt_text:?} produced no masks"),
                        });
                    }
                    StepValue::Mask {
                        candidates,
                        prompt: prompt_text,
                    }
                }
                Step::Checkpoint { description, on } => match on {
                    CheckpointOn::SelectMask(mask_handle) => {
                        let candidates = values
                            .get(mask_handle.step)
                            .and_then(StepValue::as_candidates)
                            .cloned()
                            .ok_or_else(|| PipelineError::Step {
                                step: index,
                                message: "checkpoint references an already-confirmed mask"
                                    .to_owned(),
                            })?;
                        let blocked = BlockedOn::SelectMask {
                            description: description.clone(),
                            candidates,
                        };
                        let decision = options.approvals.decide(blocked.clone());
                        decisions.push(BlockRecord {
                            step_index: index,
                            blocked_on: blocked,
                            decision: decision.clone(),
                        });
                        match decision {
                            Decision::SelectMask(choice) => {
                                let candidates = values[mask_handle.step]
                                    .as_candidates()
                                    .cloned()
                                    .expect("checked above");
                                let chosen =
                                    candidates.get(choice).ok_or_else(|| PipelineError::Step {
                                        step: index,
                                        message: format!(
                                            "mask choice {choice} out of {} candidates",
                                            candidates.len()
                                        ),
                                    })?;
                                values[mask_handle.step] =
                                    StepValue::MaskConfirmed(chosen.path.clone());
                            }
                            Decision::Reject => return Err(PipelineError::Rejected),
                            other => {
                                return Err(PipelineError::Step {
                                    step: index,
                                    message: format!(
                                        "invalid decision for mask checkpoint: {other:?}"
                                    ),
                                })
                            }
                        }
                        StepValue::Nothing
                    }
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
                Step::Generate {
                    prompt,
                    reference,
                    mask,
                    init,
                } => {
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
                    let mask_path = mask
                        .map(|handle| {
                            values
                                .get(handle.step)
                                .and_then(StepValue::as_selected_mask)
                                .cloned()
                                .ok_or_else(|| PipelineError::Step {
                                    step: index,
                                    message: "generate mask is not confirmed".to_owned(),
                                })
                        })
                        .transpose()?;
                    let request = GenerateRequest {
                        prompt: prompt_text,
                        reference_image_path: reference_path
                            .clone()
                            .map(|path| path.display().to_string()),
                        reference_strength: self.params.reference_strength,
                        width: Some(self.params.width),
                        height: Some(self.params.height),
                        steps: Some(self.params.steps),
                        seed,
                        model: Some(self.params.model.clone()),
                        device: Some(ComputeDevice::Auto),
                        loras: self.params.loras.clone(),
                        mask_path: mask_path.clone().map(|path| path.display().to_string()),
                        // Masked-latent inpaint: the original image whose
                        // masked region is re-rendered (the reference is the
                        // same file in the modify flow). The backend keeps
                        // the unmasked latent pixels fixed per step; the
                        // composite step below is the fallback.
                        init_image_path: if mask_path.is_some() {
                            // The pre-filled init when there is one, else the
                            // reference in both roles.
                            match init {
                                Some(handle) => Some(
                                    value_image(values.get(handle.step), index)?
                                        .display()
                                        .to_string(),
                                ),
                                None => reference_path.map(|path| path.display().to_string()),
                            }
                        } else {
                            None
                        },
                        // Only meaningful alongside an init image; sending
                        // it otherwise would make the backend log a
                        // phantom IMG2IMG.
                        denoise_strength: if mask_path.is_some() {
                            self.params.denoise_strength
                        } else {
                            None
                        },
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
                Step::Composite {
                    original,
                    mask,
                    replacement,
                    feather_radius,
                    mask_grow,
                } => {
                    let original_path = value_image(values.get(original.step), index)?;
                    let mask_path = value_selected_mask(values.get(mask.step), index)?;
                    let replacement_path = value_image(values.get(replacement.step), index)?;
                    let output = options
                        .work_dir
                        .join(format!("composite-{}.png", Uuid::new_v4()));
                    image_ops::composite(
                        &original_path,
                        &mask_path,
                        &replacement_path,
                        *feather_radius,
                        *mask_grow,
                        &output,
                    )?;
                    StepValue::Image(output)
                }
                Step::Invert { mask } => {
                    let mask_path = value_selected_mask(values.get(mask.step), index)?;
                    let output = options
                        .work_dir
                        .join(format!("invert-{}.png", Uuid::new_v4()));
                    image_ops::invert(&mask_path, &output)?;
                    StepValue::MaskConfirmed(output)
                }
                Step::Feather { mask, radius } => {
                    let mask_path = value_selected_mask(values.get(mask.step), index)?;
                    let output = options
                        .work_dir
                        .join(format!("feather-{}.png", Uuid::new_v4()));
                    image_ops::feather(&mask_path, *radius, &output)?;
                    StepValue::MaskConfirmed(output)
                }
                Step::GrowMask { mask, radius } => {
                    let mask_path = value_selected_mask(values.get(mask.step), index)?;
                    let output = options
                        .work_dir
                        .join(format!("grow-mask-{}.png", Uuid::new_v4()));
                    image_ops::grow(&mask_path, *radius, &output)?;
                    StepValue::MaskConfirmed(output)
                }
                Step::Union { masks } => {
                    let mut paths = Vec::new();
                    for handle in masks {
                        paths.push(value_selected_mask(values.get(handle.step), index)?);
                    }
                    let output = options
                        .work_dir
                        .join(format!("union-{}.png", Uuid::new_v4()));
                    image_ops::union(&paths, &output)?;
                    StepValue::MaskConfirmed(output)
                }
                Step::PaintStrokes { .. } => {
                    return Err(PipelineError::Step {
                        step: index,
                        message: "paint strokes are not executable in slice 1".to_owned(),
                    })
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
                    LlmStepKind::Plan { request } => {
                        // Soft dependency: degrades to manual input.
                        let produced = match backend.llm_plan(request).await {
                            Ok(Some(plan)) => Some(plan),
                            Ok(None) | Err(_) => None,
                        };
                        match produced {
                            Some(plan) => StepValue::Plan(plan),
                            None => {
                                let blocked = BlockedOn::ProvideInput {
                                    description: format!("plan the edit: {request:?}"),
                                    purpose: InputPurpose::PlannedEdit,
                                };
                                let decision = options.approvals.decide(blocked.clone());
                                decisions.push(BlockRecord {
                                    step_index: index,
                                    blocked_on: blocked,
                                    decision: decision.clone(),
                                });
                                match decision {
                                    Decision::ProvidePlan(plan) => StepValue::Plan(plan),
                                    Decision::Reject => return Err(PipelineError::Rejected),
                                    other => {
                                        return Err(PipelineError::Step {
                                            step: index,
                                            message: format!(
                                                "invalid decision for degraded plan: {other:?}"
                                            ),
                                        })
                                    }
                                }
                            }
                        }
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

fn value_selected_mask(value: Option<&StepValue>, step: usize) -> Result<PathBuf, PipelineError> {
    value
        .and_then(StepValue::as_selected_mask)
        .cloned()
        .ok_or_else(|| PipelineError::Step {
            step,
            message: "expected a confirmed mask intermediate".to_owned(),
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
        PromptSource::PlanPart(handle, part) => {
            let plan = values
                .get(handle.step)
                .and_then(StepValue::as_plan)
                .ok_or_else(|| PipelineError::Step {
                    step,
                    message: "plan handle does not reference a plan step".to_owned(),
                })?;
            Ok(match part {
                PlanPart::MaskPrompt => plan.mask_prompt.clone(),
                PlanPart::InpaintPrompt => plan.inpaint_prompt.clone(),
            })
        }
    }
}

fn step_label(step: &Step) -> &'static str {
    match step {
        Step::LoadImage { .. } => "load_image",
        Step::LoadMask { .. } => "load_mask",
        Step::FillMasked { .. } => "fill_masked",
        Step::Segment { .. } => "segment",
        Step::Checkpoint { .. } => "checkpoint",
        Step::Generate { .. } => "generate",
        Step::Composite { .. } => "composite",
        Step::Invert { .. } => "invert",
        Step::Feather { .. } => "feather",
        Step::GrowMask { .. } => "grow_mask",
        Step::Union { .. } => "union",
        Step::PaintStrokes { .. } => "paint_strokes",
        Step::Llm { .. } => "llm",
    }
}

fn step_output(value: &StepValue, label: &str) -> (OutputKind, Option<PathBuf>, Option<String>) {
    match value {
        StepValue::Image(path) | StepValue::MaskConfirmed(path) => {
            (OutputKind::Image, Some(path.clone()), None)
        }
        StepValue::Mask { candidates, prompt } => (
            OutputKind::Mask,
            candidates.first().map(|candidate| candidate.path.clone()),
            Some(prompt.clone()),
        ),
        StepValue::Prompt(text) => (OutputKind::Text, None, Some(text.clone())),
        StepValue::Plan(plan) => (
            OutputKind::Plan,
            None,
            Some(format!("{} | {}", plan.mask_prompt, plan.inpaint_prompt)),
        ),
        StepValue::Feedback(text) => (OutputKind::Text, None, Some(text.clone())),
        StepValue::Nothing => (OutputKind::Void, None, Some(label.to_owned())),
    }
}

/// The runtime value of a step.
#[derive(Clone, Debug, PartialEq)]
pub enum StepValue {
    Image(PathBuf),
    /// The candidate set produced by `Segment`, before confirmation. The
    /// prompt that grounded the segmentation travels with it so the stored
    /// mask keeps its provenance.
    Mask {
        candidates: Vec<MaskCandidate>,
        prompt: String,
    },
    /// A single confirmed or derived mask path.
    MaskConfirmed(PathBuf),
    Prompt(String),
    Plan(PlannedEdit),
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

    fn as_candidates(&self) -> Option<&Vec<MaskCandidate>> {
        match self {
            StepValue::Mask { candidates, .. } => Some(candidates),
            _ => None,
        }
    }

    fn as_selected_mask(&self) -> Option<&PathBuf> {
        match self {
            StepValue::MaskConfirmed(path) => Some(path),
            _ => None,
        }
    }

    fn as_prompt(&self) -> Option<&String> {
        match self {
            StepValue::Prompt(text) => Some(text),
            _ => None,
        }
    }

    fn as_plan(&self) -> Option<&PlannedEdit> {
        match self {
            StepValue::Plan(plan) => Some(plan),
            _ => None,
        }
    }
}

/// What a checkpoint blocks on.
#[derive(Clone, Debug, PartialEq)]
pub enum BlockedOn {
    SelectMask {
        description: String,
        candidates: Vec<MaskCandidate>,
    },
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
    PlannedEdit,
    Caption,
    Feedback,
}

/// What the approval policy answers with.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    SelectMask(usize),
    AcceptText,
    Reject,
    ProvideText(String),
    ProvidePlan(PlannedEdit),
}

/// How checkpoints are resolved.
pub enum ApprovalPolicy {
    /// Non-interactive: picks candidate `mask_index`, accepts/rejects text
    /// per `accept_text`, and rejects degraded LLM steps.
    Auto {
        mask_index: usize,
        accept_text: bool,
    },
    /// Interactive: the closure is called for every block.
    Interactive(Box<dyn Fn(BlockedOn) -> Decision + Send>),
}

impl ApprovalPolicy {
    pub fn auto() -> Self {
        Self::Auto {
            mask_index: 0,
            accept_text: true,
        }
    }

    fn decide(&self, blocked: BlockedOn) -> Decision {
        match self {
            ApprovalPolicy::Auto {
                mask_index,
                accept_text,
            } => match blocked {
                BlockedOn::SelectMask { .. } => Decision::SelectMask(*mask_index),
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
        BlockedOn::SelectMask { .. } => Decision::SelectMask(0),
        BlockedOn::ApproveText { .. } => Decision::AcceptText,
        BlockedOn::ProvideInput { .. } => Decision::Reject,
    }
}

/// Execution options for [`Pipeline::run`].
pub struct RunOptions {
    /// Where pure image steps (composite, feather, …) write their outputs.
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
    Mask,
    Text,
    Plan,
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

/// The backend the pipeline executes against. The live implementation wraps
/// `CreativeRuntime` + `LmStudioClient` (see `backend.rs`); tests inject a
/// fake to assert request bodies and state transitions.
pub trait GenerationBackend: Send + Sync {
    fn segment(
        &self,
        request: &SegmentRequest,
    ) -> BoxFuture<'_, Result<SegmentResponse, PipelineError>>;
    fn generate(
        &self,
        request: &GenerateRequest,
    ) -> BoxFuture<'_, Result<crate::vision::GenerateResponse, PipelineError>>;
    /// Returns `None` when no LLM is available (soft dependency).
    fn llm_draft(&self, request: &str) -> BoxFuture<'_, Result<Option<String>, PipelineError>>;
    /// Returns `None` when no LLM is available (soft dependency).
    fn llm_plan(&self, request: &str) -> BoxFuture<'_, Result<Option<PlannedEdit>, PipelineError>>;
    /// Rewrites a terse edit instruction into a grounded one. Returns
    /// `None` when no LLM is available; the caller holds a deterministic
    /// fallback, so this never blocks. The default declines, which makes
    /// every backend without an LLM use that fallback.
    fn llm_ground(&self, _request: &str) -> BoxFuture<'_, Result<Option<String>, PipelineError>> {
        Box::pin(async { Ok(None) })
    }
    /// Moves a locally-produced final image (e.g. the composite fallback
    /// output) into the persistent asset store, returning its stored path.
    /// The default keeps the source path — backends without a store (the
    /// test doubles) leave results in place.
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
    #[error("image operation failed: {0}")]
    Image(#[from] ImageOpsError),
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
            Self::Image(error) => error.code(),
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
    #[error("a native mask requires a reference image")]
    MaskWithoutReference,
    #[error("unsupported model {0}; expected one of the native Krea profiles")]
    BadModel(String),
    #[error("invalid size {width}x{height}: dimensions must be multiples of 32 within 256..=2048")]
    BadSize { width: u32, height: u32 },
    #[error("invalid step count {0}: expected 1..=50")]
    BadSteps(u32),
    #[error("too many LoRAs ({0}); the backend accepts at most 8")]
    TooManyLoras(usize),
    #[error("LoRA multiplier {0} is outside the -2.0..=2.0 contract bounds")]
    BadLoraMultiplier(f32),
    #[error("feather radius {0} is outside the 1..=64 bounds")]
    BadFeatherRadius(u32),
}

impl CodedError for PipelineBuildError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Empty => ErrorCode::OpBuildEmpty,
            Self::Ordering { .. } => ErrorCode::OpBuildOrdering,
            Self::MaskWithoutReference => ErrorCode::OpBuildMaskWithoutReference,
            Self::BadModel(_) => ErrorCode::OpBuildBadModel,
            Self::BadSize { .. } => ErrorCode::OpBuildBadSize,
            Self::BadSteps(_) => ErrorCode::OpBuildBadSteps,
            Self::TooManyLoras(_) => ErrorCode::OpBuildTooManyLoras,
            Self::BadLoraMultiplier(_) => ErrorCode::OpBuildBadLoraMultiplier,
            Self::BadFeatherRadius(_) => ErrorCode::OpBuildBadFeatherRadius,
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

    pub fn seed(&mut self, seed: u64) -> &mut Self {
        self.params.seed = Some(seed);
        self
    }

    /// How strongly a reference image guides the generation (0–1). The
    /// sd.cpp default (0.9) renoises almost everything and distorts
    /// turbo-model outputs, so ops that pass a reference set a tuned value.
    pub fn reference_strength(&mut self, strength: f32) -> &mut Self {
        self.params.reference_strength = Some(strength);
        self
    }

    pub fn denoise_strength(&mut self, strength: f32) -> &mut Self {
        self.params.denoise_strength = Some(strength);
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

    /// A generation step: fresh (no reference), an edit (reference), or a
    /// native-masked inpaint. -> Image
    pub fn generate(
        &mut self,
        prompt: impl Into<PromptSource>,
        reference: Option<ImageHandle>,
        mask: Option<SelectedMaskHandle>,
    ) -> ImageHandle {
        let step = self.steps.len();
        self.steps.push(Step::Generate {
            prompt: prompt.into(),
            reference,
            mask,
            init: None,
        });
        ImageHandle { step }
    }

    /// A masked generation that starts from a distinct init image (see
    /// [`Step::Generate`]'s `init`). -> Image
    pub fn generate_from(
        &mut self,
        prompt: impl Into<PromptSource>,
        reference: Option<ImageHandle>,
        mask: Option<SelectedMaskHandle>,
        init: ImageHandle,
    ) -> ImageHandle {
        let step = self.steps.len();
        self.steps.push(Step::Generate {
            prompt: prompt.into(),
            reference,
            mask,
            init: Some(init),
        });
        ImageHandle { step }
    }

    /// Pre-fills the masked region with colour diffused in from outside it.
    /// -> Image
    pub fn fill_masked(&mut self, image: ImageHandle, mask: SelectedMaskHandle) -> ImageHandle {
        let step = self.steps.len();
        self.steps.push(Step::FillMasked { image, mask });
        ImageHandle { step }
    }

    /// Loads a mask PNG as an already-confirmed mask. -> SelectedMask
    ///
    /// The counterpart to `segment` + `confirm_mask`: same downstream type,
    /// no checkpoint. See [`Step::LoadMask`].
    pub fn load_mask(&mut self, source: impl Into<PathBuf>) -> SelectedMaskHandle {
        let step = self.steps.len();
        self.steps.push(Step::LoadMask {
            source: source.into(),
        });
        SelectedMaskHandle { step }
    }

    /// Segments the working image into a candidate mask set. -> Mask
    pub fn segment(&mut self, image: ImageHandle, prompt: impl Into<PromptSource>) -> MaskHandle {
        let step = self.steps.len();
        self.steps.push(Step::Segment {
            image,
            prompt: prompt.into(),
            points: Vec::new(),
            boxes: Vec::new(),
        });
        MaskHandle { step }
    }

    /// Appends the mask-confirmation checkpoint. The returned handle is a
    /// confirmed mask; only confirmed masks can feed inpaint/composite. The
    /// handle points at the segment step's slot, which the checkpoint's
    /// approval resolves to the chosen candidate at run time.
    pub fn confirm_mask(&mut self, mask: MaskHandle) -> SelectedMaskHandle {
        self.steps.push(Step::Checkpoint {
            description: "confirm the segmentation mask".to_owned(),
            on: CheckpointOn::SelectMask(mask),
        });
        SelectedMaskHandle { step: mask.step }
    }

    /// The canonical regional edit: masked-latent inpaint (generate with the
    /// reference, mask, and init image so the backend keeps the unmasked
    /// pixels fixed), then composite to smooth the latent-level seam.
    ///
    /// The mask is grown twice, for two different reasons. First by
    /// [`MASK_GENERATION_GROW`], widening what the backend is *allowed to
    /// re-render* — a segmentation mask stops at the silhouette it can
    /// resolve, so softer edges fall outside it and survive the edit
    /// untouched. Then the composite grows again by [`COMPOSITE_MASK_GROW`]
    /// to widen the *blend*, which is why it does not leave the outside
    /// bit-identical. Callers needing the strict guarantee should use
    /// [`Self::composite`] with `grow` 0. -> Image
    pub fn inpaint(
        &mut self,
        prompt: impl Into<PromptSource>,
        image: ImageHandle,
        mask: SelectedMaskHandle,
    ) -> ImageHandle {
        let mask = self.grow_mask(mask, MASK_GENERATION_GROW);
        // The reference stays the untouched original — it is what the edit
        // adapter reads identity from. Only the init is pre-filled, so the
        // masked region starts from a neutral prior rather than from the
        // content the edit is removing.
        let init = self.fill_masked(image, mask);
        let generated = self.generate_from(prompt, Some(image), Some(mask), init);
        let step = self.steps.len();
        self.steps.push(Step::Composite {
            original: image,
            mask,
            replacement: generated,
            feather_radius: COMPOSITE_FEATHER_RADIUS,
            mask_grow: COMPOSITE_MASK_GROW,
        });
        ImageHandle { step }
    }

    /// Explicit composite step (e.g. with a custom feather radius, or
    /// `mask_grow` 0 for the bit-identical-outside guarantee). -> Image
    pub fn composite(
        &mut self,
        original: ImageHandle,
        mask: SelectedMaskHandle,
        replacement: ImageHandle,
        feather_radius: u32,
        mask_grow: u32,
    ) -> ImageHandle {
        let step = self.steps.len();
        self.steps.push(Step::Composite {
            original,
            mask,
            replacement,
            feather_radius,
            mask_grow,
        });
        ImageHandle { step }
    }

    pub fn invert(&mut self, mask: SelectedMaskHandle) -> SelectedMaskHandle {
        let step = self.steps.len();
        self.steps.push(Step::Invert { mask });
        SelectedMaskHandle { step }
    }

    pub fn feather(&mut self, mask: SelectedMaskHandle, radius: u32) -> SelectedMaskHandle {
        let step = self.steps.len();
        self.steps.push(Step::Feather { mask, radius });
        SelectedMaskHandle { step }
    }

    /// Widens the mask the backend re-renders through. See
    /// [`crate::registry::image_ops::grow`].
    pub fn grow_mask(&mut self, mask: SelectedMaskHandle, radius: u32) -> SelectedMaskHandle {
        let step = self.steps.len();
        self.steps.push(Step::GrowMask { mask, radius });
        SelectedMaskHandle { step }
    }

    pub fn union(&mut self, masks: Vec<SelectedMaskHandle>) -> SelectedMaskHandle {
        let step = self.steps.len();
        self.steps.push(Step::Union { masks });
        SelectedMaskHandle { step }
    }

    pub fn paint_strokes(
        &mut self,
        image: ImageHandle,
        strokes: Vec<Stroke>,
    ) -> SelectedMaskHandle {
        let step = self.steps.len();
        self.steps.push(Step::PaintStrokes { image, strokes });
        SelectedMaskHandle { step }
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

    /// An LLM edit plan: description -> mask/inpaint prompts. -> Plan
    pub fn llm_plan(&mut self, request: impl Into<String>) -> PlanHandle {
        let step = self.steps.len();
        self.steps.push(Step::Llm {
            kind: LlmStepKind::Plan {
                request: request.into(),
            },
        });
        PlanHandle { step }
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
    if params.model != "krea-2-turbo-q2" && params.model != "krea-2-turbo-q4" {
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
        match step {
            Step::Generate {
                mask: Some(_),
                reference: None,
                ..
            } => return Err(PipelineBuildError::MaskWithoutReference),
            Step::Feather { radius, .. } if *radius > 64 => {
                return Err(PipelineBuildError::BadFeatherRadius(*radius))
            }
            _ => {}
        }
    }
    Ok(())
}

/// All step indices a step consumes.
fn step_references(step: &Step) -> Vec<usize> {
    match step {
        Step::LoadImage { .. } | Step::LoadMask { .. } => Vec::new(),
        Step::FillMasked { image, mask } => vec![image.step, mask.step],
        Step::Segment { image, .. } => vec![image.step],
        Step::Checkpoint { on, .. } => match on {
            CheckpointOn::SelectMask(mask) => vec![mask.step],
            CheckpointOn::ApproveText(text) => vec![text.step],
        },
        Step::Generate {
            reference, mask, ..
        } => {
            let mut handles = Vec::new();
            if let Some(image) = reference {
                handles.push(image.step);
            }
            if let Some(mask) = mask {
                handles.push(mask.step);
            }
            handles
        }
        Step::Composite {
            original,
            mask,
            replacement,
            ..
        } => {
            vec![original.step, mask.step, replacement.step]
        }
        Step::Invert { mask } => vec![mask.step],
        Step::Feather { mask, .. } | Step::GrowMask { mask, .. } => vec![mask.step],
        Step::Union { masks } => masks.iter().map(|mask| mask.step).collect(),
        Step::PaintStrokes { image, .. } => vec![image.step],
        Step::Llm { kind } => match kind {
            LlmStepKind::Draft { .. } | LlmStepKind::Plan { .. } | LlmStepKind::Ground { .. } => {
                Vec::new()
            }
            LlmStepKind::Describe { image } => vec![image.step],
            LlmStepKind::Critique { image, .. } => vec![image.step],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    // ---------------------------------------------------------------- helpers

    fn work_dir() -> PathBuf {
        std::env::temp_dir().join(format!("svs-pipeline-{}", Uuid::new_v4()))
    }

    fn write_rgba(path: &Path, width: u32, height: u32, color: [u8; 4]) {
        image::ImageBuffer::from_pixel(width, height, image::Rgba(color))
            .save(path)
            .expect("test image should save");
    }

    fn write_mask(path: &Path, width: u32, height: u32, left_half: bool) {
        let mut mask = image::GrayImage::from_pixel(width, height, image::Luma([0]));
        for y in 0..height {
            for x in 0..width / 2 {
                mask.put_pixel(x, y, image::Luma([if left_half { 255 } else { 0 }]));
            }
        }
        mask.save(path).expect("test mask should save");
    }

    #[derive(Clone, Debug)]
    enum CapturedRequest {
        Segment(SegmentRequest),
        Generate(GenerateRequest),
    }

    #[derive(Default)]
    struct FakeBackend {
        masks: Mutex<Vec<MaskCandidate>>,
        generated_image: Mutex<Option<PathBuf>>,
        draft_text: Mutex<Option<String>>,
        plan: Mutex<Option<PlannedEdit>>,
        captured: Mutex<Vec<CapturedRequest>>,
    }

    impl FakeBackend {
        fn with_generated(path: PathBuf) -> Self {
            let fake = Self::default();
            *fake.generated_image.lock().unwrap() = Some(path);
            fake
        }

        fn with_masks(self, masks: Vec<MaskCandidate>) -> Self {
            *self.masks.lock().unwrap() = masks;
            self
        }

        fn requests(&self) -> Vec<CapturedRequest> {
            self.captured.lock().unwrap().clone()
        }
    }

    impl GenerationBackend for FakeBackend {
        fn segment(
            &self,
            request: &SegmentRequest,
        ) -> BoxFuture<'_, Result<SegmentResponse, PipelineError>> {
            let request = request.clone();
            let masks = self.masks.lock().unwrap().clone();
            Box::pin(async move {
                self.captured
                    .lock()
                    .unwrap()
                    .push(CapturedRequest::Segment(request.clone()));
                Ok(SegmentResponse {
                    status: "completed".to_owned(),
                    masks: masks
                        .iter()
                        .map(|candidate| crate::vision::SegmentMask {
                            path: candidate.path.display().to_string(),
                            score: candidate.score,
                            area_pixels: candidate.area_pixels,
                            bounding_box: candidate.bounding_box.clone(),
                        })
                        .collect(),
                    detections: Vec::new(),
                    model: None,
                    device: None,
                    dtype: None,
                    duration_ms: None,
                    error: None,
                })
            })
        }

        fn generate(
            &self,
            request: &GenerateRequest,
        ) -> BoxFuture<'_, Result<crate::vision::GenerateResponse, PipelineError>> {
            let request = request.clone();
            let image = self.generated_image.lock().unwrap().clone();
            Box::pin(async move {
                self.captured
                    .lock()
                    .unwrap()
                    .push(CapturedRequest::Generate(request.clone()));
                match image {
                    Some(path) => Ok(crate::vision::GenerateResponse {
                        status: "completed".to_owned(),
                        image_path: Some(path.display().to_string()),
                        model: request.model.clone(),
                        device: None,
                        dtype: None,
                        seed: request.seed,
                        width: request.width,
                        height: request.height,
                        duration_ms: None,
                        error: None,
                    }),
                    None => Ok(crate::vision::GenerateResponse {
                        status: "failed".to_owned(),
                        image_path: None,
                        model: request.model.clone(),
                        device: None,
                        dtype: None,
                        seed: None,
                        width: None,
                        height: None,
                        duration_ms: None,
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

        fn llm_plan(
            &self,
            _request: &str,
        ) -> BoxFuture<'_, Result<Option<PlannedEdit>, PipelineError>> {
            let plan = self.plan.lock().unwrap().clone();
            Box::pin(async move { Ok(plan) })
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
        builder.generate("a frame", None, None);
        assert!(matches!(
            builder.build().unwrap_err(),
            PipelineBuildError::BadSize { .. }
        ));

        let mut builder = PipelineBuilder::new();
        builder.steps(0);
        builder.generate("a frame", None, None);
        assert!(matches!(
            builder.build().unwrap_err(),
            PipelineBuildError::BadSteps(0)
        ));

        let mut builder = PipelineBuilder::new();
        builder.model("sd-xl");
        builder.generate("a frame", None, None);
        assert!(matches!(
            builder.build().unwrap_err(),
            PipelineBuildError::BadModel(_)
        ));
    }

    #[test]
    fn build_rejects_a_native_mask_without_a_reference_image() {
        let steps = vec![
            Step::Generate {
                prompt: PromptSource::Text("a face".into()),
                reference: None,
                mask: None,
                init: None,
            },
            Step::Checkpoint {
                description: "confirm".into(),
                on: CheckpointOn::SelectMask(MaskHandle { step: 0 }),
            },
            Step::Generate {
                prompt: PromptSource::Text("new hair".into()),
                reference: None,
                mask: Some(SelectedMaskHandle { step: 1 }),
                init: None,
            },
        ];
        let pipeline = Pipeline {
            steps,
            params: GenerationParams::default(),
        };
        let error = validate(&pipeline.steps, &pipeline.params).unwrap_err();
        assert!(matches!(error, PipelineBuildError::MaskWithoutReference));
    }

    #[test]
    fn build_rejects_out_of_order_handles() {
        let steps = vec![Step::Generate {
            prompt: PromptSource::Text("a frame".into()),
            reference: Some(ImageHandle { step: 1 }), // references a later step
            mask: None,
            init: None,
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
        builder.size(768, 448).steps(4).seed(42);
        let image = builder.reference_image("frame.png");
        builder.generate("make it warmer", Some(image), None);
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
        builder.size(768, 448).steps(4).seed(7);
        let image = builder.reference_image(&original);
        builder.generate("a lighthouse at dusk", Some(image), None);
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
                && request.mask_path.is_none()
                && request.init_image_path.is_none()
                && request.denoise_strength.is_none()
                && request.width == Some(768)
                && request.height == Some(448)
                && request.steps == Some(4)
                && request.seed == Some(7)
                && request.model.as_deref() == Some(DEFAULT_MODEL)
                && request.loras.is_empty()
            )
        );
    }

    #[test]
    fn modify_pipeline_confirms_the_mask_and_composites() {
        let dir = work_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let generated = dir.join("generated.png");
        let candidate = dir.join("mask-candidate.png");
        // Large enough that "beyond the grown mask" is a real region: the
        // mask is widened once for generation and again for the blend.
        write_rgba(&original, 256, 192, [200, 40, 40, 255]);
        write_rgba(&generated, 256, 192, [40, 40, 200, 255]);
        write_mask(&candidate, 256, 192, true);

        let backend = FakeBackend::with_generated(generated.clone()).with_masks(vec![
            MaskCandidate {
                path: candidate.clone(),
                score: 0.97,
                area_pixels: 1536,
                bounding_box: crate::vision::SegmentBox {
                    x_min: 0.0,
                    y_min: 0.0,
                    x_max: 128.0,
                    y_max: 192.0,
                },
            },
            MaskCandidate {
                path: dir.join("other.png"),
                score: 0.2,
                area_pixels: 100,
                bounding_box: crate::vision::SegmentBox {
                    x_min: 40.0,
                    y_min: 40.0,
                    x_max: 60.0,
                    y_max: 48.0,
                },
            },
        ]);

        let mut builder = PipelineBuilder::new();
        let image = builder.reference_image(&original);
        let candidates = builder.segment(image, "her hair");
        let mask = builder.confirm_mask(candidates);
        builder.denoise_strength(MASKED_LATENT_DENOISE);
        builder.inpaint("a short bob cut", image, mask);
        let pipeline = builder.build().unwrap();

        let run = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run(&backend, &RunOptions::new(dir.join("work"))))
            .unwrap();

        // Auto policy picked the highest-scoring candidate (index 0).
        let decision = &run.decisions[0];
        assert_eq!(decision.decision, Decision::SelectMask(0));

        // The segment request carried the image and the mask prompt.
        let requests = backend.requests();
        assert!(matches!(&requests[0], CapturedRequest::Segment(request) if
            request.image_path == original.to_str().unwrap()
            && request.prompt.as_deref() == Some("her hair")
        ));
        // The generate request is a masked-latent inpaint: reference + mask
        // + init image (the backend keeps the unmasked pixels fixed); the
        // composite step below remains the fallback.
        //
        // Reference and init are deliberately *different* files here: the
        // reference stays the untouched original so the edit adapter reads
        // identity from it, while the init is the pre-filled image so the
        // masked region starts from the surrounding palette.
        assert!(matches!(&requests[1], CapturedRequest::Generate(request) if
            request.prompt == "a short bob cut"
            && request.reference_image_path.as_deref() == Some(original.to_str().unwrap())
            // Not the raw candidate: the mask handed to the backend is
            // widened first, so softer edges than SAM resolved still get
            // re-rendered.
            && request.mask_path.as_deref().is_some_and(|path|
                path.contains("grow-mask") && path != candidate.to_str().unwrap())
            && request.init_image_path.as_deref().is_some_and(|path|
                path.contains("fill-masked") && path != original.to_str().unwrap())
            // The full sigma schedule: truncating it (the backend's 0.75
            // default) only weakens the edit inside the mask.
            && request.denoise_strength == Some(MASKED_LATENT_DENOISE)
        ));

        // The composite is the final image and preserves the outside pixels.
        let composite = run
            .final_image()
            .expect("composite should be the final output");
        let result = image::open(composite).unwrap().to_rgba8();
        let reference = image::open(&original).unwrap().to_rgba8();
        // `inpaint` grows the mask by COMPOSITE_MASK_GROW before feathering,
        // so the blend deliberately reaches past the boundary — that band is
        // what covers a mask running tighter than its subject. Beyond the
        // grow plus the feather, the original still survives untouched, so
        // this stays a regional edit and never a whole-frame paste.
        let reach = MASK_GENERATION_GROW + COMPOSITE_MASK_GROW + COMPOSITE_FEATHER_RADIUS;
        for y in 0..192 {
            for x in (128 + reach)..256 {
                assert_eq!(
                    result.get_pixel(x, y),
                    reference.get_pixel(x, y),
                    "pixel ({x}, {y}) beyond the grown mask must be bit-identical"
                );
            }
        }
        assert_ne!(
            result.get_pixel(130, 96),
            reference.get_pixel(130, 96),
            "the grown band just outside the mask must be blended"
        );
    }

    /// The same inpaint, sourced from a mask file instead of a segmentation:
    /// same request to the backend, same preserved pixels, but no segment
    /// call and nothing to decide.
    #[test]
    fn a_mask_file_inpaints_without_segmenting_or_blocking() {
        let dir = work_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let generated = dir.join("generated.png");
        let fixture = dir.join("fixture-mask.png");
        write_rgba(&original, 256, 192, [200, 40, 40, 255]);
        write_rgba(&generated, 256, 192, [40, 40, 200, 255]);
        write_mask(&fixture, 256, 192, true);

        let backend = FakeBackend::with_generated(generated.clone());

        let mut builder = PipelineBuilder::new();
        let image = builder.reference_image(&original);
        let mask = builder.load_mask(&fixture);
        builder.denoise_strength(MASKED_LATENT_DENOISE);
        builder.inpaint("a short bob cut", image, mask);
        let pipeline = builder.build().unwrap();

        let run = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run(&backend, &RunOptions::new(dir.join("work"))))
            .unwrap();

        assert!(
            run.decisions.is_empty(),
            "a mask file was already chosen; there is no candidate to confirm"
        );
        let requests = backend.requests();
        assert!(
            !requests
                .iter()
                .any(|request| matches!(request, CapturedRequest::Segment(_))),
            "a mask file must not reach the segmenter"
        );
        assert!(matches!(&requests[0], CapturedRequest::Generate(request) if
            request.mask_path.as_deref().is_some_and(|path|
                path.contains("grow-mask") && path != fixture.to_str().unwrap())
            && request.reference_image_path.as_deref() == Some(original.to_str().unwrap())
            && request.init_image_path.as_deref().is_some_and(|path| path.contains("fill-masked"))
            && request.denoise_strength == Some(MASKED_LATENT_DENOISE)
        ));

        let composite = run
            .final_image()
            .expect("composite should be the final output");
        let result = image::open(composite).unwrap().to_rgba8();
        let reference = image::open(&original).unwrap().to_rgba8();
        // `inpaint` grows the mask by COMPOSITE_MASK_GROW before feathering,
        // so the blend deliberately reaches past the boundary — that band is
        // what covers a mask running tighter than its subject. Beyond the
        // grow plus the feather, the original still survives untouched, so
        // this stays a regional edit and never a whole-frame paste.
        let reach = MASK_GENERATION_GROW + COMPOSITE_MASK_GROW + COMPOSITE_FEATHER_RADIUS;
        for y in 0..192 {
            for x in (128 + reach)..256 {
                assert_eq!(
                    result.get_pixel(x, y),
                    reference.get_pixel(x, y),
                    "pixel ({x}, {y}) beyond the grown mask must be bit-identical"
                );
            }
        }
        assert_ne!(
            result.get_pixel(130, 96),
            reference.get_pixel(130, 96),
            "the grown band just outside the mask must be blended"
        );
    }

    /// A missing mask file fails the step by name rather than reaching the
    /// backend with an empty mask — the failure mode that produced the
    /// "nonsensical edit" when `mask_images` was silently dropped.
    #[test]
    fn a_missing_mask_file_fails_the_step() {
        let dir = work_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let generated = dir.join("generated.png");
        write_rgba(&original, 64, 48, [200, 40, 40, 255]);
        write_rgba(&generated, 64, 48, [40, 40, 200, 255]);

        let backend = FakeBackend::with_generated(generated);
        let mut builder = PipelineBuilder::new();
        let image = builder.reference_image(&original);
        let mask = builder.load_mask(dir.join("absent.png"));
        builder.inpaint("a short bob cut", image, mask);
        let pipeline = builder.build().unwrap();

        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run(&backend, &RunOptions::new(dir.join("work"))))
            .unwrap_err();
        assert!(
            matches!(&error, PipelineError::Step { message, .. } if message.contains("absent.png")),
            "{error:?}"
        );
        assert!(
            backend.requests().is_empty(),
            "the run must stop before generating"
        );
    }

    #[test]
    fn rejecting_the_mask_ends_the_stack_cleanly() {
        let dir = work_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let candidate = dir.join("mask.png");
        write_rgba(&original, 64, 48, [0, 0, 0, 255]);
        write_mask(&candidate, 64, 48, true);
        let backend = FakeBackend::with_generated(dir.join("generated.png")).with_masks(vec![
            MaskCandidate {
                path: candidate,
                score: 0.9,
                area_pixels: 100,
                bounding_box: crate::vision::SegmentBox {
                    x_min: 0.0,
                    y_min: 0.0,
                    x_max: 10.0,
                    y_max: 10.0,
                },
            },
        ]);

        let mut builder = PipelineBuilder::new();
        let image = builder.reference_image(&original);
        let candidates = builder.segment(image, "hair");
        let mask = builder.confirm_mask(candidates);
        builder.inpaint("new look", image, mask);
        let pipeline = builder.build().unwrap();

        let options = RunOptions {
            work_dir: dir.join("work"),
            approvals: ApprovalPolicy::Auto {
                mask_index: 0,
                accept_text: true,
            },
            seed: None,
        };
        // A policy that rejects the mask.
        let options = RunOptions {
            approvals: ApprovalPolicy::Interactive(Box::new(|_| Decision::Reject)),
            ..options
        };

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run(&backend, &options));
        assert!(matches!(result, Err(PipelineError::Rejected)));
    }

    #[test]
    fn auto_decision_matches_the_auto_policy_defaults() {
        assert_eq!(
            auto_decision(BlockedOn::SelectMask {
                description: String::new(),
                candidates: Vec::new()
            }),
            Decision::SelectMask(0)
        );
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
                purpose: InputPurpose::PlannedEdit
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
                _ => Decision::Reject,
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
        builder.generate("doomed", Some(image), None);
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
        builder.generate("frame", Some(image), None);
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
