//! The native `sd-server` (stable-diffusion.cpp) generation client.
//!
//! Rust talks to the native server directly over its `sdcpp` HTTP API:
//! `POST /sdcpp/v1/img_gen` submits an async job, `GET /sdcpp/v1/jobs/{id}`
//! polls it, and `GET /sdcpp/v1/capabilities` reports the resident model
//! (see `stable-diffusion.cpp/examples/server/api.md`). The request the app
//! builds ([`GenerateRequest`]) is mapped onto the server's wire body
//! ([`SdcppImageRequest`]) here, including the base64 encoding of any
//! reference image.

use std::path::PathBuf;

use base64::Engine as _;
use reqwest::{Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

use crate::errors::{CodedError, ErrorCode};

/// The sampling method the app requests by default (Qwen-Image-2.1 has no
/// few-step turbo path; the model is sampled with CFG at ~25 steps).
pub const DEFAULT_SAMPLE_METHOD: &str = "euler";
/// The text-guidance scale the app requests by default.
pub const DEFAULT_CFG_SCALE: f32 = 6.0;
/// Qwen-Image-2.1's scheduler. The native default is left unnamed by the
/// server's compatibility fields, so the client pins it explicitly.
const SCHEDULER: &str = "discrete";
/// The sentinel the server reads as "pick a random seed".
const RANDOM_SEED: i64 = -1;
/// VAE decode/encode tile edge, in image pixels. Qwen-Image-2.1 reaches ~23.4
/// GiB at a 1024x1536 canvas, leaving no room for the untiled decode buffers;
/// sd.cpp then OOMs and retries tiled (~20 s wasted plus an error burst).
/// Sending tiling up front makes the tiled path the only path.
const VAE_TILE_PIXELS: u32 = 256;
/// Tile overlap as a fraction of the tile size (sd.cpp's default).
const VAE_TILE_OVERLAP: f32 = 0.5;

/// One reference edit request: a prompt plus the parameters the app cares
/// about. [`Self::reference_image_path`] feeds the server's
/// `ref_images[0]`.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerateRequest {
    pub prompt: String,
    /// The image being edited, sent as in-context reference tokens. `None`
    /// is a plain text-to-image render.
    pub reference_image_path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    /// Text-guidance scale (`sample_params.guidance.txt_cfg`).
    pub cfg_scale: f32,
    /// Sampling method (`sample_params.sample_method`).
    pub sample_method: String,
    /// `None` lets the server pick a random seed.
    pub seed: Option<u64>,
    /// The generation profile id; carried for provenance.
    pub model: Option<String>,
    /// The compute device the caller would prefer. The native backend runs
    /// on the device it was launched with, so this is boundary metadata.
    pub device: Option<ComputeDevice>,
    pub loras: Vec<LoraSelection>,
}

impl GenerateRequest {
    pub fn new(prompt: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            prompt: prompt.into(),
            reference_image_path: None,
            width,
            height,
            steps: crate::registry::pipeline::DEFAULT_STEPS,
            cfg_scale: DEFAULT_CFG_SCALE,
            sample_method: DEFAULT_SAMPLE_METHOD.to_owned(),
            seed: None,
            model: None,
            device: None,
            loras: Vec::new(),
        }
    }

    /// Maps the app request onto the native server's wire body.
    /// `reference_images` are the base64-encoded reference bytes, in order.
    fn wire(&self, reference_images: Vec<String>) -> SdcppImageRequest {
        SdcppImageRequest {
            prompt: self.prompt.clone(),
            width: self.width,
            height: self.height,
            seed: self.seed.map(|seed| seed as i64).unwrap_or(RANDOM_SEED),
            batch_count: 1,
            ref_images: reference_images,
            sample_params: SampleParams {
                scheduler: SCHEDULER.to_owned(),
                sample_method: self.sample_method.clone(),
                sample_steps: self.steps,
                guidance: Guidance {
                    txt_cfg: self.cfg_scale,
                },
            },
            lora: self
                .loras
                .iter()
                .map(|lora| SdcppLora {
                    path: lora.path.clone(),
                    multiplier: lora.multiplier,
                })
                .collect(),
            vae_tiling_params: VaeTilingParams {
                enabled: true,
                tile_size_w: VAE_TILE_PIXELS,
                tile_size_h: VAE_TILE_PIXELS,
                target_overlap: VAE_TILE_OVERLAP,
            },
            output_format: "png".to_owned(),
        }
    }
}

