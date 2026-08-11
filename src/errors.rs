//! The error-code registry — the single collection every error in the app
//! maps to.
//!
//! Every [`thiserror`] type implements [`CodedError`], which yields a stable
//! [`ErrorCode`]. A code carries three strings:
//!
//! - `code()` — the stable identifier (`SVS-<DOMAIN>-<NNN>`) shown in logs
//!   and in the UI;
//! - `message()` — a short human label for the error banner;
//! - `hint()` — the full explanation the details modal shows (what went
//!   wrong, what to do about it).
//!
//! Errors are surfaced through [`CodedError::report`], which logs the error
//! with its code and returns a [`ReportedError`] — the payload the UI
//! consumes (banner: code + message; modal: full details + hint + copy).

use std::fmt;

/// The master error-code registry. Variants are grouped by domain; the code
/// strings are stable and must not be reused or renumbered (new codes are
/// appended at the end of their domain's block).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    // ---- REG · registry & refs (src/registry/mod.rs) ----
    RefInvalid,
    RefNotFound,
    RefAmbiguous,
    RegistryNoArtifact,
    RegistryNotVariantable,
    RegistryVariantOfVariant,
    RegistryVariantKindMismatch,
    RegistryBadParent,
    RegistryNoParentAllowed,
    RegistryNotABeat,
    RegistryNotInScene,
    RegistryNoRevision,
    RegistryInvalidSize,
    RegistryNoLoraEntry,
    RegistryLoraNotFound,
    RegistryLoraAmbiguous,
    RegistryBadLoraMultiplier,

    // ---- OP · ops, compile, pipeline (src/registry/ops.rs, pipeline.rs) ----
    OpBackendRequired,
    OpEmptyDescription,
    OpInvalidSize,
    OpNotVariantable,
    OpNotAScene,
    OpNotAnEnvironment,
    OpBadLayerKind,
    OpNoActiveImage,
    OpNoPrompt,
    OpIncompleteModify,
    OpConflictingMaskSource,
    OpBadLoraMultiplier,
    OpBadSteps,
    OpLoraFileMissing,
    OpApplyFailed,
    OpBackendFailed,
    OpStepFailed,
    OpRejected,
    OpBuildEmpty,
    OpBuildOrdering,
    OpBuildMaskWithoutReference,
    OpBuildBadModel,
    OpBuildBadSize,
    OpBuildBadSteps,
    OpBuildTooManyLoras,
    OpBuildBadLoraMultiplier,
    OpBuildBadFeatherRadius,

    // ---- IMG · image operations (src/registry/image_ops.rs) ----
    ImgOpen,
    ImgWrite,
    ImgDimensionMismatch,
    ImgEmptyUnion,

    // ---- STO · persistence (src/persistence/mod.rs) ----
    StoreOpen,
    StoreConfigure,
    StoreMigrate,
    StoreSave,
    StoreSerialize,
    StoreLoad,
    StoreCorrupt,

    // ---- CFG · config (src/app/config.rs) ----
    ConfigPlatformDirectories,
    ConfigRead,
    ConfigParse,
    ConfigWrite,
    ConfigGenerationProfile,
    ConfigInvalidSize,
    ConfigMalformedSize,

    // ---- LLM · LM Studio client (src/llm/mod.rs) ----
    LlmBuild,
    LlmRequest,
    LlmStatus,
    LlmPlanner,

    // ---- VIS · vision client (src/vision/mod.rs) ----
    VisionRequest,
    VisionStatus,

    // ---- RTE · runtimes (src/runtime/) ----
    RuntimeAlreadyRunning,
    RuntimeMissingPython,
    RuntimeResolve,
    RuntimeStart,
    RuntimeStop,
    RuntimeGenerationAlreadyRunning,
    RuntimeGenerationMissingExecutable,
    RuntimeGenerationMissingArtifact,
    RuntimeGenerationCreateLoraDirectory,
    RuntimeGenerationStart,
    RuntimeGenerationStop,
    RuntimeReadinessTimeout,
    RuntimeJobFailed,
    RuntimeMissingImage,
    RuntimeProfileMismatch,
    RuntimeCreateAssetDirectory,
    RuntimeImportAsset,
    RuntimeModelCreateDirectory,
    RuntimeModelInspect,
    RuntimeModelRequest,
    RuntimeModelDownloadStatus,
    RuntimeModelWrite,
    RuntimeModelSize,
    RuntimeModelChecksum,

    // ---- APP · app-level conditions without a typed error ----
    AppLlmUnavailable,
    AppProposalFailed,
    AppCommandInvalid,
    AppCheckpointRejected,
    AppStackEmpty,
}

