//! Assessing a generated image with the vision model — the evaluator's eyes.
//!
//! One home for the two things an image assessment needs: encoding an image
//! file for an OpenAI vision request (downscaled to a fixed maximum edge), and
//! starting the managed `llama.cpp` server a local `vision` provider names.
//! the plan driver's
//! `expect_vision` step, and any future assessment surface all go through
//! [`VisionSession`], so neither is reimplemented per caller.
//!
//! The capability itself lives in the shared `agentkit-ai` crate
//! ([`agentkit_ai::deployments::chat::Client::complete_with_images`]); this module
//! only adapts storyfella's config and images to it.

use std::path::{Path, PathBuf};

use agentkit_ai::deployments::chat::{self, ChatImage};
use agentkit_ai::deployments::ChatDeployment;
use agentkit_ai::native::service;
use thiserror::Error;

use crate::errors::{CodedError, ErrorCode};

/// The largest edge an image is downscaled to before it is sent. The endpoint
/// rejects oversized payloads (two full 1024x1536 PNGs reliably come back
/// empty), so this is a correctness bound, not an optimization.
pub const MAX_DIMENSION: u32 = 1024;

/// How the evaluator is framed, so every assessment asks the same way.
pub const SYSTEM_PROMPT: &str = "You are a rigorous image evaluator for a visual-storytelling \
     application. Judge what you actually see in the attached images: composition, identity \
     consistency, localized edits, artifacts, distortion, and quality. Answer the user's \
     question directly.";

/// Default answer budget. A reasoning vision model can exhaust a small budget
/// while thinking, so this is deliberately generous.
pub const DEFAULT_MAX_TOKENS: u32 = 2048;

#[derive(Debug, Error)]
pub enum VisionError {
    #[error("could not read image {path}: {source}")]
    Image {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("no `vision` provider is configured in clipforge.toml")]
    NotConfigured,
    #[error("could not start the vision server: {0}")]
    Server(String),
    #[error("the vision model did not answer: {0}")]
    Chat(String),
}

impl CodedError for VisionError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Image { .. } => ErrorCode::LlmVisionImage,
            Self::NotConfigured => ErrorCode::AppLlmUnavailable,
            Self::Server(_) => ErrorCode::LlmServer,
            Self::Chat(_) => ErrorCode::LlmRequest,
        }
    }
}

/// Reads `path` and encodes it as a [`ChatImage`], downscaling so its largest
/// edge is at most [`MAX_DIMENSION`]. A file that cannot be decoded as an image
/// is passed through unchanged (the endpoint decides what to make of it).
pub fn image_for_chat(path: &Path) -> Result<ChatImage, VisionError> {
    let bytes = std::fs::read(path).map_err(|source| VisionError::Image {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(ChatImage::png(downscale_png(bytes)))
}

fn downscale_png(bytes: Vec<u8>) -> Vec<u8> {
    let Ok(image) = image::load_from_memory(&bytes) else {
        return bytes;
    };
    use image::GenericImageView as _;
    let (width, height) = image.dimensions();
    let scale = (width.max(height) as f32 / MAX_DIMENSION as f32).max(1.0);
    if scale <= 1.0 {
        return bytes;
    }
    let resized = image.resize(
        ((width as f32 / scale).round() as u32).max(1),
        ((height as f32 / scale).round() as u32).max(1),
        image::imageops::FilterType::Lanczos3,
    );
    let mut buffer = std::io::Cursor::new(Vec::new());
    match resized.write_to(&mut buffer, image::ImageFormat::Png) {
        Ok(()) => buffer.into_inner(),
        Err(_) => bytes,
    }
}

/// Whether `answer` satisfies a substring gate: it must contain `expect`
/// (case-insensitively) and must not contain `expect_not`. The `Err` carries
/// why it failed, for the surface to report.
pub fn check(answer: &str, expect: Option<&str>, expect_not: Option<&str>) -> Result<(), String> {
    let lowered = answer.to_lowercase();
    if let Some(expect) = expect {
        if !lowered.contains(&expect.to_lowercase()) {
            return Err(format!("the answer does not contain {expect:?}"));
        }
    }
    if let Some(expect_not) = expect_not {
        if lowered.contains(&expect_not.to_lowercase()) {
            return Err(format!("the answer contains {expect_not:?}"));
        }
    }
    Ok(())
}

/// A vision call target using the shared managed-server lifecycle.
pub struct VisionSession {
    deployment: ChatDeployment,
    service: service::Client,
}

impl VisionSession {
    pub fn new(deployment: ChatDeployment, state_dir: &Path) -> Self {
        Self {
            deployment,
            service: service::Client::new(state_dir),
        }
    }

    /// The model id requests go to.
    pub fn model(&self) -> &str {
        &self.deployment.model
    }

    /// Asks `question` about `images` (in order: image 1, image 2, …) and
    /// returns the model's answer.
    pub async fn assess(
        &self,
        images: Vec<ChatImage>,
        question: &str,
        max_tokens: u32,
    ) -> Result<String, VisionError> {
        let (deployment, _lease) = self
            .service
            .chat(&self.deployment)
            .await
            .map_err(|error| VisionError::Server(error.to_string()))?;
        chat::Client::new(deployment)
            .complete_with_images(Some(SYSTEM_PROMPT), question, images, Some(max_tokens))
            .await
            .map_err(|error| VisionError::Chat(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_expectation_gate_is_case_insensitive() {
        assert!(check("Yes, the hair is auburn.", Some("yes"), None).is_ok());
        assert!(check("No, the hair is dark.", Some("yes"), None).is_err());
        assert!(check("No, dark hair remains.", None, Some("auburn")).is_ok());
        assert!(check("Yes, auburn.", None, Some("auburn")).is_err());
    }

    #[test]
    fn a_missing_file_is_reported_with_its_path() {
        let error = image_for_chat(Path::new("/nonexistent/missing.png")).unwrap_err();
        assert!(matches!(error, VisionError::Image { .. }));
        assert_eq!(error.code(), ErrorCode::LlmVisionImage);
    }

    #[test]
    fn an_undecodable_file_is_passed_through() {
        // Not a valid image: the bytes survive unchanged for the endpoint to
        // judge, so a caller never fails only because we could not decode.
        let bytes = downscale_png(vec![0, 1, 2, 3]);
        assert_eq!(bytes, vec![0, 1, 2, 3]);
    }
}
