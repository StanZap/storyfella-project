use std::time::Duration;

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