impl ErrorCode {
    /// The three strings for a code: `(code, message, hint)`.
    pub fn parts(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::RefInvalid => (
                "SVS-REG-101",
                "Invalid c: ref format",
                "Refs look like c:<name>, a UUID, c:<uuid>, or c:<8-hex short id>. \
                 Check the reference and retry.",
            ),
            Self::RefNotFound => (
                "SVS-REG-102",
                "No artifact matches this ref",
                "The ref does not resolve to any artifact in this project. \
                 Check the spelling, or list the artifacts in the sidebar.",
            ),
            Self::RefAmbiguous => (
                "SVS-REG-103",
                "Ref matches several artifacts",
                "Several artifacts share this name. Refs must be exact \
                 (case-insensitive); use the short id shown under each name in the sidebar.",
            ),
            Self::RegistryNoArtifact => (
                "SVS-REG-110",
                "Artifact does not exist",
                "The operation references an artifact that is not in the project. \
                 It may have been removed; reload the project if the state looks stale.",
            ),
            Self::RegistryNotVariantable => (
                "SVS-REG-111",
                "This artifact kind cannot have variants",
                "Only characters, environments, and objects take variants. \
                 Use /create for a new artifact instead.",
            ),
            Self::RegistryVariantOfVariant => (
                "SVS-REG-112",
                "Variants cannot have their own variants",
                "A variant must derive from a base artifact. Create a new base if \
                 you need a different line of variation.",
            ),
            Self::RegistryVariantKindMismatch => (
                "SVS-REG-113",
                "Variant kind does not match its base",
                "A variant must have the same kind as its base artifact. \
                 Create the variant from the correct base.",
            ),
            Self::RegistryBadParent => (
                "SVS-REG-114",
                "Artifact kind cannot be a child of that parent",
                "Not every kind can be nested under every parent. \
                 Check the parent/child rules for the artifact kinds involved.",
            ),
            Self::RegistryNoParentAllowed => (
                "SVS-REG-115",
                "This artifact kind cannot have a parent",
                "The artifact kind does not support nesting under another artifact. \
                 Retry without the parent reference.",
            ),
            Self::RegistryNotABeat => (
                "SVS-REG-116",
                "Compositions require a beat",
                "Composition layers only exist on beats. The referenced artifact is \
                 not a beat.",
            ),
            Self::RegistryNotInScene => (
                "SVS-REG-117",
                "Beat is not inside a scene",
                "The beat must belong to a scene before it can be grouped. \
                 Add the beat to a scene first.",
            ),
            Self::RegistryNoRevision => (
                "SVS-REG-118",
                "Revision does not exist",
                "The revision id does not belong to the referenced artifact. \
                 Pick a revision from the artifact's history.",
            ),
            Self::RegistryInvalidSize => (
                "SVS-REG-119",
                "Invalid generation size",
                "Dimensions must be multiples of 32 within 256..=2048. \
                 Retry with a size such as 512x768.",
            ),
            Self::RegistryNoLoraEntry => (
                "SVS-REG-120",
                "LoRA entry does not exist",
                "The LoRA binding references an entry that is not in the project's \
                 LoRA registry.",
            ),
            Self::RegistryLoraNotFound => (
                "SVS-REG-121",
                "No LoRA matches this name",
                "The LoRA name does not match any registry entry (name or file). \
                 Check the LoRA registry in Settings.",
            ),
            Self::RegistryLoraAmbiguous => (
                "SVS-REG-122",
                "LoRA name matches several entries",
                "Several LoRA entries share this name. Use the full file name \
                 instead of the bare name.",
            ),
            Self::RegistryBadLoraMultiplier => (
                "SVS-REG-123",
                "LoRA multiplier out of range",
                "The multiplier must be within -2..=2. Retry with a value in that range.",
            ),
            Self::OpBackendRequired => (
                "SVS-OP-101",
                "This operation needs the generation backend",
                "The operation requires a generation runtime, but none was \
                 available. Start the backend in Settings and retry.",
            ),
            Self::OpEmptyDescription => (
                "SVS-OP-102",
                "Create needs a description",
                "A /create command must describe what to create. \
                 Add a non-empty description.",
            ),
            Self::OpInvalidSize => (
                "SVS-OP-103",
                "Invalid generation size",
                "Dimensions must be multiples of 32 within 256..=2048. \
                 Retry with a size such as 1024x576.",
            ),
            Self::OpNotVariantable => (
                "SVS-OP-104",
                "This artifact kind cannot have variants",
                "Only characters, environments, and objects take variants. \
                 Use /create for a new artifact instead.",
            ),
            Self::OpNotAScene => (
                "SVS-OP-105",
                "Compose requires a scene",
                "/compose targets a scene; the referenced artifact is not a scene.",
            ),
            Self::OpNotAnEnvironment => (
                "SVS-OP-106",
                "Compose background must be an environment",
                "The background layer of a composition must reference an \
                 environment artifact.",
            ),
            Self::OpBadLayerKind => (
                "SVS-OP-107",
                "Compose layers must be characters, objects, or environments",
                "A composition layer can only reference characters, objects, or \
                 environments.",
            ),
            Self::OpNoActiveImage => (
                "SVS-OP-108",
                "Artifact has no completed image",
                "The operation needs a completed generation to work from. \
                 Generate the artifact's first image (/create with a description, \
                 or /regenerate) first.",
            ),
            Self::OpNoPrompt => (
                "SVS-OP-109",
                "Regenerate has nothing to work from",
                "Regenerate needs a prompt, but the artifact has neither an active \
                 revision nor a description.",
            ),
            Self::OpIncompleteModify => (
                "SVS-OP-110",
                "Modify needs a mask region and an inpaint look",
                "/modify requires both a mask prompt and an inpaint prompt, or \
                 LM Studio to plan them. Start LM Studio, or write the full \
                 command: /modify c:<ref> <region> into <look>.",
            ),
            Self::OpConflictingMaskSource => (
                "SVS-OP-149",
                "Modify was given two mask sources",
                "A mask comes from one place: --mask-prompt segments it, \
                 --mask-file loads one already on disk. Drop whichever you \
                 did not mean.",
            ),
            Self::OpBadLoraMultiplier => (
                "SVS-OP-111",
                "LoRA multiplier out of range",
                "The multiplier must be within -2..=2. Retry with a value in that range.",
            ),
            Self::OpBadSteps => (
                "SVS-OP-112",
                "Step count out of range",
                "The step count must be within 1..=50. Retry with a valid value.",
            ),
            Self::OpLoraFileMissing => (
                "SVS-OP-113",
                "LoRA file not found",
                "The LoRA file does not exist in the configured lora directory. \
                 Check the file name and the lora_dir path in Settings.",
            ),
            Self::OpApplyFailed => (
                "SVS-OP-120",
                "Could not apply the operation result",
                "The operation ran, but recording its result in the registry \
                 failed. The project may be left partially updated.",
            ),
            Self::OpBackendFailed => (
                "SVS-OP-130",
                "Generation backend error",
                "A pipeline step failed in the generation backend (runtime or \
                 vision service). Check the service status in Settings and the \
                 logs for details.",
            ),
            Self::OpStepFailed => (
                "SVS-OP-131",
                "Pipeline step failed",
                "One of the pipeline steps failed. The full message names the \
                 step; check the logs for details.",
            ),
            Self::OpRejected => (
                "SVS-OP-132",
                "Operation rejected at a checkpoint",
                "A checkpoint was rejected, so the operation did not complete. \
                 Re-run it and accept the checkpoint, or cancel the operation.",
            ),
            Self::OpBuildEmpty => (
                "SVS-OP-140",
                "Empty pipeline",
                "A pipeline must contain at least one step; none were built.",
            ),
            Self::OpBuildOrdering => (
                "SVS-OP-141",
                "Step ordering violation",
                "A step references the output of a later step. This is an \
                 internal pipeline construction error.",
            ),
            Self::OpBuildMaskWithoutReference => (
                "SVS-OP-142",
                "Mask step needs a reference image",
                "A native mask step requires a reference image to attach to; \
                 none was provided.",
            ),
            Self::OpBuildBadModel => (
                "SVS-OP-143",
                "Unsupported generation model",
                "The requested model is not one of the native Krea profiles \
                 (krea-2-turbo-q2, krea-2-turbo-q4).",
            ),
            Self::OpBuildBadSize => (
                "SVS-OP-144",
                "Invalid generation size",
                "Dimensions must be multiples of 32 within 256..=2048. \
                 Retry with a valid size.",
            ),
            Self::OpBuildBadSteps => (
                "SVS-OP-145",
                "Step count out of range",
                "The step count must be within 1..=50. Retry with a valid value.",
            ),
            Self::OpBuildTooManyLoras => (
                "SVS-OP-146",
                "Too many LoRAs",
                "The backend accepts at most 8 LoRAs per generation. Remove some \
                 bindings and retry.",
            ),
            Self::OpBuildBadLoraMultiplier => (
                "SVS-OP-147",
                "LoRA multiplier out of range",
                "The multiplier must be within -2..=2. Retry with a value in that range.",
            ),
            Self::OpBuildBadFeatherRadius => (
                "SVS-OP-148",
                "Feather radius out of range",
                "The feather radius must be within 1..=64 pixels.",
            ),
            Self::ImgOpen => (
                "SVS-IMG-101",
                "Could not open image file",
                "The image file could not be read. It may be missing or \
                 corrupted; regenerate the artifact.",
            ),
            Self::ImgWrite => (
                "SVS-IMG-102",
                "Could not write image file",
                "The image file could not be written. Check the asset directory \
                 permissions and disk space.",
            ),
            Self::ImgDimensionMismatch => (
                "SVS-IMG-103",
                "Image dimension mismatch",
                "The images being composited have different dimensions. \
                 Regenerate the layers at the same size.",
            ),
            Self::ImgEmptyUnion => (
                "SVS-IMG-104",
                "Empty mask union",
                "Mask compositing needs at least one mask; none were provided.",
            ),
            Self::StoreOpen => (
                "SVS-STO-101",
                "Could not open project database",
                "The project database could not be opened. Check that the file \
                 exists and is not locked by another instance.",
            ),
            Self::StoreConfigure => (
                "SVS-STO-102",
                "Could not configure project database",
                "The project database could not be configured (WAL mode). \
                 Check file permissions.",
            ),
            Self::StoreMigrate => (
                "SVS-STO-103",
                "Could not migrate project database",
                "The project database could not be upgraded to the current \
                 schema. It may be from an incompatible tool version.",
            ),
            Self::StoreSave => (
                "SVS-STO-104",
                "Could not save project",
                "The project could not be written to disk. Check disk space and \
                 file permissions, then try again (Cmd/Ctrl+S).",
            ),
            Self::StoreSerialize => (
                "SVS-STO-105",
                "Could not serialize project data",
                "The project state could not be serialized for storage. \
                 This is an internal error; check the logs.",
            ),
            Self::StoreLoad => (
                "SVS-STO-106",
                "Could not load project",
                "The project could not be read from disk. It may be corrupted; \
                 check the logs for the underlying cause.",
            ),
            Self::StoreCorrupt => (
                "SVS-STO-107",
                "Corrupt project data",
                "A value stored in the project database could not be parsed. \
                 The file may be corrupted or written by a newer version.",
            ),
            Self::ConfigPlatformDirectories => (
                "SVS-CFG-101",
                "Could not find platform directories",
                "The application data directory could not be determined for this \
                 platform.",
            ),
            Self::ConfigRead => (
                "SVS-CFG-102",
                "Could not read config file",
                "config/app.toml could not be read. Check the file exists and \
                 is readable.",
            ),
            Self::ConfigParse => (
                "SVS-CFG-103",
                "Could not parse config file",
                "config/app.toml is not valid TOML or has unknown keys. Fix the \
                 file, or restore the default config.",
            ),
            Self::ConfigWrite => (
                "SVS-CFG-104",
                "Could not write config file",
                "config/app.toml could not be written. Check file permissions.",
            ),
            Self::ConfigGenerationProfile => (
                "SVS-CFG-105",
                "Unsupported generation profile",
                "The configured generation profile must be krea-2-turbo-q2 or \
                 krea-2-turbo-q4. Fix it in config/app.toml.",
            ),
            Self::ConfigInvalidSize => (
                "SVS-CFG-106",
                "Invalid size in config",
                "A generation size in config/app.toml is not a multiple of 32 \
                 within 256..=2048. Fix the entry and retry.",
            ),
            Self::ConfigMalformedSize => (
                "SVS-CFG-107",
                "Malformed size in config",
                "A generation size in config/app.toml is not in WxH form (e.g. \
                 512x768). Fix the entry and retry.",
            ),
            Self::LlmBuild => (
                "SVS-LLM-101",
                "Could not build HTTP client",
                "The LM Studio HTTP client could not be constructed. \
                 This is an internal error; check the logs.",
            ),
            Self::LlmRequest => (
                "SVS-LLM-102",
                "LM Studio request failed",
                "The request to LM Studio failed. Check that LM Studio is \
                 running and the Base URL in Settings is correct.",
            ),
            Self::LlmStatus => (
                "SVS-LLM-103",
                "LM Studio returned an error",
                "LM Studio answered with an error status. Check that the \
                 planner model is loaded and the request is valid.",
            ),
            Self::LlmPlanner => (
                "SVS-LLM-104",
                "LM Studio planning did not finish",
                "LM Studio did not produce a usable plan. Retry, or use slash \
                 commands directly.",
            ),
            Self::VisionRequest => (
                "SVS-VIS-101",
                "Vision service request failed",
                "The vision service (Python runtime) failed to answer. Start it \
                 from Settings and retry.",
            ),
            Self::VisionStatus => (
                "SVS-VIS-102",
                "Vision service returned an error",
                "The vision service answered with an error status. Check its \
                 logs for the underlying cause.",
            ),
            Self::RuntimeAlreadyRunning => (
                "SVS-RTE-101",
                "Python runtime already running",
                "The vision service is already running; stop it before starting \
                 again.",
            ),
            Self::RuntimeMissingPython => (
                "SVS-RTE-102",
                "Python virtual environment not found",
                "The Python venv for the vision service does not exist. Create it \
                 with `uv sync --project python`.",
            ),
            Self::RuntimeResolve => (
                "SVS-RTE-103",
                "Could not resolve runtime directory",
                "The Python runtime directory could not be resolved. Check the \
                 configured paths.",
            ),
            Self::RuntimeStart => (
                "SVS-RTE-104",
                "Could not start Python runtime",
                "The vision service failed to start. Check the runtime logs for \
                 the underlying cause.",
            ),
            Self::RuntimeStop => (
                "SVS-RTE-105",
                "Could not stop Python runtime",
                "The vision service could not be stopped cleanly. It may still \
                 be running in the background.",
            ),
            Self::RuntimeGenerationAlreadyRunning => (
                "SVS-RTE-110",
                "Generation runtime already running",
                "The native generation runtime is already running; stop it \
                 before starting again.",
            ),
            Self::RuntimeGenerationMissingExecutable => (
                "SVS-RTE-111",
                "sd-server executable not found",
                "The stable-diffusion.cpp server binary could not be found at \
                 the configured path. Check Settings → Storage.",
            ),
            Self::RuntimeGenerationMissingArtifact => (
                "SVS-RTE-112",
                "Model artifact not found",
                "A model file for the selected Krea profile is missing. Run \
                 model_setup to provision the models.",
            ),
            Self::RuntimeGenerationCreateLoraDirectory => (
                "SVS-RTE-113",
                "Could not create LoRA directory",
                "The LoRA directory could not be created. Check permissions and \
                 the configured lora_dir.",
            ),
            Self::RuntimeGenerationStart => (
                "SVS-RTE-114",
                "Could not start generation runtime",
                "The native generation runtime failed to start. Check the \
                 runtime logs for the underlying cause.",
            ),
            Self::RuntimeGenerationStop => (
                "SVS-RTE-115",
                "Could not stop generation runtime",
                "The native generation runtime could not be stopped cleanly.",
            ),
            Self::RuntimeReadinessTimeout => (
                "SVS-RTE-120",
                "Vision service did not become ready",
                "The vision service did not report readiness in time. Restart \
                 it from Settings and retry.",
            ),
            Self::RuntimeJobFailed => (
                "SVS-RTE-121",
                "Generation job failed",
                "The generation job failed in the backend. Check the runtime \
                 logs for the underlying cause.",
            ),
            Self::RuntimeMissingImage => (
                "SVS-RTE-122",
                "Generation returned no image",
                "The generation job finished without producing an image. \
                 Regenerate to retry.",
            ),
            Self::RuntimeProfileMismatch => (
                "SVS-RTE-123",
                "Generation server has a different model loaded",
                "The generation server on :7861 has a different model profile \
                 loaded than requested. Stop stale instances first \
                 (`svs runtime serve --force` or pkill sd-server).",
            ),
            Self::RuntimeCreateAssetDirectory => (
                "SVS-RTE-124",
                "Could not create asset directory",
                "The generated-asset directory could not be created. Check \
                 permissions and disk space.",
            ),
            Self::RuntimeImportAsset => (
                "SVS-RTE-125",
                "Could not import generated image",
                "The generated image could not be copied into the asset \
                 directory. Check disk space and permissions.",
            ),
            Self::RuntimeModelCreateDirectory => (
                "SVS-RTE-130",
                "Could not create model directory",
                "The model directory could not be created. Check permissions \
                 and the configured model_dir.",
            ),
            Self::RuntimeModelInspect => (
                "SVS-RTE-131",
                "Could not inspect model artifact",
                "A local model artifact could not be inspected. The file may be \
                 missing or unreadable.",
            ),
            Self::RuntimeModelRequest => (
                "SVS-RTE-132",
                "Model download request failed",
                "The model download request failed. Check the network connection \
                 and retry.",
            ),
            Self::RuntimeModelDownloadStatus => (
                "SVS-RTE-133",
                "Model download returned an error",
                "The model download server answered with an error status. \
                 Retry, or check the model URL.",
            ),
            Self::RuntimeModelWrite => (
                "SVS-RTE-134",
                "Could not write model artifact",
                "The downloaded model could not be written to disk. Check disk \
                 space and permissions.",
            ),
            Self::RuntimeModelSize => (
                "SVS-RTE-135",
                "Model file has the wrong size",
                "The downloaded model file does not match the expected size. \
                 The download was truncated or corrupted; redownload.",
            ),
            Self::RuntimeModelChecksum => (
                "SVS-RTE-136",
                "Model failed verification",
                "The model file failed SHA-256 verification. It is corrupted; \
                 redownload it.",
            ),
            Self::AppLlmUnavailable => (
                "SVS-APP-101",
                "LM Studio unavailable",
                "LM Studio is not configured or reachable. Start it (and load a \
                 planner model), or use slash commands instead.",
            ),
            Self::AppProposalFailed => (
                "SVS-APP-102",
                "Could not plan operations",
                "The LLM could not plan your request as operations. Retry with \
                 a different phrasing, or use slash commands directly.",
            ),
            Self::AppCommandInvalid => (
                "SVS-APP-103",
                "Command not understood",
                "The input is not a valid slash command or free-form request. \
                 Commands start with / — e.g. /create character <description>.",
            ),
            Self::AppCheckpointRejected => (
                "SVS-APP-104",
                "Operation rejected at a checkpoint",
                "A checkpoint was rejected, so the operation did not complete. \
                 Re-run it and accept the checkpoint, or cancel the operation.",
            ),
            Self::AppStackEmpty => (
                "SVS-APP-105",
                "Nothing to run",
                "The operation stack contained no operations. This is an \
                 internal error; check the logs.",
            ),
        }
    }

    /// The stable code string (`SVS-<DOMAIN>-<NNN>`).
    pub fn code(self) -> &'static str {
        self.parts().0
    }

    /// The short banner label.
    pub fn message(self) -> &'static str {
        self.parts().1
    }

    /// The full explanation shown in the details modal.
    pub fn hint(self) -> &'static str {
        self.parts().2
    }
}

