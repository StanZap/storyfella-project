//! The live [`GenerationBackend`]: `CreativeRuntime` for generation,
//! `LlmClient` for the soft LLM steps.

use agentkit_ai::deployments::{Capability, ChatDeployment, ProviderKind};
use agentkit_ai::native::sd::GenerateRequest;
use agentkit_ai::native::{control::Managed, service};
use futures_util::future::BoxFuture;

use crate::{
    config::{AppConfig, HostedImageProvider},
    llm::{strip_json_fence, ChatMessage, ChatOptions, LlmClient},
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
    let mut context = vec![
        ("prompt", request.prompt.clone()),
        ("width", request.width.to_string()),
        ("height", request.height.to_string()),
        ("steps", request.steps.to_string()),
    ];
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
    edit_runtime: CreativeRuntime,
    llm: Option<LlmClient>,
    /// Current shared chat deployment and server state directory.
    chat: Option<ChatDeployment>,
    hosted_image: Option<HostedImageProvider>,
    hosted_edit: Option<HostedImageProvider>,
    state_dir: std::path::PathBuf,
}

fn selected_hosted_image(config: &AppConfig, editing: bool) -> Option<HostedImageProvider> {
    let id = if editing {
        &config.image_edit_provider_id
    } else {
        &config.image_provider_id
    };
    id.as_ref()
        .and_then(|id| {
            config.image_providers.iter().find(|p| {
                &p.id == id
                    && p.capabilities.contains(&if editing {
                        Capability::Edit
                    } else {
                        Capability::T2i
                    })
            })
        })
        .and_then(|p| p.hosted.clone())
}

fn edit_runtime(config: &AppConfig) -> CreativeRuntime {
    let image = config
        .image_edit_provider_id
        .as_ref()
        .and_then(|id| {
            config.image_providers.iter().find(|provider| {
                &provider.id == id && provider.capabilities.contains(&Capability::Edit)
            })
        })
        .and_then(|provider| provider.deployment.as_ref());
    CreativeRuntime::with_image(config, image)
}

fn hosted_client(
    provider: &HostedImageProvider,
    editing: bool,
) -> Result<agentkit_ai::images::Client, String> {
    use agentkit_ai::images::{ApiKey, Client, HostedDeployment, HostedProvider, ImageOperation};
    let capability = if editing {
        Capability::Edit
    } else {
        Capability::T2i
    };
    if !provider.capabilities.contains(&capability) {
        return Err(format!("The selected image provider does not support {capability}; choose a compatible provider in Clipforge."));
    }
    let kind = match provider.kind {
        ProviderKind::Fal => HostedProvider::Fal,
        ProviderKind::Replicate => HostedProvider::Replicate,
        ProviderKind::Alibaba => HostedProvider::Alibaba,
        ProviderKind::GptImage => HostedProvider::GptImage,
        _ => return Err("The selected image provider is not hosted".into()),
    };
    let mut deployment = HostedDeployment::at(kind, &provider.model, &provider.base_url)
        .map_err(|e| e.to_string())?;
    deployment.timeout = std::time::Duration::from_secs(provider.timeout_seconds.unwrap_or(300));
    let key = provider
        .api_key
        .clone()
        .or_else(|| std::env::var(provider.api_key_env.as_deref().unwrap_or(kind.key_env())).ok())
        .ok_or("Configure an API key for this provider in Clipforge")?;
    Client::hosted(deployment, ApiKey::new(key).map_err(|e| e.to_string())?)
        .and_then(|client| {
            client.for_operation(if editing {
                ImageOperation::Edit
            } else {
                ImageOperation::Generate
            })
        })
        .map_err(|e| e.to_string())
}

fn hosted_request(request: &GenerateRequest) -> Result<agentkit_ai::images::Request, String> {
    use agentkit_ai::native::sd;
    if request.cfg_scale != sd::DEFAULT_CFG_SCALE
        || request.sample_method != sd::DEFAULT_SAMPLE_METHOD
    {
        return Err("Cloud image providers use their own CFG and sampling method; remove the custom settings.".into());
    }
    let mut result =
        agentkit_ai::images::Request::new(&request.prompt, request.width, request.height);
    result.steps = (request.steps != sd::DEFAULT_STEPS).then_some(request.steps);
    result.seed = request.seed;
    result.loras = request.loras.clone();
    Ok(result)
}

