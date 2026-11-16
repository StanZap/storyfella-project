//! Opt-in real-provider acceptance. All configuration and outputs stay in the checkout.
use agentkit_ai::{
    deployments::{Capability, Providers},
    images::{Client, Error, Request},
    native::{control::Managed, service},
};
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

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
#[ignore = "requires a real configured native image server; see ../storyfella-canvas/docs/atelier.md"]
async fn native_generation_reuse_abort_and_saved_output() -> Result {
    let providers = providers()?;
    let deployment = providers.image_deployment(Capability::T2i)?;
    let state = tempfile::tempdir_in(workspace().join(".svs-e2e/native-acceptance"))?;
    let service = service::Client::new(state.path());
    let managed = Managed::Image(deployment.clone());
    service.warm(&managed).await?;
    let pid = service.status().await?.resident[0].pid;
    let client = Client::native(deployment.clone(), state.path());
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
        job_id: Some(id), ..
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
    let mut recovery = request();
    recovery.steps = Some(2);
    let output = client.generate(recovery).await?;
    save_and_reopen("native-after-abort.png", &output.bytes)?;
    assert_ne!(service.status().await?.resident[0].pid, pid);
    service.shutdown().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "loads real Qwen and Krea weights on an isolated port; see ../storyfella-canvas/docs/atelier.md"]
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
    let state = tempfile::tempdir_in(workspace().join(".svs-e2e/native-acceptance"))?;
    let client = service::Client::new(state.path());
    client
        .set_policy(service::Policy {
            max_models: 1,
            ..service::Policy::default()
        })
        .await?;
    let outcome: Result = async {
        let mut previous_pid = None;
        for (index, deployment) in [qwen.clone(), krea, qwen.clone()].into_iter().enumerate() {
            let managed = Managed::Image(deployment.clone());
            let started = Instant::now();
            client.warm(&managed).await?;
            let resident = client.status().await?.resident;
            assert_eq!(resident.len(), 1);
            let pid = resident[0].pid;
            assert_ne!(previous_pid, Some(pid));
            client.warm(&managed).await?;
            assert_eq!(client.status().await?.resident[0].pid, pid);
            previous_pid = Some(pid);
            let mut smoke = request();
            smoke.steps = Some(if deployment.architecture == "krea2-turbo" {
                8
            } else {
                2
            });
            let output = Client::native(deployment.clone(), state.path())
                .generate(smoke)
                .await?;
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
    let cleanup = client.shutdown().await;
    outcome?;
    cleanup?;
    Ok(())
}
