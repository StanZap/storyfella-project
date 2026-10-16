//! Atelier's storage and configuration boundary over Clipforge's image client.
use crate::{
    document::{self, Reference},
    providers::{self, Provider, Request},
};
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clipforge_inference::images::{ApiKey, Client, HostedDeployment, Request as ImageRequest};

pub fn generate(request: Request) -> Result<Reference> {
    let provider = request.settings.provider;
    let editing = request.source.is_some();
    let model = if editing {
        &request.settings.edit_model
    } else {
        &request.settings.model
    };
    let deployment = if let Some(hosted) = provider.hosted() {
        ensure!(!model.trim().is_empty(), "Choose a model in Settings");
        let endpoint = if provider == Provider::Alibaba {
            &request.settings.endpoint
        } else {
            hosted.default_endpoint()
        };
        Some(HostedDeployment::at(hosted, model.trim(), endpoint)?)
    } else {
        None
    };
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
    image_request.source = source;
    image_request.references = request
        .references
        .iter()
        .map(|r| STANDARD.decode(&r.image))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    image_request.mask = request.mask.as_ref().map(|mask| mask.png()).transpose()?;
    image_request.steps = request.settings.steps;
    image_request.loras = request
        .settings
        .loras
        .iter()
        .map(|lora| clipforge_inference::sd::LoraSelection {
            path: lora.name.clone(),
            multiplier: lora.multiplier,
        })
        .collect();
    image_request.cancel = request.cancel;
    image_request.progress = request.progress;
    // The GPUI worker owns its runtime; Clipforge remains async and UI-independent.
    let output = if let Some(deployment) = deployment {
        let key = ApiKey::new(providers::api_key(provider)?)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            Client::hosted(deployment, key)?
                .generate(image_request)
                .await
        })?
    } else {
        request.native.generate(&request.settings, image_request)?
    };
    let job = output
        .job_id
        .map(|id| format!("\nJob: {id}"))
        .unwrap_or_default();
    document::reference_from_bytes(
        if editing {
            "Edited reference"
        } else {
            "Generated reference"
        }
        .into(),
        &output.bytes,
        format!(
            "{} · {}\n{prompt}{source_note}{job}",
            output.provider, output.model
        ),
    )
}
