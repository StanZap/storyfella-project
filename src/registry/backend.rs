//! The live [`GenerationBackend`]: `CreativeRuntime` for vision/generation,
//! `LmStudioClient` for the soft LLM steps.

use futures_util::future::BoxFuture;

use crate::{
    app::AppConfig,
    llm::{ChatMessage, ChatOptions, LmStudioClient},
    runtime::{CreativeRuntime, CreativeRuntimeError, KreaQuantization},
    vision::{GenerateRequest, GenerateResponse, SegmentRequest, SegmentResponse},
};

use super::pipeline::{GenerationBackend, PipelineError, PlannedEdit};

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

    /// A backend whose runtime starts with a specific Krea profile (the
    /// resident-server profile is otherwise taken from `config/app.toml`).
    pub fn with_profile(config: &AppConfig, profile: KreaQuantization) -> Self {
        let mut config = config.clone();
        config.generation.profile = profile;
        Self::new(&config)
    }

    pub fn profile(&self) -> KreaQuantization {
        self.runtime.profile()
    }

    /// The [`CreativeRuntime`] this backend drives (the Settings screen
    /// controls the same processes the canvas generates through).
    pub fn runtime(&self) -> CreativeRuntime {
        self.runtime.clone()
    }

    /// Ensures the native server is resident with the requested profile.
    pub async fn ensure_profile_ready(
        &self,
        profile: KreaQuantization,
    ) -> Result<(), CreativeRuntimeError> {
        self.runtime.ensure_profile_ready(profile).await
    }

    /// Stops the generation server this backend owns (keeps Python up).
    pub async fn stop_generation(&self) -> Result<(), CreativeRuntimeError> {
        self.runtime.stop_generation_runtime().await
    }

    /// Whether an LLM is configured (soft dependency).
    pub fn has_llm(&self) -> bool {
        self.llm.is_some()
    }

    fn draft_system_prompt() -> &'static str {
        "You are the story writer for a visual storybook project. Write the \
         requested story text (premise, plot, scene, or beat narration) as \
         plain, evocative prose. Respond with only the text — no commentary, \
         no markdown."
    }

    fn plan_system_prompt() -> &'static str {
        "You split visual-edit descriptions into two prompts. Respond with \
         JSON only, exactly this shape: {\"mask_prompt\": \"...\", \
         \"inpaint_prompt\": \"...\"}. The mask_prompt names the region to \
         select (e.g. \"her hair\"). The inpaint_prompt describes the new \
         look (e.g. \"a short bob cut\")."
    }
}

impl GenerationBackend for CreativeBackend {
    fn segment(
        &self,
        request: &SegmentRequest,
    ) -> BoxFuture<'_, Result<SegmentResponse, PipelineError>> {
        let this = self.clone();
        let request = request.clone();
        Box::pin(async move {
            this.runtime
                .start_vision_runtime()
                .await
                .map_err(PipelineError::backend)?;
            this.runtime
                .vision_client()
                .segment(&request)
                .await
                .map_err(PipelineError::backend)
        })
    }

    fn generate(
        &self,
        request: &GenerateRequest,
    ) -> BoxFuture<'_, Result<GenerateResponse, PipelineError>> {
        let this = self.clone();
        let request = request.clone();
        Box::pin(async move {
            // The requested model drives which profile must be resident; a
            // mismatched server is restarted automatically (only sd-server).
            let profile = match request.model.as_deref() {
                Some("krea-2-turbo-q2") => KreaQuantization::Q2,
                Some("krea-2-turbo-q4") => KreaQuantization::Q4,
                _ => this.runtime.profile(),
            };
            this.runtime
                .ensure_profile_ready(profile)
                .await
                .map_err(PipelineError::backend)?;
            let job = this
                .runtime
                .vision_client()
                .submit_generation(&request, false)
                .await
                .map_err(PipelineError::backend)?;
            let generated = this
                .runtime
                .wait_for_job(job)
                .await
                .map_err(PipelineError::backend)?;
            let imported = this
                .runtime
                .import_asset(generated)
                .await
                .map_err(PipelineError::backend)?;
            Ok(GenerateResponse {
                status: "completed".to_owned(),
                image_path: Some(imported),
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

    fn llm_plan(&self, request: &str) -> BoxFuture<'_, Result<Option<PlannedEdit>, PipelineError>> {
        let this = self.clone();
        let request = request.to_owned();
        Box::pin(async move {
            let Some(client) = &this.llm else {
                return Ok(None);
            };
            let messages = [
                ChatMessage::text("system", Self::plan_system_prompt()),
                ChatMessage::text("user", request),
            ];
            let options = ChatOptions {
                response_format: Some(serde_json::json!({"type": "json_object"})),
                ..ChatOptions::default()
            };
            match client.chat_with_options(&messages, &options).await {
                Ok(completion) => {
                    let Some(content) = completion
                        .choices
                        .first()
                        .and_then(|choice| choice.message.content.clone())
                    else {
                        return Ok(None);
                    };
                    match serde_json::from_str::<PlannedEdit>(&content) {
                        Ok(plan) => Ok(Some(plan)),
                        Err(error) => {
                            tracing::warn!(%error, "LLM plan did not parse as JSON");
                            Ok(None)
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "LLM plan degraded to manual input");
                    Ok(None)
                }
            }
        })
    }
}
