//! The text model a request talks to: the provider chosen in Settings,
//! started and routed by the shared inference service.
use std::sync::Arc;

use agentkit_ai::providers::openai::OpenAiCompatibleChatModel;
use agentkit_workflow::SharedStreamingModel;
use anyhow::Result;

use super::Cancellation;
use crate::{
    llm::LlmConfig,
    planner::{PlannerAllocation, PlannerBackend},
};

pub(super) fn chat_model(config: &LlmConfig) -> SharedStreamingModel {
    let mut model = OpenAiCompatibleChatModel::new(&config.base_url, &config.model)
        .with_timeout(config.timeout);
    if let Some(key) = &config.api_key {
        model = model.with_api_key(key.clone());
    }
    Arc::new(model)
}

/// Start the selected provider and route the request to its worker. A managed
/// model may take minutes to load, so cancellation applies while waiting.
pub(super) async fn wait_for_allocation(
    backend: &PlannerBackend,
    cancel: &Cancellation,
) -> Result<Option<PlannerAllocation>> {
    let allocation = backend.planner_allocation();
    tokio::pin!(allocation);
    loop {
        tokio::select! {
            result = &mut allocation => {
                let Some(allocation) = result? else {
                    anyhow::bail!("No text provider is configured. Choose one in Settings.");
                };
                return Ok(Some(allocation));
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(None);
                }
            }
        }
    }
}
