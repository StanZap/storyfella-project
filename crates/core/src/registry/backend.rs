//! The live [`GenerationBackend`]: `CreativeRuntime` for generation,
//! `LmStudioClient` for the soft LLM steps.

use clipforge_inference::sd::GenerateRequest;
use futures_util::future::BoxFuture;

use crate::{
    config::AppConfig,
    llm::{strip_json_fence, ChatMessage, ChatOptions, LmStudioClient},
    runtime::{CreativeRuntime, CreativeRuntimeError},
};

use super::{
    ops::OperationStack,
    pipeline::{GenerateResponse, GenerationBackend, PipelineError},
    Artifact, ArtifactRegistry, GenerationDefaults,
};
use crate::registry::discover_loras;

/// The request context for a failed generation: the exact prompt that was
/// sent (the LLM-derived one or the manual one), the model, the
/// parameters, and the job id for cross-referencing the runtime logs.
/// This is what makes "why did this prompt not become a generation?"
/// answerable from a single log line.
fn generate_context(
    request: &GenerateRequest,
    job_id: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut context = Vec::new();
    context.push(("prompt", request.prompt.clone()));
    context.push(("width", request.width.to_string()));
    context.push(("height", request.height.to_string()));
    context.push(("steps", request.steps.to_string()));
    if let Some(seed) = request.seed {
        context.push(("seed", seed.to_string()));
    }
    if let Some(path) = &request.reference_image_path {
        context.push(("reference", path.clone()));
    }
    if !request.loras.is_empty() {
        context.push((
            "loras",
            request
                .loras
                .iter()
                .map(|lora| format!("{}@{}", lora.path, lora.multiplier))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    if let Some(job) = job_id {
        context.push(("job", job.to_owned()));
    }
    context
}

/// The real backend the CLI (and the canvas) executes pipelines against.
#[derive(Clone)]
pub struct CreativeBackend {
    runtime: CreativeRuntime,
    llm: Option<LmStudioClient>,
}

impl PartialEq for CreativeBackend {
    /// The runtime is the identity — the LLM client is a soft dependency
    /// that does not affect reactivity.
    fn eq(&self, other: &Self) -> bool {
        self.runtime == other.runtime
    }
}

impl CreativeBackend {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            runtime: CreativeRuntime::new(config),
            llm: LmStudioClient::new(config.lm_studio.clone()).ok(),
        }
    }

    /// The [`CreativeRuntime`] this backend drives (the Settings screen
    /// controls the same processes the canvas generates through).
    pub fn runtime(&self) -> CreativeRuntime {
        self.runtime.clone()
    }

    /// Ensures the native generation server is resident and ready.
    pub async fn ensure_ready(&self) -> Result<(), CreativeRuntimeError> {
        self.runtime.ensure_ready().await
    }

    /// Stops the generation server this backend owns.
    pub async fn stop_generation(&self) -> Result<(), CreativeRuntimeError> {
        self.runtime.stop_generation_runtime().await
    }

    /// Whether an LLM is configured (soft dependency).
    pub fn has_llm(&self) -> bool {
        self.llm.is_some()
    }

    /// The model identifier the planner currently requests (the Settings
    /// picker shows this; `None` when no LLM client was built).
    pub fn llm_model(&self) -> Option<String> {
        self.llm
            .as_ref()
            .map(LmStudioClient::model)
            .map(str::to_owned)
    }

    /// Swaps the planner model at runtime. The backend lives in app state
    /// (a signal), so the canvas's next proposal/plan/draft uses the new
    /// model immediately; `config/app.toml` persists the choice across runs.
    pub fn set_llm_model(&mut self, model: String) {
        if let Some(client) = &mut self.llm {
            client.set_model(model);
        }
    }

    fn draft_system_prompt() -> &'static str {
        "You are the story writer for a visual storybook project. Write the \
         requested story text (premise, plot, scene, or beat narration) as \
         plain, evocative prose. Respond with only the text — no commentary, \
         no markdown."
    }

    /// Qwen-Image-2.1's instruction edit takes an image plus a plain-language
    /// instruction and preserves what the instruction does not mention. A
    /// terse instruction under-specifies the result and the render collapses
    /// into base-model priors — measured, `"make him smile"` returned an anime
    /// illustration of a different person. Restating the invariants pulls it
    /// back, but a template can only restate them *all*, including the ones
    /// the edit is meant to override. Doing that subtraction is the LLM's job
    /// here, and the only one.
    fn ground_system_prompt() -> &'static str {
        "You rewrite image-edit instructions for a photographic image editor. \
         You are given the edit the user asked for and a description of the \
         image being edited. Rewrite it as one instruction with all three of \
         these parts, in this order:\n\
         1. the edit itself;\n\
         2. the exact words \"A photograph of the same person\" (or \"the same \
         subject\" if it is not a person) — never omit this, it is what stops \
         the editor inventing someone new;\n\
         3. the traits from the description that must stay the same.\n\n\
         Two rules govern part 3, and breaking either one visibly corrupts \
         the image:\n\
         - Use ONLY traits written in the description you are given. Never \
         introduce an attribute it does not state — no invented colour, \
         garment, or setting. A trait you invent contradicts the edit and the \
         editor renders both at once.\n\
         - Drop the specific attribute the edit changes, but keep the rest of \
         the phrase it sits in. A cut changes length, not colour: drop the \
         length words and keep the colour word the description gave. \
         Restating the changed attribute makes the editor split the \
         difference; dropping the unchanged ones lets it wander.\n\n\
         Keep it under 60 words. Respond with the instruction only — no \
         commentary, no markdown, no quotes."
    }

    /// The shared op vocabulary + intent rules block of the planner prompt
    /// (used by both the tool-calling and the static-fallback prompts).
    fn vocabulary_block() -> String {
        "You plan operations for a visual story tool. Respond with JSON only, exactly \
         this shape: {{\"operations\": [ ... ]}} where every operation is one of:\n\
         {{\"op\": \"create\", \"kind\": \"story|scene|beat|character|environment|object\", \"description\": \"...\", \"name\": \"...\", \"size\": [w, h], \"steps\": n, \"loras\": [{{\"path\": \"...\", \"multiplier\": 0.8}}]}}\n\
         {{\"op\": \"variant\", \"target\": \"c:<name>\", \"description\": \"...\", \"axis\": \"outfit|age|body|hair|expression|time-of-day|weather|season|mood\", \"size\": [w, h], \"steps\": n, \"loras\": [...]}}\n\
         {{\"op\": \"regenerate\", \"target\": \"c:<name>\", \"prompt\": \"...\", \"size\": [w, h], \"steps\": n, \"loras\": [...]}}\n\
         {{\"op\": \"resize\", \"target\": \"c:<name>\", \"width\": w, \"height\": h}}\n\
         {{\"op\": \"compose\", \"scene\": \"c:<name>\", \"description\": \"...\", \"background\": \"c:<name>\", \"layers\": [{{\"artifact\": \"c:<name>\", \"variant\": \"c:<name>\"}}]}}\n\
         {{\"op\": \"draft\", \"target\": \"c:<name>\", \"request\": \"...\"}}\n\
         {{\"op\": \"modify\", \"target\": \"c:<name>\", \"description\": \"...\", \"size\": [w, h], \"steps\": n, \"loras\": [...]}}\n\
         {{\"op\": \"attach-lora\", \"target\": \"c:<name>\", \"lora\": \"<adapter file>\", \"multiplier\": 0.8}}\n\
         {{\"op\": \"lora-scale\", \"lora\": \"<adapter name>\", \"multiplier\": 0.9}}\n\
         {{\"op\": \"lora-remove\", \"lora\": \"<adapter name>\"}}\n\
         Choose the op by intent, not by wording:\n\
         - create: a new artifact that does not exist yet.\n\
         - variant: a whole new version of an existing visual artifact — a \
         different look, age, outfit, or mood of the same subject. Never use \
         it for a small localized change.\n\
         - regenerate: redo an existing artifact's image in full, keeping the \
         same subject — quality, style, or composition fixes of the whole image.\n\
         - resize: change the image canvas — set the artifact's default size \
         and regenerate at it. Only when the user asks for a size.\n\
         - modify: a localized change to one region of an existing image (add \
         glasses, change the hair, change the sky). A change to what an image \
         shows is modify, not regenerate. Give it a plain-language \
         instruction: the edit model preserves whatever the instruction does \
         not mention.\n\
         - compose: assemble a scene from existing artifacts (background plus \
         layered characters).\n\
         - draft: ONLY a text note or idea attached to an artifact. It never \
         changes or generates an image — never use draft for any request that \
         changes what an image shows.\n\
         - attach-lora: bind an adapter to an artifact (character consistency, \
         style, or a specialist edit adapter) when the request implies one. Use \
         only real adapter file names.\n\
         - lora-scale/lora-remove: adjust or unbind an adapter the user names.\n\
         Size and steps rules: sizes are [width, height] multiples of 32 within \
         256..=2048; steps within 1..=50. Include \"size\", \"steps\", or \
         \"loras\" in an op ONLY when the user explicitly asks for a specific \
         size, more steps, or a named adapter — never invent them.\n\
         Never invent adapter names.\n\
         Examples: \"add glasses to mia\" → modify; \"make mia 10 years \
         older\" → variant with axis age; \"the sky looks flat, redo it\" → \
         regenerate; \"make the image 1024x1024\" → resize; \"use 8 steps\" → \
         the op with \"steps\": 8; \"note that mia's ring was her \
         grandmother's\" → draft.\n\
         This vocabulary is closed — never invent an op name. References are the \
         c:<name> keys (case-insensitive); use only those."
            .to_owned()
    }

    /// The viewing note: requests that do not name another artifact apply
    /// to the artifact currently shown in the canvas.
    fn viewing_note(registry: &ArtifactRegistry, viewing: Option<&Artifact>) -> String {
        match viewing {
            Some(artifact) => format!(
                "\nCurrently viewed artifact: {} ({} — {}).\nA request that does not name \
                 another artifact explicitly applies to the viewed artifact: prefer it as \
                 the target over its base or other artifacts.\n",
                registry.ref_of(artifact.id),
                artifact.kind,
                if artifact.name.trim().is_empty() {
                    "unnamed".to_owned()
                } else {
                    artifact.name.clone()
                },
            ),
            None => String::new(),
        }
    }

    /// The registry listing fed to the planner (a tool result in tool mode,
    /// inline in the static fallback).
    fn artifact_listing(registry: &ArtifactRegistry) -> String {
        let mut listing = String::new();
        for artifact in registry.iter() {
            listing.push_str(&format!(
                "  {} {} {:?}",
                registry.ref_of(artifact.id),
                artifact.kind,
                artifact.name
            ));
            if !artifact.description.is_empty() {
                listing.push_str(&format!(" — {}", artifact.description));
            }
            listing.push('\n');
        }
        listing
    }

    /// The adapter listing (discovered files + config bindings).
    fn adapter_listing(defaults: &GenerationDefaults) -> String {
        let mut adapters: Vec<String> = discover_loras(&defaults.loras.dir);
        for binding in defaults
            .loras
            .kinds
            .character
            .iter()
            .chain(defaults.loras.kinds.environment.iter())
            .chain(defaults.loras.kinds.object.iter())
        {
            if !adapters.iter().any(|file| file == &binding.file) {
                adapters.push(binding.file.clone());
            }
        }
        adapters.sort();
        if adapters.is_empty() {
            "none".to_owned()
        } else {
            adapters.join(", ")
        }
    }

    /// The closed-vocabulary planner prompt for the static fallback (no
    /// tool calling): registry and adapter listings are inlined.
    /// `viewing` names the artifact currently shown in the canvas — requests
    /// that do not name another artifact apply to it, so a change typed
    /// while viewing a variant targets the variant, not its base.
    fn propose_system_prompt(
        registry: &ArtifactRegistry,
        defaults: &GenerationDefaults,
        viewing: Option<&Artifact>,
    ) -> String {
        format!(
            "{}\n\nRegistry:\n{listing}{viewing_note}\nAdapters available: {adapter_listing}",
            Self::vocabulary_block(),
            listing = Self::artifact_listing(registry),
            viewing_note = Self::viewing_note(registry, viewing),
            adapter_listing = Self::adapter_listing(defaults),
        )
    }

    /// The tool-calling planner prompt: the model discovers the registry
    /// and the adapters by calling tools, then answers with the JSON stack.
    fn propose_tools_prompt(registry: &ArtifactRegistry, viewing: Option<&Artifact>) -> String {
        let viewing = match viewing {
            Some(artifact) => format!(
                "\nCurrently viewed artifact: {} ({} — {}). A request that does not name \
                 another artifact explicitly applies to the viewed artifact.\n",
                registry.ref_of(artifact.id),
                artifact.kind,
                artifact.name,
            ),
            None => String::new(),
        };
        format!(
            "{}\n\nCall list_artifacts and list_loras as many times as you need to discover \
             the available artifacts and adapters, then respond with ONLY the JSON stack \
             described above (no commentary, no markdown).{viewing}",
            Self::vocabulary_block(),
        )
    }

    /// The discovery tools the planner may call.
    fn planner_tools() -> Vec<crate::llm::ToolDefinition> {
        use crate::llm::{ToolDefinition, ToolFunction};
        vec![
            ToolDefinition {
                r#type: "function",
                function: ToolFunction {
                    name: "list_artifacts".to_owned(),
                    description: "List every artifact in the project with its c:<ref> key, \
                                  kind, name, and description."
                        .to_owned(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }),
                },
            },
            ToolDefinition {
                r#type: "function",
                function: ToolFunction {
                    name: "list_loras".to_owned(),
                    description: "List the available LoRA adapter files (relative to the lora \
                                  directory) plus the configured use-case bindings."
                        .to_owned(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }),
                },
            },
        ]
    }
}

