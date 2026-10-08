use std::{
    fs,
    path::{Path, PathBuf},
};

use clipforge_inference::{
    sd, Capability, ChatDeployment, ImageDeployment, ProviderError, Providers,
};
use serde::Deserialize;
use thiserror::Error;

use crate::{
    errors::{CodedError, ErrorCode},
    llm::LlmConfig,
    paths,
    registry::{GenerationDefaults, KindLoraBindings, LoraBinding, LoraBindings, SizeDefaults},
};

#[derive(Clone, Debug, PartialEq)]
pub struct AppConfig {
    /// The `text` deployment resolved from `storyfella.toml` — the planner's
    /// OpenAI-compatible endpoint, model, key and timeout. `None` when no
    /// provider serves `text`: the planner is a soft dependency, so the app
    /// still runs and its LLM steps degrade to manual input.
    pub text: Option<ChatDeployment>,
    /// The `vision` deployment resolved from `storyfella.toml` — what `svs eval`
    /// and future vision steps call. `None` when no provider serves `vision`.
    pub vision: Option<ChatDeployment>,
    /// The provider id serving `text`, kept so Settings can persist a model
    /// choice under the right `[providers.<id>]` section. `None` when the
    /// capability is unconfigured.
    pub text_provider_id: Option<String>,
    /// The provider id serving `vision`. `None` when unconfigured.
    pub vision_provider_id: Option<String>,
    /// The provider id serving `t2i`, kept so Settings can persist the
    /// residency choice under the right `[providers.<id>]` section.
    pub image_provider_id: Option<String>,
    /// The image-generation deployment resolved from `storyfella.toml`: the
    /// `sd-server` executable, model tree, LoRA root, endpoint, and weights.
    pub image: ImageDeployment,
    pub cache_dir: PathBuf,
    pub asset_dir: PathBuf,
    /// Where the GUI keeps `.svs-project.db` files (the Projects screen).
    pub project_dir: PathBuf,
    /// The user-level config layered over `config/storyfella.toml` (see
    /// [`paths::user_config_path`]). `None` when the platform has no config
    /// directory, or the config was read from an explicit path. The Settings
    /// screen shows it so machine-local secrets have a known home.
    pub user_config_path: Option<PathBuf>,
    /// The configurable sizes, step count, and LoRA bindings that shape
    /// every generation (editable in Settings, persisted to this file).
    pub generation_defaults: GenerationDefaults,
    /// Start the resident generation server as soon as a project opens, so
    /// the first generation is not a multi-minute weight load (`[runtime]
    /// prewarm`). Off by default — it holds the card's VRAM from the outset.
    pub prewarm: bool,
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
    #[error("failed to assemble the providers registry: {source}")]
    Serialize { source: toml::ser::Error },
    #[error(transparent)]
    Providers(#[from] ProviderError),
}

impl CodedError for ConfigError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::PlatformDirectories => ErrorCode::ConfigPlatformDirectories,
            Self::Read { .. } => ErrorCode::ConfigRead,
            Self::Parse { .. } => ErrorCode::ConfigParse,
            Self::Write { .. } => ErrorCode::ConfigWrite,
            Self::InvalidSize { .. } => ErrorCode::ConfigInvalidSize,
            Self::MalformedSize { .. } => ErrorCode::ConfigMalformedSize,
            Self::Serialize { .. } => ErrorCode::ConfigSerialize,
            Self::Providers(_) => ErrorCode::ConfigProviders,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    #[serde(default)]
    paths: PathOverrides,
    #[serde(default)]
    generation: GenerationFileConfig,
    #[serde(default)]
    lora: LoraFileConfig,
    #[serde(default)]
    runtime: RuntimeFileConfig,
}

/// `[runtime]` — how the app manages the native server processes.
#[derive(Debug, Default, Deserialize)]
struct RuntimeFileConfig {
    #[serde(default)]
    prewarm: bool,
}

#[derive(Debug, Deserialize)]
struct GenerationFileConfig {
    #[serde(default = "default_steps")]
    steps: u32,
    #[serde(default = "default_cfg_scale")]
    cfg_scale: f32,
    #[serde(default = "default_sample_method")]
    sample_method: String,
    #[serde(default)]
    sizes: SizeFileConfig,
}

