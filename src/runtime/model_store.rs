use std::{
    fs,
    path::{Path, PathBuf},
};

use futures_util::StreamExt;
use reqwest::{header::RANGE, Client, StatusCode};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::generation_runtime::{krea_profile, KreaQuantization, ModelArtifact};

#[derive(Clone, Debug)]
pub struct ModelStore {
    root: PathBuf,
}

#[derive(Debug, Error)]
pub enum ModelStoreError {
    #[error("failed to create model directory {path}: {source}")]
    CreateDirectory {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to inspect model artifact {path}: {source}")]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("model download request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("model download returned HTTP {status} for {url}")]
    DownloadStatus { status: StatusCode, url: String },
    #[error("failed to write model artifact {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("model artifact {path} has size {actual}, expected {expected}")]
    Size {
        path: PathBuf,
        actual: u64,
        expected: u64,
    },
    #[error("model artifact {path} failed SHA-256 verification")]
    Checksum { path: PathBuf },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadProgress {
    pub filename: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

impl ModelStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn prepare(&self) -> Result<(), ModelStoreError> {
        fs::create_dir_all(&self.root).map_err(|source| ModelStoreError::CreateDirectory {
            path: self.root.display().to_string(),
            source,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn ensure_krea_profile<F>(
        &self,
        quantization: KreaQuantization,
        mut progress: F,
    ) -> Result<(), ModelStoreError>
    where
        F: FnMut(DownloadProgress),
    {
        self.prepare()?;
        let profile = krea_profile(quantization);
        let model_root = self.root.join("krea-2");
        fs::create_dir_all(&model_root).map_err(|source| ModelStoreError::CreateDirectory {
            path: model_root.display().to_string(),
            source,
        })?;
        let client = Client::new();
        for artifact in [profile.diffusion, profile.text_encoder, profile.vae] {
            self.ensure_artifact(&client, &model_root, artifact, &mut progress)
                .await?;
        }
        Ok(())
    }

    async fn ensure_artifact<F>(
        &self,
        client: &Client,
        model_root: &Path,
        artifact: ModelArtifact,
        progress: &mut F,
    ) -> Result<(), ModelStoreError>
    where
        F: FnMut(DownloadProgress),
    {
        let destination = model_root.join(artifact.filename);
        if destination.is_file() {
            self.verify_artifact(&destination, artifact).await?;
            progress(DownloadProgress {
                filename: artifact.filename.to_owned(),
                downloaded_bytes: artifact.size_bytes,
                total_bytes: artifact.size_bytes,
            });
            return Ok(());
        }

        let partial = destination.with_file_name(format!("{}.part", artifact.filename));
        let existing = match tokio::fs::metadata(&partial).await {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(source) => {
                return Err(ModelStoreError::Inspect {
                    path: partial,
                    source,
                });
            }
        };
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            artifact.repository, artifact.revision, artifact.remote_path
        );
        let mut request = client.get(&url);
        if existing > 0 {
            request = request.header(RANGE, format!("bytes={existing}-"));
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(ModelStoreError::DownloadStatus {
                status: response.status(),
                url,
            });
        }
        let append = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
        let mut downloaded = if append { existing } else { 0 };
        let mut output = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&partial)
            .await
            .map_err(|source| ModelStoreError::Write {
                path: partial.clone(),
                source,
            })?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            output
                .write_all(&chunk)
                .await
                .map_err(|source| ModelStoreError::Write {
                    path: partial.clone(),
                    source,
                })?;
            downloaded += chunk.len() as u64;
            progress(DownloadProgress {
                filename: artifact.filename.to_owned(),
                downloaded_bytes: downloaded,
                total_bytes: artifact.size_bytes,
            });
        }
        output
            .flush()
            .await
            .map_err(|source| ModelStoreError::Write {
                path: partial.clone(),
                source,
            })?;
        drop(output);
        self.verify_artifact(&partial, artifact).await?;
        tokio::fs::rename(&partial, &destination)
            .await
            .map_err(|source| ModelStoreError::Write {
                path: destination,
                source,
            })?;
        Ok(())
    }

    async fn verify_artifact(
        &self,
        path: &Path,
        artifact: ModelArtifact,
    ) -> Result<(), ModelStoreError> {
        let actual = tokio::fs::metadata(path)
            .await
            .map_err(|source| ModelStoreError::Inspect {
                path: path.to_path_buf(),
                source,
            })?
            .len();
        if actual != artifact.size_bytes {
            return Err(ModelStoreError::Size {
                path: path.to_path_buf(),
                actual,
                expected: artifact.size_bytes,
            });
        }
        let mut file =
            tokio::fs::File::open(path)
                .await
                .map_err(|source| ModelStoreError::Inspect {
                    path: path.to_path_buf(),
                    source,
                })?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|source| ModelStoreError::Inspect {
                    path: path.to_path_buf(),
                    source,
                })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        let actual_checksum = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if actual_checksum != artifact.sha256 {
            return Err(ModelStoreError::Checksum {
                path: path.to_path_buf(),
            });
        }
        Ok(())
    }
}

// TODO: Connect cancellation to the application task manager and persist progress events.
