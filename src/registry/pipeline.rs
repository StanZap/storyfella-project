//! The pipeline execution layer (`docs/artifact-canvas.md` §7).
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

use crate::vision::{
    ComputeDevice, GenerateRequest, LoraSelection, SegmentBox, SegmentPoint, SegmentRequest,
    SegmentResponse,
};

use super::image_ops::{self, ImageOpsError};

/// The focus generation model (Krea 2 via stable-diffusion.cpp).
pub const DEFAULT_MODEL: &str = "krea-2-turbo-q2";
/// The interactive draft profile used by the UI and the CLI defaults.
pub const DEFAULT_WIDTH: u32 = 768;
pub const DEFAULT_HEIGHT: u32 = 448;
pub const DEFAULT_STEPS: u32 = 4;
/// Seam-blending radius for the composite fallback.
pub const COMPOSITE_FEATHER_RADIUS: u32 = 8;

/// Generation parameters shared by every `Generate` step in a pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationParams {
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: Option<u64>,
    pub loras: Vec<LoraSelection>,
}

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_owned(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            steps: DEFAULT_STEPS,
            seed: None,
            loras: Vec::new(),
        }
    }
}

/// Per-run generation overrides (CLI flags like `--steps`/`--size`/`--model`;
/// the operation itself carries only intent). Applied to the compiled
/// pipeline and re-validated before execution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerationOverrides {
    pub steps: Option<u32>,
    pub size: Option<(u32, u32)>,
    pub model: Option<String>,
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
    },
    /// The guaranteed mask-edit primitive: blends `replacement` into
    /// `original` everywhere the confirmed mask is active. -> Image
    Composite {
        original: ImageHandle,
        mask: SelectedMaskHandle,
        replacement: ImageHandle,
        feather_radius: u32,
    },
    Invert {
        mask: SelectedMaskHandle,
    }, // -> SelectedMask
    Feather {
        mask: SelectedMaskHandle,
        radius: u32,
    }, // -> SelectedMask
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
                        reference_image_path: reference_path.map(|path| path.display().to_string()),
                        width: Some(self.params.width),
                        height: Some(self.params.height),
                        steps: Some(self.params.steps),
                        seed,
                        model: Some(self.params.model.clone()),
                        device: Some(ComputeDevice::Auto),
                        loras: self.params.loras.clone(),
                        mask_path: mask_path.map(|path| path.display().to_string()),
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
        Step::Segment { .. } => "segment",
        Step::Checkpoint { .. } => "checkpoint",
        Step::Generate { .. } => "generate",
        Step::Composite { .. } => "composite",
        Step::Invert { .. } => "invert",
        Step::Feather { .. } => "feather",
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
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("backend error: {0}")]
    Backend(String),
    #[error("step {step} failed: {message}")]
    Step { step: usize, message: String },
    #[error("the stack was rejected at a checkpoint")]
    Rejected,
    #[error("image operation failed: {0}")]
    Image(#[from] ImageOpsError),
}

impl PipelineError {
    pub fn backend(error: impl std::fmt::Display) -> Self {
        Self::Backend(error.to_string())
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
        });
        ImageHandle { step }
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

    /// The canonical regional edit: generate with the reference image, then
    /// composite so everything outside the confirmed mask is bit-identical
    /// to the original. -> Image
    pub fn inpaint(
        &mut self,
        prompt: impl Into<PromptSource>,
        image: ImageHandle,
        mask: SelectedMaskHandle,
    ) -> ImageHandle {
        let generated = self.generate(prompt, Some(image), None);
        let step = self.steps.len();
        self.steps.push(Step::Composite {
            original: image,
            mask,
            replacement: generated,
            feather_radius: COMPOSITE_FEATHER_RADIUS,
        });
        ImageHandle { step }
    }

    /// Explicit composite step (e.g. with a custom feather radius). -> Image
    pub fn composite(
        &mut self,
        original: ImageHandle,
        mask: SelectedMaskHandle,
        replacement: ImageHandle,
        feather_radius: u32,
    ) -> ImageHandle {
        let step = self.steps.len();
        self.steps.push(Step::Composite {
            original,
            mask,
            replacement,
            feather_radius,
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

fn validate(steps: &[Step], params: &GenerationParams) -> Result<(), PipelineBuildError> {
    if steps.is_empty() {
        return Err(PipelineBuildError::Empty);
    }
    if !params.width.is_multiple_of(32)
        || !params.height.is_multiple_of(32)
        || !(256..=2048).contains(&params.width)
        || !(256..=2048).contains(&params.height)
    {
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
        Step::LoadImage { .. } => Vec::new(),
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
        Step::Feather { mask, .. } => vec![mask.step],
        Step::Union { masks } => masks.iter().map(|mask| mask.step).collect(),
        Step::PaintStrokes { image, .. } => vec![image.step],
        Step::Llm { kind } => match kind {
            LlmStepKind::Draft { .. } | LlmStepKind::Plan { .. } => Vec::new(),
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
            },
            Step::Checkpoint {
                description: "confirm".into(),
                on: CheckpointOn::SelectMask(MaskHandle { step: 0 }),
            },
            Step::Generate {
                prompt: PromptSource::Text("new hair".into()),
                reference: None,
                mask: Some(SelectedMaskHandle { step: 1 }),
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
        write_rgba(&original, 64, 48, [200, 40, 40, 255]);
        write_rgba(&generated, 64, 48, [40, 40, 200, 255]);
        write_mask(&candidate, 64, 48, true);

        let backend = FakeBackend::with_generated(generated.clone()).with_masks(vec![
            MaskCandidate {
                path: candidate.clone(),
                score: 0.97,
                area_pixels: 1536,
                bounding_box: crate::vision::SegmentBox {
                    x_min: 0.0,
                    y_min: 0.0,
                    x_max: 32.0,
                    y_max: 48.0,
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
        // The generate request was a plain edit (composite mode: no native mask).
        assert!(matches!(&requests[1], CapturedRequest::Generate(request) if
            request.prompt == "a short bob cut"
            && request.reference_image_path.as_deref() == Some(original.to_str().unwrap())
            && request.mask_path.is_none()
        ));

        // The composite is the final image and preserves the outside pixels.
        let composite = run
            .final_image()
            .expect("composite should be the final output");
        let result = image::open(composite).unwrap().to_rgba8();
        let reference = image::open(&original).unwrap().to_rgba8();
        for y in 0..48 {
            for x in 32..64 {
                assert_eq!(
                    result.get_pixel(x, y),
                    reference.get_pixel(x, y),
                    "pixel ({x}, {y}) outside the mask must be bit-identical"
                );
            }
        }
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