impl Default for GenerationFileConfig {
    fn default() -> Self {
        Self {
            steps: default_steps(),
            cfg_scale: default_cfg_scale(),
            sample_method: default_sample_method(),
            sizes: SizeFileConfig::default(),
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
/// that kind of work". Values are file names within the provider's LoRA
/// directory.
#[derive(Debug, Deserialize)]
struct LoraFileConfig {
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
            character: None,
            environment: None,
            object: None,
            multiplier: default_lora_multiplier(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct PathOverrides {
    cache_dir: Option<PathBuf>,
    asset_dir: Option<PathBuf>,
    project_dir: Option<PathBuf>,
}

impl AppConfig {
    /// Loads the effective config: `config/storyfella.toml` beneath the project root
    /// (see [`crate::paths`]), with the user config layered over it.
    pub fn load() -> Result<Self, ConfigError> {
        let user = paths::user_config_path();
        if let Some(path) = &user {
            migrate_user_config(path)?;
        }
        Self::load_layered(paths::config_path(), user)
    }

    /// Loads one complete settings file without the machine's user overlay.
    pub fn load_from(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::load_layered(path.as_ref().to_path_buf(), None)
    }

    /// The planner's [`LlmConfig`] — the `text` provider deployment, or `None`
    /// when no provider serves `text` (the planner then degrades to manual
    /// input rather than failing).
    pub fn planner_config(&self) -> Option<LlmConfig> {
        self.text.as_ref().map(LlmConfig::from)
    }

    /// Reads `base`, merges `user` over it (deeply; see [`merge_toml`]), and
    /// resolves the result. A missing file contributes nothing rather than
    /// failing, so the default config needs neither file to exist. The
    /// same merged value supplies both application and inference settings.
    fn load_layered(base: PathBuf, user: Option<PathBuf>) -> Result<Self, ConfigError> {
        let path = base.as_path();
        let mut value = read_config_value(path)?;
        if let Some(overlay) = user.as_deref() {
            if overlay != path {
                merge_toml(&mut value, read_config_value(overlay)?);
            }
        }
        // A type error in the merged tree is most likely in the overlay — the
        // file the user just edited — so blame the user path when there is one.
        let parse_path = user.clone().unwrap_or_else(|| base.clone());
        let file: FileConfig = value
            .clone()
            .try_into()
            .map_err(|source| ConfigError::Parse {
                path: parse_path,
                source,
            })?;

        // The deployments — the `sd-server` image backend and the chat
        // (`text`/`vision`) endpoints — come from the providers registry at
        // same general settings file and user overlay.
        let providers = providers_from_value(&value)?;
        let image = providers.image_deployment(Capability::T2i)?;
        // The chat capabilities are optional: an unconfigured planner or vision
        // endpoint leaves the app fully usable, with those features degraded.
        let text = providers.chat_deployment(Capability::Text).ok();
        let vision = providers.chat_deployment(Capability::Vision).ok();
        let text_provider_id = providers
            .default_provider(Capability::Text)
            .map(str::to_owned);
        let vision_provider_id = providers
            .default_provider(Capability::Vision)
            .map(str::to_owned);
        let image_provider_id = providers
            .default_provider(Capability::T2i)
            .map(str::to_owned);

        let dirs = paths::project_dirs().ok_or(ConfigError::PlatformDirectories)?;
        let data_dir = dirs.data_dir();

        let size_override =
            |key: &str, value: Option<String>| -> Result<Option<(u32, u32)>, ConfigError> {
                match value {
                    None => Ok(None),
                    Some(value) => parse_size_config(path, key, &value).map(Some),
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
            dir: image.lora_dir.clone(),
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
            text,
            vision,
            text_provider_id,
            vision_provider_id,
            image_provider_id,
            image,
            cache_dir: file
                .paths
                .cache_dir
                .map(paths::resolve)
                .unwrap_or_else(|| dirs.cache_dir().to_path_buf()),
            asset_dir: file
                .paths
                .asset_dir
                .map(paths::resolve)
                .unwrap_or_else(|| data_dir.join("assets")),
            project_dir: file
                .paths
                .project_dir
                .map(paths::resolve)
                .unwrap_or_else(|| data_dir.join("projects")),
            user_config_path: user,
            generation_defaults: GenerationDefaults {
                sizes,
                steps: file.generation.steps,
                cfg_scale: file.generation.cfg_scale,
                sample_method: file.generation.sample_method,
                loras: lora_bindings,
            },
            prewarm: file.runtime.prewarm,
        })
    }

    /// Persists the planner model to the user settings file (see
    /// [`paths::user_config_path`]) — the Settings picker writes here so the
    /// choice survives restarts without touching the checked-in
    /// `storyfella.toml`. Every other line in the file is left untouched.
    pub fn persist_text_model(&self, model: &str) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        let provider_id = self
            .text_provider_id
            .as_deref()
            .ok_or(ConfigError::Providers(ProviderError::NoProvider(
                Capability::Text,
            )))?;
        Self::persist_text_model_to(path, provider_id, model)
    }

    /// Testable variant of [`AppConfig::persist_text_model`] that targets an
    /// arbitrary path and provider id; creates the file when it does not exist
    /// yet. The write is line-preserving: only the `model` key under
    /// `[providers.<provider_id>]` changes, so comments and key order
    /// elsewhere survive.
    pub fn persist_text_model_to(
        path: impl AsRef<Path>,
        provider_id: &str,
        model: &str,
    ) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let contents = read_config_contents(path)?;
        let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();
        let section = format!("providers.{provider_id}");
        write_key(&mut lines, &section, "model", Some(format!("{model:?}")));
        write_config_file(path, lines.join("\n"))
    }

    /// Persists the generation defaults (steps, per-kind sizes, LoRA bindings)
    /// to the user config (see [`paths::user_config_path`]) — the Settings
    /// screen writes here so the choices survive restarts without touching the
    /// checked-in `config/storyfella.toml`. Line-preserving: only the touched keys
    /// change.
    pub fn persist_generation_defaults(&self) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        Self::persist_generation_defaults_to(path, &self.generation_defaults)
    }

    /// Testable variant of [`AppConfig::persist_generation_defaults`].
    pub fn persist_generation_defaults_to(
        path: impl AsRef<Path>,
        defaults: &GenerationDefaults,
    ) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let contents = read_config_contents(path)?;
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
            "generation",
            "cfg_scale",
            Some(format!("{:?}", defaults.cfg_scale)),
        );
        write_key(
            &mut lines,
            "generation",
            "sample_method",
            Some(format!("{:?}", defaults.sample_method)),
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

        write_config_file(path, lines.join("\n"))
    }

    /// Persists the image provider's residency to the user settings file (see
    /// [`paths::user_config_path`]) — the Settings control writes here so
    /// the choice survives restarts without touching the checked-in
    /// `storyfella.toml`.
    pub fn persist_image_residency(&self, residency: &str) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        let provider_id = self
            .image_provider_id
            .as_deref()
            .ok_or(ConfigError::Providers(ProviderError::NoProvider(
                Capability::T2i,
            )))?;
        Self::persist_image_residency_to(path, provider_id, residency)
    }

