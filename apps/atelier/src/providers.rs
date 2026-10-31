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
}
impl Provider {
    pub const ALL: [Self; 7] = [
        Self::QwenNative,
        Self::KreaNative,
        Self::Fal,
        Self::Replicate,
        Self::Alibaba,
        Self::GptImage,
        Self::Comfy,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::QwenNative => "Qwen Image 2.1 · local",
            Self::KreaNative => "Krea 2 Turbo · local",
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
        }
    }
    pub fn key_env(self) -> &'static str {
        self.hosted().map(|p| p.key_env()).unwrap_or("")
    }
    pub fn supports_edit(self) -> bool {
        !matches!(self, Self::Comfy | Self::KreaNative)
    }
    pub fn hosted(self) -> Option<agentkit_ai::images::HostedProvider> {
        use agentkit_ai::images::HostedProvider;
        match self {
            Self::Fal => Some(HostedProvider::Fal),
            Self::Replicate => Some(HostedProvider::Replicate),
            Self::Alibaba => Some(HostedProvider::Alibaba),
            Self::GptImage => Some(HostedProvider::GptImage),
            Self::Comfy | Self::QwenNative | Self::KreaNative => None,
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
            _ => None,
        }
    }
    pub fn presets(self) -> Vec<(&'static str, &'static str, &'static str)> {
        self.hosted()
            .map(|p| p.presets().to_vec())
            .unwrap_or_default()
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
        self.model = profile.model;
        self.edit_model = profile.edit_model;
        self.endpoint = profile.endpoint;
    }
}
pub struct Request {
    pub native: Arc<crate::native_images::NativeImages>,
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

pub(crate) fn shared_config_key(
    provider: Provider,
    registry: &agentkit_ai::deployments::Providers,
) -> Option<String> {
    let configured = shared_provider(registry, provider)?;
    shared_stored_api_key(provider, registry).or_else(|| {
        configured
            .api_key_env
            .as_deref()
            .and_then(|name| std::env::var(name).ok())
            .filter(|key| !key.trim().is_empty())
            .map(|key| key.trim().to_owned())
    })
}

pub(crate) fn shared_stored_api_key(
    provider: Provider,
    registry: &agentkit_ai::deployments::Providers,
) -> Option<String> {
    shared_provider(registry, provider)?
        .api_key
        .clone()
        .filter(|key| !key.trim().is_empty())
        .map(|key| key.trim().to_owned())
}

fn shared_provider(
    registry: &agentkit_ai::deployments::Providers,
    provider: Provider,
) -> Option<&agentkit_ai::deployments::Provider> {
    let hosted = provider.hosted()?;
    let matches = |configured: &agentkit_ai::deployments::Provider| {
        matches!(
            (hosted, configured.kind),
            (
                agentkit_ai::images::HostedProvider::Fal,
                agentkit_ai::deployments::ProviderKind::Fal,
            ) | (
                agentkit_ai::images::HostedProvider::Replicate,
                agentkit_ai::deployments::ProviderKind::Replicate,
            ) | (
                agentkit_ai::images::HostedProvider::Alibaba,
                agentkit_ai::deployments::ProviderKind::Alibaba,
            ) | (
                agentkit_ai::images::HostedProvider::GptImage,
                agentkit_ai::deployments::ProviderKind::GptImage,
            )
        )
    };
    registry
        .provider_for(agentkit_ai::deployments::Capability::T2i)
        .filter(|(_, configured)| matches(configured))
        .or_else(|| {
            registry
                .provider_for(agentkit_ai::deployments::Capability::Edit)
                .filter(|(_, configured)| matches(configured))
        })
        .or_else(|| registry.list().find(|(_, configured)| matches(configured)))
        .map(|(_, configured)| configured)
}

pub fn shared_endpoint(
    provider: Provider,
    registry: &agentkit_ai::deployments::Providers,
) -> Option<String> {
    shared_provider(registry, provider).map(|p| p.base_url.clone())
}

pub(crate) fn api_key(provider: Provider) -> Result<String> {
    if let Some(key) = std::env::var(provider.key_env())
        .ok()
        .filter(|key| !key.trim().is_empty())
    {
        return Ok(key.trim().to_owned());
    }
    if let Ok(registry) = storyfella_studio_core::ai_config::user_or_built_in() {
        if let Some(key) = shared_config_key(provider, &registry) {
            return Ok(key);
        }
    }
    bail!(
        "Configure {} in the shared clipforge.toml or set {}",
        provider.name(),
        provider.key_env()
    )
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

fn image_capabilities(
    settings: &Settings,
    editing: bool,
) -> Result<Option<agentkit_ai::images::ImageCapabilities>> {
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
        !editing || settings.provider.supports_edit(),
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
    if settings.provider.hosted().is_some() {
        api_key(settings.provider)
            .context("Set up this provider in Clipforge, then refresh readiness")?;
    } else if settings.provider.native_architecture().is_some() {
        let registry = storyfella_studio_core::ai_config::user_or_built_in()?;
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
