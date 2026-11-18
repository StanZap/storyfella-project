//! Routing for Studio's selected text provider.

use std::path::PathBuf;

use agentkit_ai::{deployments::ChatDeployment, native::service};
use anyhow::{Context, Result};

use crate::{config::AppConfig, llm::LlmConfig};

/// The routed endpoint and its managed-worker lease for one writing operation.
pub struct PlannerAllocation {
    config: LlmConfig,
    _lease: Option<service::Lease>,
}

impl PlannerAllocation {
    pub fn config(&self) -> &LlmConfig {
        &self.config
    }
}

/// A snapshot of the configured text deployment for future writing requests.
#[derive(Clone, PartialEq)]
pub struct PlannerBackend {
    deployment: Option<ChatDeployment>,
    state_dir: PathBuf,
}

impl PlannerBackend {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            deployment: config.text.clone(),
            state_dir: config.state_dir.clone(),
        }
    }

    pub fn has_llm(&self) -> bool {
        self.deployment.is_some()
    }

    pub fn set_config(&mut self, config: &AppConfig) {
        *self = Self::new(config);
    }

    pub async fn planner_allocation(&self) -> Result<Option<PlannerAllocation>> {
        let Some(deployment) = &self.deployment else {
            return Ok(None);
        };
        let (routed, lease) = service::Client::new(&self.state_dir)
            .chat(deployment)
            .await
            .context("routing the selected text provider")?;
        Ok(Some(PlannerAllocation {
            config: (&routed).into(),
            _lease: lease,
        }))
    }
}
