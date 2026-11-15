use crate::document::{self, Reference};
use anyhow::{bail, ensure, Context, Result};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    GptImage,
    Comfy,
    Fal,
    Replicate,
    Alibaba,
    QwenNative,
    KreaNative,
    MossNative,
}
impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::QwenNative => "Qwen Image 2.1 · local",
            Self::KreaNative => "Krea 2 Turbo · local",
            Self::MossNative => "MOSS Image SFT · local",
            _ => self.hosted().map(|p| p.name()).unwrap_or("ComfyUI"),
        }
    }
    pub fn account(self) -> &'static str {
        match self {
            Self::GptImage => "openai",
            Self::Comfy => "comfy",
            Self::Fal => "fal",
            Self::Replicate => "replicate",
            Self::Alibaba => "alibaba",
            Self::QwenNative => "qwen-native",
            Self::KreaNative => "krea-native",
            Self::MossNative => "moss-native",
        }
    }
    pub fn supports_edit(self) -> bool {
        if let Some(architecture) = self.native_architecture() {
            agentkit_ai::native::sd::profile_by_id(architecture)
                .is_some_and(|profile| profile.supports_edit())
        } else {
            self != Self::Comfy
        }
    }
    pub fn hosted(self) -> Option<agentkit_ai::images::HostedProvider> {
        use agentkit_ai::images::HostedProvider;
        match self {
            Self::Fal => Some(HostedProvider::Fal),
            Self::Replicate => Some(HostedProvider::Replicate),
            Self::Alibaba => Some(HostedProvider::Alibaba),
            Self::GptImage => Some(HostedProvider::GptImage),
            Self::Comfy | Self::QwenNative | Self::KreaNative | Self::MossNative => None,
        }
    }
    pub fn defaults(self) -> ProviderProfile {
        let (model, edit_model, endpoint) = match self.hosted() {
            Some(provider) => {
                let (model, edit_model) = provider.default_models();
                (model, edit_model, provider.default_endpoint())
            }
            None => ("", "", "http://127.0.0.1:8188"),
        };
        ProviderProfile {
            model: model.into(),
            edit_model: edit_model.into(),
            endpoint: endpoint.into(),
        }
    }
    pub fn native_architecture(self) -> Option<&'static str> {
        match self {
            Self::QwenNative => Some("qwen-image-2.1"),
            Self::KreaNative => Some("krea2-turbo"),
            Self::MossNative => Some("moss-image-sft"),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub model: String,
    pub edit_model: String,
    pub endpoint: String,
}
/// A LoRA adapter applied to a local generation: the name the server registers
/// it under (see `agentkit_ai::native::sd::prepare_lora_links`) and its
/// strength.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoraSetting {
    pub name: String,
    pub multiplier: f32,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub provider: Provider,
    /// The exact image provider selected from Clipforge; ComfyUI is app-owned.
    pub provider_id: Option<String>,
    pub provider_supports_edit: bool,
    /// Captured for a queued request only; never written to settings or projects.
    #[serde(skip)]
    pub(crate) provider_snapshot: Option<ProviderSnapshot>,
    pub model: String,
    pub edit_model: String,
    pub endpoint: String,
    pub profiles: std::collections::BTreeMap<String, ProviderProfile>,
    pub workflow: Value,
    pub positive_node: String,
    pub latent_node: String,
    pub seed_node: String,
    /// Sampling steps for the local model; `None` uses its profile default.
    pub steps: Option<u32>,
    /// The generation canvas size, in pixels.
    pub width: u32,
    pub height: u32,
    /// LoRA adapters applied to local generations.
    pub loras: Vec<LoraSetting>,
    pub reduced_motion: bool,
    /// Generation wait limit captured with each request; excludes queue waiting.
    pub request_timeout_minutes: u32,
}
impl Default for Settings {
    fn default() -> Self {
        let profile = Provider::GptImage.defaults();
        Self {
            provider: Provider::GptImage,
            provider_id: None,
            provider_supports_edit: false,
            provider_snapshot: None,
            model: profile.model,
            edit_model: profile.edit_model,
            endpoint: "http://127.0.0.1:8188".into(),
            profiles: Default::default(),
            workflow: Value::Null,
            positive_node: "6".into(),
            latent_node: "5".into(),
            seed_node: "3".into(),
            steps: None,
            width: 1024,
            height: 1024,
            loras: Vec::new(),
            reduced_motion: false,
            request_timeout_minutes: 30,
        }
    }
}
impl Settings {
    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(u64::from(self.request_timeout_minutes.clamp(1, 1440)) * 60)
    }
    pub fn remember_provider(&mut self) {
        self.profiles.insert(
            self.provider.account().into(),
            ProviderProfile {
                model: self.model.clone(),
                edit_model: self.edit_model.clone(),
                endpoint: self.endpoint.clone(),
            },
        );
    }
    pub fn select_provider(&mut self, provider: Provider) {
        self.remember_provider();
        let profile = self
            .profiles
            .get(provider.account())
            .cloned()
            .unwrap_or_else(|| provider.defaults());
        self.provider = provider;
        self.provider_id = None;
        self.provider_snapshot = None;
        self.provider_supports_edit = provider.supports_edit();
        self.model = profile.model;
        self.edit_model = profile.edit_model;
        self.endpoint = profile.endpoint;
    }
}

