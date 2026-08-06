use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::errors::{CodedError, ErrorCode};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<AssistantToolCall>>,
}

impl ChatMessage {
    fn with_tool_call_id(mut self, tool_call_id: impl Into<String>) -> Self {
        self.tool_call_id = Some(tool_call_id.into());
        self
    }

    fn with_tool_calls_field(mut self, tool_calls: Vec<AssistantToolCall>) -> Self {
        self.tool_calls = Some(tool_calls);
        self
    }
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
    /// OpenAI-style function tools the model may call (multi-round
    /// planning); see [`LmStudioClient::chat_with_tools`].
    pub tools: Option<Vec<ToolDefinition>>,
}

/// One OpenAI-style function tool definition.
#[derive(Clone, Debug, Serialize)]
pub struct ToolDefinition {
    pub r#type: &'static str,
    pub function: ToolFunction,
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// A function call the model requested (the assistant message carries
/// these; the caller runs the tool and replies with a `tool` message).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantToolCall {
    pub id: String,
    pub r#type: String,
    pub function: ToolCallFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    /// The model's argument JSON as a string.
    pub arguments: String,
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

#[derive(Clone, Debug, Deserialize)]
pub struct ChatResponseMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<AssistantToolCall>>,
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
    #[error("LM Studio planning did not finish: {0}")]
    Planner(String),
}

impl CodedError for LmStudioError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Build(_) => ErrorCode::LlmBuild,
            Self::Request(_) => ErrorCode::LlmRequest,
            Self::Status { .. } => ErrorCode::LlmStatus,
            Self::Planner(_) => ErrorCode::LlmPlanner,
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolDefinition]>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: ChatContent::Text(text.into()),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    /// An assistant message carrying tool calls (the `content` may be
    /// empty or a short summary; the calls follow).
    pub fn with_tool_calls(content: Option<String>, tool_calls: Vec<AssistantToolCall>) -> Self {
        Self {
            role: "assistant".to_owned(),
            content: ChatContent::Text(content.unwrap_or_default()),
            tool_call_id: None,
            tool_calls: None,
        }
        .with_tool_calls_field(tool_calls)
    }

    /// A tool result: `role: "tool"` with the call id the model must see.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_owned(),
            content: ChatContent::Text(content.into()),
            tool_call_id: None,
            tool_calls: None,
        }
        .with_tool_call_id(tool_call_id)
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
            tool_call_id: None,
            tool_calls: None,
        }
    }
}

/// Strips a markdown fence around JSON (some models wrap their output in
/// ```json … ``` even under a structured response format).
pub fn strip_json_fence(value: &str) -> &str {
    let trimmed = value.trim();
    let without_start = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    without_start
        .strip_suffix("```")
        .unwrap_or(without_start)
        .trim()
}

impl LmStudioClient {
    pub fn new(config: LmStudioConfig) -> Result<Self, LmStudioError> {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(LmStudioError::Build)?;
        Ok(Self { http, config })
    }

    /// The model identifier every request carries (what LM Studio routes to).
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// Swaps the requested model at runtime (the Settings picker; the config
    /// file persists the choice across runs).
    pub fn set_model(&mut self, model: String) {
        self.config.model = model;
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
            tools: options.tools.as_deref(),
        });

        let response = request.send().await.map_err(LmStudioError::Request)?;
        Self::decode(response).await
    }

    /// One multi-round tool-calling session: the model may call the given
    /// tools any number of times; `run_tool` executes each call and its
    /// result is fed back as a `tool` message. `response_format` (strict
    /// JSON schema when available) constrains the final answer so the
    /// model cannot drift out of the vocabulary after the tool rounds.
    /// Returns the final completion (the message whose `tool_calls` is
    /// `None`), or `Err` if the round limit is hit.
    pub async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: Vec<ToolDefinition>,
        response_format: Option<Value>,
        run_tool: impl Fn(&str, &str) -> String,
        max_rounds: usize,
    ) -> Result<ChatCompletion, LmStudioError> {
        let mut conversation = messages.to_vec();
        for _ in 0..max_rounds {
            let options = ChatOptions {
                tools: Some(tools.clone()),
                response_format: response_format.clone(),
                ..ChatOptions::default()
            };
            let completion = self.chat_with_options(&conversation, &options).await?;
            let message = completion
                .choices
                .first()
                .map(|choice| choice.message.clone())
                .ok_or_else(|| {
                    LmStudioError::Planner("completion carried no message".to_owned())
                })?;
            let Some(calls) = message.tool_calls.clone() else {
                return Ok(completion);
            };
            conversation.push(ChatMessage::with_tool_calls(
                message.content.clone(),
                calls.clone(),
            ));
            for call in &calls {
                let result = run_tool(&call.function.name, &call.function.arguments);
                conversation.push(ChatMessage::tool_result(&call.id, result));
            }
        }
        Err(LmStudioError::Planner(
            "tool-calling round limit reached".to_owned(),
        ))
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
    fn tool_messages_serialize_to_the_openai_wire_shape() {
        // A tool result message.
        let result = ChatMessage::tool_result("call_1", "c:mia character Mia");
        let value = serde_json::to_value(&result).expect("tool result should serialize");
        assert_eq!(value["role"], "tool");
        assert_eq!(value["tool_call_id"], "call_1");
        assert_eq!(value["content"], "c:mia character Mia");

        // An assistant message carrying tool calls.
        let assistant = ChatMessage::with_tool_calls(
            Some("checking".to_owned()),
            vec![AssistantToolCall {
                id: "call_1".to_owned(),
                r#type: "function".to_owned(),
                function: ToolCallFunction {
                    name: "list_loras".to_owned(),
                    arguments: "{}".to_owned(),
                },
            }],
        );
        let value = serde_json::to_value(&assistant).expect("assistant should serialize");
        assert_eq!(value["role"], "assistant");
        assert_eq!(value["tool_calls"][0]["id"], "call_1");
        assert_eq!(value["tool_calls"][0]["type"], "function");
        assert_eq!(value["tool_calls"][0]["function"]["name"], "list_loras");
        assert_eq!(value["tool_calls"][0]["function"]["arguments"], "{}");

        // Tool responses round-trip through the deserializer the client
        // uses for completions.
        let wire = serde_json::json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_2",
                "type": "function",
                "function": { "name": "list_artifacts", "arguments": "{}" }
            }]
        });
        let parsed: ChatResponseMessage =
            serde_json::from_value(wire).expect("completion message should parse");
        let calls = parsed.tool_calls.expect("tool calls present");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "list_artifacts");
    }

    #[test]
    fn tools_are_carried_in_the_request_payload() {
        let options = ChatOptions {
            tools: Some(vec![ToolDefinition {
                r#type: "function",
                function: ToolFunction {
                    name: "list_loras".to_owned(),
                    description: "List adapters".to_owned(),
                    parameters: serde_json::json!({ "type": "object" }),
                },
            }]),
            ..ChatOptions::default()
        };
        let request = ChatRequest {
            model: "m",
            messages: &[ChatMessage::text("user", "hi")],
            stream: false,
            temperature: None,
            max_tokens: None,
            seed: None,
            response_format: None,
            tools: options.tools.as_deref(),
        };
        let value = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(value["tools"][0]["function"]["name"], "list_loras");
        // Without tools the key is absent, not empty.
        let request = ChatRequest {
            tools: None,
            ..request
        };
        let value = serde_json::to_value(&request).expect("request should serialize");
        assert!(value.get("tools").is_none());
    }
}

// TODO: Add planner prompts after the application workflow is defined.
