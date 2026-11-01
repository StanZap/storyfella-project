//! Opt-in real-provider acceptance. All configuration and outputs stay in the checkout.
use agentkit_ai::{
    deployments::{Capability, Providers},
    images::{Client, Error, Request},
    native::control,
};
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
mod support;
use support::ServerCleanup;

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn providers() -> Result<Providers> {
    let path = workspace().join(".svs-e2e/native-acceptance/providers.toml");
    Ok(Providers::load(&path)?)
}

fn request() -> Request {
    let mut request = Request::new(
        "A red ceramic cup on a wooden table, simple studio photograph",
        512,
        512,
    );
    request.steps = Some(20);
    request.seed = Some(42);
    request.timeout = Some(Duration::from_secs(1200));
    request
}

fn save_and_reopen(name: &str, bytes: &[u8]) -> Result {
    let path = workspace().join(".svs-e2e/native-acceptance").join(name);
    std::fs::write(&path, bytes)?;
    let reopened = std::fs::read(&path)?;
    assert_eq!(reopened, bytes);
    let image = image::load_from_memory(&reopened)?;
    assert_eq!((image.width(), image.height()), (512, 512));
    eprintln!(
        "Saved and reopened {} ({} bytes)",
        path.display(),
        bytes.len()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires a real configured native image server; see docs/atelier.md"]
async fn native_generation_reuse_abort_and_saved_output() -> Result {
    let providers = providers()?;
    let deployment = providers.image_deployment(Capability::T2i)?;
    let runtime = deployment.runtime();
    // This scenario deliberately reuses a server started by the user or the switching test.
    let wire = runtime.client(&deployment.host, deployment.port);
    let before = wire.capabilities().await?;
    runtime
        .ensure_ready(&deployment.host, deployment.port)
        .await?;
    assert_eq!(runtime.pid(), None, "must reuse the resident server");
    let client = Client::native(deployment.clone());
    let started = Instant::now();
    let output = client.generate(request()).await?;
    eprintln!(
        "Generation completed in {:?}; job {:?}",
        started.elapsed(),
        output.job_id
    );
    save_and_reopen("native-create.png", &output.bytes)?;

    let mut cancelled = request();
    cancelled.steps = Some(2);
    let flag = cancelled.cancel.clone();
    let abort_time = Arc::new(Mutex::new(None));
    let captured_time = abort_time.clone();
    cancelled.progress = Some(Box::new(move |message| {
        eprintln!("{message}");
        if message.starts_with("Image job:") {
            *captured_time.lock().unwrap() = Some(Instant::now());
            flag.store(true, Ordering::Relaxed);
        }
    }));
    let started = Instant::now();
    let error = client.generate(cancelled).await.unwrap_err();
    let abort_latency = abort_time.lock().unwrap().expect("job progress").elapsed();
    assert!(abort_latency < Duration::from_secs(1));
    let Error::Cancelled {
        job_id: Some(id),
        remote_cancelled: false,
    } = error
    else {
        return Err(
            format!("expected local cancellation with a submitted job, got {error}").into(),
        );
    };
    eprintln!(
        "Local abort latency {abort_latency:?}; total request {:?}; submitted job {id}",
        started.elapsed(),
    );
    // There is no verified active-sampling cancellation endpoint. Drain the actual
    // job explicitly so subsequent acceptance does not leave hidden GPU work.
    let drained = wire.wait(&id, Duration::from_secs(1200)).await?;
    assert!(!drained.is_empty());
    assert_eq!(wire.capabilities().await?.model.name, before.model.name);
    eprintln!("Aborted remote job finished; resident server remains available");

    let mut recovery = request();
    recovery.steps = Some(2);
    let output = client.generate(recovery).await?;
    save_and_reopen("native-after-abort.png", &output.bytes)?;
    assert_eq!(runtime.pid(), None);
    Ok(())
}

#[tokio::test]
#[ignore = "loads real Qwen and Krea weights on an isolated port; see docs/atelier.md"]
async fn native_model_switching_and_reuse() -> Result {
    let providers = providers()?;
    let qwen = providers.image_deployment_for("acceptance-qwen", Capability::T2i)?;
    let krea = providers.image_deployment_for("acceptance-krea", Capability::T2i)?;
    assert_eq!(qwen.architecture, "qwen-image-2.1");
    assert_eq!(krea.architecture, "krea2-turbo");
    assert_eq!(qwen.host, "127.0.0.1");
    assert_eq!(qwen.host, krea.host);
    assert_eq!(qwen.port, krea.port);
    assert_ne!(qwen.port, providers.image_deployment(Capability::T2i)?.port);
    let state = workspace().join(".svs-e2e/native-acceptance/servers");
    std::fs::create_dir_all(&state)?;
    // Refuse to replace anything already running on the acceptance port.
    assert!(qwen
        .runtime()
        .client(&qwen.host, qwen.port)
        .capabilities()
        .await
        .is_err());
    let _cleanup = ServerCleanup {
        state: state.clone(),
        port: qwen.port,
    };
    let outcome: Result = async {
        for (index, deployment) in [qwen.clone(), krea, qwen.clone()].into_iter().enumerate() {
            let managed = control::Managed::Image(deployment.clone());
            let started = Instant::now();
            let pid = control::load(&managed, &state).await?;
            assert!(
                pid.is_some(),
                "each model switch must start a new owned process"
            );
            assert_eq!(control::load(&managed, &state).await?, None);
            let mut smoke = request();
            smoke.steps = Some(if deployment.architecture == "krea2-turbo" {
                8
            } else {
                2
            });
            let output = Client::native(deployment.clone()).generate(smoke).await?;
            save_and_reopen(
                &format!("switch-{index}-{}.png", deployment.architecture),
                &output.bytes,
            )?;
            eprintln!(
                "{} loaded, reused and generated in {:?}; pid {pid:?}",
                deployment.architecture,
                started.elapsed()
            );
        }
        Ok(())
    }
    .await;
    let cleanup = control::stop(&state, qwen.port);
    outcome?;
    cleanup?;
    Ok(())
}
