//! Atelier owns the captured image request and result attachment. Agentkit
//! executes its asset-iteration step and records the durable run lifecycle.
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use image::RgbaImage;
use storyfella_studio_core::workflows::{
    self,
    framework::{
        DataType, RunError, RunErrorCode, Runner, StepDescriptor, StepExecutionContext,
        StepExecutionRegistry, StepFuture, StepInputs, StepOutputs, StepRegistry, Value, Workflow,
        WorkflowRunStatus,
    },
    RunStore,
};

use crate::{
    document::{self, Reference},
    providers::Request,
};

const IMAGE_STEP: &str = "atelier_image_iteration";
const SAVE_STEP: &str = "atelier_store_iteration";

pub fn generate(
    request: Request,
    directory: &Path,
    provider: fn(Request) -> Result<Reference>,
) -> Result<(Reference, RgbaImage)> {
    std::fs::create_dir_all(directory)?;
    let store = RunStore::open(&directory.join("workflows.sqlite3"))?;
    // No keys, endpoint URLs, encoded images or full provider Settings enter
    // durable state. The captured request remains an ephemeral host binding.
    let context = serde_json::json!({
        "prompt": request.prompt,
        "source": request.source.as_ref().map(|source| crate::library::id(source)),
        "references": request.references.iter().map(|image| crate::library::id(image)).collect::<Vec<_>>(),
        "size": request.size,
        "provider": request.settings.provider.name(),
        "model": if request.source.is_some() { &request.settings.edit_model } else { &request.settings.model },
    });
    let mask = request.mask.clone();
    let source = request.source.clone();
    let prompt = request.prompt.clone();
    let cancel = request.cancel.clone();
    let request = Arc::new(Mutex::new(Some(request)));
    let result = Arc::new(Mutex::new(None));
    let output = result.clone();
    let stored = Arc::new(Mutex::new(None));
    let saved = stored.clone();
    let mut registry = StepRegistry::builtin();
    registry.register(
        StepDescriptor::new(IMAGE_STEP, "Iterate artwork").output("result", DataType::Text),
    );
    registry.register(
        StepDescriptor::new(SAVE_STEP, "Store artwork")
            .input("image", DataType::Text)
            .output("path", DataType::Text),
    );
    let mut graph = Workflow::builder("atelier-asset-iteration");
    let image = graph.add(IMAGE_STEP);
    let save = graph.add(SAVE_STEP);
    let workflow = graph
        .wire(image, "result", save, "image")
        .expose("result", save, "path")
        .build(&registry)
        .map_err(|error| anyhow::anyhow!("Atelier workflow: {error:?}"))?;
    let state = store.create(&workflow, &context)?;
    let run_id = state.identity.run_id.clone();
    let mut executors = StepExecutionRegistry::default();
    executors.register_function(
        IMAGE_STEP,
        move |_: StepInputs, _: StepExecutionContext| -> StepFuture<'static> {
            let request = request.lock().unwrap().take();
            let output = output.clone();
            Box::pin(async move {
                let request = request.ok_or_else(|| {
                    RunError::new(
                        RunErrorCode::BadInput,
                        "An image request cannot be submitted twice",
                    )
                })?;
                let reference = tokio::task::spawn_blocking(move || provider(request))
                    .await
                    .map_err(|error| RunError::new(RunErrorCode::BadInput, error.to_string()))?
                    .map_err(|error| RunError::new(RunErrorCode::ModelCall, error.to_string()))?;
                let identity = crate::library::id(&reference);
                *output.lock().unwrap() = Some(reference);
                Ok(StepOutputs::from([(
                    "result".into(),
                    Value::Text(identity),
                )]))
            })
        },
    );
    let directory = directory.join("generated");
    executors.register_function(
        SAVE_STEP,
        move |_: StepInputs, context: StepExecutionContext| -> StepFuture<'static> {
            let reference = result.lock().unwrap().take();
            let saved = saved.clone();
            let source = source.clone();
            let mask = mask.clone();
            let prompt = prompt.clone();
            let directory = directory.clone();
            Box::pin(async move {
                let stored = tokio::task::spawn_blocking(move || -> Result<_> {
                    let mut reference = reference.context("Image step returned no artwork")?;
                    if let (Some(mask), Some(source)) = (&mask, &source) {
                        reference = mask.composite(source, &reference)?;
                    }
                    reference.parent = source.as_ref().map(|source| crate::library::id(source));
                    reference.prompt = prompt;
                    if let Some(source) = &source {
                        reference.name = format!("Edit · {}", source.name);
                    }
                    let decoded = document::decode_reference(&reference)?;
                    let image = document::thumbnail(decoded.clone(), 800).to_rgba8();
                    let stem = format!("reference-{}", context.run_id);
                    let mut png = std::io::Cursor::new(Vec::new());
                    decoded.write_to(&mut png, image::ImageFormat::Png)?;
                    document::atomic_write(
                        &directory.join(format!("{stem}.png")),
                        &png.into_inner(),
                    )?;
                    let path = directory.join(format!("{stem}.json"));
                    document::atomic_write(&path, &serde_json::to_vec(&reference)?)?;
                    Ok((reference, image, path))
                })
                .await
                .map_err(|error| RunError::new(RunErrorCode::BadInput, error.to_string()))?
                .map_err(|error| RunError::new(RunErrorCode::BadInput, error.to_string()))?;
                let (reference, image, path) = stored;
                *saved.lock().unwrap() = Some((reference, image));
                Ok(StepOutputs::from([(
                    "path".into(),
                    Value::Text(path.to_string_lossy().into_owned()),
                )]))
            })
        },
    );
    let runner = Runner::new(workflow, None, Vec::new())
        .with_registry(registry)
        .with_step_executors(executors);
    let runtime = tokio::runtime::Runtime::new()?;
    let state = runtime.block_on(workflows::execute(
        runner,
        state,
        &store,
        cancel.clone(),
        |_| {},
    ))?;
    anyhow::ensure!(
        state.status == WorkflowRunStatus::Completed
            && !cancel.load(std::sync::atomic::Ordering::Relaxed),
        "Generation cancelled"
    );
    let reference = stored.lock().unwrap().take();
    reference.with_context(|| format!("Workflow {run_id} completed without an image"))
}
