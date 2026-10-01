use std::{fmt::Display, future::Future, time::Duration};

use clipforge_inference::sd::qwen_image_2_1;

use crate::{config::AppConfig, llm::LlmClient};

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
    /// The OpenAI-compatible LLM endpoint (llama.cpp, LM Studio, …).
    Llm,
    /// The native stable-diffusion.cpp generation server.
    ImageGeneration,
}

impl ServiceId {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Llm => "LLM endpoint",
            Self::ImageGeneration => "Image generation (Qwen-Image-2.1)",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceHealth {
    pub id: ServiceId,
    pub status: HealthStatus,
    pub detail: String,
}

impl ServiceHealth {
    fn new(id: ServiceId, status: HealthStatus, detail: impl Into<String>) -> Self {
        Self {
            id,
            status,
            detail: detail.into(),
        }
    }
}

/// Probes every service the application depends on. The checks are read-only:
/// nothing is started, stopped, or downloaded as a side effect.
pub async fn check_all(config: &AppConfig, runtime: &CreativeRuntime) -> Vec<ServiceHealth> {
    let (llm, generation) = tokio::join!(check_llm(config), check_generation(config, runtime),);
    vec![llm, generation]
}

async fn check_llm(config: &AppConfig) -> ServiceHealth {
    let mut lm_config = config.llm.clone();
    lm_config.timeout = Duration::from_secs(CHECK_TIMEOUT_SECONDS);
    let client = match LlmClient::new(lm_config) {
        Ok(client) => client,
        Err(error) => {
            return ServiceHealth::new(
                ServiceId::Llm,
                HealthStatus::Offline,
                format!("client error: {error}"),
            );
        }
    };
    match with_timeout(client.list_models(), CHECK_TIMEOUT_SECONDS + 5).await {
        Ok(models) => {
            let configured = &config.llm.model;
            if models.data.iter().any(|model| &model.id == configured) {
                ServiceHealth::new(
                    ServiceId::Llm,
                    HealthStatus::Online,
                    format!(
                        "reachable · {} model(s) · '{configured}' loaded",
                        models.data.len()
                    ),
                )
            } else {
                ServiceHealth::new(
                    ServiceId::Llm,
                    HealthStatus::Degraded,
                    format!("reachable · model '{configured}' is not loaded"),
                )
            }
        }
        Err(error) => ServiceHealth::new(
            ServiceId::Llm,
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
        );
    }
    if !runtime.has_artifacts() {
        return ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Degraded,
            "model artifacts missing · run `cargo run --bin model_setup`".to_owned(),
        );
    }
    let process_running = runtime.is_running();
    match with_timeout(runtime.client().capabilities(), CHECK_TIMEOUT_SECONDS).await {
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

    #[test]
    fn service_ids_have_display_labels() {
        assert_eq!(ServiceId::Llm.label(), "LLM endpoint");
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