/// One usable Clipforge image provider, including distinct entries for the
/// same service or model architecture.
#[derive(Clone, Debug)]
pub(crate) struct SharedChoice {
    pub id: String,
    pub label: String,
    pub provider: Provider,
    pub model: String,
    pub endpoint: String,
    pub supports_edit: bool,
}

pub(crate) fn shared_choices(registry: &agentkit_ai::deployments::Providers) -> Vec<SharedChoice> {
    use agentkit_ai::deployments::{Capability, ProviderKind};
    registry
        .list()
        .filter_map(|(id, configured)| {
            if !configured
                .capabilities
                .iter()
                .any(|cap| matches!(cap, Capability::T2i | Capability::Edit))
            {
                return None;
            }
            let provider = match configured.kind {
                ProviderKind::Sdcpp => match configured.architecture.as_deref()? {
                    "qwen-image-2.1" => Provider::QwenNative,
                    "krea2-turbo" => Provider::KreaNative,
                    "moss-image-sft" => Provider::MossNative,
                    _ => return None,
                },
                ProviderKind::Fal => Provider::Fal,
                ProviderKind::Replicate => Provider::Replicate,
                ProviderKind::Alibaba => Provider::Alibaba,
                ProviderKind::GptImage => Provider::GptImage,
                _ => return None,
            };
            let model = configured
                .model
                .clone()
                .or(configured.architecture.clone())
                .unwrap_or_default();
            Some(SharedChoice {
                id: id.to_owned(),
                label: format!("{id} · {model}"),
                provider,
                model,
                endpoint: configured.base_url.clone(),
                supports_edit: configured.capabilities.contains(&Capability::Edit),
            })
        })
        .collect()
}

impl Settings {
    pub fn supports_edit(&self) -> bool {
        if self.provider_id.is_some() {
            self.provider_supports_edit
        } else {
            self.provider.supports_edit()
        }
    }

    pub(crate) fn select_shared(&mut self, choice: &SharedChoice) {
        self.provider = choice.provider;
        self.provider_id = Some(choice.id.clone());
        self.provider_snapshot = None;
        self.provider_supports_edit = choice.supports_edit;
        self.model = choice.model.clone();
        self.edit_model = choice.model.clone();
        self.endpoint = choice.endpoint.clone();
    }

    /// Capture only non-secret routing and model settings for a queued job.
    pub(crate) fn capture_provider(&mut self) -> Result<()> {
        if self.provider == Provider::Comfy && self.provider_id.is_none() {
            self.provider_snapshot = None;
            return Ok(());
        }
        let registry = storyfella_studio_core::ai_config::shared_registry()?;
        self.capture_provider_from(&registry)
    }

    pub(crate) fn capture_provider_from(
        &mut self,
        registry: &agentkit_ai::deployments::Providers,
    ) -> Result<()> {
        let Some(id) = self.provider_id.as_deref() else {
            ensure!(
                self.provider == Provider::Comfy,
                "Choose an image provider configured in Clipforge"
            );
            self.provider_snapshot = None;
            return Ok(());
        };
        let choice = shared_choices(registry)
            .into_iter()
            .find(|choice| choice.id == id)
            .with_context(|| format!("Provider {id:?} is no longer configured in Clipforge"))?;
        ensure!(choice.provider == self.provider && choice.model == self.model && choice.endpoint == self.endpoint && choice.supports_edit == self.provider_supports_edit,
            "Provider configuration changed. Refresh Clipforge providers and submit the request again.");
        self.provider_snapshot = Some(ProviderSnapshot::read(registry, id)?);
        Ok(())
    }

