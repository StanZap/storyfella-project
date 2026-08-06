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
    /// Where the GUI keeps `.svs-project.db` files (the Projects screen).
    pub project_dir: PathBuf,
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
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
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
    project_dir: Option<PathBuf>,
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
            project_dir: file
                .paths
                .project_dir
                .unwrap_or_else(|| data_dir.join("projects")),
            generation,
        })
    }

    /// Persists the planner model to `config/app.toml` (the Settings
    /// picker writes here so the choice survives restarts). Every other key
    /// in the file is left untouched.
    pub fn persist_lm_model(&self, model: &str) -> Result<(), ConfigError> {
        Self::persist_lm_model_to(DEFAULT_CONFIG_PATH, model)
    }

    /// Testable variant of [`AppConfig::persist_lm_model`] that targets an
    /// arbitrary path; creates the file when it does not exist yet. The
    /// write is line-preserving: only the `model` key under `[lm_studio]`
    /// changes, so comments and key order elsewhere survive.
    pub fn persist_lm_model_to(path: impl AsRef<Path>, model: &str) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let contents = if path.exists() {
            fs::read_to_string(path).map_err(|source| ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })?
        } else {
            String::new()
        };
        let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();

        // Find the `model = "..."` line inside the `[lm_studio]` section.
        let mut in_lm_studio = false;
        let mut section_index = None;
        let mut model_line_index = None;
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_lm_studio = trimmed == "[lm_studio]";
                if in_lm_studio {
                    section_index = Some(index);
                }
                continue;
            }
            if in_lm_studio {
                if let Some(rest) = trimmed.strip_prefix("model") {
                    if rest.trim_start().starts_with('=') {
                        model_line_index = Some(index);
                    }
                }
            }
        }

        let replacement = format!("model = {model:?}");
        match model_line_index {
            Some(index) => {
                let indent: String = lines[index]
                    .chars()
                    .take_while(|character| character.is_whitespace())
                    .collect();
                lines[index] = format!("{indent}{replacement}");
            }
            None => match section_index {
                Some(index) => lines.insert(index + 1, replacement),
                None => {
                    lines.push("[lm_studio]".to_owned());
                    lines.push(replacement);
                }
            },
        }

        let mut output = lines.join("\n");
        output.push('\n');
        fs::write(path, output).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
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
    "krea-2-turbo-q4".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_lm_model_updates_only_the_model_key() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        let config = AppConfig::load_from(&path).expect("defaults load from a missing file");
        assert_eq!(config.lm_studio.model, "local-model");

        AppConfig::persist_lm_model_to(&path, "google/gemma-3-27b-it")
            .expect("persist should write");
        let reloaded = AppConfig::load_from(&path).expect("persisted file should load");
        assert_eq!(reloaded.lm_studio.model, "google/gemma-3-27b-it");
        // The other sections survive the round trip.
        assert_eq!(reloaded.generation.profile, KreaQuantization::Q4);
        assert_eq!(
            reloaded.lm_studio.base_url,
            "http://localhost:1234/v1",
            "base_url must not be clobbered by the model write"
        );
    }

    #[test]
    fn persist_lm_model_preserves_unrelated_keys_in_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(
            &path,
            "# api_key = \"lm-studio\"\n[lm_studio]\nbase_url = \"http://localhost:1234/v1\"\nmodel = \"old-model\"\n\n\
             [generation]\nprofile = \"krea-2-turbo-q2\"\n",
        )
        .expect("fixture should write");

        AppConfig::persist_lm_model_to(&path, "new-model").expect("persist should write");
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(contents.contains("old-model") == false, "{contents}");
        assert!(contents.contains("new-model"), "{contents}");
        assert!(
            contents.contains("krea-2-turbo-q2"),
            "generation profile must survive: {contents}"
        );
        assert!(
            contents.contains("http://localhost:1234/v1"),
            "base_url must survive: {contents}"
        );
        assert!(
            contents.contains("# api_key = \"lm-studio\""),
            "comments must survive the targeted write: {contents}"
        );
        assert!(
            contents.find("[lm_studio]") < contents.find("[generation]"),
            "section order must survive the targeted write: {contents}"
        );
    }

    #[test]
    fn persist_lm_model_adds_the_section_when_missing() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(&path, "[generation]\nprofile = \"krea-2-turbo-q2\"\n")
            .expect("fixture should write");

        AppConfig::persist_lm_model_to(&path, "new-model").expect("persist should write");
        let reloaded = AppConfig::load_from(&path).expect("persisted file should load");
        assert_eq!(reloaded.lm_studio.model, "new-model");
        assert_eq!(reloaded.generation.profile, KreaQuantization::Q2);
    }
}
