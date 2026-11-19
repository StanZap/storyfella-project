//! How the assistant's workflows ask the model and how hard they try: the
//! `[assistant]` section of Studio's settings.

pub use agentkit_workflow::Thinking;
use serde::{Deserialize, Serialize};

pub const MAX_STEPS: std::ops::RangeInclusive<u32> = 4..=200;
pub const MAX_OUTPUT_TOKENS: std::ops::RangeInclusive<u32> = 256..=1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantSettings {
    /// How much the model reasons before answering; `None` leaves it to the
    /// model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Thinking>,
    /// The most tokens one answer may take, reasoning included; `None`
    /// leaves it to the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// How many times one request may ask the model, each tool round
    /// included, before it stops and offers to continue.
    pub max_steps: u32,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            thinking: None,
            max_output_tokens: None,
            max_steps: 30,
        }
    }
}

impl AssistantSettings {
    /// The settings with each number inside its range.
    pub fn bounded(self) -> Self {
        let clamp = |value: u32, range: std::ops::RangeInclusive<u32>| {
            value.clamp(*range.start(), *range.end())
        };
        Self {
            thinking: self.thinking,
            max_output_tokens: self
                .max_output_tokens
                .map(|tokens| clamp(tokens, MAX_OUTPUT_TOKENS)),
            max_steps: clamp(self.max_steps, MAX_STEPS),
        }
    }
}