impl CreativeBackend {
    /// Drives the LLM to plan a free-form request as an operation stack
    /// against the closed vocabulary (the VLLM contract test bed, shared by
    /// the `svs stack propose` CLI and the canvas composer). `Err` when the
    /// LLM is unavailable or its output is not a valid stack. `defaults`
    /// lists the adapters the planner may propose; `viewing` names the
    /// artifact currently shown in the canvas so ref-less requests target
    /// it (the CLI passes `None`).
    pub async fn propose_stack(
        &self,
        registry: &ArtifactRegistry,
        defaults: &GenerationDefaults,
        viewing: Option<&Artifact>,
        message: &str,
    ) -> Result<OperationStack, String> {
        let Some(client) = &self.llm else {
            return Err(
                "LM Studio is not configured or unreachable — use slash commands instead"
                    .to_owned(),
            );
        };
        // Preferred: the model discovers artifacts/adapters by calling
        // tools (any number of rounds), then emits the stack.
        match Self::propose_with_tools(client, registry, defaults, viewing, message).await {
            Ok(stack) => return Ok(stack),
            Err(ProposeError::ToolsUnsupported(error)) => {
                tracing::warn!(%error, "tool calling unavailable; falling back to the static planner");
            }
            Err(ProposeError::Failed(error)) => return Err(error),
        }
        // Fallback: the static listing + strict JSON schema.
        Self::propose_static(client, registry, defaults, viewing, message).await
    }

