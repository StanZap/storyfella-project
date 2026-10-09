//! Atelier's storage/credential boundary over Clipforge's shared image client.
use crate::{
    document::{self, Reference},
    providers::{self, Provider, Request},
};
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clipforge_inference::images::{ApiKey, Client, HostedDeployment, Request as ImageRequest};

pub fn generate(request: Request) -> Result<Reference> {
    let provider = request.settings.provider;
    let hosted = provider
        .hosted()
        .context("Choose a hosted image provider")?;
    let editing = request.source.is_some();
    let model = if editing {
        &request.settings.edit_model
    } else {
        &request.settings.model
    };
    ensure!(
        !model.trim().is_empty(),
        "Choose a {} model in Settings",
        if editing { "editing" } else { "generation" }
    );
    let endpoint = if provider == Provider::Alibaba {
        &request.settings.endpoint
    } else {
        hosted.default_endpoint()
    };
    let deployment = HostedDeployment::at(hosted, model.trim(), endpoint)?;
    if let Some(progress) = &request.progress {
        progress("Accessing your provider credential…".into());
    }
    let key = ApiKey::new(providers::api_key(provider)?)?;
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
    image_request.cancel = request.cancel;
    image_request.progress = request.progress;
    // The GPUI worker owns its runtime; Clipforge remains async and UI-independent.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let output = runtime.block_on(async {
        Client::hosted(deployment, key)?
            .generate(image_request)
            .await
    })?;
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
