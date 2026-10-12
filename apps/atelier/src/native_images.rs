use crate::providers::Settings;
use anyhow::{ensure, Context, Result};
use clipforge_inference::{
    config,
    control::{self, Managed},
    images::{Client, Output, Request},
    Capability, ImageDeployment, ProviderKind, Providers,
};
use std::sync::Mutex;

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
    let mut candidates = providers.list().filter(|(_, p)| {
        p.kind == ProviderKind::Sdcpp
            && p.architecture.as_deref() == Some(architecture)
            && p.capabilities.contains(&capability)
    });
    let (id, _) = candidates
        .next()
        .with_context(|| format!("Configure a {architecture} provider for {capability}"))?;
    ensure!(
        candidates.next().is_none(),
        "Configure only one {architecture} provider for {capability}"
    );
    Ok(providers.image_deployment_for(id, capability)?)
}

/// The LoRA adapters the selected local provider registers, named as the server
/// expects (see `sd::prepare_lora_links`). Empty when no local provider is
/// selected or its directory is missing.
pub fn available_loras(settings: &Settings) -> Vec<String> {
    let Ok(providers) = config::user_or_built_in() else {
        return Vec::new();
    };
    let Ok(deployment) = deployment(settings, &providers, false) else {
        return Vec::new();
    };
    clipforge_inference::sd::available_loras(&deployment.lora_dir)
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
}

/// Probes the machine's shared `sd-server` for the selected local provider.
/// Read-only: nothing is started or stopped.
pub fn server_status(settings: &Settings) -> ServerStatus {
    if settings.provider.native_architecture().is_none() {
        return ServerStatus::NotLocal;
    }
    let Ok(providers) = config::user_or_built_in() else {
        return ServerStatus::Stopped;
    };
    let Ok(deployment) = deployment(settings, &providers, false) else {
        return ServerStatus::Stopped;
    };
    let Some(state_dir) = config::default_state_dir() else {
        return ServerStatus::Stopped;
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return ServerStatus::Stopped;
    };
    runtime.block_on(async {
        if !matches!(
            control::status(&Managed::Image(deployment.clone()), &state_dir).await,
            control::ServerState::Up { .. }
        ) {
            return ServerStatus::Stopped;
        }
        let model = clipforge_inference::sd::Client::new(format!(
            "http://{}:{}",
            deployment.host, deployment.port
        ))
        .capabilities()
        .await
        .ok()
        .map(|capabilities| capabilities.model.name);
        ServerStatus::Running { model }
    })
}

struct CachedClient {
    deployment: ImageDeployment,
    client: Client,
}

#[derive(Default)]
pub struct NativeImages {
    client: Mutex<Option<CachedClient>>,
}
impl NativeImages {
    pub fn generate(&self, settings: &Settings, request: Request) -> Result<Output> {
        ensure!(
            settings.provider.supports_edit() || request.source.is_none(),
            "Krea 2 Turbo supports generation only"
        );
        ensure!(
            request.references.is_empty() && request.mask.is_none(),
            "Local models accept one source image and no mask"
        );
        let providers = config::user_or_built_in()?;
        let editing = request.source.is_some();
        let selected = deployment(settings, &providers, editing)?;
        let state_dir = config::default_state_dir().context("No platform data directory")?;
        let mut cached = self
            .client
            .lock()
            .map_err(|_| anyhow::anyhow!("Local image worker stopped; restart Atelier"))?;
        if cached.as_ref().is_none_or(|c| c.deployment != selected) {
            // Drop the previous client before loading another model on the GPU.
            *cached = None;
            *cached = Some(CachedClient {
                deployment: selected.clone(),
                client: Client::native(selected.clone()),
            });
        }
        let client = &cached
            .as_ref()
            .context("Missing local image client")?
            .client;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            // Ensure the machine's shared `sd-server` — started detached, owned
            // through its marker, reused if another front end already runs it —
            // before generating; the client then finds it resident.
            control::start(&Managed::Image(selected), &state_dir)
                .await
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            client.generate(request).await.map_err(Into::into)
        })
    }
}
