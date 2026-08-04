use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use directories::ProjectDirs;
use serde::Deserialize;
use thiserror::Error;

use crate::llm::LmStudioConfig;
use crate::runtime::KreaQuantization;

const DEFAULT_CONFIG_PATH: &str = "config/app.toml";

#[derive(Clone, Debug, PartialEq)]
pub struct AppConfig {
    pub lm_studio: LmStudioConfig,
    pub python_runtime: PathBuf,
    pub model_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub asset_dir: PathBuf,
    pub generation: GenerationConfig,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GenerationConfig {
    pub base_url: String,
    pub executable: PathBuf,
    pub lora_dir: PathBuf,
    pub profile: KreaQuantization,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine platform application directories")]
    PlatformDirectories,
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error(
        "unsupported Krea generation profile {0}; expected krea-2-turbo-q2 or krea-2-turbo-q4"
    )]
    GenerationProfile(String),
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    lm_studio: LmStudioFileConfig,
    #[serde(default)]
    paths: PathOverrides,
    #[serde(default)]
    generation: GenerationFileConfig,
}

#[derive(Debug, Deserialize)]
struct GenerationFileConfig {
    #[serde(default = "default_generation_url")]
    base_url: String,
    #[serde(default = "default_generation_profile")]
    profile: String,
    executable: Option<PathBuf>,
    lora_dir: Option<PathBuf>,
}

impl Default for GenerationFileConfig {
    fn default() -> Self {
        Self {
            base_url: default_generation_url(),
            profile: default_generation_profile(),
            executable: None,
            lora_dir: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct LmStudioFileConfig {
    #[serde(default = "default_lm_url")]
    base_url: String,
    #[serde(default = "default_lm_model")]
    model: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
}

impl Default for LmStudioFileConfig {
    fn default() -> Self {
        Self {
            base_url: default_lm_url(),
            model: default_lm_model(),
            api_key: None,
            timeout_seconds: default_timeout(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct PathOverrides {
    python_runtime: Option<PathBuf>,
    model_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    asset_dir: Option<PathBuf>,
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(DEFAULT_CONFIG_PATH)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let file = if path.exists() {
            let contents = fs::read_to_string(path).map_err(|source| ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            toml::from_str(&contents).map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            })?
        } else {
            FileConfig::default()
        };

        let dirs = ProjectDirs::from("dev", "Smart Visual Sequencer", "Smart Visual Sequencer")
            .ok_or(ConfigError::PlatformDirectories)?;
        let data_dir = dirs.data_dir();

        let model_dir = file
            .paths
            .model_dir
            .unwrap_or_else(|| data_dir.join("models"));
        let profile = match file.generation.profile.as_str() {
            "krea-2-turbo-q2" => KreaQuantization::Q2,
            "krea-2-turbo-q4" => KreaQuantization::Q4,
            other => return Err(ConfigError::GenerationProfile(other.to_owned())),
        };
        let generation = GenerationConfig {
            base_url: file.generation.base_url,
            executable: file.generation.executable.unwrap_or_else(|| {
                model_dir
                    .join("runtime")
                    .join("stable-diffusion.cpp")
                    .join("bin")
                    .join("sd-server")
            }),
            lora_dir: file
                .generation
                .lora_dir
                .unwrap_or_else(|| model_dir.join("loras")),
            profile,
        };

        Ok(Self {
            lm_studio: LmStudioConfig {
                base_url: file.lm_studio.base_url,
                model: file.lm_studio.model,
                api_key: file.lm_studio.api_key,
                timeout: Duration::from_secs(file.lm_studio.timeout_seconds),
            },
            python_runtime: file
                .paths
                .python_runtime
                .unwrap_or_else(|| PathBuf::from("python")),
            model_dir,
            cache_dir: file
                .paths
                .cache_dir
                .unwrap_or_else(|| dirs.cache_dir().to_path_buf()),
            asset_dir: file
                .paths
                .asset_dir
                .unwrap_or_else(|| data_dir.join("assets")),
            generation,
        })
    }
}

fn default_lm_url() -> String {
    "http://localhost:1234/v1".to_owned()
}

fn default_lm_model() -> String {
    "local-model".to_owned()
}

const fn default_timeout() -> u64 {
    60
}

fn default_generation_url() -> String {
    "http://127.0.0.1:7861".to_owned()
}

fn default_generation_profile() -> String {
    "krea-2-turbo-q2".to_owned()
}
