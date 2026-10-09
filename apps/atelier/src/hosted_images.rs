//! Hosted image adapters. Provider credentials are never sent to result URLs.
use crate::{
    document::{self, Reference},
    providers::{self, Provider, Request},
};
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{
    io::Cursor,
    time::{Duration, Instant},
};

const IMAGE_LIMIT: u64 = 20 * 1024 * 1024;

fn model(request: &Request) -> Result<&str> {
    let name = if request.source.is_some() {
        &request.settings.edit_model
    } else {
        &request.settings.model
    };
    ensure!(
        !name.trim().is_empty(),
        "Choose a {} model in Settings",
        if request.source.is_some() {
            "editing"
        } else {
            "generation"
        }
    );
    Ok(name.trim())
}

fn source_url(request: &Request) -> Result<Option<String>> {
    request
        .source
        .as_ref()
        .map(|source| {
        let image = document::decode_reference(source)?;
            let mut bytes = Cursor::new(Vec::new());
            // Qwen 3 on fal accepts opaque PNG inputs only. Other adapters retain alpha.
            if request.settings.provider == Provider::Fal
                && model(request)?.starts_with("alibaba/qwen-image-3/")
            {
            let mut opaque = image.to_rgba8();
            for pixel in opaque.pixels_mut() {
                let alpha = u32::from(pixel[3]);
                for channel in &mut pixel.0[..3] {
                    *channel = ((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
                }
                pixel[3] = 255;
            }
            let mut image = image::DynamicImage::ImageRgba8(opaque);
            let minimum = image.width().min(image.height());
            if minimum < 384 {
                let width = image.width() * 384 / minimum;
                let height = image.height() * 384 / minimum;
                ensure!(width <= 2048 && height <= 2048, "This reference is too narrow for Qwen Image 3; use another editing model or a less extreme aspect ratio");
                image = image.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
            }
            image::DynamicImage::ImageRgb8(image.to_rgb8()).write_to(&mut bytes, image::ImageFormat::Png)?;
            } else {
                image.write_to(&mut bytes, image::ImageFormat::Png)?;
            }
            ensure!(
                bytes.get_ref().len() <= 10 * 1024 * 1024,
                "Editing input exceeds the provider's 10 MB limit"
            );
            Ok(format!(
                "data:image/png;base64,{}",
                STANDARD.encode(bytes.into_inner())
            ))
        })
        .transpose()
}

pub(crate) fn payload(request: &Request, source: Option<&str>) -> Result<Value> {
    let name = model(request)?;
    let (width, height) = request.size;
    let size = format!("{width}x{height}");
    let mut body = match request.settings.provider {
        Provider::GptImage => {
            json!({"model":name,"prompt":request.prompt,"size":size,"quality":"medium","output_format":"png","n":1})
        }
        Provider::Alibaba => {
            json!({"model":name,"prompt":request.prompt,"size":size,"n":1,"prompt_extend":false,"watermark":false})
        }
        Provider::Fal => {
            json!({"prompt":request.prompt,"image_size":{"width":width,"height":height},"num_images":1,"output_format":"png"})
        }
        Provider::Replicate => {
            json!({"prompt":request.prompt,"aspect_ratio":if width == height {"1:1"} else if width > height {"3:2"} else {"2:3"}})
        }
        Provider::Comfy => bail!("ComfyUI uses workflow requests"),
    };
    match request.settings.provider {
        Provider::Fal => {
            if name.starts_with("alibaba/qwen-image-3/") {
                body["enable_prompt_expansion"] = json!(false);
            }
            if let Some(source) = source {
                if name.contains("z-image/") {
                    body["image_url"] = json!(source);
                } else {
                    body["image_urls"] = json!([source]);
                }
            }
        }
        Provider::Replicate => {
            if name == "alibaba/qwen-image-3" {
                body["enable_prompt_expansion"] = json!(false);
                body["match_input_image"] = json!(false);
            }
            if let Some(source) = source {
                body["image"] = json!(source);
            }
        }
        Provider::Alibaba => {
            if let Some(source) = source {
                body["image"] = json!([source]);
            }
        }
        Provider::GptImage => {
            if let Some(source) = source {
                body["images"] = json!([{"image_url":source}]);
            }
        }
        Provider::Comfy => unreachable!(),
    }
    Ok(body)
}

fn identifier(value: &str) -> Result<&str> {
    ensure!(
        !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "Invalid provider job ID"
    );
    Ok(value)
}

pub(crate) fn model_path(value: &str) -> Result<&str> {
    ensure!(
        value.split('/').count() >= 2
            && value.split('/').all(|p| !p.is_empty()
                && p != "."
                && p != ".."
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')),
        "Use a provider model ID, not a URL"
    );
    Ok(value)
}

pub(crate) fn alibaba_base(endpoint: &str) -> Result<String> {
    let url = reqwest::Url::parse(endpoint.trim())
        .context("Enter your regional Alibaba OpenAI-compatible base URL in Settings")?;
    let host = url.host_str().unwrap_or_default();
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.port_or_known_default() == Some(443)
            && (host.ends_with(".maas.aliyuncs.com")
                || matches!(
                    host,
                    "dashscope-intl.aliyuncs.com"
                        | "dashscope.aliyuncs.com"
                        | "dashscope-us.aliyuncs.com"
                ))
            && url.path().trim_end_matches('/') == "/compatible-mode/v1",
        "Use your Alibaba regional HTTPS base URL ending in /compatible-mode/v1"
    );
    Ok(endpoint.trim().trim_end_matches('/').into())
}

fn pause(request: &Request) {
    for _ in 0..5 {
        if request.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn progress(request: &Request, message: &str) {
    if let Some(callback) = &request.progress {
        callback(message.into());
    }
}

// Queue URLs returned by providers can point at a parent endpoint rather than the
// submitted subroute. Verify their origin before attaching authorization.
fn queue_url(value: &Value, field: &str, base: &str) -> Result<String> {
    let endpoint = value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("Provider omitted {field}"))?;
    let url = reqwest::Url::parse(endpoint)?;
    let origin = reqwest::Url::parse(base)?;
    ensure!(
        url.origin() == origin.origin()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "Provider returned an unexpected job URL"
    );
    Ok(endpoint.into())
}

pub(crate) fn fal(
    client: &Client,
    request: &Request,
    key: &str,
    base: &str,
    body: Value,
) -> Result<(Value, String)> {
    let name = model_path(model(request)?)?;
    let auth = format!("Key {key}");
    providers::cancelled(&request.cancel)?;
    let queued = providers::response_json(
        client
            .post(format!("{base}/{name}"))
            .header("Authorization", &auth)
            .json(&body)
            .timeout(Duration::from_secs(30))
            .send()?,
    )?;
    let id = identifier(
        queued
            .get("request_id")
            .and_then(Value::as_str)
            .context("fal did not return a request ID")?,
    )?
    .to_string();
    let status_url = queue_url(&queued, "status_url", base)?;
    let response_url = queue_url(&queued, "response_url", base)?;
    let cancel_url = queue_url(&queued, "cancel_url", base)?;
    let started = Instant::now();
    loop {
        if request.cancel.load(std::sync::atomic::Ordering::Relaxed)
            || started.elapsed() >= Duration::from_secs(300)
        {
            let _ = client
                .put(&cancel_url)
                .header("Authorization", &auth)
                .timeout(Duration::from_secs(5))
                .send();
            providers::cancelled(&request.cancel)?;
            bail!("fal timed out; cancellation requested for job {id}");
        }
        let status = providers::response_json(
            client
                .get(&status_url)
                .header("Authorization", &auth)
                .timeout(Duration::from_secs(15))
                .send()?,
        )?;
        match status.get("status").and_then(Value::as_str) {
            Some("COMPLETED") => {
                ensure!(
                    status.get("error").is_none_or(Value::is_null),
                    "fal job {id} failed"
                );
                let result = providers::response_json(
                    client
                        .get(&response_url)
                        .header("Authorization", &auth)
                        .timeout(Duration::from_secs(30))
                        .send()?,
                )?;
                return Ok((result, id));
            }
            Some("IN_QUEUE") => progress(request, "Queued on fal.ai…"),
            Some("IN_PROGRESS") => progress(request, "Creating image on fal.ai…"),
            Some("FAILED" | "CANCELLED") => bail!("fal job {id} failed or was cancelled"),
            _ => bail!("fal returned an unknown job status for {id}"),
        }
        pause(request);
    }
}

pub(crate) fn replicate(
    client: &Client,
    request: &Request,
    key: &str,
    base: &str,
    body: Value,
) -> Result<(Value, String)> {
    let name = model_path(model(request)?)?;
    ensure!(
        name.split('/').count() == 2,
        "Replicate model ID must be owner/model"
    );
    providers::cancelled(&request.cancel)?;
    let mut prediction = providers::response_json(
        client
            .post(format!("{base}/models/{name}/predictions"))
            .bearer_auth(key)
            .header("Cancel-After", "5m")
            .json(&json!({"input":body}))
            .timeout(Duration::from_secs(30))
            .send()?,
    )?;
    let id = identifier(
        prediction
            .get("id")
            .and_then(Value::as_str)
            .context("Replicate did not return a prediction ID")?,
    )?
    .to_string();
    let started = Instant::now();
    loop {
        if request.cancel.load(std::sync::atomic::Ordering::Relaxed)
            || started.elapsed() >= Duration::from_secs(300)
        {
            let _ = client
                .post(format!("{base}/predictions/{id}/cancel"))
                .bearer_auth(key)
                .timeout(Duration::from_secs(5))
                .send();
            providers::cancelled(&request.cancel)?;
            bail!("Replicate timed out; cancellation requested for job {id}");
        }
        match prediction.get("status").and_then(Value::as_str) {
            Some("succeeded") => return Ok((prediction, id)),
            Some("failed") => bail!(
                "Replicate job {id} failed: {}",
                prediction
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("See your provider dashboard")
                    .chars()
                    .take(400)
                    .collect::<String>()
            ),
            Some("canceled") => bail!("Replicate job {id} was cancelled"),
            Some("starting") => progress(request, "Starting image model on Replicate…"),
            Some("processing") => progress(request, "Creating image on Replicate…"),
            _ => bail!("Replicate returned an unknown job status for {id}"),
        }
        pause(request);
        if request.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }
        prediction = providers::response_json(
            client
                .get(format!("{base}/predictions/{id}"))
                .bearer_auth(key)
                .timeout(Duration::from_secs(15))
                .send()?,
        )?;
    }
}

pub(crate) fn image_location(provider: Provider, value: &Value) -> Result<&str> {
    let location = match provider {
        Provider::GptImage | Provider::Alibaba => value
            .pointer("/data/0/b64_json")
            .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            .or_else(|| value.pointer("/data/0/url")),
        Provider::Fal => value.pointer("/images/0/url"),
        Provider::Replicate => value.pointer("/output/0").or_else(|| value.get("output")),
        Provider::Comfy => None,
    };
    location
        .and_then(Value::as_str)
        .context("Provider completed without returning an image")
}

pub(crate) fn download(client: &Client, provider: Provider, value: &Value) -> Result<Vec<u8>> {
    let location = image_location(provider, value)?;
    let encoded = if let Some(data) = location.strip_prefix("data:image/") {
        let (format, bytes) = data.split_once(",").context("Invalid image data URL")?;
        ensure!(
            format.ends_with(";base64"),
            "Expected a base64 image data URL"
        );
        Some(bytes)
    } else if matches!(provider, Provider::GptImage | Provider::Alibaba)
        && value
            .pointer("/data/0/b64_json")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    {
        Some(location)
    } else {
        None
    };
    if let Some(encoded) = encoded {
        ensure!(
            encoded.len() as u64 <= IMAGE_LIMIT * 4 / 3 + 4,
            "Generated image exceeds size limit"
        );
        return STANDARD
            .decode(encoded)
            .context("Invalid generated image encoding");
    }
    let url = reqwest::Url::parse(location).context("Invalid generated image URL")?;
    let loopback_test = cfg!(test)
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    ensure!(
        (url.scheme() == "https" || loopback_test)
            && url.username().is_empty()
            && url.password().is_none(),
        "Expected an HTTPS image URL without credentials"
    );
    // Deliberately unauthenticated: images are often on a separate CDN.
    let response = client
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()?
        .error_for_status()?;
    providers::read_response(response, IMAGE_LIMIT)
}

pub fn generate(client: &Client, request: Request) -> Result<Reference> {
    let provider = request.settings.provider;
    model(&request)?;
    let base = match provider {
        Provider::Alibaba => alibaba_base(&request.settings.endpoint)?,
        _ => provider.defaults().endpoint,
    };
    progress(&request, "Accessing your provider credential…");
    let key = providers::api_key(provider)?;
    generate_with_key(client, request, &key, &base)
}

fn generate_with_key(
    client: &Client,
    request: Request,
    key: &str,
    base: &str,
) -> Result<Reference> {
    let provider = request.settings.provider;
    let name = model(&request)?.to_string();
    let source = source_url(&request)?;
    let body = payload(&request, source.as_deref())?;
    providers::cancelled(&request.cancel)?;
    let (result, job) = match provider {
        Provider::Fal => fal(client, &request, key, base, body)?,
        Provider::Replicate => replicate(client, &request, key, base, body)?,
        Provider::Alibaba | Provider::GptImage => {
            progress(&request, "Waiting for image provider…");
            let route = if provider == Provider::GptImage && source.is_some() {
                "edits"
            } else {
                "generations"
            };
            let result = providers::response_json(
                client
                    .post(format!("{base}/images/{route}"))
                    .bearer_auth(key)
                    .json(&body)
                    .send()?,
            )?;
            (result, String::new())
        }
        Provider::Comfy => unreachable!(),
    };
    providers::cancelled(&request.cancel)?;
    progress(&request, "Downloading image…");
    let bytes = download(client, provider, &result)?;
    providers::cancelled(&request.cancel)?;
    let source_note = request
        .source
        .as_ref()
        .map(|s| format!("\nEdited from: {}", s.name))
        .unwrap_or_default();
    document::reference_from_bytes(
        if request.source.is_some() {
            "Edited reference"
        } else {
            "Generated reference"
        }
        .into(),
        &bytes,
        format!(
            "{} · {name}\n{}{}{}",
            provider.name(),
            request.prompt,
            source_note,
            if job.is_empty() {
                String::new()
            } else {
                format!("\nJob: {job}")
            }
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Settings;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };

    fn png() -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            400,
            400,
            image::Rgba([20, 40, 80, 128]),
        ))
        .write_to(&mut buffer, image::ImageFormat::Png)
        .unwrap();
        buffer.into_inner()
    }
    fn request(provider: Provider, editing: bool) -> Request {
        let profile = provider.defaults();
        Request {
            settings: Settings {
                provider,
                model: profile.model,
                edit_model: profile.edit_model,
                ..Settings::default()
            },
            prompt: "Keep the character; change the lighting".into(),
            size: (1536, 1024),
            source: editing.then(|| {
                Arc::new(
                    document::reference_from_bytes("Original".into(), &png(), "Imported".into())
                        .unwrap(),
                )
            }),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: None,
        }
    }
    fn client() -> Client {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }
    fn read_request(stream: &mut TcpStream) -> (String, String, Value) {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        let header_end = loop {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|s| s.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while bytes.len() < header_end + length {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
        }
        let path = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        let body = if length == 0 {
            Value::Null
        } else {
            serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
        };
        (path, headers.to_ascii_lowercase(), body)
    }
    fn accept(listener: &TcpListener) -> TcpStream {
        listener.set_nonblocking(true).unwrap();
        let started = Instant::now();
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(5),
                        "Mock provider did not receive the expected request"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        }
    }
    fn respond(stream: &mut TcpStream, code: u16, body: &[u8]) {
        write!(
            stream,
            "HTTP/1.1 {code} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    }

    #[test]
    fn hosted_create_and_edit_round_trips() {
        for provider in [
            Provider::Fal,
            Provider::Replicate,
            Provider::Alibaba,
            Provider::GptImage,
        ] {
            for editing in [false, true] {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let base = format!("http://{}", listener.local_addr().unwrap());
                let url = base.clone();
                let request = request(provider, editing);
                let source = request.source.clone();
                let model_name = model(&request).unwrap().to_string();
                let server = std::thread::spawn(move || {
                    let mut stream = accept(&listener);
                    let (path, headers, body) = read_request(&mut stream);
                    let expected_auth = if provider == Provider::Fal {
                        "authorization: key test-key"
                    } else {
                        "authorization: bearer test-key"
                    };
                    assert!(headers.contains(expected_auth));
                    let input = if provider == Provider::Replicate {
                        &body["input"]
                    } else {
                        &body
                    };
                    assert_eq!(input["prompt"], "Keep the character; change the lighting");
                    let image = match provider {
                        Provider::Fal => &input["image_urls"][0],
                        Provider::Replicate => &input["image"],
                        Provider::Alibaba => &input["image"][0],
                        Provider::GptImage => &input["images"][0]["image_url"],
                        _ => unreachable!(),
                    };
                    assert_eq!(image.is_string(), editing);
                    if editing {
                        assert!(image
                            .as_str()
                            .unwrap()
                            .starts_with("data:image/png;base64,"));
                    }
                    let output_url = format!("{url}/output.png");
                    let response = match provider {
                        Provider::Fal => {
                            assert_eq!(path, format!("POST /{model_name}"));
                            json!({"request_id":"job-1","status_url":format!("{url}/jobs/job-1/status"),"response_url":format!("{url}/jobs/job-1"),"cancel_url":format!("{url}/jobs/job-1/cancel")})
                        }
                        Provider::Replicate => {
                            assert_eq!(path, format!("POST /models/{model_name}/predictions"));
                            json!({"id":"job-1","status":"starting"})
                        }
                        Provider::Alibaba | Provider::GptImage => {
                            assert_eq!(
                                path,
                                format!(
                                    "POST /images/{}",
                                    if provider == Provider::GptImage && editing {
                                        "edits"
                                    } else {
                                        "generations"
                                    }
                                )
                            );
                            assert_eq!(input["model"], model_name);
                            if provider == Provider::GptImage {
                                json!({"data":[{"b64_json":STANDARD.encode(png())}]})
                            } else {
                                json!({"data":[{"url":output_url}]})
                            }
                        }
                        _ => unreachable!(),
                    };
                    respond(&mut stream, 200, &serde_json::to_vec(&response).unwrap());
                    if provider == Provider::Fal {
                        let mut stream = accept(&listener);
                        let (path, _, _) = read_request(&mut stream);
                        assert_eq!(path, "GET /jobs/job-1/status");
                        respond(&mut stream, 200, br#"{"status":"COMPLETED"}"#);
                        let mut stream = accept(&listener);
                        let (path, _, _) = read_request(&mut stream);
                        assert_eq!(path, "GET /jobs/job-1");
                        respond(
                            &mut stream,
                            200,
                            &serde_json::to_vec(&json!({"images":[{"url":output_url}]})).unwrap(),
                        );
                    } else if provider == Provider::Replicate {
                        let mut stream = accept(&listener);
                        let (path, _, _) = read_request(&mut stream);
                        assert_eq!(path, "GET /predictions/job-1");
                        respond(
                            &mut stream,
                            200,
                            &serde_json::to_vec(
                                &json!({"id":"job-1","status":"succeeded","output":[output_url]}),
                            )
                            .unwrap(),
                        );
                    }
                    if provider != Provider::GptImage {
                        let mut stream = accept(&listener);
                        let (path, headers, _) = read_request(&mut stream);
                        assert_eq!(path, "GET /output.png");
                        assert!(
                            !headers.contains("authorization:"),
                            "Credential leaked to image CDN"
                        );
                        respond(&mut stream, 200, &png());
                    }
                });
                let reference = generate_with_key(&client(), request, "test-key", &base).unwrap();
                assert_eq!(document::decode_reference(&reference).unwrap().width(), 400);
                assert!(reference.provenance.contains(provider.name()));
                assert_eq!(
                    reference.provenance.contains("Edited from: Original"),
                    editing
                );
                if let Some(source) = source {
                    assert_eq!(source.name, "Original");
                    assert_eq!(source.provenance, "Imported");
                }
                server.join().unwrap();
            }
        }
    }

    #[test]
    fn cancellation_targets_only_submitted_hosted_job() {
        for provider in [Provider::Fal, Provider::Replicate] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let url = base.clone();
            let request = request(provider, false);
            let cancel = request.cancel.clone();
            let server = std::thread::spawn(move || {
                let mut stream = accept(&listener);
                read_request(&mut stream);
                cancel.store(true, Ordering::Relaxed);
                let response = if provider == Provider::Fal {
                    json!({"request_id":"my-job","status_url":format!("{url}/jobs/my-job/status"),"response_url":format!("{url}/jobs/my-job"),"cancel_url":format!("{url}/jobs/my-job/cancel")})
                } else {
                    json!({"id":"my-job","status":"starting"})
                };
                respond(&mut stream, 200, &serde_json::to_vec(&response).unwrap());
                let mut stream = accept(&listener);
                let (path, _, _) = read_request(&mut stream);
                assert_eq!(
                    path,
                    if provider == Provider::Fal {
                        "PUT /jobs/my-job/cancel"
                    } else {
                        "POST /predictions/my-job/cancel"
                    }
                );
                respond(&mut stream, 200, b"{}");
            });
            let error = generate_with_key(&client(), request, "test-key", &base).unwrap_err();
            assert!(error.to_string().contains("cancelled"));
            server.join().unwrap();
        }
    }

    #[test]
    fn provider_profiles_and_legacy_settings_round_trip() {
        let mut settings: Settings = serde_json::from_value(
            json!({"provider":"Comfy","endpoint":"http://localhost:8188","model":"legacy"}),
        )
        .unwrap();
        settings.select_provider(Provider::Alibaba);
        settings.model = "qwen-image-2.1-pro".into();
        settings.edit_model = "qwen-image-2.1-pro".into();
        settings.endpoint =
            "https://workspace.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1".into();
        settings.select_provider(Provider::Fal);
        let mut settings: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        settings.select_provider(Provider::Alibaba);
        assert_eq!(settings.model, "qwen-image-2.1-pro");
        assert_eq!(settings.edit_model, "qwen-image-2.1-pro");
        assert!(settings.endpoint.contains("workspace"));
        settings.select_provider(Provider::Comfy);
        assert_eq!(settings.endpoint, "http://localhost:8188");
        assert_eq!(settings.model, "legacy");
        assert!(!serde_json::to_string(&settings)
            .unwrap()
            .contains("api_key"));
    }

    #[test]
    fn rejects_unexpected_api_origins_and_invalid_model_paths() {
        assert!(alibaba_base(
            "https://workspace.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/"
        )
        .is_ok());
        for value in [
            "http://dashscope-intl.aliyuncs.com/compatible-mode/v1",
            "https://evil.example/compatible-mode/v1",
            "https://dashscope-intl.aliyuncs.com.evil.example/compatible-mode/v1",
            "https://key@dashscope-intl.aliyuncs.com/compatible-mode/v1",
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1?key=secret",
        ] {
            assert!(alibaba_base(value).is_err());
        }
        assert!(queue_url(
            &json!({"status_url":"https://evil.example/status"}),
            "status_url",
            "https://queue.fal.run"
        )
        .is_err());
        for value in [
            "owner/../model",
            "owner//model",
            "https://example.com/model",
            "owner/model?query=1",
        ] {
            assert!(model_path(value).is_err());
        }
    }

    #[test]
    fn handles_inline_images_missing_output_and_provider_errors() {
        let bytes = png();
        assert_eq!(download(&client(), Provider::Fal, &json!({"images":[{"url":format!("data:image/png;base64,{}", STANDARD.encode(&bytes))}]})).unwrap(), bytes);
        assert!(image_location(Provider::Replicate, &json!({"output":null})).is_err());
        assert_eq!(
            image_location(
                Provider::Alibaba,
                &json!({"data":[{"b64_json":null,"url":"https://cdn.example/image.png"}]})
            )
            .unwrap(),
            "https://cdn.example/image.png"
        );
        assert!(download(
            &client(),
            Provider::Fal,
            &json!({"images":[{"url":"file:///tmp/secret"}]})
        )
        .is_err());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut stream = accept(&listener);
            read_request(&mut stream);
            respond(
                &mut stream,
                401,
                br#"{"message":"Invalid account credential"}"#,
            );
        });
        let error = generate_with_key(
            &client(),
            request(Provider::Alibaba, false),
            "test-key",
            &url,
        )
        .unwrap_err();
        assert!(error.to_string().contains("Invalid account credential"));
        server.join().unwrap();
    }

    #[test]
    fn qwen3_input_flattens_alpha_but_other_adapters_retain_it() {
        let qwen = request(Provider::Fal, true);
        let source = source_url(&qwen).unwrap().unwrap();
        let decoded =
            image::load_from_memory(&STANDARD.decode(source.split_once(',').unwrap().1).unwrap())
                .unwrap()
                .to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0)[3], 255);
        assert!(
            decoded.get_pixel(0, 0)[0] > 100,
            "Transparent input should be composited onto white"
        );
        let alibaba = request(Provider::Alibaba, true);
        let source = source_url(&alibaba).unwrap().unwrap();
        let decoded =
            image::load_from_memory(&STANDARD.decode(source.split_once(',').unwrap().1).unwrap())
                .unwrap()
                .to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0)[3], 128);
    }
}
