use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use directories::ProjectDirs;
use serde::Deserialize;
use thiserror::Error;

use crate::{
    errors::{CodedError, ErrorCode},
    llm::LmStudioConfig,
    registry::{GenerationDefaults, KindLoraBindings, LoraBinding, LoraBindings, SizeDefaults},
    runtime::KreaQuantization,
};

pub const DEFAULT_CONFIG_PATH: &str = "config/app.toml";

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
    /// The configurable sizes, step count, and LoRA bindings that shape
    /// every generation (editable in Settings, persisted to this file).
    pub generation_defaults: GenerationDefaults,
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
    #[error(
        "invalid {key} = {value:?} in {path}: dimensions must be multiples of 32 within 256..=2048"
    )]
    InvalidSize {
        path: PathBuf,
        key: String,
        value: String,
    },
    #[error("invalid {key} = {value:?} in {path}: expected WxH (e.g. 512x768)")]
    MalformedSize {
        path: PathBuf,
        key: String,
        value: String,
    },
}

impl CodedError for ConfigError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::PlatformDirectories => ErrorCode::ConfigPlatformDirectories,
            Self::Read { .. } => ErrorCode::ConfigRead,
            Self::Parse { .. } => ErrorCode::ConfigParse,
            Self::Write { .. } => ErrorCode::ConfigWrite,
            Self::GenerationProfile(_) => ErrorCode::ConfigGenerationProfile,
            Self::InvalidSize { .. } => ErrorCode::ConfigInvalidSize,
            Self::MalformedSize { .. } => ErrorCode::ConfigMalformedSize,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    lm_studio: LmStudioFileConfig,
    #[serde(default)]
    paths: PathOverrides,
    #[serde(default)]
    generation: GenerationFileConfig,
    #[serde(default)]
    lora: LoraFileConfig,
}

#[derive(Debug, Deserialize)]
struct GenerationFileConfig {
    #[serde(default = "default_generation_url")]
    base_url: String,
    #[serde(default = "default_generation_profile")]
    profile: String,
    #[serde(default = "default_steps")]
    steps: u32,
    #[serde(default)]
    sizes: SizeFileConfig,
    executable: Option<PathBuf>,
    lora_dir: Option<PathBuf>,
}

impl Default for GenerationFileConfig {
    fn default() -> Self {
        Self {
            base_url: default_generation_url(),
            profile: default_generation_profile(),
            steps: default_steps(),
            sizes: SizeFileConfig::default(),
            executable: None,
            lora_dir: None,
        }
    }
}

/// Per-kind default sizes in `[generation.sizes]`, written as `WxH`
/// strings (the same syntax as the CLI `--size` flag).
#[derive(Debug, Default, Deserialize)]
struct SizeFileConfig {
    character: Option<String>,
    environment: Option<String>,
    object: Option<String>,
    scene: Option<String>,
    beat: Option<String>,
}

/// `[lora]` — the config-level adapter bindings: "use this adapter for
/// that kind of work". Values are file names within `generation.lora_dir`.
#[derive(Debug, Deserialize)]
struct LoraFileConfig {
    /// Adapter applied to mask-guided edits (`modify`).
    inpaint: Option<String>,
    character: Option<String>,
    environment: Option<String>,
    object: Option<String>,
    /// The weight for every binding that does not carry its own.
    #[serde(default = "default_lora_multiplier")]
    multiplier: f32,
}

