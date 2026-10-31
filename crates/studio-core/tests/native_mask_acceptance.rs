//! Controlled real-server verification of native Qwen's latent mask mechanism.
use agentkit_ai::{
    deployments::{Capability, Providers},
    images::{Client, MaskSupport, Request},
    native::sd,
};
use image::{GrayImage, Luma, RgbaImage};
use std::{
    io::Cursor,
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn png(image: GrayImage) -> Result<Vec<u8>> {
    let mut buffer = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image).write_to(&mut buffer, image::ImageFormat::Png)?;
    Ok(buffer.into_inner())
}

fn rgb_error(left: &RgbaImage, right: &RgbaImage, columns: std::ops::Range<u32>) -> f64 {
    assert_eq!(left.dimensions(), right.dimensions());
    let mut sum = 0_u64;
    let mut count = 0_u64;
    for y in 0..left.height() {
        for x in columns.clone() {
            for channel in 0..3 {
                sum += u64::from(left[(x, y)][channel].abs_diff(right[(x, y)][channel]));
                count += 1;
            }
        }
    }
    sum as f64 / count as f64
}

#[tokio::test]
#[ignore = "runs three native Qwen mask probes; requires the native acceptance source PNG"]
async fn native_qwen_mask_controls_and_protected_pixel_drift() -> Result {
    let root = workspace();
    let directory = root.join(".svs-e2e/native-mask-acceptance");
    std::fs::create_dir_all(&directory)?;
    let providers = Providers::load(&root.join(".svs-e2e/native-acceptance/providers.toml"))?;
    let deployment = providers.image_deployment(Capability::T2i)?;
    assert_eq!(deployment.architecture, "qwen-image-2.1");
    let wire = deployment
        .runtime()
        .client(&deployment.host, deployment.port);
    wire.capabilities().await?;
    let source = std::fs::read(root.join(".svs-e2e/native-acceptance/native-create.png"))?;
    let original = image::load_from_memory(&source)?.to_rgba8();
    assert_eq!(original.dimensions(), (512, 512));
    std::fs::write(directory.join("source.png"), &source)?;
    let mut request = sd::GenerateRequest::for_profile(
        deployment.profile(),
        "Replace the red cup with a bright blue sphere on the wooden table, studio photograph",
        512,
        512,
    );
    request.steps = 2;
    request.seed = Some(42);
    let mut outputs = Vec::new();
    for (name, mask) in [
        ("black", GrayImage::from_pixel(512, 512, Luma([0]))),
        ("white", GrayImage::from_pixel(512, 512, Luma([255]))),
        (
            "half",
            GrayImage::from_fn(512, 512, |x, _| Luma([if x < 256 { 255 } else { 0 }])),
        ),
    ] {
        let mask = png(mask)?;
        std::fs::write(directory.join(format!("mask-{name}.png")), &mask)?;
        let started = Instant::now();
        let job = wire.generate_masked(&request, &source, &mask, &[]).await?;
        eprintln!("Submitted {name} mask: {}", job.id);
        let bytes = wire.wait(&job.id, Duration::from_secs(1200)).await?;
        let path = directory.join(format!("qwen-mask-{name}.png"));
        std::fs::write(&path, &bytes)?;
        let reopened = std::fs::read(path)?;
        assert_eq!(reopened, bytes);
        let output = image::load_from_memory(&reopened)?.to_rgba8();
        assert_eq!(output.dimensions(), original.dimensions());
        eprintln!("{name} mask completed in {:?}", started.elapsed());
        outputs.push(output);
    }
    let black = &outputs[0];
    let white = &outputs[1];
    let half = &outputs[2];
    let full_change = rgb_error(white, black, 0..512);
    let editable_change = rgb_error(half, black, 0..128);
    let protected_change = rgb_error(half, black, 384..512);
    let reconstruction_error = rgb_error(black, &original, 0..512);
    let protected_source_error = rgb_error(half, &original, 384..512);
    let result = format!(
        "RGB MAE on 0–255 scale:\nwhite versus black: {full_change:.6}\nhalf editable versus black: {editable_change:.6}\nhalf far-protected versus black: {protected_change:.6}\nblack versus source (VAE reconstruction): {reconstruction_error:.6}\nhalf far-protected versus source: {protected_source_error:.6}\n"
    );
    eprintln!("{result}");
    std::fs::write(directory.join("RESULTS.txt"), result)?;
    assert!(
        full_change > 5.0,
        "white and black masks must produce different results"
    );
    assert!(editable_change > 5.0, "the editable region must change");
    assert!(
        protected_change < editable_change / 10.0,
        "far-protected latents must stay anchored"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires the controlled mask outputs and RGBA references; run after the controls"]
async fn native_qwen_shared_masked_edit_with_ordered_rgba_references() -> Result {
    let root = workspace();
    let directory = root.join(".svs-e2e/native-mask-acceptance");
    let providers = Providers::load(&root.join(".svs-e2e/native-acceptance/providers.toml"))?;
    let deployment = providers.image_deployment(Capability::T2i)?;
    assert_eq!(deployment.architecture, "qwen-image-2.1");
    let client = Client::native(deployment);
    assert_eq!(client.capabilities().mask, MaskSupport::OptionalLatent);
    let mut request = Request::new("Replace the red cup with a bright blue sphere, using image 2 and image 3 as color references. Keep the table and composition.", 512, 512);
    request.source = Some(std::fs::read(directory.join("source.png"))?);
    request.mask = Some(std::fs::read(directory.join("mask-half.png"))?);
    request.references = ["qwen-alpha-create.png", "qwen-alpha-edit.png"]
        .into_iter()
        .map(|name| std::fs::read(root.join(".svs-e2e/image-capabilities").join(name)))
        .collect::<std::io::Result<Vec<_>>>()?;
    request.preserve_input_alpha = true;
    request.steps = Some(2);
    request.seed = Some(42);
    request.timeout = Some(Duration::from_secs(1200));
    let started = Instant::now();
    let output = client.generate(request).await?;
    let path = directory.join("qwen-mask-shared.png");
    std::fs::write(&path, &output.bytes)?;
    let reopened = std::fs::read(&path)?;
    assert_eq!(reopened, output.bytes);
    let output_image = image::load_from_memory(&reopened)?.to_rgba8();
    let baseline = image::open(directory.join("qwen-mask-black.png"))?.to_rgba8();
    assert_eq!(output_image.dimensions(), baseline.dimensions());
    let editable = rgb_error(&output_image, &baseline, 0..128);
    let protected = rgb_error(&output_image, &baseline, 384..512);
    eprintln!("Shared masked edit with three ordered inputs: {:?}; job {:?}; editable RGB MAE {editable:.6}; far-protected RGB MAE {protected:.6}", started.elapsed(), output.job_id);
    assert!(editable > 5.0);
    assert!(protected < editable / 10.0);
    Ok(())
}
