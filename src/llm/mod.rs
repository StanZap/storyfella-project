use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq)]
pub struct LmStudioConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct LmStudioClient {
    http: Client,
    config: LmStudioConfig,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletion {
    pub id: String,
    pub choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoice {
    pub message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponseMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Error)]
pub enum LmStudioError {
    #[error("failed to construct HTTP client: {0}")]
    Build(#[source] reqwest::Error),
    #[error("LM Studio request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("LM Studio returned HTTP {status}: {body}")]
    Status { status: StatusCode, body: String },
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
}

impl LmStudioClient {
    pub fn new(config: LmStudioConfig) -> Result<Self, LmStudioError> {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(LmStudioError::Build)?;
        Ok(Self { http, config })
    }

    pub async fn chat(&self, messages: &[ChatMessage]) -> Result<ChatCompletion, LmStudioError> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let mut request = self.http.post(url).json(&ChatRequest {
            model: &self.config.model,
            messages,
        });
        if let Some(api_key) = &self.config.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await.map_err(LmStudioError::Request)?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unreadable response".to_owned());
            return Err(LmStudioError::Status { status, body });
        }
        response.json().await.map_err(LmStudioError::Request)
    }
}

// TODO: Add planner prompts and structured output after the application workflow is defined.
