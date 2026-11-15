//! Opt-in hosted acceptance using existing credentials, without changing user config.
use agentkit_ai::{
    deployments::{Capability, ProviderKind, Providers},
    images::{Client, Error, ImageDimensions, Request},
};
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

// Replicate can limit accounts to six submissions/minute with a burst of one.
const SUBMISSION_INTERVAL: Duration = Duration::from_secs(12);

fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/hosted-acceptance")
}

fn request() -> Request {
    let mut request = Request::new(
        "A red ceramic cup on a wooden table, simple studio photograph",
        512,
        512,
    );
    request.timeout = Some(Duration::from_secs(300));
    request
}

fn save_and_reopen(name: &str, bytes: &[u8]) -> Result {
    let path = directory().join(name);
    std::fs::write(&path, bytes)?;
    let reopened = std::fs::read(&path)?;
    assert_eq!(reopened, bytes);
    let image = image::load_from_memory(&reopened)?;
    // This route advertises aspect ratios, not exact pixel dimensions.
    assert!(image.width() > 0);
    assert_eq!(image.width(), image.height());
    eprintln!(
        "Saved and decoded {} ({} bytes, {}×{})",
        path.display(),
        bytes.len(),
        image.width(),
        image.height()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "submits real billed Replicate requests; see docs/atelier.md"]
async fn hosted_replicate_abort_deadline_and_recovery() -> Result {
    let path = storyfella_studio_core::paths::clipforge_config_path()
        .ok_or("No shared provider configuration path")?;
    let providers = Providers::load(&path)?;
    let id = std::env::var("SF_ACCEPTANCE_PROVIDER").unwrap_or_else(|_| "replicate".into());
    let provider = providers
        .list()
        .find(|(key, _)| *key == id)
        .map(|(_, provider)| provider)
        .ok_or("Acceptance provider is not configured")?;
    assert_eq!(provider.kind, ProviderKind::Replicate);
    let client = Client::from_registry_for(&providers, &id, Capability::T2i, None)?;
    let editor = Client::from_registry_for(&providers, &id, Capability::Edit, None)?;
    assert_eq!(
        client.capabilities().dimensions,
        ImageDimensions::AspectRatios
    );
    assert_eq!(
        editor.capabilities().dimensions,
        ImageDimensions::AspectRatios
    );
    std::fs::create_dir_all(directory())?;
    let mut evidence = vec![format!(
        "Provider: {id}; model: {}",
        provider.model.as_deref().ok_or("No configured model")?
    )];

    // These progress messages are emitted only after the provider returns a job ID.
    let mut aborted = request();
    let cancel = aborted.cancel.clone();
    let abort_time = Arc::new(Mutex::new(None));
    let captured_time = abort_time.clone();
    aborted.progress = Some(Box::new(move |message| {
        eprintln!("{message}");
        if message.contains("on Replicate") {
            let mut time = captured_time.lock().unwrap();
            if time.is_none() {
                *time = Some(Instant::now());
                cancel.store(true, Ordering::Relaxed);
            }
        }
    }));
    let error = client.generate(aborted).await.unwrap_err();
    let latency = abort_time
        .lock()
        .unwrap()
        .ok_or_else(|| format!("No submitted-job progress: {error}"))?
        .elapsed();
    assert!(latency < Duration::from_secs(7), "abort took {latency:?}");
    assert!(
        matches!(
            &error,
            Error::Cancelled {
                job_id: Some(_),
                remote_cancelled: true
            }
        ),
        "expected a remotely accepted abort, got {error}"
    );
    evidence.push(format!("Abort latency: {latency:?}; {error}"));
    eprintln!("{}", evidence.last().unwrap());

    tokio::time::sleep(SUBMISSION_INTERVAL).await;
    // A positive near-zero polling deadline expires after submission, while
    // leaving the submission's own network timeout intact.
    let mut expired = request();
    expired.timeout = Some(Duration::from_millis(1));
    let started = Instant::now();
    let error = client.generate(expired).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(40));
    assert!(
        matches!(
            &error,
            Error::Timeout {
                job_id: Some(_),
                remote_cancelled: true
            }
        ),
        "expected timeout with remotely accepted cancellation, got {error}"
    );
    evidence.push(format!(
        "Deadline request elapsed: {:?}; {error}",
        started.elapsed()
    ));
    eprintln!("{}", evidence.last().unwrap());

    tokio::time::sleep(SUBMISSION_INTERVAL).await;
    let started = Instant::now();
    let output = client.generate(request()).await?;
    save_and_reopen("hosted-create.png", &output.bytes)?;
    evidence.push(format!(
        "Recovery generation: {:?}; job {:?}",
        started.elapsed(),
        output.job_id
    ));

    tokio::time::sleep(SUBMISSION_INTERVAL).await;
    let mut edit = request();
    edit.prompt = "Change the red cup to blue; retain the table and composition".into();
    edit.source = Some(output.bytes);
    let started = Instant::now();
    let output = editor.generate(edit).await?;
    save_and_reopen("hosted-edit.png", &output.bytes)?;
    evidence.push(format!(
        "Reference editing: {:?}; job {:?}",
        started.elapsed(),
        output.job_id
    ));
    for line in &evidence {
        eprintln!("{line}");
    }
    std::fs::write(directory().join("RESULTS.txt"), evidence.join("\n") + "\n")?;
    Ok(())
}
