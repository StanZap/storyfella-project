use std::{fmt::Display, future::Future, time::Duration};

use crate::{app::AppConfig, llm::LmStudioClient};

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
    LmStudio,
    VisionRuntime,
    ImageGeneration,
    Segmentation,
}

impl ServiceId {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LmStudio => "LM Studio",
            Self::VisionRuntime => "Vision runtime",
            Self::ImageGeneration => "Image generation (Krea 2)",
            Self::Segmentation => "Segmentation (SAM 2 + DINO)",
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
    let vision = check_vision_runtime(runtime).await;
    let (lm_studio, generation, segmentation) = tokio::join!(
        check_lm_studio(config),
        check_generation(runtime, &vision),
        check_segmentation(runtime, &vision),
    );
    vec![lm_studio, vision, generation, segmentation]
}

async fn check_lm_studio(config: &AppConfig) -> ServiceHealth {
    let mut lm_config = config.lm_studio.clone();
    lm_config.timeout = Duration::from_secs(CHECK_TIMEOUT_SECONDS);
    let client = match LmStudioClient::new(lm_config) {
        Ok(client) => client,
        Err(error) => {
            return ServiceHealth::new(
                ServiceId::LmStudio,
                HealthStatus::Offline,
                format!("client error: {error}"),
            );
        }
    };
    match with_timeout(client.list_models(), CHECK_TIMEOUT_SECONDS + 5).await {
        Ok(models) => {
            let configured = &config.lm_studio.model;
            if models.data.iter().any(|model| &model.id == configured) {
                ServiceHealth::new(
                    ServiceId::LmStudio,
                    HealthStatus::Online,
                    format!(
                        "reachable · {} model(s) · '{configured}' loaded",
                        models.data.len()
                    ),
                )
            } else {
                ServiceHealth::new(
                    ServiceId::LmStudio,
                    HealthStatus::Degraded,
                    format!("reachable · model '{configured}' is not loaded"),
                )
            }
        }
        Err(error) => ServiceHealth::new(
            ServiceId::LmStudio,
            HealthStatus::Offline,
            format!("unreachable: {}", truncate(&error)),
        ),
    }
}

async fn check_vision_runtime(runtime: &CreativeRuntime) -> ServiceHealth {
    let process_running = runtime.python_runtime().is_running();
    match with_timeout(runtime.vision_client().health(), CHECK_TIMEOUT_SECONDS).await {
        Ok(_) => ServiceHealth::new(
            ServiceId::VisionRuntime,
            HealthStatus::Online,
            "API ready · 127.0.0.1:8765".to_owned(),
        ),
        Err(error) if process_running => ServiceHealth::new(
            ServiceId::VisionRuntime,
            HealthStatus::Offline,
            format!("API unhealthy: {}", truncate(&error)),
        ),
        Err(_) => ServiceHealth::new(
            ServiceId::VisionRuntime,
            HealthStatus::Idle,
            "not started · starts on demand".to_owned(),
        ),
    }
}

async fn check_generation(runtime: &CreativeRuntime, vision: &ServiceHealth) -> ServiceHealth {
    let generation = runtime.generation_runtime();
    if !generation.executable().is_file() {
        return ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Offline,
            format!(
                "sd-server binary not found at {}",
                generation.executable().display()
            ),
        );
    }
    if !generation.has_profile_artifacts(runtime.profile()) {
        return ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Degraded,
            "model artifacts missing · run `cargo run --bin model_setup -- --accept-krea-license`"
                .to_owned(),
        );
    }
    let process_running = generation.is_running();
    match vision.status {
        HealthStatus::Idle if !process_running => {
            return ServiceHealth::new(
                ServiceId::ImageGeneration,
                HealthStatus::Idle,
                "follows the vision runtime".to_owned(),
            );
        }
        HealthStatus::Idle => {
            return ServiceHealth::new(
                ServiceId::ImageGeneration,
                HealthStatus::Degraded,
                "native server is resident but the vision runtime is not running".to_owned(),
            );
        }
        HealthStatus::Offline => {
            return ServiceHealth::new(
                ServiceId::ImageGeneration,
                HealthStatus::Offline,
                "vision runtime unavailable".to_owned(),
            );
        }
        HealthStatus::Online | HealthStatus::Degraded => {}
    }
    match with_timeout(
        runtime.vision_client().generation_capabilities(),
        CHECK_TIMEOUT_SECONDS,
    )
    .await
    {
        Ok(capabilities) if capabilities.status == "ready" => ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Online,
            format!(
                "{} ready · {} resident",
                runtime.profile().profile_id(),
                capabilities.model.as_deref().unwrap_or("model")
            ),
        ),
        Ok(capabilities) => ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Degraded,
            capabilities
                .error
                .unwrap_or_else(|| "backend not ready".to_owned()),
        ),
        Err(error) if process_running => ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Offline,
            format!(
                "backend unreachable through the vision runtime: {}",
                truncate(&error)
            ),
        ),
        Err(_) => ServiceHealth::new(
            ServiceId::ImageGeneration,
            HealthStatus::Idle,
            "not started · starts on demand".to_owned(),
        ),
    }
}

async fn check_segmentation(runtime: &CreativeRuntime, vision: &ServiceHealth) -> ServiceHealth {
    match vision.status {
        HealthStatus::Online => {
            match with_timeout(runtime.vision_client().capabilities(), CHECK_TIMEOUT_SECONDS).await
            {
                Ok(capabilities) if capabilities.torch_available => ServiceHealth::new(
                    ServiceId::Segmentation,
                    HealthStatus::Online,
                    format!(
                        "SAM 2.1 + Grounding DINO ready · {} device",
                        capabilities.recommended_device
                    ),
                ),
                Ok(_) => ServiceHealth::new(
                    ServiceId::Segmentation,
                    HealthStatus::Degraded,
                    "torch extras not installed · run `uv sync --project python --extra segmentation`"
                        .to_owned(),
                ),
                Err(error) => ServiceHealth::new(
                    ServiceId::Segmentation,
                    HealthStatus::Degraded,
                    format!("device discovery failed: {}", truncate(&error)),
                ),
            }
        }
        HealthStatus::Idle => ServiceHealth::new(
            ServiceId::Segmentation,
            HealthStatus::Idle,
            "follows the vision runtime".to_owned(),
        ),
        _ => ServiceHealth::new(
            ServiceId::Segmentation,
            HealthStatus::Offline,
            "vision runtime unavailable".to_owned(),
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
        assert_eq!(ServiceId::LmStudio.label(), "LM Studio");
        assert_eq!(
            ServiceId::ImageGeneration.label(),
            "Image generation (Krea 2)"
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
        let entry = ServiceHealth::new(ServiceId::VisionRuntime, HealthStatus::Online, "ok");
        assert_eq!(entry.id, ServiceId::VisionRuntime);
        assert_eq!(entry.status, HealthStatus::Online);
        assert_eq!(entry.detail, "ok");
    }
}
