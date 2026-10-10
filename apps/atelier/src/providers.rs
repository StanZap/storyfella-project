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
    pub fn hosted(self) -> Option<clipforge_inference::images::HostedProvider> {
        use clipforge_inference::images::HostedProvider;
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
    pub reduced_motion: bool,
    pub native_config: String,
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
            reduced_motion: false,
            native_config: String::new(),
        }
    }
}
impl Settings {
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
    pub keys_path: std::path::PathBuf,
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
fn saved_keys(path: &std::path::Path) -> Result<std::collections::BTreeMap<String, String>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("Could not read saved API keys"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error).context("Could not read saved API keys"),
    }
}
pub fn store_key(path: &std::path::Path, provider: Provider, key: &str) -> Result<()> {
    ensure!(!key.trim().is_empty(), "Enter an API key");
    let mut keys = saved_keys(path)?;
    keys.insert(provider.account().into(), key.trim().into());
    document::atomic_write(path, &serde_json::to_vec(&keys)?).context("Could not save API key")
}
pub(crate) fn resolve_key(
    path: &std::path::Path,
    provider: Provider,
    environment: Option<String>,
) -> Result<String> {
    if let Some(key) = environment.filter(|key| !key.trim().is_empty()) {
        return Ok(key.trim().to_owned());
    }
    if let Some(key) = saved_keys(path)?
        .remove(provider.account())
        .filter(|key| !key.trim().is_empty())
    {
        return Ok(key.trim().to_owned());
    }
    bail!(
        "Add your {} key in Settings or set {}",
        provider.name(),
        provider.key_env()
    )
}
pub(crate) fn api_key(path: &std::path::Path, provider: Provider) -> Result<String> {
    resolve_key(path, provider, std::env::var(provider.key_env()).ok())
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
            started.elapsed() < Duration::from_secs(300),
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