    /// Testable variant of [`AppConfig::persist_image_residency`]; creates the
    /// file when it does not exist yet. Line-preserving: only the `residency`
    /// key under `[providers.<provider_id>]` changes.
    pub fn persist_image_residency_to(
        path: impl AsRef<Path>,
        provider_id: &str,
        residency: &str,
    ) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let contents = read_config_contents(path)?;
        let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();
        let section = format!("providers.{provider_id}");
        write_key(
            &mut lines,
            &section,
            "residency",
            Some(format!("{residency:?}")),
        );
        write_config_file(path, lines.join("\n"))
    }

    /// Persists the prewarm choice to the user config (see
    /// [`paths::user_config_path`]), line-preserving like the other settings.
    pub fn persist_prewarm(&self, enabled: bool) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        Self::persist_prewarm_to(path, enabled)
    }

    /// Testable variant of [`AppConfig::persist_prewarm`] targeting an
    /// arbitrary path; creates the file when it does not exist yet.
    pub fn persist_prewarm_to(path: impl AsRef<Path>, enabled: bool) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let contents = read_config_contents(path)?;
        let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();
        write_key(&mut lines, "runtime", "prewarm", Some(enabled.to_string()));
        write_config_file(path, lines.join("\n"))
    }

    /// Save an absolute model-tree override in the same general settings file.
    /// Files stay in place: selecting an existing tree reuses its downloads.
    pub fn persist_model_dir(&self, directory: &Path) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        let provider = self
            .image_provider_id
            .as_deref()
            .ok_or(ConfigError::Providers(ProviderError::NoProvider(
                Capability::T2i,
            )))?;
        Self::persist_model_dir_to(path, provider, directory)
    }

    pub fn persist_model_dir_to(
        path: &Path,
        provider: &str,
        directory: &Path,
    ) -> Result<(), ConfigError> {
        let directory = std::path::absolute(directory).map_err(|source| ConfigError::Read {
            path: directory.to_owned(),
            source,
        })?;
        let contents = read_config_contents(path)?;
        let encoded = toml::Value::String(directory.to_string_lossy().into_owned()).to_string();
        let mut lines = contents.lines().map(str::to_owned).collect();
        write_key(
            &mut lines,
            &format!("providers.{provider}"),
            "model_dir",
            Some(encoded),
        );
        write_config_file(path, lines.join("\n"))
    }
}