/// An error that maps to a stable [`ErrorCode`].
pub trait CodedError {
    /// The stable error code for this error.
    fn code(&self) -> ErrorCode;

    /// Builds the UI-ready report. This is the payload the surface shows;
    /// the surface emits the log line by calling [`ReportedError::log`]
    /// (after attaching any context it knows — operation, prompt, seed, …),
    /// so every surfaced error is logged exactly once, with its code.
    fn report(&self) -> ReportedError
    where
        Self: fmt::Display,
    {
        ReportedError {
            code: self.code(),
            details: self.to_string(),
            context: Vec::new(),
        }
    }
}

/// The UI-ready error payload: the stable code, the rendered details, and
/// the structured context (operation, generation prompt, model, seed, job
/// id, …) that makes a failure debuggable. Context travels inside the
/// error chain (e.g. [`crate::registry::pipeline::PipelineError`] carries
/// it from the failure site) and lands in the log line, the details modal,
/// and the copy payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportedError {
    pub code: ErrorCode,
    pub details: String,
    /// Ordered `key=value` context; the first pair is the operation when
    /// one is known.
    pub context: Vec<(&'static str, String)>,
}

impl ReportedError {
    /// Builds a report for a string-only error that maps to an app-level
    /// code (no typed error exists for the condition).
    pub fn plain(code: ErrorCode, details: impl Into<String>) -> Self {
        Self {
            code,
            details: details.into(),
            context: Vec::new(),
        }
    }

