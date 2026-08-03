use reqwest::{Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct VisionClient {
    http: Client,
    base_url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SegmentRequest {
    pub image_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<SegmentPoint>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub boxes: Vec<SegmentBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<ComputeDevice>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SegmentPoint {
    pub x: f64,
    pub y: f64,
    pub label: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SegmentBox {
    pub x_min: f64,
    pub y_min: f64,
    pub x_max: f64,
    pub y_max: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SegmentMask {
    pub path: String,
    pub score: f64,
    pub area_pixels: u64,
    pub bounding_box: SegmentBox,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SegmentDetection {
    pub label: String,
    pub score: f64,
    pub bounding_box: SegmentBox,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SegmentResponse {
    pub status: String,
    pub masks: Vec<SegmentMask>,
    pub detections: Vec<SegmentDetection>,
    pub model: Option<String>,
    pub device: Option<String>,
    pub dtype: Option<String>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

impl SegmentRequest {
    pub fn new(image_path: impl Into<String>) -> Self {
        Self {
            image_path: image_path.into(),
            prompt: None,
            points: Vec::new(),
            boxes: Vec::new(),
            model: None,
            device: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GenerateRequest {
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<ComputeDevice>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeDevice {
    Auto,
    Cuda,
    Mps,
    Cpu,
}

impl GenerateRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            width: None,
            height: None,
            steps: None,
            seed: None,
            model: None,
            device: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenerateResponse {
    pub status: String,
    pub image_path: Option<String>,
    pub model: Option<String>,
    pub device: Option<String>,
    pub dtype: Option<String>,
    pub seed: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceCapabilitiesResponse {
    pub torch_available: bool,
    pub torch_version: Option<String>,
    pub cuda_available: bool,
    pub cuda_devices: Vec<String>,
    pub mps_available: bool,
    pub recommended_device: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaptionRequest {
    pub image_path: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CaptionResponse {
    pub status: String,
    pub caption: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Debug, Error)]
pub enum VisionClientError {
    #[error("vision runtime request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("vision runtime returned HTTP {status}: {body}")]
    Status { status: StatusCode, body: String },
}

impl VisionClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.into(),
        }
    }

    pub async fn health(&self) -> Result<HealthResponse, VisionClientError> {
        self.get("health").await
    }

    pub async fn capabilities(&self) -> Result<DeviceCapabilitiesResponse, VisionClientError> {
        self.get("capabilities").await
    }

    pub async fn segment(
        &self,
        request: &SegmentRequest,
    ) -> Result<SegmentResponse, VisionClientError> {
        self.post("segment", request).await
    }

    pub async fn generate(
        &self,
        request: &GenerateRequest,
    ) -> Result<GenerateResponse, VisionClientError> {
        self.post("generate", request).await
    }

    pub async fn caption(
        &self,
        request: &CaptionRequest,
    ) -> Result<CaptionResponse, VisionClientError> {
        self.post("caption", request).await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, VisionClientError> {
        let response = self
            .http
            .get(self.url(path))
            .send()
            .await
            .map_err(VisionClientError::Request)?;
        Self::decode(response).await
    }

    async fn post<T: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, VisionClientError> {
        let response = self
            .http
            .post(self.url(path))
            .json(body)
            .send()
            .await
            .map_err(VisionClientError::Request)?;
        Self::decode(response).await
    }

    async fn decode<T: DeserializeOwned>(
        response: reqwest::Response,
    ) -> Result<T, VisionClientError> {
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unreadable response".to_owned());
            return Err(VisionClientError::Status { status, body });
        }
        response.json().await.map_err(VisionClientError::Request)
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

// TODO: Add job progress streaming before long-running production inference.

#[cfg(test)]
mod tests {
    use super::{ComputeDevice, GenerateRequest, SegmentBox, SegmentRequest};

    #[test]
    fn generation_request_omits_unspecified_options() {
        let request = GenerateRequest::new("a storyboard frame");
        let value = serde_json::to_value(request).expect("request should serialize");

        assert_eq!(value, serde_json::json!({"prompt": "a storyboard frame"}));
    }

    #[test]
    fn compute_device_uses_python_contract_name() {
        let mut request = GenerateRequest::new("a storyboard frame");
        request.device = Some(ComputeDevice::Mps);
        let value = serde_json::to_value(request).expect("request should serialize");

        assert_eq!(value["device"], "mps");
    }

    #[test]
    fn segmentation_request_serializes_box_prompt() {
        let mut request = SegmentRequest::new("frame.png");
        request.boxes.push(SegmentBox {
            x_min: 10.0,
            y_min: 20.0,
            x_max: 100.0,
            y_max: 200.0,
        });
        let value = serde_json::to_value(request).expect("request should serialize");

        assert_eq!(value["boxes"][0]["x_max"], 100.0);
        assert!(value.get("points").is_none());
    }
}