    pub(crate) fn verify_provider(
        &self,
        registry: &agentkit_ai::deployments::Providers,
    ) -> Result<()> {
        let id = self
            .provider_id
            .as_deref()
            .context("Choose an image provider configured in Clipforge")?;
        let captured = self
            .provider_snapshot
            .as_ref()
            .context("Provider configuration was not captured; submit the request again")?;
        ensure!(*captured == ProviderSnapshot::read(registry, id)?, "Provider configuration changed while this request was queued. Review the settings and submit it again.");
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderSnapshot {
    kind: agentkit_ai::deployments::ProviderKind,
    capabilities: Vec<agentkit_ai::deployments::Capability>,
    address: String,
    model: Option<String>,
    architecture: Option<String>,
    model_dir: Option<std::path::PathBuf>,
    lora_dir: Option<std::path::PathBuf>,
    executable: Option<std::path::PathBuf>,
    weights: [Option<std::path::PathBuf>; 5],
    residency: Option<String>,
    timeout_seconds: Option<u64>,
}

impl ProviderSnapshot {
    fn read(registry: &agentkit_ai::deployments::Providers, id: &str) -> Result<Self> {
        let provider = registry
            .list()
            .find(|(name, _)| *name == id)
            .map(|(_, provider)| provider)
            .with_context(|| format!("Provider {id:?} is no longer configured in Clipforge"))?;
        Ok(Self {
            kind: provider.kind,
            capabilities: provider.capabilities.clone(),
            address: provider.base_url.clone(),
            model: provider.model.clone(),
            architecture: provider.architecture.clone(),
            model_dir: provider.model_dir.clone(),
            lora_dir: provider.lora_dir.clone(),
            executable: provider.executable.clone(),
            weights: [
                provider.weights.diffusion.clone(),
                provider.weights.text_encoder.clone(),
                provider.weights.vae.clone(),
                provider.weights.vision.clone(),
                provider.weights.tokenizer.clone(),
            ],
            residency: provider.residency.clone(),
            timeout_seconds: provider.timeout_seconds,
        })
    }
}
pub struct Request {
    pub settings: Settings,
    pub prompt: String,
    pub size: (u32, u32),
    pub source: Option<Arc<Reference>>,
    pub references: Vec<Arc<Reference>>,
    pub mask: Option<crate::edit::MaskDraft>,
    pub progress: Option<Box<dyn Fn(String) + Send + Sync>>,
    pub cancel: Arc<AtomicBool>,
}
pub(crate) fn cancelled(flag: &AtomicBool) -> Result<()> {
    ensure!(!flag.load(Ordering::Relaxed), "Generation cancelled");
    Ok(())
}
pub(crate) fn read_response(response: Response, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Provider response exceeds size limit"
    );
    Ok(bytes)
}
pub(crate) fn response_json(response: Response) -> Result<Value> {
    let status = response.status();
    let text = read_response(response, 32 * 1024 * 1024)?;
    let value: Value = serde_json::from_slice(&text)
        .map_err(|_| anyhow::anyhow!("Provider returned invalid JSON (HTTP {status})"))?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .or_else(|| value.get("error"))
            .or_else(|| value.get("message"))
            .or_else(|| value.get("detail"))
            .and_then(Value::as_str)
            .unwrap_or("Request failed");
        bail!(
            "Provider error {status}: {}",
            message.chars().take(400).collect::<String>()
        );
    }
    Ok(value)
}

pub fn generate(request: Request) -> Result<Reference> {
    ensure!(
        !request.prompt.trim().is_empty(),
        "Enter a reference prompt"
    );
    cancelled(&request.cancel)?;
    if request.settings.provider != Provider::Comfy {
        return crate::image_execution::generate(request);
    }
    ensure!(
        request.references.is_empty() && request.mask.is_none(),
        "The ComfyUI workflow adapter does not accept additional references or masks"
    );
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    ensure!(
        request.source.is_none(),
        "ComfyUI editing is not configured; choose a hosted provider"
    );
    comfy(&client, request)
}

pub fn reference_limit(settings: &Settings) -> Result<usize> {
    Ok(image_capabilities(settings, true)?.map_or(0, |capabilities| capabilities.max_input_images))
}