    /// Attaches a context pair (builder-style). Order is preserved: the
    /// operation context goes first, request details follow.
    pub fn with_context(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.context.push((key, value.into()));
        self
    }

    /// Emits the single coded log line for this report (`code`, `error`,
    /// and the full `context`) and returns the report, so surfaces can
    /// chain: `error_report.set(Some(report.log()))` or `anyhow!(report.log())`.
    pub fn log(self) -> Self {
        if self.context.is_empty() {
            tracing::error!(code = %self.code.code(), error = %self.details, "operation failed");
        } else {
            tracing::error!(
                code = %self.code.code(),
                error = %self.details,
                context = %self.context_line(),
                "operation failed"
            );
        }
        self
    }

    /// `key=value, key2=value2` — the compact context form for log lines
    /// and Display.
    fn context_line(&self) -> String {
        self.context
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The full copy payload (banner code, message, details, context,
    /// hint).
    pub fn copy_text(&self) -> String {
        let mut text = format!(
            "[{}] {}\n\n{}\n\nWhat to do: {}",
            self.code.code(),
            self.code.message(),
            self.details,
            self.code.hint(),
        );
        if !self.context.is_empty() {
            text.push_str("\n\nContext:\n");
            for (key, value) in &self.context {
                text.push_str(&format!("- {key}: {value}\n"));
            }
        }
        text
    }
}

impl fmt::Display for ReportedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.code.code(), self.details)?;
        if !self.context.is_empty() {
            write!(f, " ({})", self.context_line())?;
        }
        Ok(())
    }
}

