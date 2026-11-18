use std::{fmt::Display, future::Future, time::Duration};

use agentkit_ai::deployments::chat::Client as ChatClient;
use agentkit_ai::deployments::ChatDeployment;
use agentkit_ai::native::{control::Managed, service};

use crate::config::AppConfig;

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
    /// The OpenAI-compatible vision endpoint.
    Vision,
}

impl ServiceId {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Llm => "LLM endpoint",
            Self::Vision => "Vision endpoint",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceHealth {
    pub id: ServiceId,
    pub status: HealthStatus,
    pub detail: String,
    /// Models reported by the endpoint, or the single configured managed model.
    pub models: Option<Vec<String>>,
}

impl ServiceHealth {
    fn new(id: ServiceId, status: HealthStatus, detail: impl Into<String>) -> Self {
        Self {
            id,
            status,
            detail: detail.into(),
            models: None,
        }
    }
}

/// Probes every service the application depends on. The checks are read-only:
/// nothing is started, stopped, or downloaded as a side effect.
pub async fn check_all(config: &AppConfig) -> Vec<ServiceHealth> {
    let (llm, vision) = tokio::join!(check_llm(config), check_vision(config));
    vec![llm, vision]
}

async fn check_llm(config: &AppConfig) -> ServiceHealth {
    match &config.text {
        Some(deployment) => check_chat(ServiceId::Llm, deployment.clone(), &config.state_dir).await,
        None => unconfigured(ServiceId::Llm, "text"),
    }
}

async fn check_vision(config: &AppConfig) -> ServiceHealth {
    match &config.vision {
        Some(deployment) => {
            check_chat(ServiceId::Vision, deployment.clone(), &config.state_dir).await
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
    mut deployment: ChatDeployment,
    state_dir: &std::path::Path,
) -> ServiceHealth {
    let configured = deployment.model.clone();
    if let Some(server) = deployment.server.as_ref() {
        let mut health = check_native(id, &Managed::Chat(server.clone()), state_dir).await;
        health.models = Some(vec![configured]);
        return health;
    }
    deployment.timeout = Duration::from_secs(CHECK_TIMEOUT_SECONDS);
    let client = ChatClient::new(deployment);
    match with_timeout(client.list_models(), CHECK_TIMEOUT_SECONDS + 5).await {
        Ok(models) => {
            let mut result = if models.iter().any(|model| model == &configured) {
                ServiceHealth::new(
                    id,
                    HealthStatus::Online,
                    format!(
                        "reachable · {} model(s) · '{configured}' loaded",
                        models.len()
                    ),
                )
            } else {
                ServiceHealth::new(
                    id,
                    HealthStatus::Degraded,
                    format!("reachable · model '{configured}' is not loaded"),
                )
            };
            result.models = Some(models);
            result
        }
        Err(error) => ServiceHealth::new(
            id,
            HealthStatus::Offline,
            format!("unreachable: {}", truncate(&error)),
        ),
    }
}

async fn check_native(
    id: ServiceId,
    model: &Managed,
    state_dir: &std::path::Path,
) -> ServiceHealth {
    let status = service::Client::new(state_dir).status().await;
    match (service::model_id(model), status) {
        (Ok(model_id), Ok(status)) => {
            if status.resident.iter().any(|worker| worker.id == model_id) {
                return ServiceHealth::new(
                    id,
                    HealthStatus::Online,
                    format!("{} is resident", model.model()),
                );
            }
            if let Some(failure) = status
                .failures
                .iter()
                .find(|failure| failure.model.id == model_id)
            {
                return ServiceHealth::new(id, HealthStatus::Degraded, truncate(&failure.message));
            }
            ServiceHealth::new(id, HealthStatus::Idle, "Loads on demand")
        }
        (Err(error), _) | (_, Err(error)) => {
            ServiceHealth::new(id, HealthStatus::Offline, truncate(&error.to_string()))
        }
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
    async fn endpoint_check_returns_discovered_models_even_when_selection_is_missing() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).unwrap() > 0);
            let body = r#"{"data":[{"id":"model-a"},{"id":"model-b"}]}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let deployment = agentkit_ai::deployments::ChatDeployment {
            base_url: format!("http://{addr}/v1"),
            model: "missing".into(),
            api_key: None,
            timeout: std::time::Duration::from_secs(5),
            server: None,
        };
        let result = super::check_chat(ServiceId::Llm, deployment, std::path::Path::new(".")).await;
        server.join().unwrap();
        assert_eq!(result.status, HealthStatus::Degraded);
        assert_eq!(result.models.unwrap(), vec!["model-a", "model-b"]);
    }

    #[test]
    fn service_ids_have_display_labels() {
        assert_eq!(ServiceId::Llm.label(), "LLM endpoint");
        assert_eq!(ServiceId::Vision.label(), "Vision endpoint");
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
        let entry = ServiceHealth::new(ServiceId::Vision, HealthStatus::Online, "ok");
        assert_eq!(entry.id, ServiceId::Vision);
        assert_eq!(entry.status, HealthStatus::Online);
        assert_eq!(entry.detail, "ok");
    }
}
