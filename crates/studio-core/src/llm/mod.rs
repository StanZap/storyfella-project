use std::time::Duration;

use agentkit_ai::providers::openai::{ChatGenerationOptions, OpenAiCompatibleChatModel};
use agentkit_ai::{ChatRequest as FrameworkRequest, ToolChoice};
use agentkit_core::{Message, MessagePart, MessageRole, RuntimeError, ToolRisk};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::errors::{CodedError, ErrorCode};

#[derive(Clone, Debug, PartialEq)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub timeout: Duration,
}

impl From<&agentkit_ai::deployments::ChatDeployment> for LlmConfig {
    fn from(deployment: &agentkit_ai::deployments::ChatDeployment) -> Self {
        Self {
            base_url: deployment.base_url.clone(),
            model: deployment.model.clone(),
            api_key: deployment.api_key.clone(),
            timeout: deployment.timeout,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LlmClient {
    config: LlmConfig,
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
    /// planning); see [`LlmClient::chat_with_tools`].
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

#[derive(Debug, Error)]
pub enum LlmError {
    #[error(transparent)]
    Framework(#[from] RuntimeError),
    #[error("Invalid planner message: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("LLM planning did not finish: {0}")]
    Planner(String),
}

impl CodedError for LlmError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Framework(_) if self.tool_request_rejected() => ErrorCode::LlmStatus,
            Self::Framework(_) | Self::Encoding(_) => ErrorCode::LlmRequest,
            Self::Planner(_) => ErrorCode::LlmPlanner,
        }
    }
}

impl LlmError {
    pub(crate) fn tool_request_rejected(&self) -> bool {
        matches!(
            self,
            Self::Framework(RuntimeError::Provider {
                kind: agentkit_core::ProviderErrorKind::Provider
                    | agentkit_core::ProviderErrorKind::UnsupportedCapability
                    | agentkit_core::ProviderErrorKind::UnsupportedProvider
                    | agentkit_core::ProviderErrorKind::UnsupportedModel
                    | agentkit_core::ProviderErrorKind::Authentication
                    | agentkit_core::ProviderErrorKind::BadSchema,
                ..
            })
        )
    }
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

    /// A user message carrying text plus several images (`data:` URLs) —
    /// the vision-evaluation surface (`svs eval`): the model sees the
    /// images in order and answers the question.
    pub fn with_images(
        role: impl Into<String>,
        text: impl Into<String>,
        image_data_urls: Vec<String>,
    ) -> Self {
        let mut parts = vec![ChatContentPart::Text { text: text.into() }];
        parts.extend(
            image_data_urls
                .into_iter()
                .map(|url| ChatContentPart::ImageUrl {
                    image_url: ImageUrl { url },
                }),
        );
        Self {
            role: role.into(),
            content: ChatContent::Parts(parts),
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

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self { config }
    }

    /// The model identifier every request carries (what the endpoint routes to).
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// Swaps the requested model at runtime (the Settings picker; the config
    /// file persists the choice across runs).
    pub fn set_model(&mut self, model: String) {
        self.config.model = model;
    }

    pub async fn chat(&self, messages: &[ChatMessage]) -> Result<ChatCompletion, LlmError> {
        self.chat_with_options(messages, &ChatOptions::default())
            .await
    }

    pub async fn chat_with_options(
        &self,
        messages: &[ChatMessage],
        options: &ChatOptions,
    ) -> Result<ChatCompletion, LlmError> {
        let request = Self::request(messages, options)?;
        let mut model = OpenAiCompatibleChatModel::new(&self.config.base_url, &self.config.model)
            .with_timeout(self.config.timeout);
        if let Some(key) = &self.config.api_key {
            model = model.with_api_key(key);
        }
        let response = model
            .chat_with_options(
                request,
                ChatGenerationOptions {
                    temperature: options.temperature,
                    max_output_tokens: options.max_tokens,
                    seed: options.seed,
                    response_format: options.response_format.clone(),
                },
            )
            .await?;
        let mut envelope = response
            .metadata
            .ok_or_else(|| LlmError::Planner("Missing completion envelope".into()))?;
        let message = &mut envelope["choices"][0]["message"];
        message["content"] = serde_json::json!(response.assistant_message);
        if response.tool_calls.is_empty() {
            message
                .as_object_mut()
                .expect("validated response message")
                .remove("tool_calls");
        } else {
            message["tool_calls"] = Value::Array(response.tool_calls.into_iter().map(|call| serde_json::json!({
                "id":call.id, "type":"function", "function":{"name":call.name,"arguments":call.arguments.to_string()}
            })).collect());
        }
        Ok(serde_json::from_value(envelope)?)
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
    ) -> Result<ChatCompletion, LlmError> {
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
                .ok_or_else(|| LlmError::Planner("completion carried no message".to_owned()))?;
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
        Err(LlmError::Planner(
            "tool-calling round limit reached".to_owned(),
        ))
    }

    fn request(
        messages: &[ChatMessage],
        options: &ChatOptions,
    ) -> Result<FrameworkRequest, LlmError> {
        let messages = messages
            .iter()
            .map(Self::message)
            .collect::<Result<Vec<_>, _>>()?;
        let tools = options
            .tools
            .iter()
            .flatten()
            .map(|tool| agentkit_core::ToolDefinition {
                name: tool.function.name.clone(),
                description: tool.function.description.clone(),
                input_schema: tool.function.parameters.clone(),
                risk: ToolRisk::ReadOnly,
                capabilities: Vec::new(),
            })
            .collect();
        Ok(FrameworkRequest {
            messages,
            tools,
            tool_choice: ToolChoice::Auto,
            metadata: None,
        })
    }

    fn message(message: &ChatMessage) -> Result<Message, LlmError> {
        let role = match message.role.as_str() {
            "system" => MessageRole::System,
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "tool" => MessageRole::Tool,
            other => return Err(LlmError::Planner(format!("Unknown message role {other}"))),
        };
        let mut content = String::new();
        let mut parts = Vec::new();
        match &message.content {
            ChatContent::Text(text) => content = text.clone(),
            ChatContent::Parts(input) => {
                for part in input {
                    match part {
                        ChatContentPart::Text { text } => parts.push(MessagePart::text(text)),
                        ChatContentPart::ImageUrl { image_url } => {
                            let (mime, bytes) = image_url
                                .url
                                .strip_prefix("data:")
                                .and_then(|url| url.split_once(";base64,"))
                                .ok_or_else(|| {
                                    LlmError::Planner("Expected an inline base64 image".into())
                                })?;
                            parts.push(MessagePart::image(mime, bytes));
                        }
                    }
                }
            }
        }
        let mut metadata = serde_json::Map::new();
        if let Some(id) = &message.tool_call_id {
            metadata.insert("tool_call_id".into(), Value::String(id.clone()));
        }
        if let Some(calls) = &message.tool_calls {
            let calls = calls
                .iter()
                .map(|call| {
                    Ok(serde_json::json!({
                        "id":call.id, "name":call.function.name,
                        "arguments":serde_json::from_str::<Value>(&call.function.arguments)?,
                    }))
                })
                .collect::<Result<Vec<_>, serde_json::Error>>()?;
            metadata.insert("tool_calls".into(), Value::Array(calls));
        }
        Ok(Message {
            role,
            content,
            parts,
            tool_name: None,
            metadata: (!metadata.is_empty()).then_some(Value::Object(metadata)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_deployment_maps_to_the_llm_config() {
        let deployment = agentkit_ai::deployments::ChatDeployment {
            base_url: "http://localhost:8080/v1".to_owned(),
            model: "prism-ml/bonsai-27b".to_owned(),
            api_key: Some("sk-test".to_owned()),
            timeout: Duration::from_secs(42),
            server: None,
        };
        let config = LlmConfig::from(&deployment);
        assert_eq!(config.base_url, deployment.base_url);
        assert_eq!(config.model, deployment.model);
        assert_eq!(config.api_key, deployment.api_key);
        assert_eq!(config.timeout, Duration::from_secs(42));
    }

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
        let request = LlmClient::request(&[ChatMessage::text("user", "hi")], &options).unwrap();
        assert_eq!(request.tools[0].name, "list_loras");
        assert_eq!(request.tools[0].input_schema["type"], "object");
        assert!(LlmClient::request(&[], &ChatOptions::default())
            .unwrap()
            .tools
            .is_empty());
    }
}
