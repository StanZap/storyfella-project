//! Atelier's storage and configuration boundary over Clipforge's image client.
use crate::{
    document::{self, Reference},
    providers::Request,
};
use agentkit_ai::images::{Client, Request as ImageRequest};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};

pub fn generate(request: Request) -> Result<Reference> {
    let provider = request.settings.provider;
    let editing = request.source.is_some();
    let source_note = request
        .source
        .as_ref()
        .map(|s| format!("\nEdited from: {}", s.name))
        .unwrap_or_default();
    let source = request
        .source
        .as_ref()
        .map(|s| STANDARD.decode(&s.image))
        .transpose()
        .context("Invalid reference image encoding")?;
    let prompt = request.prompt.clone();
    let mut image_request = ImageRequest::new(request.prompt, request.size.0, request.size.1);
    image_request.timeout = Some(request.settings.request_timeout());
    image_request.source = source;
    image_request.references = request
        .references
        .iter()
        .map(|r| STANDARD.decode(&r.image))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    image_request.mask = request.mask.as_ref().map(|mask| mask.png()).transpose()?;
    if provider.native_architecture().is_some() {
        image_request.steps = request.settings.steps;
        image_request.loras = request
            .settings
            .loras
            .iter()
            .map(|lora| agentkit_ai::capabilities::images::LoraSelection {
                path: lora.name.clone(),
                multiplier: lora.multiplier,
            })
            .collect();
    }
    image_request.cancel = request.cancel;
    image_request.progress = request.progress;
    // The GPUI worker owns its runtime; Agentkit remains async and UI-independent.
    let output = if provider.hosted().is_some() {
        let id = request
            .settings
            .provider_id
            .as_deref()
            .context("Choose an image provider configured in Clipforge")?;
        let registry = storyfella_studio_core::ai_config::shared_registry()?;
        request.settings.verify_provider(&registry)?;
        let capability = if editing {
            agentkit_ai::deployments::Capability::Edit
        } else {
            agentkit_ai::deployments::Capability::T2i
        };
        let client = Client::from_registry_for(&registry, id, capability, None)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async { client.generate(image_request).await })?
    } else {
        crate::native_images::generate(&request.settings, image_request)?
    };
    let job = output
        .job_id
        .map(|id| format!("\nJob: {id}"))
        .unwrap_or_default();
    document::reference_from_bytes(
        if editing {
            "Edited artwork"
        } else {
            "Generated artwork"
        }
        .into(),
        &output.bytes,
        format!(
            "{} · {}\n{prompt}{source_note}{job}",
            output.provider, output.model
        ),
    )
}