    /// The tool-calling planner: multi-round discovery, then the final JSON
    /// stack. `ToolsUnsupported` when the server rejects `tools` outright
    /// (older LM Studio builds), so the caller can fall back.
    async fn propose_with_tools(
        client: &LmStudioClient,
        registry: &ArtifactRegistry,
        defaults: &GenerationDefaults,
        viewing: Option<&Artifact>,
        message: &str,
    ) -> Result<OperationStack, ProposeError> {
        let messages = [
            ChatMessage::text("system", Self::propose_tools_prompt(registry, viewing)),
            ChatMessage::text("user", message),
        ];
        let completion = client
            .chat_with_tools(
                &messages,
                Self::planner_tools(),
                // The strict schema rides every round: the model may call
                // discovery tools, but its final answer stays inside the
                // closed vocabulary (validated live against LM Studio).
                Some(Self::stack_response_format()),
                |name, _arguments| match name {
                    "list_artifacts" => Self::artifact_listing(registry),
                    "list_loras" => Self::adapter_listing(defaults),
                    other => format!("unknown tool {other}"),
                },
                6,
            )
            .await
            .map_err(|error| match error {
                crate::llm::LmStudioError::Status { .. } => {
                    ProposeError::ToolsUnsupported(error.to_string())
                }
                other => ProposeError::Failed(format!("LM Studio request failed: {other}")),
            })?;
        let content = completion
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone())
            .ok_or_else(|| ProposeError::Failed("LM Studio returned no content".to_owned()))?;
        Self::parse_stack(&content)
    }

    /// The static planner: one strict-schema call with the listings inline.
    async fn propose_static(
        client: &LmStudioClient,
        registry: &ArtifactRegistry,
        defaults: &GenerationDefaults,
        viewing: Option<&Artifact>,
        message: &str,
    ) -> Result<OperationStack, String> {
        let options = ChatOptions {
            response_format: Some(Self::stack_response_format()),
            ..ChatOptions::default()
        };
        let completion = client
            .chat_with_options(
                &[
                    ChatMessage::text(
                        "system",
                        Self::propose_system_prompt(registry, defaults, viewing),
                    ),
                    ChatMessage::text("user", message),
                ],
                &options,
            )
            .await
            .map_err(|error| format!("LM Studio request failed: {error}"))?;
        let content = completion
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone())
            .ok_or_else(|| "LM Studio returned no content".to_owned())?;
        Self::parse_stack(&content).map_err(|error| match error {
            ProposeError::Failed(message) => message,
            ProposeError::ToolsUnsupported(_) => unreachable!("the static path has no tools"),
        })
    }

    fn parse_stack(content: &str) -> Result<OperationStack, ProposeError> {
        let stack: OperationStack =
            serde_json::from_str(strip_json_fence(content)).map_err(|error| {
                ProposeError::Failed(format!(
                    "the LLM output was not a valid operation stack: {error}\n\n{content}"
                ))
            })?;
        if stack.operations.is_empty() {
            return Err(ProposeError::Failed(
                "the LLM returned no operations".to_owned(),
            ));
        }
        Ok(stack)
    }
}

