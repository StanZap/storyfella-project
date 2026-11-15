use crate::providers::Settings;
use agentkit_ai::deployments::{Capability, ImageDeployment, Providers};
use agentkit_ai::images::{Client, Output, Request};
use agentkit_ai::native::{control::Managed, service};
use anyhow::{ensure, Context, Result};
use storyfella_studio_core::ai_config as config;

/// The deployment Atelier generates with: the shared registry's `sdcpp` provider
/// whose architecture matches the selected provider, for `editing`'s capability.
/// Neither a server nor a model tree is chosen here — Atelier reads the machine's
/// `clipforge.toml` and shares whatever it names.
pub(crate) fn deployment(
    settings: &Settings,
    providers: &Providers,
    editing: bool,
) -> Result<ImageDeployment> {
    let architecture = settings
        .provider
        .native_architecture()
        .context("Choose a local image provider")?;
    let capability = if editing {
        Capability::Edit
    } else {
        Capability::T2i
    };
    let id = settings
        .provider_id
        .as_deref()
        .context("Choose an image provider configured in Clipforge")?;
    let deployment = providers.image_deployment_for(id, capability)?;
    ensure!(
        deployment.architecture == architecture,
        "The selected provider changed in Clipforge; refresh the provider list"
    );
    Ok(deployment)
}

/// The LoRA adapters the selected local provider registers, named as the server
/// expects (see `sd::prepare_lora_links`). Empty when no local provider is
/// selected or its directory is missing.
pub fn available_loras(settings: &Settings) -> Vec<String> {
    let Ok(providers) = config::shared_registry() else {
        return Vec::new();
    };
    let Ok(deployment) = deployment(settings, &providers, false) else {
        return Vec::new();
    };
    agentkit_ai::native::sd::available_loras(&deployment.lora_dir)
}

/// Whether the machine's shared local image server is answering.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerStatus {
    /// The selected provider is not local, so no server is used.
    NotLocal,
    /// The shared `sd-server` is answering; the resident model when reported.
    Running { model: Option<String> },
    /// Not answering; the app starts it on demand.
    Stopped,
    /// Configuration or probing could not be resolved; readiness is unknown.
    Unavailable,
}

/// Probes the machine's shared `sd-server` for the selected local provider.
/// Read-only: nothing is started or stopped.
pub fn server_status(settings: &Settings) -> ServerStatus {
    if settings.provider.native_architecture().is_none() {
        return ServerStatus::NotLocal;
    }
    let Ok(providers) = config::shared_registry() else {
        return ServerStatus::Unavailable;
    };
    let Ok(deployment) = deployment(settings, &providers, false) else {
        return ServerStatus::Unavailable;
    };
    let Some(state_dir) = storyfella_studio_core::paths::state_dir() else {
        return ServerStatus::Unavailable;
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return ServerStatus::Unavailable;
    };
    runtime.block_on(async {
        let client = service::Client::new(state_dir);
        let Ok(id) = service::model_id(&Managed::Image(deployment)) else {
            return ServerStatus::Unavailable;
        };
        match client.status().await {
            Ok(status) => status
                .resident
                .into_iter()
                .find(|worker| worker.id == id)
                .map(|worker| ServerStatus::Running {
                    model: Some(worker.model),
                })
                .unwrap_or(ServerStatus::Stopped),
            Err(_) => ServerStatus::Unavailable,
        }
    })
}

pub fn generate(settings: &Settings, request: Request) -> Result<Output> {
    ensure!(
        settings.supports_edit() || request.source.is_none(),
        "{} supports generation only",
        settings.provider.name()
    );
    let providers = config::shared_registry()?;
    settings.verify_provider(&providers)?;
    let editing = request.source.is_some();
    let selected = deployment(settings, &providers, editing)?;
    let state_dir =
        storyfella_studio_core::paths::state_dir().context("No platform data directory")?;
    let client = Client::native(selected, state_dir);
    client.validate_request(&request)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async { client.generate(request).await.map_err(Into::into) })
}
