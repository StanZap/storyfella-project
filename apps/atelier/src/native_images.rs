use crate::providers::Settings;
use anyhow::{ensure, Context, Result};
use clipforge_inference::{
    images::{Client, Output, Request},
    Capability, ImageDeployment, ProviderKind, Providers,
};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

#[cfg(test)]
pub(crate) fn deployment(
    settings: &Settings,
    config_path: &Path,
    editing: bool,
) -> Result<ImageDeployment> {
    let configuration =
        std::fs::read_to_string(config_path).context("Could not read local providers file")?;
    deployment_from_text(settings, config_path, &configuration, editing)
}

fn deployment_from_text(
    settings: &Settings,
    config_path: &Path,
    configuration: &str,
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
    let registry = Providers::from_toml(
        configuration,
        config_path.parent().unwrap_or(Path::new(".")),
    )
    .context("Could not load local providers file")?;
    let mut candidates = registry.list().filter(|(_, p)| {
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
    Ok(registry.image_deployment_for(id, capability)?)
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
    pub fn generate(
        &self,
        settings: &Settings,
        keys_path: &Path,
        request: Request,
    ) -> Result<Output> {
        ensure!(
            !settings.native_config.trim().is_empty(),
            "Set the Local providers file in Settings"
        );
        ensure!(
            settings.provider.supports_edit() || request.source.is_none(),
            "Krea 2 Turbo supports generation only"
        );
        ensure!(
            request.references.is_empty() && request.mask.is_none(),
            "Local models accept one source image and no mask"
        );
        let path = PathBuf::from(&settings.native_config);
        let path = if path.is_absolute() {
            path
        } else {
            keys_path
                .parent()
                .context("Missing app data directory")?
                .join(path)
        };
        let configuration =
            std::fs::read_to_string(&path).context("Could not read local providers file")?;
        let editing = request.source.is_some();
        let selected = deployment_from_text(settings, &path, &configuration, editing)?;
        let mut cached = self
            .client
            .lock()
            .map_err(|_| anyhow::anyhow!("Local image worker stopped; restart Atelier"))?;
        if cached.as_ref().is_none_or(|c| c.deployment != selected) {
            // Drop the previous managed server before loading another model on the GPU.
            *cached = None;
            *cached = Some(CachedClient {
                deployment: selected.clone(),
                client: Client::native(selected),
            });
        }
        let client = &cached
            .as_ref()
            .context("Missing local image client")?
            .client;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Ok(runtime.block_on(client.generate(request))?)
    }
}