fn hosted_png(bytes: Vec<u8>) -> Result<(Vec<u8>, u32, u32), String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    if u64::from(image.width()) * u64::from(image.height()) > 32_000_000 {
        return Err("Provider image exceeds 32 megapixels".into());
    }
    let (width, height) = (image.width(), image.height());
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok((png.into_inner(), width, height))
}

impl PartialEq for CreativeBackend {
    /// The runtime is the identity — the LLM client and the managed chat
    /// servers are soft dependencies that do not affect reactivity.
    fn eq(&self, other: &Self) -> bool {
        self.runtime == other.runtime
    }
}

impl CreativeBackend {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            runtime: CreativeRuntime::new(config),
            edit_runtime: edit_runtime(config),
            llm: config.planner_config().map(LlmClient::new),
            chat: config.text.clone(),
            hosted_image: selected_hosted_image(config, false),
            hosted_edit: selected_hosted_image(config, true),
            state_dir: config.state_dir.clone(),
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

    /// Changes the image residency for the next server start (see
    /// [`CreativeRuntime::set_residency`]); `false` for an unknown id.
    pub fn set_image_residency(&mut self, residency: &str) -> bool {
        self.runtime.set_residency(residency)
    }

    /// Whether an LLM is configured (soft dependency).
    pub fn has_llm(&self) -> bool {
        self.llm.is_some()
    }

    /// The model identifier the planner currently requests; `None` when no LLM
    /// client was built (no provider serves `text`).
    pub fn llm_model(&self) -> Option<String> {
        self.llm.as_ref().map(LlmClient::model).map(str::to_owned)
    }

    /// Refresh the selected shared chat deployment without stopping servers.
    pub fn set_chat(&mut self, config: &AppConfig) {
        self.llm = config.planner_config().map(LlmClient::new);
        self.chat = config.text.clone();
        self.state_dir = config.state_dir.clone();
    }

    /// Replace the shared deployments for future jobs; running jobs retain the
    /// runtime snapshot they started with.
    pub fn set_config(&mut self, config: &AppConfig) {
        self.set_chat(config);
        self.runtime = CreativeRuntime::new(config);
        self.hosted_image = selected_hosted_image(config, false);
        self.hosted_edit = selected_hosted_image(config, true);
        self.edit_runtime = edit_runtime(config);
    }

    /// Make the selected managed text model resident before a planner operation.
    pub async fn ensure_planner_ready(&self) -> Result<(), CreativeRuntimeError> {
        if let Some(server) = self
            .chat
            .as_ref()
            .and_then(|deployment| deployment.server.as_ref())
        {
            service::Client::new(&self.state_dir)
                .warm(&Managed::Chat(server.clone()))
                .await
                .map_err(|error| CreativeRuntimeError::ChatServer(error.to_string()))?;
        }
        Ok(())
    }

    async fn acquire_planner(
        &self,
    ) -> Result<Option<(LlmClient, Option<service::Lease>)>, CreativeRuntimeError> {
        let Some(deployment) = &self.chat else {
            return Ok(self.llm.clone().map(|client| (client, None)));
        };
        let (routed, lease) = service::Client::new(&self.state_dir)
            .chat(deployment)
            .await
            .map_err(|error| CreativeRuntimeError::ChatServer(error.to_string()))?;
        Ok(Some((LlmClient::new((&routed).into()), lease)))
    }