impl std::error::Error for ReportedError {}

#[cfg(test)]
mod tests {
    use super::{CodedError, ErrorCode};

    /// Every code has a distinct, well-formed identifier and non-empty
    /// message/hint — the registry's invariants.
    #[test]
    fn every_code_is_well_formed_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for code in codes() {
            let (identifier, message, hint) = code.parts();
            assert!(
                identifier.starts_with("SVS-"),
                "{code:?}: code {identifier} must start with SVS-"
            );
            assert!(
                seen.insert(identifier),
                "{code:?}: duplicate code {identifier}"
            );
            assert!(!message.is_empty(), "{code:?}: message is empty");
            assert!(!hint.is_empty(), "{code:?}: hint is empty");
        }
    }

    /// Codes are stable identifiers, so the variant list may only grow.
    /// This test pins the count so renumbering or dropping codes (which
    /// would break reports already shared with users) is caught.
    #[test]
    fn registry_size_is_pinned() {
        assert_eq!(codes().len(), 97, "error codes must never be renumbered");
    }

    #[test]
    fn report_builds_the_payload() {
        let error = crate::registry::RefError::NotFound("c:mia".to_owned());
        let report = error.report();
        assert_eq!(report.code, ErrorCode::RefNotFound);
        assert!(report.details.contains("c:mia"));
        assert_eq!(
            report.to_string(),
            "SVS-REG-102 ref `c:mia` does not match any artifact"
        );
        assert!(report.copy_text().contains("SVS-REG-102"));
        assert!(report.copy_text().contains("What to do"));
    }

    /// Context rides on the report: attached pairs appear in Display, in
    /// the copy payload, and (via `log`) in the log line.
    #[test]
    fn context_rides_on_the_report() {
        let report = super::ReportedError::plain(ErrorCode::OpBackendFailed, "backend error")
            .with_context("operation", "/create character Mia")
            .with_context("prompt", "a lantern on a shelf")
            .with_context("model", "krea-2-turbo-q4");
        assert_eq!(report.context.len(), 3);
        assert_eq!(
            report.to_string(),
            "SVS-OP-130 backend error (operation=/create character Mia, \
             prompt=a lantern on a shelf, model=krea-2-turbo-q4)"
        );
        let copy = report.copy_text();
        assert!(copy.contains("- operation: /create character Mia"));
        assert!(copy.contains("- prompt: a lantern on a shelf"));
    }

    /// The app-level plain constructor follows the same shape.
    #[test]
    fn plain_reports_carry_app_codes() {
        let report = super::ReportedError::plain(
            ErrorCode::AppStackEmpty,
            "the stack contained no operations",
        );
        assert_eq!(report.code, ErrorCode::AppStackEmpty);
        assert!(report.to_string().starts_with("SVS-APP-105"));
    }

    /// The `report` overrides on the wrapped op/pipeline errors must not
    /// recurse for non-context variants (regression: a trait-qualified
    /// `CodedError::report(self)` call inside an override resolves to the
    /// override itself and overflows the stack).
    #[test]
    fn wrapped_reports_do_not_recurse() {
        let compile = crate::registry::ops::CompileError::EmptyDescription;
        assert_eq!(compile.report().code, ErrorCode::OpEmptyDescription);

        let op = crate::registry::ops::OpError::Compile(compile);
        assert_eq!(op.report().code, ErrorCode::OpEmptyDescription);

        let pipeline = crate::registry::pipeline::PipelineError::Rejected;
        assert_eq!(pipeline.report().code, ErrorCode::OpRejected);
    }

    fn codes() -> Vec<ErrorCode> {
        // The variant list: written out once here so the pin test and the
        // registry test share a single source of truth.
        vec![
            ErrorCode::RefInvalid,
            ErrorCode::RefNotFound,
            ErrorCode::RefAmbiguous,
            ErrorCode::RegistryNoArtifact,
            ErrorCode::RegistryNotVariantable,
            ErrorCode::RegistryVariantOfVariant,
            ErrorCode::RegistryVariantKindMismatch,
            ErrorCode::RegistryBadParent,
            ErrorCode::RegistryNoParentAllowed,
            ErrorCode::RegistryNotABeat,
            ErrorCode::RegistryNotInScene,
            ErrorCode::RegistryNoRevision,
            ErrorCode::RegistryInvalidSize,
            ErrorCode::RegistryNoLoraEntry,
            ErrorCode::RegistryLoraNotFound,
            ErrorCode::RegistryLoraAmbiguous,
            ErrorCode::RegistryBadLoraMultiplier,
            ErrorCode::OpBackendRequired,
            ErrorCode::OpEmptyDescription,
            ErrorCode::OpInvalidSize,
            ErrorCode::OpNotVariantable,
            ErrorCode::OpNotAScene,
            ErrorCode::OpNotAnEnvironment,
            ErrorCode::OpBadLayerKind,
            ErrorCode::OpNoActiveImage,
            ErrorCode::OpNoPrompt,
            ErrorCode::OpIncompleteModify,
            ErrorCode::OpConflictingMaskSource,
            ErrorCode::OpBadLoraMultiplier,
            ErrorCode::OpBadSteps,
            ErrorCode::OpLoraFileMissing,
            ErrorCode::OpApplyFailed,
            ErrorCode::OpBackendFailed,
            ErrorCode::OpStepFailed,
            ErrorCode::OpRejected,
            ErrorCode::OpBuildEmpty,
            ErrorCode::OpBuildOrdering,
            ErrorCode::OpBuildMaskWithoutReference,
            ErrorCode::OpBuildBadModel,
            ErrorCode::OpBuildBadSize,
            ErrorCode::OpBuildBadSteps,
            ErrorCode::OpBuildTooManyLoras,
            ErrorCode::OpBuildBadLoraMultiplier,
            ErrorCode::OpBuildBadFeatherRadius,
            ErrorCode::ImgOpen,
            ErrorCode::ImgWrite,
            ErrorCode::ImgDimensionMismatch,
            ErrorCode::ImgEmptyUnion,
            ErrorCode::StoreOpen,
            ErrorCode::StoreConfigure,
            ErrorCode::StoreMigrate,
            ErrorCode::StoreSave,
            ErrorCode::StoreSerialize,
            ErrorCode::StoreLoad,
            ErrorCode::StoreCorrupt,
            ErrorCode::ConfigPlatformDirectories,
            ErrorCode::ConfigRead,
            ErrorCode::ConfigParse,
            ErrorCode::ConfigWrite,
            ErrorCode::ConfigGenerationProfile,
            ErrorCode::ConfigInvalidSize,
            ErrorCode::ConfigMalformedSize,
            ErrorCode::LlmBuild,
            ErrorCode::LlmRequest,
            ErrorCode::LlmStatus,
            ErrorCode::LlmPlanner,
            ErrorCode::VisionRequest,
            ErrorCode::VisionStatus,
            ErrorCode::RuntimeAlreadyRunning,
            ErrorCode::RuntimeMissingPython,
            ErrorCode::RuntimeResolve,
            ErrorCode::RuntimeStart,
            ErrorCode::RuntimeStop,
            ErrorCode::RuntimeGenerationAlreadyRunning,
            ErrorCode::RuntimeGenerationMissingExecutable,
            ErrorCode::RuntimeGenerationMissingArtifact,
            ErrorCode::RuntimeGenerationCreateLoraDirectory,
            ErrorCode::RuntimeGenerationStart,
            ErrorCode::RuntimeGenerationStop,
            ErrorCode::RuntimeReadinessTimeout,
            ErrorCode::RuntimeJobFailed,
            ErrorCode::RuntimeMissingImage,
            ErrorCode::RuntimeProfileMismatch,
            ErrorCode::RuntimeCreateAssetDirectory,
            ErrorCode::RuntimeImportAsset,
            ErrorCode::RuntimeModelCreateDirectory,
            ErrorCode::RuntimeModelInspect,
            ErrorCode::RuntimeModelRequest,
            ErrorCode::RuntimeModelDownloadStatus,
            ErrorCode::RuntimeModelWrite,
            ErrorCode::RuntimeModelSize,
            ErrorCode::RuntimeModelChecksum,
            ErrorCode::AppLlmUnavailable,
            ErrorCode::AppProposalFailed,
            ErrorCode::AppCommandInvalid,
            ErrorCode::AppCheckpointRejected,
            ErrorCode::AppStackEmpty,
        ]
    }
}