/// One adapter applied to a generation. `path` is the name the native server
/// registers it under (`LoraBindings::registered`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoraSelection {
    pub path: String,
    pub multiplier: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputeDevice {
    Auto,
    Cuda,
    Mps,
    Cpu,
}

/// The native server's image-generation request body.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SdcppImageRequest {
    pub prompt: String,
    pub width: u32,
    pub height: u32,
    pub seed: i64,
    pub batch_count: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ref_images: Vec<String>,
    pub sample_params: SampleParams,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lora: Vec<SdcppLora>,
    pub vae_tiling_params: VaeTilingParams,
    pub output_format: String,
}

/// Spatial VAE tiling. The remaining server fields (`temporal_tiling`,
/// `rel_size_*`, `extra_tiling_args`) keep their server defaults.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VaeTilingParams {
    pub enabled: bool,
    pub tile_size_w: u32,
    pub tile_size_h: u32,
    pub target_overlap: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SampleParams {
    pub scheduler: String,
    pub sample_method: String,
    pub sample_steps: u32,
    pub guidance: Guidance,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Guidance {
    pub txt_cfg: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SdcppLora {
    pub path: String,
    pub multiplier: f32,
}

/// The app-level result of a generation: the stored image path (or the
/// failure reason). Mirrors the synchronous shape the pipeline expects.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct GenerateResponse {
    pub status: String,
    pub image_path: Option<String>,
    pub model: Option<String>,
    pub seed: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub error: Option<String>,
}

/// The `202 Accepted` body returned by `POST /sdcpp/v1/img_gen`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct JobSubmission {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub created: i64,
    #[serde(default)]
    pub poll_url: Option<String>,
}

/// A polled `GET /sdcpp/v1/jobs/{id}` snapshot.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct GenerationJob {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub created: i64,
    #[serde(default)]
    pub started: Option<i64>,
    #[serde(default)]
    pub completed: Option<i64>,
    #[serde(default)]
    pub queue_position: i64,
    #[serde(default)]
    pub result: Option<JobResult>,
    #[serde(default)]
    pub error: Option<JobError>,
}

