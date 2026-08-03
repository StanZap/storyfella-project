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
    pub prompt: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SegmentResponse {
    pub status: String,
    pub masks: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GenerateRequest {
    pub prompt: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenerateResponse {
    pub status: String,
    pub image_path: Option<String>,
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

// TODO: Add typed model capabilities and job progress streaming.
