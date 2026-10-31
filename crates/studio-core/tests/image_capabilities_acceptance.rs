//! Real-server native RGBA and ordered-reference acceptance through public Agentkit APIs.
use agentkit_ai::{
    deployments::{Capability, Providers},
    images::{AlphaInput, Client, Request},
    native::sd,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn save_alpha(name: &str, bytes: &[u8]) -> Result {
    let directory = workspace().join(".svs-e2e/image-capabilities");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(name);
    std::fs::write(&path, bytes)?;
    let reopened = std::fs::read(&path)?;
    assert_eq!(reopened, bytes);
    let decoded = image::load_from_memory(&reopened)?;
    assert!(decoded.color().has_alpha(), "output must encode alpha");
    let rgba = decoded.to_rgba8();
    assert_eq!(rgba.dimensions(), (512, 512));
    let transparent = rgba.pixels().filter(|pixel| pixel[3] < 16).count();
    let opaque = rgba.pixels().filter(|pixel| pixel[3] > 239).count();
    let partial = rgba
        .pixels()
        .filter(|pixel| (16..=239).contains(&pixel[3]))
        .count();
    eprintln!("{name}: transparent={transparent}, opaque={opaque}, partial={partial}");
    let pixels = rgba.pixels().len();
    assert!(
        transparent > pixels / 10,
        "at least 10% transparent background"
    );
    assert!(opaque > pixels / 100, "at least 1% opaque subject");
    Ok(())
}

#[tokio::test]
#[ignore = "runs real Qwen RGBA generation/editing; uses native acceptance config"]
async fn native_qwen_rgba_generation_and_reference_editing() -> Result {
    let providers =
        Providers::load(&workspace().join(".svs-e2e/native-acceptance/providers.toml"))?;
    let deployment = providers.image_deployment(Capability::T2i)?;
    assert_eq!(deployment.architecture, "qwen-image-2.1");
    let wire = deployment
        .runtime()
        .client(&deployment.host, deployment.port);
    wire.capabilities().await?;
    // Check the underlying native protocol independently of the shared image
    // client, which the three-reference scenario exercises with alpha flags.
    let mut request = sd::GenerateRequest::for_profile(
        deployment.profile(),
        "This is an RGBA image with transparency. A single red ceramic cup, isolated product photograph, centered, no table, no shadow. The image has alpha channel and the background is transparent.",
        512, 512,
    );
    request.steps = 20;
    request.seed = Some(42);
    let started = Instant::now();
    let submitted = wire.generate(&request).await?;
    eprintln!("Submitted RGBA generation: {}", submitted.id);
    let created = wire.wait(&submitted.id, Duration::from_secs(1200)).await?;
    save_alpha("qwen-alpha-create.png", &created)?;
    eprintln!(
        "RGBA generation: {:?}; job {}",
        started.elapsed(),
        submitted.id
    );

    request.prompt = "This is an RGBA image with transparency. Change the red ceramic cup in image 1 to blue, retaining its shape and composition. The image has alpha channel and the background is transparent.".into();
    let started = Instant::now();
    let submitted = wire.generate_with_references(&request, &[&created]).await?;
    eprintln!("Submitted RGBA reference edit: {}", submitted.id);
    let edited = wire.wait(&submitted.id, Duration::from_secs(1200)).await?;
    save_alpha("qwen-alpha-edit.png", &edited)?;
    eprintln!(
        "RGBA reference editing: {:?}; job {}",
        started.elapsed(),
        submitted.id
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires the RGBA probe outputs and a resident Qwen server; run sequentially"]
async fn native_qwen_three_ordered_rgba_inputs() -> Result {
    let providers =
        Providers::load(&workspace().join(".svs-e2e/native-acceptance/providers.toml"))?;
    let deployment = providers.image_deployment(Capability::T2i)?;
    assert_eq!(deployment.architecture, "qwen-image-2.1");
    let client = Client::native(deployment);
    assert_eq!(client.capabilities().max_input_images, 3);
    assert_eq!(client.capabilities().alpha_input, AlphaInput::Preserved);
    assert!(client.capabilities().alpha_output);
    let directory = workspace().join(".svs-e2e/image-capabilities");
    let mut request = Request::new(
        "This is an RGBA image with transparency. Create a blue cup, using image 1 for the silhouette, image 2 for the blue color and image 3 for the composition. The image has alpha channel and the background is transparent.",
        512, 512,
    );
    request.source = Some(std::fs::read(directory.join("qwen-alpha-create.png"))?);
    let edited = std::fs::read(directory.join("qwen-alpha-edit.png"))?;
    let mut composition = image::load_from_memory(&edited)?.to_rgba8();
    image::imageops::flip_horizontal_in_place(&mut composition);
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(composition).write_to(&mut encoded, image::ImageFormat::Png)?;
    request.references = vec![edited, encoded.into_inner()];
    request.preserve_input_alpha = true;
    request.require_alpha_output = true;
    request.steps = Some(8);
    request.seed = Some(42);
    request.timeout = Some(Duration::from_secs(1200));
    let last_progress = std::sync::Mutex::new(String::new());
    request.progress = Some(Box::new(move |message| {
        let mut last = last_progress.lock().unwrap();
        if *last != message {
            eprintln!("{message}");
            *last = message;
        }
    }));
    let started = Instant::now();
    let output = client.generate(request).await?;
    save_alpha("qwen-three-references.png", &output.bytes)?;
    eprintln!(
        "Three ordered RGBA inputs: {:?}; job {:?}",
        started.elapsed(),
        output.job_id
    );
    Ok(())
}
