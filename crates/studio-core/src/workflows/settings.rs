//! How the assistant's workflows ask the model and how hard they try: the
//! `[assistant]` section of Studio's settings.

pub use agentkit_workflow::Thinking;
use agentkit_workflow::{builder::NodeRef, WorkflowBuilder};
use serde::{Deserialize, Serialize};

pub const REPAIR_ATTEMPTS: std::ops::RangeInclusive<u32> = 0..=5;
pub const WHOLE_STORY_KIB: std::ops::RangeInclusive<u32> = 4..=256;
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
    /// How many times an answer that does not compile is sent back.
    pub repair_attempts: u32,
    /// A Project request sends every file's text while the story is smaller
    /// than this; a larger one is described by its files' paths and scenes.
    pub whole_story_kib: u32,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            thinking: None,
            max_output_tokens: None,
            repair_attempts: 2,
            whole_story_kib: 24,
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
            repair_attempts: clamp(self.repair_attempts, REPAIR_ATTEMPTS),
            whole_story_kib: clamp(self.whole_story_kib, WHOLE_STORY_KIB),
        }
    }

    /// The settings with thinking off, for asking again after an answer was
    /// cut off at the output limit; `None` when thinking is off already.
    pub(super) fn without_thinking(self) -> Option<Self> {
        (self.thinking != Some(Thinking::Off)).then_some(Self {
            thinking: Some(Thinking::Off),
            ..self
        })
    }

    pub(super) fn repairs(&self) -> usize {
        self.repair_attempts as usize
    }

    pub(super) fn whole_story_bytes(&self) -> usize {
        self.whole_story_kib as usize * 1024
    }

    /// Sets the model step's thinking and output limit.
    pub(super) fn configure(&self, graph: &mut WorkflowBuilder, model: NodeRef) {
        if let Some(thinking) = self.thinking {
            graph.set(model, "thinking", thinking.as_str());
        }
        if let Some(tokens) = self.max_output_tokens {
            graph.set(model, "max_output_tokens", tokens);
        }
    }
}