pub(crate) fn image_capabilities(
    settings: &Settings,
    editing: bool,
) -> Result<Option<agentkit_ai::images::ImageCapabilities>> {
    if let Some(id) = &settings.provider_id {
        let registry = storyfella_studio_core::ai_config::shared_registry()?;
        let capability = if editing {
            agentkit_ai::deployments::Capability::Edit
        } else {
            agentkit_ai::deployments::Capability::T2i
        };
        // A native client does not require credentials. Hosted capability
        // resolution below uses the validated configured route without keys.
        let configured = registry
            .list()
            .find(|(name, _)| *name == id)
            .map(|(_, p)| p)
            .with_context(|| format!("Provider {id:?} is no longer configured in Clipforge"))?;
        ensure!(
            configured.capabilities.contains(&capability),
            "Provider {id:?} does not support {capability}"
        );
        if configured.kind == agentkit_ai::deployments::ProviderKind::Sdcpp {
            return Ok(Some(
                registry
                    .image_deployment_for(id, capability)?
                    .profile()
                    .image_capabilities(),
            ));
        }
        let model = configured
            .model
            .as_deref()
            .context("Configured image model is missing")?;
        let hosted = settings
            .provider
            .hosted()
            .context("Configured image provider type changed; refresh Clipforge")?;
        return Ok(Some(
            agentkit_ai::images::HostedDeployment::at(hosted, model, &configured.base_url)?
                .capabilities(),
        ));
    }
    if let Some(provider) = settings.provider.hosted() {
        let model = if editing {
            &settings.edit_model
        } else {
            &settings.model
        };
        Ok(Some(
            agentkit_ai::images::HostedDeployment::new(provider, model)?.capabilities(),
        ))
    } else if let Some(architecture) = settings.provider.native_architecture() {
        Ok(Some(
            agentkit_ai::native::sd::profile_by_id(architecture)
                .context("Unknown local image model")?
                .image_capabilities(),
        ))
    } else {
        Ok(None)
    }
}

