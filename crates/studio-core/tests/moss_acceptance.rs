//! Opt-in real MOSS provisioning, generation and resident-server reuse acceptance.
use agentkit_ai::{
    deployments::{Capability, Providers},
    images::{Client, Request},
    native::{
        control::{self, Managed},
        sd, ModelStore,
    },
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
mod support;
use support::ServerCleanup;

fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/moss-integration")
}

#[tokio::test]
#[ignore = "downloads and checksum-verifies approximately 12 GB of MOSS model artifacts"]
async fn provision_moss_model() -> Result {
    let providers = Providers::load(&directory().join("providers.toml"))?;
    let deployment = providers.image_deployment(Capability::T2i)?;
    assert_eq!(deployment.architecture, sd::moss_image_sft().id);
    let store = ModelStore::new(deployment.model_root());
    let downloads = deployment
        .artifact_plan()
        .into_iter()
        .filter(|entry| entry.needs_download())
        .map(|entry| {
            let artifact = entry.artifact;
            let store = store.clone();
            async move {
                let mut previous = u64::MAX;
                store
                    .ensure_all(&[artifact], |progress| {
                        let percent = progress
                            .downloaded_bytes
                            .saturating_mul(100)
                            .checked_div(progress.total_bytes)
                            .unwrap_or(0);
                        if previous != percent {
                            eprintln!(
                                "{}: {percent}% ({}/{})",
                                progress.filename, progress.downloaded_bytes, progress.total_bytes
                            );
                            previous = percent;
                        }
                    })
                    .await
            }
        });
    // Let independent transfers finish even if another connection fails. A rerun
    // resumes partial artifacts and verifies already completed files.
    let results = futures_util::future::join_all(downloads).await;
    for result in results {
        result?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires provisioned MOSS weights and the MOSS-capable test backend"]
async fn moss_generation_and_server_reuse() -> Result {
    let directory = directory();
    let providers = Providers::load(&directory.join("providers.toml"))?;
    let deployment = providers.image_deployment(Capability::T2i)?;
    assert_eq!(deployment.architecture, "moss-image-sft");
    assert_eq!(deployment.host, "127.0.0.1");
    assert_eq!(deployment.port, 17864);
    let state = directory.join("servers");
    let managed = Managed::Image(deployment.clone());
    // Preserve any unrelated server already occupying the acceptance port.
    assert!(deployment
        .runtime()
        .client(&deployment.host, deployment.port)
        .capabilities()
        .await
        .is_err());
    let _cleanup = ServerCleanup {
        state: state.clone(),
        port: deployment.port,
    };
    let started = Instant::now();
    let outcome: Result = async {
        let pid = control::load(&managed, &state).await?;
        assert!(pid.is_some());
        let startup = started.elapsed();
        assert_eq!(control::load(&managed, &state).await?, None);
        let mut request = Request::new("A red ceramic cup on a wooden table, simple studio photograph", 512, 512);
        request.steps = Some(50);
        request.seed = Some(42);
        request.timeout = Some(Duration::from_secs(1800));
        let started = Instant::now();
        let output = Client::native(deployment.clone()).generate(request).await?;
        let elapsed = started.elapsed();
        let path = directory.join("moss-create.png");
        std::fs::write(&path, &output.bytes)?;
        let reopened = std::fs::read(&path)?;
        assert_eq!(reopened, output.bytes);
        let decoded = image::load_from_memory(&reopened)?;
        assert_eq!((decoded.width(), decoded.height()), (512, 512));
        let result = format!("MOSS SFT Q8_0, Qwen3.5-9B Q4_K_M, 512×512, 50 steps, seed 42\nStartup: {startup:?}\nGeneration including encode/decode: {elapsed:?}\nJob: {:?}\n", output.job_id);
        eprintln!("{result}");
        std::fs::write(directory.join("RESULTS.txt"), result)?;
        Ok(())
    }.await;
    let cleanup = control::stop(&state, deployment.port);
    outcome?;
    cleanup?;
    Ok(())
}