    pub fn planner_config(&self) -> Option<crate::llm::LlmConfig> {
        self.llm.as_ref().map(|client| client.config().clone())
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
         {{\"op\": \"create\", \"kind\": \"story|scene|beat|character|environment|object|illustration\", \"description\": \"...\", \"name\": \"...\", \"size\": [w, h], \"steps\": n, \"loras\": [{{\"path\": \"...\", \"multiplier\": 0.8}}]}}\n\
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
        let Some((client, _lease)) = self
            .acquire_planner()
            .await
            .map_err(|error| error.to_string())?
        else {
            return Err(
                "The LLM endpoint is not configured or unreachable — use slash commands instead"
                    .to_owned(),
            );
        };
        // Preferred: the model discovers artifacts/adapters by calling
        // tools (any number of rounds), then emits the stack.
        match Self::propose_with_tools(&client, registry, defaults, viewing, message).await {
            Ok(stack) => return Ok(stack),
            Err(ProposeError::ToolsUnsupported(error)) => {
                tracing::warn!(%error, "tool calling unavailable; falling back to the static planner");
            }
            Err(ProposeError::Failed(error)) => return Err(error),
        }
        // Fallback: the static listing + strict JSON schema.
        Self::propose_static(&client, registry, defaults, viewing, message).await
    }

    /// The tool-calling planner: multi-round discovery, then the final JSON
    /// stack. `ToolsUnsupported` when the server rejects `tools` outright
    /// (older builds), so the caller can fall back.
    async fn propose_with_tools(
        client: &LlmClient,
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
                // closed vocabulary (validated live against the endpoint).
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
                error if error.tool_request_rejected() => {
                    ProposeError::ToolsUnsupported(error.to_string())
                }
                other => ProposeError::Failed(format!("LLM request failed: {other}")),
            })?;
        let content = completion
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone())
            .ok_or_else(|| ProposeError::Failed("the LLM returned no content".to_owned()))?;
        Self::parse_stack(&content)
    }

    /// The static planner: one strict-schema call with the listings inline.
    async fn propose_static(
        client: &LlmClient,
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
            .map_err(|error| format!("LLM request failed: {error}"))?;
        let content = completion
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone())
            .ok_or_else(|| "the LLM returned no content".to_owned())?;
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
            let editing = request.reference_image_path.is_some();
            let runtime = if editing {
                &this.edit_runtime
            } else {
                &this.runtime
            };
            let hosted = if editing {
                &this.hosted_edit
            } else {
                &this.hosted_image
            };
            if let Some(provider) = hosted {
                let fail = |error: String| {
                    PipelineError::backend_with_context(error, generate_context(&request, None))
                };
                let client = hosted_client(provider, request.reference_image_path.is_some())
                    .map_err(&fail)?;
                let mut hosted = hosted_request(&request).map_err(&fail)?;
                if let Some(path) = &request.reference_image_path {
                    hosted.source = Some(
                        tokio::fs::read(path)
                            .await
                            .map_err(|error| fail(error.to_string()))?,
                    );
                }
                let output = client
                    .generate(hosted)
                    .await
                    .map_err(|error| fail(error.to_string()))?;
                let (png, width, height) =
                    tokio::task::spawn_blocking(move || hosted_png(output.bytes))
                        .await
                        .map_err(|error| fail(error.to_string()))?
                        .map_err(&fail)?;
                let generated = runtime
                    .store_image(&png)
                    .await
                    .map_err(|error| fail(error.to_string()))?;
                return Ok(GenerateResponse {
                    status: "completed".into(),
                    image_path: Some(generated.display().to_string()),
                    seed: request.seed,
                    width: Some(width),
                    height: Some(height),
                    error: None,
                });
            }
            let lease = runtime.acquire_image().await.map_err(|error| {
                PipelineError::backend_with_context(error, generate_context(&request, None))
            })?;
            let job = lease
                .image_client()
                .generate(&request)
                .await
                .map_err(|error| {
                    PipelineError::backend_with_context(error, generate_context(&request, None))
                })?;
            // The job id lets a failed run be cross-referenced against the
            // server logs.
            let job_id = job.id.clone();
            let generated = runtime.wait_for_job(lease, job).await.map_err(|error| {
                PipelineError::backend_with_context(
                    error,
                    generate_context(&request, Some(&job_id)),
                )
            })?;
            let imported = runtime.import_asset(generated).await.map_err(|error| {
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
            let (client, _lease) = match this.acquire_planner().await {
                Ok(Some(allocation)) => allocation,
                Ok(None) => return Ok(None),
                Err(error) => {
                    tracing::warn!(%error, "the planner server did not start; the draft is skipped");
                    return Ok(None);
                }
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
            let (client, _lease) = match this.acquire_planner().await {
                Ok(Some(allocation)) => allocation,
                Ok(None) => return Ok(None),
                Err(error) => {
                    tracing::warn!(%error, "the planner server did not start; grounding is skipped");
                    return Ok(None);
                }
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
    /// The OpenAI-style structured-output format the endpoint accepts (newer
    /// builds reject the legacy `{"type": "json_object"}`). `strict` makes
    /// the server constrain the output to the schema (validated live
    /// against the endpoint); the lenient serde parse still tolerates the
    /// optional fields the model omits.
    fn json_schema_response_format(name: &str, schema: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "json_schema",
            "json_schema": { "name": name, "strict": true, "schema": schema }
        })
    }

    /// The closed-vocabulary op shape: `{"operations": [...]}` with a
    /// oneOf union per op so strict mode enforces each op's own required
    /// fields (validated live against the endpoint).
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
                                        "kind": { "enum": super::ArtifactKind::ALL },
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
    fn hosted_sampling_uses_provider_defaults_and_rejects_unmapped_controls() {
        let mut request = GenerateRequest::new("draw a house", 1024, 1024);
        let hosted = hosted_request(&request).unwrap();
        assert_eq!(hosted.steps, None);
        request.steps = 12;
        assert_eq!(hosted_request(&request).unwrap().steps, Some(12));
        request.cfg_scale = 2.0;
        assert!(hosted_request(&request).err().unwrap().contains("CFG"));
    }

    #[test]
    fn hosted_jpeg_output_is_saved_as_png() {
        let image = image::DynamicImage::new_rgb8(8, 8);
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        let (png, width, height) = hosted_png(jpeg.into_inner()).unwrap();
        assert_eq!((width, height), (8, 8));
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(hosted_png(b"not an image".to_vec()).is_err());
    }

    #[test]
    fn hosted_provider_resolves_credentials_and_enforces_declared_capability() {
        let provider = HostedImageProvider {
            kind: ProviderKind::Replicate,
            model: "black-forest-labs/flux-schnell".into(),
            base_url: "https://api.replicate.com/v1".into(),
            api_key: Some("secret-test".into()),
            api_key_env: None,
            timeout_seconds: None,
            capabilities: vec![Capability::T2i],
        };
        assert!(hosted_client(&provider, false).is_ok());
        assert!(hosted_client(&provider, true)
            .err()
            .unwrap()
            .contains("does not support"));
        assert!(!format!("{provider:?}").contains("secret-test"));
    }

    #[tokio::test]
    async fn hosted_backend_rejects_unsupported_sampling_before_network_submission() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let config_path = directory.path().join("providers.toml");
        std::fs::write(&config_path, "[defaults]\nt2i='cloud'\n[providers.cloud]\nkind='replicate'\ncapabilities=['t2i']\nbase_url='https://api.replicate.com/v1'\nmodel='black-forest-labs/flux-schnell'\napi_key='fixture-key'\n").unwrap();
        let config = AppConfig::load_from(config_path).unwrap();
        let backend = CreativeBackend::new(&config);
        let mut request = GenerateRequest::new("test", 1024, 1024);
        request.cfg_scale = 3.0;
        let error = backend.generate(&request).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("Cloud image providers use their own CFG"));
    }

    #[test]
    fn refreshing_config_updates_the_image_runtime_for_future_jobs() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let path = directory.path().join("providers.toml");
        std::fs::write(&path, "[defaults]\nt2i='local'\n[providers.local]\nkind='sdcpp'\ncapabilities=['t2i']\narchitecture='qwen-image-2.1'\nbase_url='http://127.0.0.1:7860'\n").unwrap();
        let mut config = AppConfig::load_from(path).unwrap();
        let mut backend = CreativeBackend::new(&config);
        let previous = backend.runtime();
        config.image.as_mut().unwrap().weights.diffusion =
            Some(directory.path().join("new-weights.gguf"));
        backend.set_config(&config);
        assert_ne!(previous.expected_model().unwrap(), "new-weights.gguf");
        assert_eq!(
            backend.runtime().expected_model().unwrap(),
            "new-weights.gguf"
        );
    }

    #[tokio::test]
    async fn planner_refresh_uses_the_new_shared_model_before_admission() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(root).unwrap();
        let executable = dir.path().join("llama-server");
        std::fs::write(&executable, b"fixture").unwrap();
        let config_file = dir.path().join("config.toml");
        let write_config = |model: &str| {
            std::fs::write(&config_file, format!(
            "[defaults]\ntext='local'\n[providers.local]\nkind='llamacpp'\ncapabilities=['text']\nbase_url='http://127.0.0.1:17890/v1'\nmodel={model:?}\nmodel_path={:?}\nexecutable={:?}\n",
            dir.path().join(model).display().to_string(), executable.display().to_string()
        )).unwrap()
        };
        write_config("first.gguf");
        let mut config = AppConfig::load_from(&config_file).unwrap();
        config.state_dir = dir.path().join("state");
        let mut backend = CreativeBackend::new(&config);
        assert!(backend
            .ensure_planner_ready()
            .await
            .unwrap_err()
            .to_string()
            .contains("first.gguf"));
        write_config("second.gguf");
        let mut config = AppConfig::load_from(&config_file).unwrap();
        config.state_dir = dir.path().join("state");
        backend.set_config(&config);
        assert!(backend
            .ensure_planner_ready()
            .await
            .unwrap_err()
            .to_string()
            .contains("second.gguf"));
        assert!(
            !config.state_dir.exists(),
            "missing weights are rejected before startup"
        );
    }

    #[test]
    fn stack_schema_is_strict_with_per_op_requirements() {
        let schema = CreativeBackend::stack_response_format();
        assert_eq!(
            schema["type"], "json_schema",
            "newer builds reject the legacy json_object format"
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
            cfg_scale: agentkit_ai::native::sd::DEFAULT_CFG_SCALE,
            sample_method: agentkit_ai::native::sd::DEFAULT_SAMPLE_METHOD.to_owned(),
            loras: Default::default(),
        }
    }

    #[test]
    fn set_chat_rebuilds_the_planner_client() {
        let dir = std::env::temp_dir().join(format!("svs-backend-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("clipforge.toml");
        std::fs::write(
            &path,
            "[defaults]\nt2i = \"img\"\ntext = \"local\"\n\
             [providers.img]\nkind = \"sdcpp\"\narchitecture = \"qwen-image-2.1\"\n\
             capabilities = [\"t2i\"]\nbase_url = \"http://127.0.0.1:7861\"\n\
             [providers.local]\nkind = \"openai\"\ncapabilities = [\"text\"]\n\
             base_url = \"http://localhost:8080/v1\"\nmodel = \"prism-ml/bonsai-27b\"\n",
        )
        .expect("fixture writes");
        let mut config = crate::config::AppConfig::load_from(&path).expect("config loads");
        assert!(
            config.text_provider_id.is_some(),
            "the config serves `text`"
        );

        // A provider switch supplies a whole deployment, not just a model id.
        config.text = Some(agentkit_ai::deployments::ChatDeployment {
            base_url: "http://localhost:1234/v1".to_owned(),
            model: "local-model".to_owned(),
            api_key: None,
            timeout: std::time::Duration::from_secs(5),
            server: None,
        });
        config.text_provider_id = Some("probe".to_owned());

        let mut backend = CreativeBackend::new(&config);
        assert!(backend.has_llm());
        assert_eq!(backend.llm_model().as_deref(), Some("local-model"));

        config.text = None;
        config.text_provider_id = None;
        backend.set_chat(&config);
        assert!(!backend.has_llm(), "no deployment drops the planner");
        std::fs::remove_dir_all(dir).ok();
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