/// The tool-calling planner's failure modes: the server rejected `tools`
/// (fall back to the static path) or planning genuinely failed.
enum ProposeError {
    ToolsUnsupported(String),
    Failed(String),
}

impl GenerationBackend for CreativeBackend {
    fn generate(
        &self,
        request: &GenerateRequest,
    ) -> BoxFuture<'_, Result<GenerateResponse, PipelineError>> {
        let this = self.clone();
        let request = request.clone();
        Box::pin(async move {
            this.runtime.ensure_ready().await.map_err(|error| {
                PipelineError::backend_with_context(error, generate_context(&request, None))
            })?;
            let job = this
                .runtime
                .client()
                .generate(&request)
                .await
                .map_err(|error| {
                    PipelineError::backend_with_context(error, generate_context(&request, None))
                })?;
            // The job id lets a failed run be cross-referenced against the
            // server logs.
            let job_id = job.id.clone();
            let generated = this.runtime.wait_for_job(job).await.map_err(|error| {
                PipelineError::backend_with_context(
                    error,
                    generate_context(&request, Some(&job_id)),
                )
            })?;
            let imported = this
                .runtime
                .import_asset(generated)
                .await
                .map_err(|error| {
                    PipelineError::backend_with_context(
                        error,
                        generate_context(&request, Some(&job_id)),
                    )
                })?;
            Ok(GenerateResponse {
                status: "completed".to_owned(),
                image_path: Some(imported),
                seed: request.seed,
                width: Some(request.width),
                height: Some(request.height),
                error: None,
            })
        })
    }

    fn llm_draft(&self, request: &str) -> BoxFuture<'_, Result<Option<String>, PipelineError>> {
        let this = self.clone();
        let request = request.to_owned();
        Box::pin(async move {
            let Some(client) = &this.llm else {
                return Ok(None);
            };
            let messages = [
                ChatMessage::text("system", Self::draft_system_prompt()),
                ChatMessage::text("user", request),
            ];
            match client.chat(&messages).await {
                Ok(completion) => Ok(completion
                    .choices
                    .first()
                    .and_then(|choice| choice.message.content.clone())
                    .map(|content| content.trim().to_owned())
                    .filter(|content| !content.is_empty())),
                Err(error) => {
                    tracing::warn!(%error, "LLM draft degraded to manual input");
                    Ok(None)
                }
            }
        })
    }

    fn import_asset(
        &self,
        source: std::path::PathBuf,
    ) -> BoxFuture<'_, Result<String, PipelineError>> {
        let this = self.clone();
        Box::pin(async move {
            this.runtime
                .import_asset(source)
                .await
                .map_err(PipelineError::backend)
        })
    }

    fn llm_ground(&self, request: &str) -> BoxFuture<'_, Result<Option<String>, PipelineError>> {
        let this = self.clone();
        let request = request.to_owned();
        Box::pin(async move {
            let Some(client) = &this.llm else {
                return Ok(None);
            };
            let messages = [
                ChatMessage::text("system", Self::ground_system_prompt()),
                ChatMessage::text("user", request),
            ];
            match client.chat(&messages).await {
                Ok(completion) => Ok(completion
                    .choices
                    .first()
                    .and_then(|choice| choice.message.content.clone())
                    .map(|content| content.trim().trim_matches('"').trim().to_owned())
                    .filter(|content| !content.is_empty())),
                Err(error) => {
                    tracing::warn!(%error, "LLM grounding degraded to the deterministic template");
                    Ok(None)
                }
            }
        })
    }
}