impl Default for LoraFileConfig {
    fn default() -> Self {
        Self {
            inpaint: None,
            character: None,
            environment: None,
            object: None,
            multiplier: default_lora_multiplier(),
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
        let lora_dir = file
            .generation
            .lora_dir
            .unwrap_or_else(|| model_dir.join("loras"));
        let generation = GenerationConfig {
            base_url: file.generation.base_url,
            executable: file.generation.executable.unwrap_or_else(|| {
                model_dir
                    .join("runtime")
                    .join("stable-diffusion.cpp")
                    .join("bin")
                    .join("sd-server")
            }),
            lora_dir: lora_dir.clone(),
            profile,
        };
        let size_override =
            |key: &str, value: Option<String>| -> Result<Option<(u32, u32)>, ConfigError> {
                match value {
                    None => Ok(None),
                    Some(value) => parse_size_config(&path, key, &value).map(Some),
                }
            };
        let sizes = SizeDefaults {
            character: size_override("sizes.character", file.generation.sizes.character)?,
            environment: size_override("sizes.environment", file.generation.sizes.environment)?,
            object: size_override("sizes.object", file.generation.sizes.object)?,
            scene: size_override("sizes.scene", file.generation.sizes.scene)?,
            beat: size_override("sizes.beat", file.generation.sizes.beat)?,
        };
        let file_lora = &file.lora;
        let lora_bindings = LoraBindings {
            dir: lora_dir,
            inpaint: file_lora.inpaint.clone().map(|file| LoraBinding { file }),
            kinds: KindLoraBindings {
                character: file_lora.character.clone().map(|file| LoraBinding { file }),
                environment: file_lora
                    .environment
                    .clone()
                    .map(|file| LoraBinding { file }),
                object: file_lora.object.clone().map(|file| LoraBinding { file }),
            },
            multiplier: file_lora.multiplier,
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
            generation_defaults: GenerationDefaults {
                sizes,
                steps: file.generation.steps,
                loras: lora_bindings,
            },
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

    /// Persists the generation defaults (steps, per-kind sizes, LoRA
    /// bindings) to `config/app.toml` — the Settings screen writes here so
    /// the choices survive restarts. Line-preserving like
    /// [`AppConfig::persist_lm_model`]: only the touched keys change.
    pub fn persist_generation_defaults(&self) -> Result<(), ConfigError> {
        Self::persist_generation_defaults_to(DEFAULT_CONFIG_PATH, &self.generation_defaults)
    }

    /// Testable variant of [`AppConfig::persist_generation_defaults`].
    pub fn persist_generation_defaults_to(
        path: impl AsRef<Path>,
        defaults: &GenerationDefaults,
    ) -> Result<(), ConfigError> {
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

        // Sizes are written quoted (TOML strings); the loader parses WxH.
        let size_str = |size: Option<(u32, u32)>| {
            size.map(|(width, height)| format!("{:?}", format!("{width}x{height}")))
        };
        write_key(
            &mut lines,
            "generation",
            "steps",
            Some(defaults.steps.to_string()),
        );
        write_key(
            &mut lines,
            "generation.sizes",
            "character",
            size_str(defaults.sizes.character),
        );
        write_key(
            &mut lines,
            "generation.sizes",
            "environment",
            size_str(defaults.sizes.environment),
        );
        write_key(
            &mut lines,
            "generation.sizes",
            "object",
            size_str(defaults.sizes.object),
        );
        write_key(
            &mut lines,
            "generation.sizes",
            "scene",
            size_str(defaults.sizes.scene),
        );
        write_key(
            &mut lines,
            "generation.sizes",
            "beat",
            size_str(defaults.sizes.beat),
        );
        let lora_file =
            |binding: Option<&LoraBinding>| binding.map(|binding| format!("{:?}", binding.file));
        write_key(
            &mut lines,
            "lora",
            "inpaint",
            lora_file(defaults.loras.inpaint.as_ref()),
        );
        write_key(
            &mut lines,
            "lora",
            "character",
            lora_file(defaults.loras.kinds.character.as_ref()),
        );
        write_key(
            &mut lines,
            "lora",
            "environment",
            lora_file(defaults.loras.kinds.environment.as_ref()),
        );
        write_key(
            &mut lines,
            "lora",
            "object",
            lora_file(defaults.loras.kinds.object.as_ref()),
        );
        write_key(
            &mut lines,
            "lora",
            "multiplier",
            Some(format!("{:?}", defaults.loras.multiplier)),
        );

        let mut output = lines.join("\n");
        output.push('\n');
        fs::write(path, output).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Line-preserving key writer: replaces the `key = …` line inside
/// `[section]` (creating it right after the section header when missing),
/// or removes it when `value` is `None`. A missing section is inserted
/// before the next section header (or appended at the end).
fn write_key(lines: &mut Vec<String>, section: &str, key: &str, value: Option<String>) {
    let mut in_section = false;
    let mut section_index: Option<usize> = None;
    let header = format!("[{section}]");
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_section {
                break;
            }
            in_section = trimmed == header;
            if in_section {
                section_index = Some(index);
            }
            continue;
        }
        if in_section {
            if let Some(rest) = trimmed.strip_prefix(key) {
                if rest.trim_start().starts_with('=') {
                    match value {
                        Some(value) => {
                            let indent: String = line
                                .chars()
                                .take_while(|character| character.is_whitespace())
                                .collect();
                            lines[index] = format!("{indent}{key} = {value}");
                        }
                        None => {
                            lines.remove(index);
                        }
                    }
                    return;
                }
            }
        }
    }
    let Some(value) = value else {
        return; // nothing to insert
    };
    let replacement = format!("{key} = {value}");
    match section_index {
        Some(index) => {
            lines.insert(index + 1, replacement);
        }
        None => {
            // Insert the section before the next existing section header.
            let insert_at = lines
                .iter()
                .position(|line| line.trim_start().starts_with('['))
                .unwrap_or(lines.len());
            lines.insert(insert_at, header);
            lines.insert(insert_at + 1, replacement);
        }
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

const fn default_steps() -> u32 {
    4
}

const fn default_lora_multiplier() -> f32 {
    1.0
}

/// Parses a `WxH` config value with the same bounds as the CLI
/// (`pipeline::is_valid_size`).
fn parse_size_config(path: &Path, key: &str, value: &str) -> Result<(u32, u32), ConfigError> {
    let normalized = value.replace('×', "x");
    let (width, height) = normalized
        .split_once('x')
        .ok_or_else(|| ConfigError::MalformedSize {
            path: path.to_path_buf(),
            key: key.to_owned(),
            value: value.to_owned(),
        })?;
    let width = width
        .trim()
        .parse::<u32>()
        .map_err(|_| ConfigError::MalformedSize {
            path: path.to_path_buf(),
            key: key.to_owned(),
            value: value.to_owned(),
        })?;
    let height = height
        .trim()
        .parse::<u32>()
        .map_err(|_| ConfigError::MalformedSize {
            path: path.to_path_buf(),
            key: key.to_owned(),
            value: value.to_owned(),
        })?;
    if !crate::registry::pipeline::is_valid_size(width, height) {
        return Err(ConfigError::InvalidSize {
            path: path.to_path_buf(),
            key: key.to_owned(),
            value: value.to_owned(),
        });
    }
    Ok((width, height))
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
    fn generation_defaults_parse_from_the_file() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(
            &path,
            "[generation]\nsteps = 8\n[generation.sizes]\ncharacter = \"384x576\"\n\
             [lora]\ninpaint = \"inpaintkrea.safetensors\"\ncharacter = \"mia-v2.safetensors\"\nmultiplier = 0.8\n",
        )
        .expect("fixture should write");

        let config = AppConfig::load_from(&path).expect("config should load");
        assert_eq!(config.generation_defaults.steps, 8);
        assert_eq!(config.generation_defaults.sizes.character, Some((384, 576)));
        assert_eq!(config.generation_defaults.sizes.object, None);
        assert_eq!(
            config
                .generation_defaults
                .loras
                .inpaint
                .as_ref()
                .map(|b| b.file.as_str()),
            Some("inpaintkrea.safetensors")
        );
        assert_eq!(
            config
                .generation_defaults
                .loras
                .kinds
                .character
                .as_ref()
                .map(|b| b.file.as_str()),
            Some("mia-v2.safetensors")
        );
        assert_eq!(config.generation_defaults.loras.multiplier, 0.8);
    }

    #[test]
    fn invalid_size_keys_are_rejected_with_the_key_named() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(&path, "[generation.sizes]\ncharacter = \"100x100\"\n")
            .expect("fixture should write");
        let error = AppConfig::load_from(&path).expect_err("invalid sizes must be rejected");
        assert!(error.to_string().contains("sizes.character"), "{error}");

        std::fs::write(&path, "[generation.sizes]\ncharacter = \"square\"\n")
            .expect("fixture should write");
        let error = AppConfig::load_from(&path).expect_err("malformed sizes must be rejected");
        assert!(error.to_string().contains("sizes.character"), "{error}");
    }

    #[test]
    fn generation_defaults_persist_and_round_trip() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(
            &path,
            "# keep me\n[generation]\nbase_url = \"http://127.0.0.1:7861\"\nprofile = \"krea-2-turbo-q4\"\n",
        )
        .expect("fixture should write");

        let mut defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 6,
            loras: Default::default(),
        };
        defaults.sizes.character = Some((384, 576));
        defaults.loras.inpaint = Some(LoraBinding {
            file: "inpaintkrea.safetensors".into(),
        });
        defaults.loras.kinds.character = Some(LoraBinding {
            file: "mia-v2.safetensors".into(),
        });
        defaults.loras.multiplier = 0.8;
        AppConfig::persist_generation_defaults_to(&path, &defaults)
            .expect("defaults should persist");

        let reloaded = AppConfig::load_from(&path).expect("persisted file should load");
        assert_eq!(reloaded.generation_defaults.steps, 6);
        assert_eq!(
            reloaded.generation_defaults.sizes.character,
            Some((384, 576))
        );
        assert_eq!(
            reloaded
                .generation_defaults
                .loras
                .inpaint
                .map(|b| b.file)
                .as_deref(),
            Some("inpaintkrea.safetensors")
        );
        assert_eq!(reloaded.generation_defaults.loras.multiplier, 0.8);
        // Untouched keys survive.
        assert_eq!(reloaded.generation.base_url, "http://127.0.0.1:7861");
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(contents.contains("# keep me"), "{contents}");
        assert!(
            contents.contains("profile = \"krea-2-turbo-q4\""),
            "{contents}"
        );
    }

    #[test]
    fn persist_generation_defaults_adds_sections_when_missing() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(&path, "[lm_studio]\nmodel = \"m\"\n").expect("fixture should write");

        let mut defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 5,
            loras: Default::default(),
        };
        defaults.sizes.beat = Some((1024, 576));
        defaults.loras.kinds.object = Some(LoraBinding {
            file: "brass.safetensors".into(),
        });
        AppConfig::persist_generation_defaults_to(&path, &defaults)
            .expect("defaults should persist");
        let reloaded = AppConfig::load_from(&path).expect("persisted file should load");
        assert_eq!(reloaded.generation_defaults.steps, 5);
        assert_eq!(reloaded.generation_defaults.sizes.beat, Some((1024, 576)));
        assert_eq!(
            reloaded
                .generation_defaults
                .loras
                .kinds
                .object
                .map(|b| b.file)
                .as_deref(),
            Some("brass.safetensors")
        );
    }

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
            reloaded.lm_studio.base_url, "http://localhost:1234/v1",
            "base_url must not be clobbered by the model write"
        );
    }

    #[test]
    fn a_nested_lora_binding_survives_the_persist_and_reload_cycle() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(&path, "[lm_studio]\nmodel = \"m\"\n").expect("fixture should write");

        let mut defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 4,
            loras: Default::default(),
        };
        defaults.loras.inpaint = Some(LoraBinding {
            file: "krea/INPAINTKREA-V1.safetensors".into(),
        });
        AppConfig::persist_generation_defaults_to(&path, &defaults)
            .expect("defaults should persist");
        let reloaded = AppConfig::load_from(&path).expect("persisted file should load");
        assert_eq!(
            reloaded
                .generation_defaults
                .loras
                .inpaint
                .map(|binding| binding.file)
                .as_deref(),
            Some("krea/INPAINTKREA-V1.safetensors"),
            "a nested (symlinked-folder) binding must survive a reload"
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