/// Missing settings contribute nothing; other read failures remain actionable.
fn read_config_contents(path: &Path) -> Result<String, ConfigError> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(ConfigError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Reads a config file as a TOML value; a missing file is an empty table, so
/// the merge treats it as contributing nothing.
fn read_config_value(path: &Path) -> Result<toml::Value, ConfigError> {
    let contents = read_config_contents(path)?;
    toml::from_str(&contents).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Deep-merges `overlay` into `base`: tables merge key by key, and every other
/// value replaces what was there. The user config can therefore set one nested
/// key (say `[generation.sizes] character`) without discarding the rest of the
/// checked-in file.
fn merge_toml(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(key.as_str()) {
                    Some(existing) => merge_toml(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

/// Extract inference sections from general settings; the shared registry owns
/// validation. Inject the host's storage default before resolving deployments.
fn providers_from_value(value: &toml::Value) -> Result<Providers, ConfigError> {
    let mut registry = toml::Table::new();
    for key in ["defaults", "providers"] {
        if let Some(section) = value.get(key) {
            registry.insert(key.into(), section.clone());
        }
    }
    let empty = match registry.get("providers") {
        None => true,
        Some(toml::Value::Table(providers)) => providers.is_empty(),
        Some(_) => false,
    };
    if empty {
        let built_in = format!(
            "[defaults]\n\
             t2i = 'qwen-image'\n\
             edit = 'qwen-image'\n\
             [providers.qwen-image]\n\
             kind = 'sdcpp'\n\
             architecture = '{}'\n\
             capabilities = ['t2i', 'edit']\n\
             base_url = '{}'\n",
            sd::qwen_image_2_1().id,
            sd::DEFAULT_BASE_URL,
        );
        let mut built_in = toml::from_str(&built_in).expect("built-in provider configuration");
        merge_toml(&mut built_in, toml::Value::Table(registry));
        registry = built_in
            .as_table()
            .expect("built-in settings table")
            .clone();
    }
    let models = paths::model_dir().ok_or(ConfigError::PlatformDirectories)?;
    if let Some(providers) = registry
        .get_mut("providers")
        .and_then(toml::Value::as_table_mut)
    {
        for provider in providers
            .iter_mut()
            .filter_map(|(_, value)| value.as_table_mut())
        {
            if matches!(
                provider.get("kind").and_then(toml::Value::as_str),
                Some("sdcpp" | "llamacpp")
            ) {
                provider
                    .entry("model_dir")
                    .or_insert_with(|| toml::Value::String(models.to_string_lossy().into_owned()));
            }
        }
    }
    let contents =
        toml::to_string(&registry).map_err(|source| ConfigError::Serialize { source })?;
    Providers::from_toml(&contents, &paths::root()).map_err(ConfigError::Providers)
}

/// One-time settings migration, not a second runtime config format. The new
/// file is authoritative once present; originals remain available for recovery.
fn migrate_user_config(destination: &Path) -> Result<(), ConfigError> {
    if destination.exists() {
        return Ok(());
    }
    let Some(directory) = destination.parent() else {
        return Ok(());
    };
    let old_app = directory.join("app.toml");
    let old_providers = directory.join("providers.toml");
    if destination == old_app
        || destination == old_providers
        || (!old_app.exists() && !old_providers.exists())
    {
        return Ok(());
    }
    let mut settings = read_config_value(&old_app)?;
    merge_toml(&mut settings, read_config_value(&old_providers)?);
    let contents =
        toml::to_string_pretty(&settings).map_err(|source| ConfigError::Serialize { source })?;
    atomic_write_config(destination, &contents, true)
}

/// Writes a config file, creating its directory first — the user config lives
/// in a platform directory that may not exist yet.
fn write_config_file(path: &Path, contents: String) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut output = contents;
    output.push('\n');
    atomic_write_config(path, &output, false)
}

/// Stage beside the destination so failures cannot truncate an existing config.
/// Migration publishes without replacing a configuration another process wrote.
fn atomic_write_config(path: &Path, contents: &str, new_only: bool) -> Result<(), ConfigError> {
    use std::io::Write;
    let staged = path.with_extension(format!("{}.pending", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&staged)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);
        if new_only {
            match fs::hard_link(&staged, path) {
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                result => result,
            }
        } else {
            fs::rename(&staged, path)
        }
    })();
    if staged.exists() {
        let _ = fs::remove_file(&staged);
    }
    result.map_err(|source| ConfigError::Write {
        path: path.to_owned(),
        source,
    })
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

fn default_steps() -> u32 {
    sd::DEFAULT_STEPS
}

fn default_cfg_scale() -> f32 {
    sd::DEFAULT_CFG_SCALE
}

fn default_sample_method() -> String {
    sd::DEFAULT_SAMPLE_METHOD.to_owned()
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
            "[generation]\nsteps = 8\ncfg_scale = 7.5\nsample_method = \"euler_a\"\n\
             [generation.sizes]\ncharacter = \"384x576\"\n\
             [lora]\ncharacter = \"mia-v2.safetensors\"\nmultiplier = 0.8\n",
        )
        .expect("fixture should write");

        let config = AppConfig::load_from(&path).expect("config should load");
        assert_eq!(config.generation_defaults.steps, 8);
        assert_eq!(config.generation_defaults.cfg_scale, 7.5);
        assert_eq!(config.generation_defaults.sample_method, "euler_a");
        assert_eq!(config.generation_defaults.sizes.character, Some((384, 576)));
        assert_eq!(config.generation_defaults.sizes.object, None);
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
        std::fs::write(&path, "# keep me\n[paths]\ncache_dir = \"cache\"\n")
            .expect("fixture should write");

        let mut defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 6,
            cfg_scale: 7.0,
            sample_method: "euler_a".to_owned(),
            loras: Default::default(),
        };
        defaults.sizes.character = Some((384, 576));
        defaults.loras.kinds.character = Some(LoraBinding {
            file: "mia-v2.safetensors".into(),
        });
        defaults.loras.multiplier = 0.8;
        AppConfig::persist_generation_defaults_to(&path, &defaults)
            .expect("defaults should persist");

        let reloaded = AppConfig::load_from(&path).expect("persisted file should load");
        assert_eq!(reloaded.generation_defaults.steps, 6);
        assert_eq!(reloaded.generation_defaults.cfg_scale, 7.0);
        assert_eq!(reloaded.generation_defaults.sample_method, "euler_a");
        assert_eq!(
            reloaded.generation_defaults.sizes.character,
            Some((384, 576))
        );
        assert_eq!(
            reloaded
                .generation_defaults
                .loras
                .kinds
                .character
                .map(|b| b.file)
                .as_deref(),
            Some("mia-v2.safetensors")
        );
        assert_eq!(reloaded.generation_defaults.loras.multiplier, 0.8);
        // Untouched content survives.
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(contents.contains("# keep me"), "{contents}");
        assert!(contents.contains("cache_dir = \"cache\""), "{contents}");
    }

    #[test]
    fn persist_generation_defaults_adds_sections_when_missing() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(&path, "[paths]\ncache_dir = \"cache\"\n").expect("fixture should write");

        let mut defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 5,
            cfg_scale: sd::DEFAULT_CFG_SCALE,
            sample_method: sd::DEFAULT_SAMPLE_METHOD.to_owned(),
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
    fn text_and_vision_deployments_come_from_the_providers_registry() {
        let config = AppConfig::load_from(paths::config_path()).expect("unified config loads");

        assert_eq!(config.text_provider_id.as_deref(), Some("planner"));
        let text = config.text.as_ref().expect("text deployment");
        assert_eq!(text.base_url, "http://localhost:8080/v1");
        assert_eq!(text.model, "prism-ml/bonsai-27b");
        assert_eq!(config.vision_provider_id.as_deref(), Some("vision"));
        let vision = config.vision.as_ref().expect("vision deployment");
        assert_eq!(vision.base_url, "http://localhost:8080/v1");
        assert_eq!(vision.model, "google/gemma-4-e4b");
        // The image deployment resolves from the same registry.
        assert_eq!(config.image.architecture, "qwen-image-2.1");
        // The derived planner `LlmConfig` carries the same endpoint.
        assert_eq!(
            config.planner_config().expect("planner config").model,
            text.model
        );
    }

    #[test]
    fn default_model_storage_is_os_local_even_without_a_config_file() {
        let path = std::env::temp_dir().join(format!("absent-{}.toml", uuid::Uuid::new_v4()));
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.image.model_dir, paths::model_dir().unwrap());
        assert!(config.image.executable.starts_with(&config.image.model_dir));
        assert!(config.image.lora_dir.starts_with(&config.image.model_dir));
        assert!(config.text.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn malformed_provider_sections_are_rejected_instead_of_becoming_defaults() {
        for settings in ["providers = 'invalid'", "defaults = 'invalid'"] {
            let value = toml::from_str(settings).unwrap();
            assert!(matches!(
                providers_from_value(&value),
                Err(ConfigError::Providers(_))
            ));
        }
    }

    #[test]
    fn one_overlay_preserves_app_settings_provider_choices_and_existing_models() {
        let directory = std::env::temp_dir().join(format!("svs-unified-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let base = directory.join("base.toml");
        let user = directory.join("storyfella.toml");
        std::fs::write(
            &base,
            std::fs::read_to_string(paths::config_path()).unwrap(),
        )
        .unwrap();
        std::fs::write(
            &user,
            "# retain this comment\n[providers.planner]\napi_key_env = 'TEST_PLANNER_KEY'\n",
        )
        .unwrap();
        AppConfig::persist_text_model_to(&user, "planner", "chosen-model").unwrap();
        AppConfig::persist_prewarm_to(&user, true).unwrap();
        let model_dir = directory.join("existing models");
        AppConfig::persist_model_dir_to(&user, "qwen-image", &model_dir).unwrap();
        let config = AppConfig::load_layered(base.clone(), Some(user.clone())).unwrap();
        let image_path = config.image.artifact_plan()[0].path.clone();
        std::fs::create_dir_all(image_path.parent().unwrap()).unwrap();
        std::fs::write(&image_path, b"existing bytes stay in place").unwrap();
        let mut defaults = config.generation_defaults.clone();
        defaults.steps = 7;
        AppConfig::persist_generation_defaults_to(&user, &defaults).unwrap();
        AppConfig::persist_image_residency_to(&user, "qwen-image", "lowvram").unwrap();
        let config = AppConfig::load_layered(base, Some(user.clone())).unwrap();
        assert_eq!(config.image.model_dir, model_dir);
        assert_eq!(config.text.as_ref().unwrap().model, "chosen-model");
        assert_eq!(config.generation_defaults.steps, 7);
        assert!(config.prewarm);
        assert!(crate::runtime::image_artifacts(&config)[0].is_present());
        assert_eq!(
            std::fs::read(&image_path).unwrap(),
            b"existing bytes stay in place"
        );
        let saved = std::fs::read_to_string(user).unwrap();
        assert!(saved.contains("retain this comment"));
        assert!(saved.contains("TEST_PLANNER_KEY"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migration_combines_old_settings_once_without_overwriting_or_deleting_sources() {
        let directory =
            std::env::temp_dir().join(format!("svs-migration-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let old_app = directory.join("app.toml");
        let old_providers = directory.join("providers.toml");
        let destination = directory.join("storyfella.toml");
        let original_app = "[generation]\nsteps = 9\n";
        let original_providers =
            "[providers.planner]\nmodel = 'chosen'\napi_key = 'test-only-secret'\n";
        std::fs::write(&old_app, original_app).unwrap();
        std::fs::write(&old_providers, "invalid = [").unwrap();
        assert!(migrate_user_config(&destination).is_err());
        assert!(!destination.exists());
        std::fs::write(&old_providers, original_providers).unwrap();
        migrate_user_config(&destination).unwrap();
        let value = read_config_value(&destination).unwrap();
        assert_eq!(value["generation"]["steps"].as_integer(), Some(9));
        assert_eq!(
            value["providers"]["planner"]["api_key"].as_str(),
            Some("test-only-secret")
        );
        assert_eq!(std::fs::read_to_string(old_app).unwrap(), original_app);
        assert_eq!(
            std::fs::read_to_string(&old_providers).unwrap(),
            original_providers
        );
        std::fs::write(&destination, "[generation]\nsteps = 12\n").unwrap();
        migrate_user_config(&destination).unwrap();
        assert_eq!(
            read_config_value(&destination).unwrap()["generation"]["steps"].as_integer(),
            Some(12)
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 3);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn an_image_only_registry_leaves_chat_capabilities_unconfigured() {
        // The planner and vision endpoints are soft dependencies: a registry
        // that serves only images resolves no `text`/`vision` deployment rather
        // than failing, so the app still loads and those features degrade.
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let base = dir.join("providers.toml");
        std::fs::write(
            &base,
            "[defaults]\nt2i = \"img\"\n\
             [providers.img]\nkind = \"sdcpp\"\narchitecture = \"qwen-image-2.1\"\n\
             capabilities = [\"t2i\"]\nbase_url = \"http://127.0.0.1:7861\"\n",
        )
        .expect("fixture should write");

        let providers = providers_from_value(&read_config_value(&base).unwrap())
            .expect("image-only registry loads");
        assert!(providers.image_deployment(Capability::T2i).is_ok());
        assert!(
            providers.chat_deployment(Capability::Text).is_err(),
            "no text provider resolves no deployment"
        );
        assert!(providers.chat_deployment(Capability::Vision).is_err());
    }

    #[test]
    fn a_user_providers_overlay_overrides_a_provider_model() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let base = dir.join("providers.toml");
        let user = dir.join("user-providers.toml");
        std::fs::write(
            &base,
            "[defaults]\ntext = \"planner\"\nvision = \"vision\"\n\
             [providers.planner]\nkind = \"openai\"\ncapabilities = [\"text\"]\n\
             base_url = \"http://localhost:8080/v1\"\nmodel = \"prism-ml/bonsai-27b\"\n\
             [providers.vision]\nkind = \"openai\"\ncapabilities = [\"vision\"]\n\
             base_url = \"http://localhost:8080/v1\"\nmodel = \"google/gemma-4-e4b\"\n",
        )
        .expect("base fixture should write");
        std::fs::write(
            &user,
            "[providers.planner]\nmodel = \"deepseek-chat\"\napi_key_env = \"DEEPSEEK_API_KEY\"\n",
        )
        .expect("user fixture should write");

        let mut value = read_config_value(&base).unwrap();
        merge_toml(&mut value, read_config_value(&user).unwrap());
        let providers = providers_from_value(&value).expect("providers should merge");
        let text = providers
            .chat_deployment(Capability::Text)
            .expect("text resolves");
        assert_eq!(text.model, "deepseek-chat");
        assert_eq!(text.base_url, "http://localhost:8080/v1");
        // The untouched vision provider survives the merge.
        let vision = providers
            .chat_deployment(Capability::Vision)
            .expect("vision resolves");
        assert_eq!(vision.model, "google/gemma-4-e4b");
    }

    #[test]
    fn a_nested_lora_binding_survives_the_persist_and_reload_cycle() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(&path, "[paths]\ncache_dir = \"cache\"\n").expect("fixture should write");

        let mut defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 25,
            cfg_scale: sd::DEFAULT_CFG_SCALE,
            sample_method: sd::DEFAULT_SAMPLE_METHOD.to_owned(),
            loras: Default::default(),
        };
        defaults.loras.kinds.character = Some(LoraBinding {
            file: "characters/mia-v2.safetensors".into(),
        });
        AppConfig::persist_generation_defaults_to(&path, &defaults)
            .expect("defaults should persist");
        let reloaded = AppConfig::load_from(&path).expect("persisted file should load");
        assert_eq!(
            reloaded
                .generation_defaults
                .loras
                .kinds
                .character
                .map(|binding| binding.file)
                .as_deref(),
            Some("characters/mia-v2.safetensors"),
            "a nested (symlinked-folder) binding must survive a reload"
        );
    }

    #[test]
    fn persist_text_model_writes_under_the_provider_section_line_preservingly() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("providers.toml");
        std::fs::write(
            &path,
            "# keep me\n[providers.planner]\nkind = \"openai\"\nmodel = \"old-model\"\n\n\
             [providers.vision]\nmodel = \"google/gemma-4-e4b\"\n",
        )
        .expect("fixture should write");

        AppConfig::persist_text_model_to(&path, "planner", "deepseek-chat")
            .expect("persist should write");
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(!contents.contains("old-model"), "{contents}");
        assert!(contents.contains("model = \"deepseek-chat\""), "{contents}");
        assert!(
            contents.contains("kind = \"openai\""),
            "unrelated keys must survive: {contents}"
        );
        assert!(
            contents.contains("model = \"google/gemma-4-e4b\""),
            "a sibling provider must be untouched: {contents}"
        );
        assert!(
            contents.contains("# keep me"),
            "comments must survive: {contents}"
        );
        assert!(
            contents.find("[providers.planner]") < contents.find("[providers.vision]"),
            "section order must survive: {contents}"
        );
    }

    #[test]
    fn persist_text_model_adds_the_provider_section_when_missing() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("providers.toml");
        std::fs::write(&path, "[defaults]\ntext = \"planner\"\n").expect("fixture should write");

        AppConfig::persist_text_model_to(&path, "planner", "new-model")
            .expect("persist should write");
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(contents.contains("[providers.planner]"), "{contents}");
        assert!(contents.contains("model = \"new-model\""), "{contents}");
        assert!(
            contents.contains("text = \"planner\""),
            "defaults must survive: {contents}"
        );
    }

    #[test]
    fn the_user_config_layers_over_the_base_file() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let base = dir.join("app.toml");
        let user = dir.join("user.toml");
        std::fs::write(
            &base,
            "[generation]\nsteps = 25\n\
             [generation.sizes]\nenvironment = \"1024x576\"\n",
        )
        .expect("base fixture should write");
        std::fs::write(
            &user,
            "[generation]\ncfg_scale = 7.5\n\
             [generation.sizes]\ncharacter = \"384x576\"\n",
        )
        .expect("user fixture should write");

        let config =
            AppConfig::load_layered(base, Some(user.clone())).expect("layered config should load");
        // The user file sets one nested key without clobbering the base's.
        assert_eq!(config.generation_defaults.steps, 25);
        assert_eq!(config.generation_defaults.cfg_scale, 7.5);
        assert_eq!(
            config.generation_defaults.sizes.environment,
            Some((1024, 576))
        );
        assert_eq!(config.generation_defaults.sizes.character, Some((384, 576)));
        assert_eq!(config.user_config_path.as_deref(), Some(user.as_path()));
    }

    #[test]
    fn the_user_config_wins_on_a_conflicting_key() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let base = dir.join("app.toml");
        let user = dir.join("user.toml");
        std::fs::write(&base, "[generation]\nsteps = 25\n").expect("base fixture");
        std::fs::write(&user, "[generation]\nsteps = 8\n").expect("user fixture");

        let config = AppConfig::load_layered(base, Some(user)).expect("layered config should load");
        assert_eq!(config.generation_defaults.steps, 8);
    }

    #[test]
    fn a_missing_user_file_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let base = dir.join("app.toml");
        std::fs::write(&base, "[generation]\nsteps = 10\n").expect("base fixture");

        let config = AppConfig::load_layered(base, Some(dir.join("absent.toml")))
            .expect("a missing user config is fine");
        assert_eq!(config.generation_defaults.steps, 10);
    }

    #[test]
    fn persisting_creates_the_user_config_directory() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        let path = dir.join("nested").join("providers.toml");

        AppConfig::persist_text_model_to(&path, "planner", "deepseek-chat")
            .expect("persist should create the directory and write");
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(contents.contains("[providers.planner]"), "{contents}");
        assert!(contents.contains("model = \"deepseek-chat\""), "{contents}");
    }

    #[test]
    fn image_residency_persists_under_the_provider_section() {
        let dir = std::env::temp_dir().join(format!("svs-residency-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("providers.toml");
        std::fs::write(&path, "[providers.qwen-image]\nkind = \"sdcpp\"\n").expect("seed");

        AppConfig::persist_image_residency_to(&path, "qwen-image", "lowvram").expect("persist");
        let contents = std::fs::read_to_string(&path).expect("read");
        assert!(contents.contains("residency = \"lowvram\""), "{contents}");
        assert!(contents.contains("kind = \"sdcpp\""), "{contents}");

        // Re-persisting replaces rather than duplicates the key.
        AppConfig::persist_image_residency_to(&path, "qwen-image", "resident").expect("persist");
        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(contents.matches("residency =").count(), 1, "{contents}");
        assert!(contents.contains("residency = \"resident\""), "{contents}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prewarm_persists_and_round_trips_through_the_file_config() {
        let dir = std::env::temp_dir().join(format!("svs-prewarm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("app.toml");

        AppConfig::persist_prewarm_to(&path, true).expect("persist");
        let parsed: FileConfig =
            toml::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        assert!(parsed.runtime.prewarm);

        AppConfig::persist_prewarm_to(&path, false).expect("persist");
        let parsed: FileConfig =
            toml::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        assert!(!parsed.runtime.prewarm);
        std::fs::remove_dir_all(&dir).ok();
    }
}
