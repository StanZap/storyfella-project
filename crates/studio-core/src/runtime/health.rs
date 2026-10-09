use std::{fmt::Display, future::Future, time::Duration};

use clipforge_inference::sd::{model_matches, qwen_image_2_1};

use crate::{
    config::AppConfig,
    llm::{LlmClient, LlmConfig},
};

use super::CreativeRuntime;

const CHECK_TIMEOUT_SECONDS: u64 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthStatus {
    /// Ready and serving (or ready to serve when needed).
    Online,
    /// Reachable, but with a gap that will prevent full function.
    Degraded,
    /// Deliberately not running; the app starts it on demand.
    Idle,
    /// Unreachable or unhealthy when it should be serving.
    Offline,
}

impl HealthStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Degraded => "Degraded",
            Self::Idle => "Idle",
            Self::Offline => "Offline",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceId {
    /// The OpenAI-compatible text/planner endpoint (llama.cpp, LM Studio, …).
    Llm,
    /// The OpenAI-compatible vision endpoint (`svs eval`).
    Vision,
    /// The native stable-diffusion.cpp generation server.
    ImageGeneration,
}

impl ServiceId {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Llm => "LLM endpoint",
            Self::Vision => "Vision endpoint",
            Self::ImageGeneration => "Image generation (Qwen-Image-2.1)",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceHealth {
    pub id: ServiceId,
    pub status: HealthStatus,
    pub detail: String,
    pub recovery: Option<ServiceRecovery>,
}

/// A probe's actionable recovery, independent of its human-readable detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceRecovery {
    ConfigureImageFiles,
    ReplaceImageServer,
}

impl ServiceHealth {
    fn new(id: ServiceId, status: HealthStatus, detail: impl Into<String>) -> Self {
        Self {
            id,
            status,
            detail: detail.into(),
            recovery: None,
        }
    }

    fn with_recovery(mut self, recovery: ServiceRecovery) -> Self {
        self.recovery = Some(recovery);
        self
    }
}

/// Probes every service the application depends on. The checks are read-only:
/// nothing is started, stopped, or downloaded as a side effect.
pub async fn check_all(config: &AppConfig, runtime: &CreativeRuntime) -> Vec<ServiceHealth> {
    let (llm, vision, generation) = tokio::join!(
        check_llm(config),
        check_vision(config),
        check_generation(config, runtime),
    );
    vec![llm, vision, generation]
}

async fn check_llm(config: &AppConfig) -> ServiceHealth {
    match &config.text {
        Some(deployment) => {
            check_chat(
                ServiceId::Llm,
                LlmConfig::from(deployment),
                &deployment.model,
                deployment.server.is_some(),
            )
            .await
        }
        None => unconfigured(ServiceId::Llm, "text"),
    }
}

async fn check_vision(config: &AppConfig) -> ServiceHealth {
    match &config.vision {
        Some(deployment) => {
            check_chat(
                ServiceId::Vision,
                LlmConfig::from(deployment),
                &deployment.model,
                deployment.server.is_some(),
            )
            .await
        }
        None => unconfigured(ServiceId::Vision, "vision"),
    }
}

/// A capability no provider serves: a soft, expected state the app degrades
/// around, not a failure.
fn unconfigured(id: ServiceId, capability: &str) -> ServiceHealth {
    ServiceHealth::new(
        id,
        HealthStatus::Idle,
        format!("not configured · no provider serves `{capability}`"),
    )
}

/// Probes one OpenAI-compatible chat endpoint, reporting whether its
/// configured model is among the ones it serves. `managed` marks a provider
/// whose `llama-server` the app starts on demand: an unstarted one is `Idle`,
/// not a failure, so it reads the same as the image engine before a generation.
async fn check_chat(
    id: ServiceId,
    mut lm_config: LlmConfig,
    configured: &str,
    managed: bool,
) -> ServiceHealth {
    lm_config.timeout = Duration::from_secs(CHECK_TIMEOUT_SECONDS);
    let client = match LlmClient::new(lm_config) {
        Ok(client) => client,
        Err(error) => {
            return ServiceHealth::new(id, HealthStatus::Offline, format!("client error: {error}"));
        }
    };
    match with_timeout(client.list_models(), CHECK_TIMEOUT_SECONDS + 5).await {
        Ok(models) => {
            if models.data.iter().any(|model| model.id == configured) {
                ServiceHealth::new(
                    id,
                    HealthStatus::Online,
                    format!(
                        "reachable · {} model(s) · '{configured}' loaded",
                        models.data.len()
                    ),
                )
            } else {
                ServiceHealth::new(
                    id,
                    HealthStatus::Degraded,
                    format!("reachable · model '{configured}' is not loaded"),
                )
            }
        }
        Err(_) if managed => ServiceHealth::new(
            id,
            HealthStatus::Idle,
            "not started · starts on demand".to_owned(),
        ),
        Err(error) => ServiceHealth::new(
            id,
            HealthStatus::Offline,
            format!("unreachable: {}", truncate(&error)),
        ),
    }
}

