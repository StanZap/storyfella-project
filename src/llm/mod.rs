use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    pub content: ChatContent,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Clone, Debug, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

#[derive(Clone, Debug, Default)]
pub struct ChatOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub seed: Option<u64>,
    pub response_format: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletion {
    pub id: String,
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<CompletionUsage>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoice {
    pub message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponseMessage {
    pub role: String,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CompletionUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Deserialize)]
pub struct ModelList {
    pub data: Vec<Model>,
}

#[derive(Debug, Deserialize)]
pub struct Model {
    pub id: String,
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
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<&'a Value>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: ChatContent::Text(text.into()),
        }
    }

    pub fn with_image(
        role: impl Into<String>,
        text: impl Into<String>,
        image_data_url: impl Into<String>,
    ) -> Self {
        Self {
            role: role.into(),
            content: ChatContent::Parts(vec![
                ChatContentPart::Text { text: text.into() },
                ChatContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: image_data_url.into(),
                    },
                },
            ]),
        }
    }
}

impl LmStudioClient {
    pub fn new(config: LmStudioConfig) -> Result<Self, LmStudioError> {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(LmStudioError::Build)?;
        Ok(Self { http, config })
    }

    pub async fn list_models(&self) -> Result<ModelList, LmStudioError> {
        let url = format!("{}/models", self.config.base_url.trim_end_matches('/'));
        let request = self.authorize(self.http.get(url));
        let response = request.send().await.map_err(LmStudioError::Request)?;
        Self::decode(response).await
    }

    pub async fn chat(&self, messages: &[ChatMessage]) -> Result<ChatCompletion, LmStudioError> {
        self.chat_with_options(messages, &ChatOptions::default())
            .await
    }

    pub async fn chat_with_options(
        &self,
        messages: &[ChatMessage],
        options: &ChatOptions,
    ) -> Result<ChatCompletion, LmStudioError> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let request = self.authorize(self.http.post(url)).json(&ChatRequest {
            model: &self.config.model,
            messages,
            stream: false,
            temperature: options.temperature,
            max_tokens: options.max_tokens,
            seed: options.seed,
            response_format: options.response_format.as_ref(),
        });

        let response = request.send().await.map_err(LmStudioError::Request)?;
        Self::decode(response).await
    }

    fn authorize(&self, mut request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(api_key) = &self.config.api_key {
            request = request.bearer_auth(api_key);
        }
        request
    }

    async fn decode<T: for<'de> Deserialize<'de>>(
        response: reqwest::Response,
    ) -> Result<T, LmStudioError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multimodal_message_uses_openai_content_parts() {
        let message = ChatMessage::with_image("user", "Describe it", "data:image/png;base64,abc");
        let value = serde_json::to_value(message).expect("message should serialize");

        assert_eq!(value["role"], "user");
        assert_eq!(value["content"][0]["type"], "text");
        assert_eq!(value["content"][1]["type"], "image_url");
        assert_eq!(
            value["content"][1]["image_url"]["url"],
            "data:image/png;base64,abc"
        );
    }
}

// TODO: Add planner prompts after the application workflow is defined.