impl GenerationJob {
    /// Decodes the first image of a completed job into PNG bytes.
    pub fn first_image(&self) -> Result<Vec<u8>, GenerationClientError> {
        let image = self
            .result
            .as_ref()
            .and_then(|result| result.images.first())
            .ok_or_else(|| {
                GenerationClientError::ImagePayload("job returned no image".to_owned())
            })?;
        base64::engine::general_purpose::STANDARD
            .decode(image.b64_json.as_bytes())
            .map_err(|error| GenerationClientError::ImagePayload(error.to_string()))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct JobResult {
    #[serde(default)]
    pub output_format: Option<String>,
    pub images: Vec<JobImage>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct JobImage {
    pub index: i64,
    pub b64_json: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct JobError {
    pub code: String,
    pub message: String,
}

/// `GET /sdcpp/v1/capabilities` — only the resident-model block is consumed;
/// the rest of the (large) capability document is ignored.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Capabilities {
    pub model: ModelInfo,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ModelInfo {
    pub name: String,
    #[serde(default)]
    pub stem: String,
    #[serde(default)]
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct GenerationClient {
    http: Client,
    base_url: String,
}

#[derive(Debug, Error)]
pub enum GenerationClientError {
    #[error("generation server request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("generation server returned HTTP {status}: {body}")]
    Status { status: StatusCode, body: String },
    #[error("could not read reference image {path}: {source}")]
    ReferenceImage {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("generation server returned an unreadable image payload: {0}")]
    ImagePayload(String),
}

impl CodedError for GenerationClientError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Request(_) => ErrorCode::GenRequest,
            Self::Status { .. } => ErrorCode::GenStatus,
            Self::ReferenceImage { .. } => ErrorCode::GenReference,
            Self::ImagePayload(_) => ErrorCode::GenImage,
        }
    }
}

impl GenerationClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.into(),
        }
    }

    /// The resident model, or an error when the server is not answering.
    pub async fn capabilities(&self) -> Result<Capabilities, GenerationClientError> {
        self.get("sdcpp/v1/capabilities").await
    }

    /// Submits an image generation job.
    pub async fn generate(
        &self,
        request: &GenerateRequest,
    ) -> Result<JobSubmission, GenerationClientError> {
        let reference_images = match &request.reference_image_path {
            Some(path) => vec![reference(path).await?],
            None => Vec::new(),
        };
        self.post("sdcpp/v1/img_gen", &request.wire(reference_images))
            .await
    }

    /// Polls a job.
    pub async fn job(&self, id: &str) -> Result<GenerationJob, GenerationClientError> {
        self.get(&format!("sdcpp/v1/jobs/{id}")).await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, GenerationClientError> {
        let response = self
            .http
            .get(self.url(path))
            .send()
            .await
            .map_err(GenerationClientError::Request)?;
        Self::decode(response).await
    }

    async fn post<T: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, GenerationClientError> {
        let response = self
            .http
            .post(self.url(path))
            .json(body)
            .send()
            .await
            .map_err(GenerationClientError::Request)?;
        Self::decode(response).await
    }

    async fn decode<T: DeserializeOwned>(
        response: reqwest::Response,
    ) -> Result<T, GenerationClientError> {
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unreadable response".to_owned());
            return Err(GenerationClientError::Status { status, body });
        }
        response
            .json()
            .await
            .map_err(GenerationClientError::Request)
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

/// Reads a reference image and base64-encodes it for `ref_images[]`.
async fn reference(path: &str) -> Result<String, GenerationClientError> {
    let bytes =
        tokio::fs::read(path)
            .await
            .map_err(|source| GenerationClientError::ReferenceImage {
                path: PathBuf::from(path),
                source,
            })?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_body_carries_the_sampling_controls_and_reference() {
        let mut request = GenerateRequest::new("a lighthouse at dusk", 1024, 1536);
        request.steps = 25;
        request.seed = Some(42);
        request.loras.push(LoraSelection {
            path: "style.safetensors".to_owned(),
            multiplier: 0.8,
        });
        let value = serde_json::to_value(request.wire(vec!["AAAA".to_owned()]))
            .expect("wire body should serialize");

        assert_eq!(value["prompt"], "a lighthouse at dusk");
        assert_eq!(value["width"], 1024);
        assert_eq!(value["height"], 1536);
        assert_eq!(value["seed"], 42);
        assert_eq!(value["batch_count"], 1);
        assert_eq!(value["ref_images"][0], "AAAA");
        assert_eq!(value["sample_params"]["scheduler"], "discrete");
        assert_eq!(value["sample_params"]["sample_method"], "euler");
        assert_eq!(value["sample_params"]["sample_steps"], 25);
        assert_eq!(value["sample_params"]["guidance"]["txt_cfg"], 6.0);
        assert_eq!(value["lora"][0]["path"], "style.safetensors");
        assert_eq!(value["lora"][0]["multiplier"], serde_json::json!(0.8_f32));
        assert_eq!(value["vae_tiling_params"]["enabled"], true);
        assert_eq!(value["vae_tiling_params"]["tile_size_w"], 256);
        assert_eq!(value["vae_tiling_params"]["tile_size_h"], 256);
        assert_eq!(value["output_format"], "png");
    }

    #[test]
    fn wire_body_omits_references_without_one() {
        let request = GenerateRequest::new("a storyboard frame", 768, 448);
        let value = serde_json::to_value(request.wire(Vec::new())).expect("serializes");

        assert!(value.get("ref_images").is_none());
        assert!(value.get("lora").is_none());
        assert_eq!(value["seed"], RANDOM_SEED);
    }

    #[test]
    fn completed_job_decodes_its_first_image() {
        // "hello" in standard base64.
        let job = GenerationJob {
            id: "job_1".to_owned(),
            kind: "img_gen".to_owned(),
            status: "completed".to_owned(),
            created: 1,
            started: Some(2),
            completed: Some(3),
            queue_position: 0,
            result: Some(JobResult {
                output_format: Some("png".to_owned()),
                images: vec![JobImage {
                    index: 0,
                    b64_json: "aGVsbG8=".to_owned(),
                }],
            }),
            error: None,
        };
        assert_eq!(job.first_image().expect("decodes"), b"hello");
    }

    #[test]
    fn capabilities_read_the_resident_model() {
        let capabilities: Capabilities = serde_json::from_str(
            r#"{"model":{"name":"qwen-image-2.1","stem":"qwen-image-2.1","path":"/x"},"samplers":["euler"]}"#,
        )
        .expect("capabilities should parse");
        assert_eq!(capabilities.model.name, "qwen-image-2.1");
    }
}