async fn check_generation(config: &AppConfig, runtime: &CreativeRuntime) -> ServiceHealth {
    let executable = &config.image.executable;
    if !executable.is_file() {
        return ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Offline,
            format!("sd-server binary not found at {}", executable.display()),
        )
        .with_recovery(ServiceRecovery::ConfigureImageFiles);
    }
    if !runtime.has_artifacts() {
        return ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Degraded,
            "Model files are missing. Open Settings → Services → Model files to prepare them."
                .to_owned(),
        )
        .with_recovery(ServiceRecovery::ConfigureImageFiles);
    }
    let process_running = runtime.is_running();
    match with_timeout(runtime.client().capabilities(), CHECK_TIMEOUT_SECONDS).await {
        // Reachable but serving another model: a server this app did not start
        // holds the port. Flag it so the UI offers a take-over.
        Ok(capabilities) if !model_matches(&capabilities.model, &runtime.expected_model()) => {
            ServiceHealth::new(
                ServiceId::ImageGeneration,
                HealthStatus::Degraded,
                format!(
                    "{} is resident on the port — this app did not start it · Take over to replace it",
                    capabilities.model.name
                ),
            ).with_recovery(ServiceRecovery::ReplaceImageServer)
        }
        Ok(capabilities) => ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Online,
            format!(
                "{} ready · {} resident",
                qwen_image_2_1().id,
                capabilities.model.name
            ),
        ),
        Err(error) if process_running => ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Offline,
            format!("backend unreachable: {}", truncate(&error)),
        ),
        Err(_) => ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Idle,
            "not started · starts on demand".to_owned(),
        ),
    }
}

async fn with_timeout<T, E>(
    future: impl Future<Output = Result<T, E>>,
    seconds: u64,
) -> Result<T, String>
where
    E: Display,
{
    match tokio::time::timeout(Duration::from_secs(seconds), future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("timed out after {seconds}s")),
    }
}

fn truncate(text: &str) -> &str {
    let limit = 140;
    match text.char_indices().nth(limit) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::{truncate, HealthStatus, ServiceHealth, ServiceId};

    #[tokio::test]
    async fn missing_image_files_offer_setup_before_any_server_action() {
        let directory = std::env::temp_dir().join(format!("svs-health-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut config =
            crate::config::AppConfig::load_from(directory.join("absent.toml")).unwrap();
        config.image.model_dir = directory.join("models");
        config.image.executable = directory.join("sd-server");
        config.image.lora_dir = directory.join("loras");
        let runtime = super::CreativeRuntime::new(&config);
        let missing_executable = super::check_generation(&config, &runtime).await;
        assert_eq!(missing_executable.status, HealthStatus::Offline);
        assert_eq!(
            missing_executable.recovery,
            Some(super::ServiceRecovery::ConfigureImageFiles)
        );
        std::fs::write(&config.image.executable, b"test file; never executed").unwrap();
        let missing_models = super::check_generation(&config, &runtime).await;
        assert_eq!(missing_models.status, HealthStatus::Degraded);
        assert_eq!(
            missing_models.recovery,
            Some(super::ServiceRecovery::ConfigureImageFiles)
        );
        assert!(!runtime.is_running());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn service_ids_have_display_labels() {
        assert_eq!(ServiceId::Llm.label(), "LLM endpoint");
        assert_eq!(ServiceId::Vision.label(), "Vision endpoint");
        assert_eq!(
            ServiceId::ImageGeneration.label(),
            "Image generation (Qwen-Image-2.1)"
        );
    }

    #[test]
    fn statuses_have_display_labels() {
        assert_eq!(HealthStatus::Online.label(), "Online");
        assert_eq!(HealthStatus::Idle.label(), "Idle");
    }

    #[test]
    fn long_errors_are_truncated_for_the_settings_surface() {
        let long = "x".repeat(400);
        assert_eq!(truncate(&long).len(), 140);

        let short = "short error";
        assert_eq!(truncate(short), short);
    }

    #[test]
    fn health_entries_carry_their_service_and_detail() {
        let entry = ServiceHealth::new(ServiceId::ImageGeneration, HealthStatus::Online, "ok");
        assert_eq!(entry.id, ServiceId::ImageGeneration);
        assert_eq!(entry.status, HealthStatus::Online);
        assert_eq!(entry.detail, "ok");
    }
}