impl CreativeBackend {
    /// The OpenAI-style structured-output format LM Studio accepts (newer
    /// builds reject the legacy `{"type": "json_object"}`). `strict` makes
    /// the server constrain the output to the schema (validated live
    /// against LM Studio); the lenient serde parse still tolerates the
    /// optional fields the model omits.
    fn json_schema_response_format(name: &str, schema: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "json_schema",
            "json_schema": { "name": name, "strict": true, "schema": schema }
        })
    }

    /// The closed-vocabulary op shape: `{"operations": [...]}` with a
    /// oneOf union per op so strict mode enforces each op's own required
    /// fields (validated live against LM Studio).
    fn stack_response_format() -> serde_json::Value {
        Self::json_schema_response_format(
            "operation_stack",
            serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "operations": {
                        "type": "array",
                        "items": {
                            "oneOf": [
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "create" },
                                        "kind": { "enum": ["story", "scene", "beat", "character", "environment", "object"] },
                                        "description": { "type": "string" },
                                        "name": { "type": "string" },
                                        "size": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2 },
                                        "steps": { "type": "integer" },
                                        "loras": { "type": "array", "items": { "type": "object", "additionalProperties": false, "properties": { "path": { "type": "string" }, "multiplier": { "type": "number" } }, "required": ["path"] } }
                                    },
                                    "required": ["op", "kind", "description"]
                                },
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "variant" },
                                        "target": { "type": "string" },
                                        "description": { "type": "string" },
                                        "axis": { "enum": ["outfit", "age", "body", "hair", "expression", "time-of-day", "weather", "season", "mood"] },
                                        "size": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2 },
                                        "steps": { "type": "integer" },
                                        "loras": { "type": "array", "items": { "type": "object", "additionalProperties": false, "properties": { "path": { "type": "string" }, "multiplier": { "type": "number" } }, "required": ["path"] } }
                                    },
                                    "required": ["op", "target", "description"]
                                },
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "regenerate" },
                                        "target": { "type": "string" },
                                        "prompt": { "type": "string" },
                                        "size": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2 },
                                        "steps": { "type": "integer" },
                                        "loras": { "type": "array", "items": { "type": "object", "additionalProperties": false, "properties": { "path": { "type": "string" }, "multiplier": { "type": "number" } }, "required": ["path"] } }
                                    },
                                    "required": ["op", "target"]
                                },
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "resize" },
                                        "target": { "type": "string" },
                                        "width": { "type": "integer" },
                                        "height": { "type": "integer" }
                                    },
                                    "required": ["op", "target", "width", "height"]
                                },
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "compose" },
                                        "scene": { "type": "string" },
                                        "description": { "type": "string" },
                                        "background": { "type": "string" },
                                        "layers": {
                                            "type": "array",
                                            "items": {
                                                "type": "object",
                                                "additionalProperties": false,
                                                "properties": {
                                                    "artifact": { "type": "string" },
                                                    "variant": { "type": "string" }
                                                },
                                                "required": ["artifact"]
                                            }
                                        }
                                    },
                                    "required": ["op", "scene", "description"]
                                },
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "draft" },
                                        "target": { "type": "string" },
                                        "request": { "type": "string" }
                                    },
                                    "required": ["op", "target", "request"]
                                },
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "modify" },
                                        "target": { "type": "string" },
                                        "description": { "type": "string" },
                                        "size": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2 },
                                        "steps": { "type": "integer" },
                                        "loras": { "type": "array", "items": { "type": "object", "additionalProperties": false, "properties": { "path": { "type": "string" }, "multiplier": { "type": "number" } }, "required": ["path"] } }
                                    },
                                    "required": ["op", "target", "description"]
                                },
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "attach-lora" },
                                        "target": { "type": "string" },
                                        "lora": { "type": "string" },
                                        "multiplier": { "type": "number" }
                                    },
                                    "required": ["op", "target", "lora"]
                                },
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "lora-scale" },
                                        "lora": { "type": "string" },
                                        "multiplier": { "type": "number" }
                                    },
                                    "required": ["op", "lora", "multiplier"]
                                },
                                {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": { "const": "lora-remove" },
                                        "lora": { "type": "string" }
                                    },
                                    "required": ["op", "lora"]
                                }
                            ]
                        }
                    }
                },
                "required": ["operations"]
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_schema_is_strict_with_per_op_requirements() {
        let schema = CreativeBackend::stack_response_format();
        assert_eq!(
            schema["type"], "json_schema",
            "newer LM Studio builds reject the legacy json_object format"
        );
        assert_eq!(schema["json_schema"]["strict"], true);
        let items = &schema["json_schema"]["schema"]["properties"]["operations"]["items"]["oneOf"];
        let variants = items.as_array().expect("oneOf union per op");
        assert_eq!(variants.len(), 10);
        for variant in variants {
            assert_eq!(variant["additionalProperties"], false);
            assert!(
                variant["required"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("op")),
                "every op variant requires the discriminator"
            );
        }
    }

    fn test_defaults() -> GenerationDefaults {
        GenerationDefaults {
            sizes: Default::default(),
            steps: 25,
            cfg_scale: clipforge_inference::sd::DEFAULT_CFG_SCALE,
            sample_method: clipforge_inference::sd::DEFAULT_SAMPLE_METHOD.to_owned(),
            loras: Default::default(),
        }
    }

    #[test]
    fn planner_prompt_defines_when_each_op_applies() {
        let registry = ArtifactRegistry::new();
        let defaults = test_defaults();
        let prompt = CreativeBackend::propose_system_prompt(&registry, &defaults, None);
        for expectation in [
            "modify: a localized change to one region of an existing image",
            "variant: a whole new version of an existing visual artifact",
            "regenerate: redo an existing artifact's image in full",
            "draft: ONLY a text note or idea attached to an artifact. It never",
            "never use draft for any request that changes what an image shows",
            "add glasses to mia\" → modify",
            "make mia 10 years older\" → variant",
            "attach-lora: bind an adapter to an artifact",
            "Never invent adapter names",
            "A change to what an image shows is modify, not",
        ] {
            assert!(prompt.contains(expectation), "missing: {expectation}");
        }
    }

    #[test]
    fn planner_prompt_names_the_viewed_artifact_as_the_default_target() {
        let mut registry = ArtifactRegistry::new();
        let mia = registry
            .create_artifact(
                crate::registry::ArtifactKind::Character,
                crate::registry::CreateArtifact {
                    name: "mia".into(),
                    description: "Mia, the keeper".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let variant = registry
            .create_artifact(
                crate::registry::ArtifactKind::Character,
                crate::registry::CreateArtifact {
                    name: "mia-outfit".into(),
                    description: "in yellow rain gear".into(),
                    variant_of: Some(mia),
                    axis: Some(crate::registry::VariantAxis::Outfit),
                    ..Default::default()
                },
            )
            .unwrap();
        let defaults = test_defaults();
        let prompt = CreativeBackend::propose_system_prompt(
            &registry,
            &defaults,
            registry.artifact(variant),
        );
        assert!(
            prompt.contains("Currently viewed artifact: c:mia-outfit (character — mia-outfit)"),
            "the viewed artifact is named: {prompt}"
        );
        assert!(
            prompt.contains("applies to the viewed artifact: prefer it as the target"),
            "ref-less requests bind to the viewed artifact: {prompt}"
        );
    }

    #[test]
    fn vocabulary_teaches_size_steps_and_lora_intent() {
        let registry = ArtifactRegistry::new();
        let defaults = test_defaults();
        let prompt = CreativeBackend::propose_system_prompt(&registry, &defaults, None);
        for expectation in [
            "\"op\": \"resize\", \"target\": \"c:<name>\", \"width\": w, \"height\": h",
            "Include \"size\", \"steps\", or \"loras\" in an op ONLY when the user explicitly asks",
            "\"make the image 1024x1024\" → resize",
            "\"use 8 steps\" → the op with \"steps\": 8",
            "Never invent adapter names",
        ] {
            assert!(prompt.contains(expectation), "missing: {expectation}");
        }
        // The tools prompt sends the model to the discovery tools.
        let tools_prompt = CreativeBackend::propose_tools_prompt(&registry, None);
        assert!(
            tools_prompt.contains("Call list_artifacts and list_loras"),
            "tool mode directs the model to discover: {tools_prompt}"
        );
        // The viewing note rides along in tool mode too.
        let mut registry = ArtifactRegistry::new();
        let mia = registry
            .create_artifact(
                crate::registry::ArtifactKind::Character,
                crate::registry::CreateArtifact {
                    name: "mia".into(),
                    description: "Mia".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let tools_prompt = CreativeBackend::propose_tools_prompt(&registry, registry.artifact(mia));
        assert!(
            tools_prompt.contains("Currently viewed artifact: c:mia"),
            "tool mode keeps the viewing note: {tools_prompt}"
        );
    }

    #[test]
    fn planner_tools_are_openai_shaped_and_discoverable() {
        let tools = CreativeBackend::planner_tools();
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect();
        assert_eq!(names, ["list_artifacts", "list_loras"]);
        for tool in &tools {
            assert_eq!(tool.r#type, "function");
            assert_eq!(tool.function.parameters["type"], "object");
            assert!(!tool.function.description.is_empty());
            // The tool definitions serialize to the OpenAI wire shape.
            let value = serde_json::to_value(tool).expect("tool should serialize");
            assert_eq!(value["type"], "function");
            assert!(value["function"]["name"].is_string());
        }
    }
}