/// Validate the model contract before reading credentials or deployment config.
pub(crate) fn validate_capabilities(
    settings: &Settings,
    editing: bool,
    additional: usize,
    mask: bool,
) -> Result<()> {
    ensure!(
        !editing || settings.supports_edit(),
        "This provider creates new images only; choose an editing provider"
    );
    if let Some(capabilities) = image_capabilities(settings, editing)? {
        capabilities
            .validate_inputs(
                if editing {
                    agentkit_ai::images::ImageOperation::Edit
                } else {
                    agentkit_ai::images::ImageOperation::Generate
                },
                additional + usize::from(editing),
                mask,
                (settings.width, settings.height),
            )
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

/// Validate capabilities and configuration before a request enters the queue.
pub fn validate_inputs(
    settings: &Settings,
    editing: bool,
    additional: usize,
    mask: bool,
) -> Result<()> {
    validate_capabilities(settings, editing, additional, mask)?;
    ensure!(
        settings.provider == Provider::Comfy || settings.provider_id.is_some(),
        "Choose an image provider configured in Clipforge"
    );
    if settings.provider.hosted().is_some() {
        let id = settings
            .provider_id
            .as_deref()
            .context("Choose an image provider configured in Clipforge")?;
        let registry = storyfella_studio_core::ai_config::shared_registry()?;
        let capability = if editing {
            agentkit_ai::deployments::Capability::Edit
        } else {
            agentkit_ai::deployments::Capability::T2i
        };
        agentkit_ai::images::Client::from_registry_for(&registry, id, capability, None)
            .context("Set up this provider in Clipforge, then refresh readiness")?;
    } else if settings.provider.native_architecture().is_some() {
        let registry = storyfella_studio_core::ai_config::shared_registry()?;
        crate::native_images::deployment(settings, &registry, editing)?;
    } else {
        ensure!(
            !editing && additional == 0 && !mask,
            "ComfyUI supports new-image generation only"
        );
        ensure!(
            !settings.workflow.is_null(),
            "Load a ComfyUI API workflow in Settings"
        );
    }
    Ok(())
}
pub fn prepare_workflow(
    settings: &Settings,
    prompt: &str,
    size: (u32, u32),
    seed: u64,
) -> Result<Value> {
    let mut workflow = settings.workflow.clone();
    ensure!(
        workflow.is_object() && workflow.get("nodes").is_none(),
        "Load a ComfyUI workflow exported in API format"
    );
    let inputs = workflow
        .get_mut(&settings.positive_node)
        .and_then(|v| v.get_mut("inputs"))
        .context("Positive prompt node is missing from workflow")?;
    ensure!(
        inputs.get("text").is_some(),
        "Positive prompt node must have a text input"
    );
    inputs["text"] = json!(prompt);
    if !settings.latent_node.is_empty() {
        let inputs = workflow
            .get_mut(&settings.latent_node)
            .and_then(|v| v.get_mut("inputs"))
            .context("Canvas size node is missing")?;
        ensure!(
            inputs.get("width").is_some() && inputs.get("height").is_some(),
            "Canvas size node needs width and height inputs"
        );
        inputs["width"] = json!(size.0);
        inputs["height"] = json!(size.1);
        if inputs.get("batch_size").is_some() {
            inputs["batch_size"] = json!(1);
        }
    }
    if !settings.seed_node.is_empty() {
        let inputs = workflow
            .get_mut(&settings.seed_node)
            .and_then(|v| v.get_mut("inputs"))
            .context("Sampler node is missing")?;
        let field = if inputs.get("seed").is_some() {
            "seed"
        } else if inputs.get("noise_seed").is_some() {
            "noise_seed"
        } else {
            bail!("Sampler node needs seed or noise_seed")
        };
        inputs[field] = json!(seed);
    }
    Ok(workflow)
}
fn comfy(client: &Client, request: Request) -> Result<Reference> {
    let endpoint = request.settings.endpoint.trim_end_matches('/');
    let url = reqwest::Url::parse(endpoint).context("Invalid ComfyUI address")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Use an HTTP(S) ComfyUI address without credentials or query parameters"
    );
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64
        % (1u64 << 48);
    let workflow = prepare_workflow(&request.settings, &request.prompt, request.size, seed)?;
    let queued = response_json(
        client
            .post(format!("{endpoint}/prompt"))
            .json(&json!({"prompt": workflow}))
            .timeout(Duration::from_secs(20))
            .send()?,
    )?;
    let id = queued
        .get("prompt_id")
        .and_then(Value::as_str)
        .context("ComfyUI did not return a job ID")?;
    ensure!(
        id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "Invalid ComfyUI job ID"
    );
    let started = Instant::now();
    loop {
        if request.cancel.load(Ordering::Relaxed) {
            // Delete only this queued job. Never interrupt another user's running work.
            let _ = client
                .post(format!("{endpoint}/queue"))
                .json(&json!({"delete": [id]}))
                .timeout(Duration::from_secs(5))
                .send();
            bail!("Generation cancelled; a running ComfyUI job may finish on the server");
        }
        ensure!(
            started.elapsed() < request.settings.request_timeout(),
            "ComfyUI timed out; inspect its queue before retrying"
        );
        let history = response_json(
            client
                .get(format!("{endpoint}/history/{id}"))
                .timeout(Duration::from_secs(10))
                .send()?,
        )?;
        if let Some(job) = history.get(id) {
            if job.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
                bail!("ComfyUI workflow failed; inspect the server's execution log");
            }
            if let Some(outputs) = job.get("outputs").and_then(Value::as_object) {
                if let Some(output) = outputs
                    .values()
                    .filter_map(|v| v.get("images").and_then(Value::as_array))
                    .flatten()
                    .next()
                {
                    let filename = output
                        .get("filename")
                        .and_then(Value::as_str)
                        .context("Missing output filename")?;
                    let response = client
                        .get(format!("{endpoint}/view"))
                        .query(&[
                            ("filename", filename),
                            (
                                "subfolder",
                                output
                                    .get("subfolder")
                                    .and_then(Value::as_str)
                                    .unwrap_or(""),
                            ),
                            (
                                "type",
                                output
                                    .get("type")
                                    .and_then(Value::as_str)
                                    .unwrap_or("output"),
                            ),
                        ])
                        .timeout(Duration::from_secs(30))
                        .send()?
                        .error_for_status()?;
                    let bytes = read_response(response, 20 * 1024 * 1024)?;
                    cancelled(&request.cancel)?;
                    return document::reference_from_bytes(
                        "Generated reference".into(),
                        &bytes,
                        format!(
                            "ComfyUI · seed {seed}\n{}\nWorkflow: {}",
                            request.prompt, workflow
                        ),
                    );
                }
            }
            if job.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
                bail!("Workflow completed without an image; include a SaveImage output");
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
