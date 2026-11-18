use std::{
    fs,
    path::{Path, PathBuf},
};

use agentkit_ai::deployments::{
    Capability, ChatDeployment, ImageDeployment, ProviderError, ProviderKind, Providers,
};
use agentkit_ai::native::sd;
use serde::Deserialize;
use thiserror::Error;

use crate::{
    errors::{CodedError, ErrorCode},
    llm::LlmConfig,
    paths,
    registry::{GenerationDefaults, KindLoraBindings, LoraBinding, LoraBindings, SizeDefaults},
};

/// A configured provider that serves a chat capability, with the endpoint it
/// resolves to. One entry per `[providers.<id>]` of a chat kind (`openai` or
/// `llamacpp`); `id` is the value `[defaults]` selects and the key the Settings
/// provider picker persists.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatProvider {
    /// The provider id (`[providers.<id>]`).
    pub id: String,
    /// How the provider runs: `openai` is an external endpoint the app never
    /// starts, `llamacpp` a managed server it does.
    pub kind: ProviderKind,
    /// The capabilities the provider declares (`text`, `vision`).
    pub capabilities: Vec<String>,
    /// The endpoint, model, key and timeout this provider resolves to.
    pub deployment: ChatDeployment,
}

impl ChatProvider {
    /// A one-line description for the Settings picker: the id to correlate with
    /// the config file, the provider kind, and the endpoint. It makes an opaque
    /// provider id legible as the endpoint it names.
    pub fn label(&self) -> String {
        format!("{} · {} · {}", self.id, self.kind, self.deployment.base_url)
    }
}

/// A chat capability the Settings provider editor can assign.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatCapability {
    Text,
    Vision,
}

impl ChatCapability {
    /// The capability name written to `capabilities` and `[defaults]`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Vision => "vision",
        }
    }

    /// The human label for the capability.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text (LLM)",
            Self::Vision => "Vision (LLM)",
        }
    }
}

/// Studio appearance. System is resolved by the desktop host, never core.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    Dark,
    Light,
    System,
}

impl ThemePreference {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::System];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::System => "System",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageProvider {
    pub id: String,
    pub model: String,
    pub capabilities: Vec<Capability>,
    pub deployment: Option<ImageDeployment>,
    pub hosted: Option<HostedImageProvider>,
}

#[derive(Clone, PartialEq)]
pub struct HostedImageProvider {
    pub kind: ProviderKind,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub capabilities: Vec<Capability>,
}

impl std::fmt::Debug for HostedImageProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostedImageProvider")
            .field("kind", &self.kind)
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppConfig {
    pub theme: ThemePreference,
    pub preview: PreviewSettings,
    /// False routes Vision through Text while retaining the shared registry's
    /// separate Vision selection for when the user enables it again.
    pub separate_vision: bool,
    configured_vision_provider_id: Option<String>,
    /// The `text` deployment resolved from the shared `clipforge.toml` — the
    /// planner's OpenAI-compatible endpoint, model, key and timeout. `None` when
    /// no provider serves `text`: the planner is a soft dependency, so the app
    /// still runs and its LLM steps degrade to manual input.
    pub text: Option<ChatDeployment>,
    /// Effective Vision deployment for guide vision steps.
    /// Resolves from the shared Vision selection in separate mode, or follows
    /// Text otherwise. None when that selected deployment is unconfigured.
    pub vision: Option<ChatDeployment>,
    /// The Studio-selected text provider id, or the shared default when no
    /// Studio selection has been saved.
    pub text_provider_id: Option<String>,
    /// Effective Vision provider id (the Text id when linked).
    pub vision_provider_id: Option<String>,
    /// The provider id serving `t2i`, kept so Settings can persist the
    /// residency choice under the right `[providers.<id>]` section.
    pub image_provider_id: Option<String>,
    pub image_edit_provider_id: Option<String>,
    /// The optional native image-generation deployment resolved from the shared
    /// `clipforge.toml`: the `sd-server` executable, model tree, LoRA root,
    /// endpoint, and weights. Hosted or absent t2i defaults leave this unset;
    /// authoring and playback do not require a native image engine.
    pub image: Option<ImageDeployment>,
    /// Image generation providers available in the shared registry.
    pub image_providers: Vec<ImageProvider>,
    /// Every configured provider that serves a chat capability — `openai` or
    /// `llamacpp` — in id order, with the endpoint each resolves to. Both the
    /// text and vision pickers choose from this one list.
    pub chat_providers: Vec<ChatProvider>,
    pub cache_dir: PathBuf,
    /// The shared native-server state directory (see [`paths::state_dir`]) —
    /// process markers and detached-server logs, shared by every front end so a
    /// resident server is reused rather than started twice.
    pub state_dir: PathBuf,
    pub asset_dir: PathBuf,
    /// Where the GUI keeps `.svs-project.db` files (the Projects screen).
    pub project_dir: PathBuf,
    /// The app config file (see [`paths::config_path`]) — where the app's own
    /// configuration is stored. `None` when the platform
    /// has no config directory. The Settings screen shows it.
    pub app_config_path: Option<PathBuf>,
    /// The machine's shared AI configuration (see
    /// [`paths::clipforge_config_path`]). Provider edits persist here, so every
    /// front end on the machine shares one registry. `None` when the platform
    /// has no config directory.
    pub ai_config_path: Option<PathBuf>,
    /// The configurable sizes, step count, and LoRA bindings that shape
    /// CLI generation, loaded from the application configuration file.
    pub generation_defaults: GenerationDefaults,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not migrate Studio's application data: {0:#}")]
    Identity(#[from] anyhow::Error),
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
    #[error("A local image provider is not selected. Configure one in Clipforge and select it in Studio Settings.")]
    NativeImageUnavailable,
    #[error("invalid model selection: {0}")]
    InvalidModelSelection(String),
}

impl CodedError for ConfigError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::PlatformDirectories => ErrorCode::ConfigPlatformDirectories,
            Self::Read { .. } => ErrorCode::ConfigRead,
            Self::Parse { .. } => ErrorCode::ConfigParse,
            Self::Write { .. } | Self::Identity(_) => ErrorCode::ConfigWrite,
            Self::InvalidSize { .. } => ErrorCode::ConfigInvalidSize,
            Self::MalformedSize { .. } => ErrorCode::ConfigMalformedSize,
            Self::Serialize { .. } => ErrorCode::ConfigSerialize,
            Self::Providers(_) => ErrorCode::ConfigProviders,
            Self::NativeImageUnavailable => ErrorCode::ConfigNativeImageUnavailable,
            Self::InvalidModelSelection(_) => ErrorCode::ConfigProviders,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    #[serde(default)]
    appearance: AppearanceConfig,
    #[serde(default)]
    preview: PreviewSettings,
    #[serde(default)]
    models: ModelsConfig,
    #[serde(default)]
    paths: PathOverrides,
    #[serde(default)]
    generation: GenerationFileConfig,
    #[serde(default)]
    lora: LoraFileConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PreviewSettings {
    pub autoplay_seconds: u32,
    pub text_percent: u32,
    pub contain_artwork: bool,
    pub show_diagnostics: bool,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            autoplay_seconds: 3,
            text_percent: 100,
            contain_artwork: false,
            show_diagnostics: true,
        }
    }
}

impl PreviewSettings {
    fn bounded(self) -> Self {
        Self {
            autoplay_seconds: self.autoplay_seconds.clamp(1, 30),
            text_percent: self.text_percent.clamp(75, 150),
            ..self
        }
    }

    /// Read Studio's playback preferences without initializing inference providers.
    pub fn load() -> Result<Self, ConfigError> {
        let path = paths::config_path().ok_or(ConfigError::PlatformDirectories)?;
        let contents = read_config_contents(&path)?;
        let file: FileConfig =
            toml::from_str(&contents).map_err(|source| ConfigError::Parse { path, source })?;
        Ok(file.preview.bounded())
    }
}

#[derive(Debug, Default, Deserialize)]
struct AppearanceConfig {
    #[serde(default)]
    theme: ThemePreference,
}

#[derive(Debug, Default, Deserialize)]
struct ModelsConfig {
    /// Absent preserves existing Vision routing, including an unconfigured
    /// capability. Provider credentials remain in the shared AI registry.
    separate_vision: Option<bool>,
    text_provider: Option<String>,
    vision_provider: Option<String>,
    text_model_provider: Option<String>,
    text_model: Option<String>,
    vision_model_provider: Option<String>,
    vision_model: Option<String>,
    image_provider: Option<String>,
    image_edit_provider: Option<String>,
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
    /// Loads the effective config: this app's settings from the platform
    /// `studio.toml` (created with defaults when absent; see [`paths::config_path`])
    /// and the inference settings from the machine's shared `clipforge.toml`.
    pub fn load() -> Result<Self, ConfigError> {
        if std::env::var_os(paths::APP_CONFIG_ENV).is_none_or(|value| value.is_empty()) {
            crate::identity::migrate()?;
        }
        Self::load_at(paths::config_path(), paths::clipforge_config_path())
    }

    fn load_at(app: Option<PathBuf>, ai: Option<PathBuf>) -> Result<Self, ConfigError> {
        migrate_legacy_settings(app.as_deref(), ai.as_deref())?;
        if let Some(path) = app.as_deref() {
            seed_app_config(path)?;
        }
        if let Some(path) = ai.as_deref() {
            crate::ai_config::migrate_legacy_config_at(path);
        }
        Self::load_split(app, ai)
    }

    /// Loads one complete settings file (both app and inference sections)
    /// without the machine's user overlay. The one-file path used by tests.
    pub fn load_from(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::load_layered(path.as_ref().to_path_buf(), None)
    }

    /// The planner's [`LlmConfig`] — the `text` provider deployment, or `None`
    /// when no provider serves `text` (the planner then degrades to manual
    /// input rather than failing).
    pub fn planner_config(&self) -> Option<LlmConfig> {
        self.text.as_ref().map(LlmConfig::from)
    }

    pub fn native_image(&self) -> Result<&ImageDeployment, ConfigError> {
        self.image
            .as_ref()
            .ok_or(ConfigError::NativeImageUnavailable)
    }

    /// The production loader: this app's settings from the platform `app` config
    /// file, and the provider registry from the shared inference file `ai`.
    fn load_split(app: Option<PathBuf>, ai: Option<PathBuf>) -> Result<Self, ConfigError> {
        let value = match app.as_deref() {
            Some(path) => read_config_value(path)?,
            None => toml::Value::Table(toml::Table::new()),
        };
        let parse_path = app
            .clone()
            .unwrap_or_else(|| PathBuf::from(paths::CONFIG_FILE));
        let file: FileConfig = value.try_into().map_err(|source| ConfigError::Parse {
            path: parse_path.clone(),
            source,
        })?;
        // The inference half is its own shared file (created above when absent),
        // so a provider edit is visible to every front end on the machine.
        let providers = match ai.as_deref() {
            Some(ai_path) => crate::ai_config::shared_registry_at(ai_path)?,
            None => Providers::from_toml("", Path::new("."))?,
        };
        Self::assemble(providers, file, &parse_path, app, ai)
    }

    /// Reads `base`, merges `user` over it (deeply; see [`merge_toml`]), and
    /// resolves the result as one file that carries both app and inference
    /// settings. A missing file contributes nothing rather than failing, so the
    /// default config needs neither file to exist. Test-only: production reads
    /// the app and inference halves from separate files (see
    /// [`AppConfig::load_split`]).
    fn load_layered(base: PathBuf, user: Option<PathBuf>) -> Result<Self, ConfigError> {
        let path = base.as_path();
        let mut value = read_config_value(path)?;
        if let Some(overlay) = user.as_deref() {
            if overlay != path {
                let overlay_value = read_config_value(overlay)?;
                merge_toml(&mut value, overlay_value);
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
        let providers = providers_from_value(&value, provider_base(path))?;
        Self::assemble(providers, file, path, user, None)
    }

    /// Resolves a parsed app file and a validated provider registry into the
    /// effective config. `base` names the app file for size errors; `app_config`
    /// and `ai_config` are the paths Settings persists to.
    fn assemble(
        providers: Providers,
        file: FileConfig,
        base: &Path,
        app_config: Option<PathBuf>,
        ai_config: Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let path = base;
        let image_providers: Vec<_> = providers
            .list()
            .filter(|(_, provider)| {
                provider
                    .capabilities
                    .iter()
                    .any(|cap| matches!(cap, Capability::T2i | Capability::Edit))
            })
            .filter_map(|(id, provider)| {
                if provider.kind == ProviderKind::Sdcpp {
                    let capability = if provider.capabilities.contains(&Capability::T2i) {
                        Capability::T2i
                    } else {
                        Capability::Edit
                    };
                    let deployment = providers.image_deployment_for(id, capability).ok()?;
                    Some(ImageProvider {
                        id: id.to_owned(),
                        model: deployment.architecture.clone(),
                        capabilities: provider.capabilities.clone(),
                        deployment: Some(deployment),
                        hosted: None,
                    })
                } else if matches!(
                    provider.kind,
                    ProviderKind::Fal
                        | ProviderKind::Replicate
                        | ProviderKind::Alibaba
                        | ProviderKind::GptImage
                ) {
                    let model = provider.model.clone()?;
                    Some(ImageProvider {
                        id: id.to_owned(),
                        model: model.clone(),
                        capabilities: provider.capabilities.clone(),
                        deployment: None,
                        hosted: Some(HostedImageProvider {
                            kind: provider.kind,
                            model,
                            base_url: provider.base_url.clone(),
                            api_key: provider.api_key.clone(),
                            api_key_env: provider.api_key_env.clone(),
                            timeout_seconds: provider.timeout_seconds,
                            capabilities: provider.capabilities.clone(),
                        }),
                    })
                } else {
                    None
                }
            })
            .collect();
        let image_provider_id = file.models.image_provider.clone().or_else(|| {
            providers
                .default_provider(Capability::T2i)
                .filter(|id| {
                    image_providers.iter().any(|provider| {
                        provider.id == *id && provider.capabilities.contains(&Capability::T2i)
                    })
                })
                .map(str::to_owned)
        });
        let image = image_provider_id
            .as_ref()
            .and_then(|id| {
                image_providers.iter().find(|provider| {
                    &provider.id == id && provider.capabilities.contains(&Capability::T2i)
                })
            })
            .and_then(|provider| provider.deployment.clone());
        let image_edit_provider_id = file.models.image_edit_provider.clone().or_else(|| {
            providers
                .default_provider(Capability::Edit)
                .filter(|id| {
                    image_providers
                        .iter()
                        .any(|p| p.id == *id && p.capabilities.contains(&Capability::Edit))
                })
                .map(str::to_owned)
                .or_else(|| {
                    image_provider_id
                        .as_ref()
                        .filter(|id| {
                            image_providers
                                .iter()
                                .any(|p| &p.id == *id && p.capabilities.contains(&Capability::Edit))
                        })
                        .cloned()
                })
        });
        // The chat capabilities are optional: an unconfigured planner or vision
        // endpoint leaves the app fully usable, with those features degraded.
        let chat_providers = chat_providers(&providers);
        let text_provider_id = file.models.text_provider.clone().or_else(|| {
            providers
                .default_provider(Capability::Text)
                .map(str::to_owned)
        });
        let configured_vision_provider_id = file.models.vision_provider.clone().or_else(|| {
            providers
                .default_provider(Capability::Vision)
                .map(str::to_owned)
        });
        let mut text = active_deployment(&chat_providers, &text_provider_id, Capability::Text);
        let mut configured_vision = active_deployment(
            &chat_providers,
            &configured_vision_provider_id,
            Capability::Vision,
        );
        let same_model = text.is_some()
            && configured_vision_provider_id == text_provider_id
            && configured_vision == text;
        let separate_vision = file.models.separate_vision.unwrap_or(!same_model);
        if file.models.text_model_provider == text_provider_id {
            if let (Some(deployment), Some(model)) = (&mut text, &file.models.text_model) {
                if deployment.server.is_none() && !model.trim().is_empty() {
                    deployment.model.clone_from(model);
                }
            }
        }
        if file.models.vision_model_provider == configured_vision_provider_id {
            if let (Some(deployment), Some(model)) =
                (&mut configured_vision, &file.models.vision_model)
            {
                if deployment.server.is_none() && !model.trim().is_empty() {
                    deployment.model.clone_from(model);
                }
            }
        }
        let (vision, vision_provider_id) = if separate_vision {
            (configured_vision, configured_vision_provider_id.clone())
        } else {
            (
                text.as_ref()
                    .filter(|_| {
                        active_deployment(&chat_providers, &text_provider_id, Capability::Vision)
                            .is_some()
                    })
                    .cloned(),
                text_provider_id.clone(),
            )
        };
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
            dir: match &image {
                Some(image) => image.lora_dir.clone(),
                None => paths::model_dir()
                    .ok_or(ConfigError::PlatformDirectories)?
                    .join(sd::DEFAULT_LORA_DIR),
            },
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
            theme: file.appearance.theme,
            preview: file.preview.bounded(),
            separate_vision,
            configured_vision_provider_id,
            text,
            vision,
            text_provider_id,
            vision_provider_id,
            image_provider_id,
            image_edit_provider_id,
            image,
            image_providers,
            chat_providers,
            cache_dir: file
                .paths
                .cache_dir
                .map(paths::resolve)
                .unwrap_or_else(|| dirs.cache_dir().to_path_buf()),
            state_dir: paths::state_dir().unwrap_or_else(|| data_dir.join("inference-state")),
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
            app_config_path: app_config,
            ai_config_path: ai_config,
            generation_defaults: GenerationDefaults {
                sizes,
                steps: file.generation.steps,
                cfg_scale: file.generation.cfg_scale,
                sample_method: file.generation.sample_method,
                loras: lora_bindings,
            },
        })
    }

    /// Persist application preferences without rewriting unrelated settings.
    fn persist_app_setting(
        &self,
        section: &str,
        key: &str,
        value: String,
    ) -> Result<(), ConfigError> {
        let path = self
            .app_config_path
            .as_deref()
            .ok_or(ConfigError::PlatformDirectories)?;
        let contents = read_config_contents(path)?;
        let mut lines: Vec<_> = contents.lines().map(str::to_owned).collect();
        write_key(&mut lines, section, key, Some(value));
        write_config_file(path, lines.join("\n"))
    }

    pub fn persist_theme(&self, theme: ThemePreference) -> Result<(), ConfigError> {
        self.persist_app_setting("appearance", "theme", format!("{:?}", theme.as_str()))
    }

    pub fn persist_preview_settings(&self, settings: PreviewSettings) -> Result<(), ConfigError> {
        let path = self
            .app_config_path
            .as_deref()
            .ok_or(ConfigError::PlatformDirectories)?;
        let contents = read_config_contents(path)?;
        let mut lines: Vec<_> = contents.lines().map(str::to_owned).collect();
        let settings = settings.bounded();
        for (key, value) in [
            ("autoplay_seconds", settings.autoplay_seconds.to_string()),
            ("text_percent", settings.text_percent.to_string()),
            ("contain_artwork", settings.contain_artwork.to_string()),
            ("show_diagnostics", settings.show_diagnostics.to_string()),
        ] {
            write_key(&mut lines, "preview", key, Some(value));
        }
        write_config_file(path, lines.join("\n"))
    }

    pub fn persist_separate_vision(&self, separate: bool) -> Result<(), ConfigError> {
        self.persist_app_setting("models", "separate_vision", separate.to_string())
    }

    /// Save a Studio selection of a configured image generation provider.
    pub fn persist_image_provider_selection(
        &self,
        capability: Capability,
        id: &str,
    ) -> Result<(), ConfigError> {
        if !matches!(capability, Capability::T2i | Capability::Edit)
            || !self
                .image_providers
                .iter()
                .any(|provider| provider.id == id && provider.capabilities.contains(&capability))
        {
            return Err(ProviderError::UnknownProvider { id: id.to_owned() }.into());
        }
        self.persist_app_setting(
            "models",
            if capability == Capability::Edit {
                "image_edit_provider"
            } else {
                "image_provider"
            },
            toml::Value::String(id.to_owned()).to_string(),
        )
    }

    /// Save a Studio-owned provider selection from the shared registry.
    pub fn persist_provider_selection(
        &self,
        capability: ChatCapability,
        provider: &str,
    ) -> Result<(), ConfigError> {
        let configured = self
            .chat_providers
            .iter()
            .find(|entry| entry.id == provider)
            .ok_or_else(|| ProviderError::UnknownProvider {
                id: provider.to_owned(),
            })?;
        if !configured
            .capabilities
            .iter()
            .any(|item| item == capability.as_str())
        {
            let capability = match capability {
                ChatCapability::Text => Capability::Text,
                ChatCapability::Vision => Capability::Vision,
            };
            return Err(ProviderError::UnsupportedCapability {
                id: provider.to_owned(),
                kind: configured.kind,
                capability,
            }
            .into());
        }
        let path = self
            .app_config_path
            .as_deref()
            .ok_or(ConfigError::PlatformDirectories)?;
        let contents = read_config_contents(path)?;
        let mut lines: Vec<_> = contents.lines().map(str::to_owned).collect();
        let provider_key = match capability {
            ChatCapability::Text => "text_provider",
            ChatCapability::Vision => "vision_provider",
        };
        write_key(
            &mut lines,
            "models",
            provider_key,
            Some(toml::Value::String(provider.to_owned()).to_string()),
        );
        write_config_file(path, lines.join("\n"))
    }

    /// Keep each capability's selected model in Studio's own configuration.
    pub fn persist_model_selection(
        &self,
        capability: ChatCapability,
        model: &str,
    ) -> Result<(), ConfigError> {
        let (provider_id, deployment, provider_key, model_key) = match capability {
            ChatCapability::Text => (
                &self.text_provider_id,
                &self.text,
                "text_model_provider",
                "text_model",
            ),
            ChatCapability::Vision => (
                &self.vision_provider_id,
                &self.vision,
                "vision_model_provider",
                "vision_model",
            ),
        };
        let provider = provider_id
            .as_deref()
            .ok_or_else(|| ConfigError::InvalidModelSelection("Select a provider first".into()))?;
        let deployment = deployment.as_ref().ok_or_else(|| {
            ConfigError::InvalidModelSelection(
                "The selected provider does not serve this capability".into(),
            )
        })?;
        if model.trim().is_empty() || (deployment.server.is_some() && model != deployment.model) {
            return Err(ConfigError::InvalidModelSelection(
                "Select an available model for this provider".into(),
            ));
        }
        let path = self
            .app_config_path
            .as_deref()
            .ok_or(ConfigError::PlatformDirectories)?;
        let contents = read_config_contents(path)?;
        let mut lines: Vec<_> = contents.lines().map(str::to_owned).collect();
        write_key(
            &mut lines,
            "models",
            provider_key,
            Some(toml::Value::String(provider.to_owned()).to_string()),
        );
        write_key(
            &mut lines,
            "models",
            model_key,
            Some(toml::Value::String(model.to_owned()).to_string()),
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
/// validation and path resolution.
/// Relative paths in the file resolve against `base` — the file's own directory.
fn providers_from_value(value: &toml::Value, base: &Path) -> Result<Providers, ConfigError> {
    let mut registry = toml::Table::new();
    for key in ["defaults", "providers"] {
        if let Some(section) = value.get(key) {
            registry.insert(key.into(), section.clone());
        }
    }
    let contents =
        toml::to_string(&registry).map_err(|source| ConfigError::Serialize { source })?;
    Providers::from_toml(&contents, base).map_err(ConfigError::Providers)
}

/// The deployment the selected provider id resolves to.
fn active_deployment(
    providers: &[ChatProvider],
    id: &Option<String>,
    capability: Capability,
) -> Option<ChatDeployment> {
    let id = id.as_deref()?;
    providers
        .iter()
        .find(|provider| {
            provider.id == id
                && provider
                    .capabilities
                    .iter()
                    .any(|item| item == capability.as_str())
        })
        .map(|provider| provider.deployment.clone())
}

/// Every configured provider that serves `capability`, resolved to its chat
/// deployment, in id order. A provider that names the capability but cannot be
/// resolved to a deployment is skipped: a validated registry should not contain
/// one, and one bad provider should not sink the whole provider list.
fn chat_providers(providers: &Providers) -> Vec<ChatProvider> {
    providers
        .list()
        .filter(|(_, provider)| {
            matches!(provider.kind, ProviderKind::Openai | ProviderKind::LlamaCpp)
        })
        .filter_map(|(id, provider)| {
            let capability = if provider.capabilities.contains(&Capability::Text) {
                Capability::Text
            } else {
                Capability::Vision
            };
            providers
                .chat_deployment_for(id, capability)
                .ok()
                .map(|deployment| ChatProvider {
                    id: id.to_owned(),
                    kind: provider.kind,
                    capabilities: provider
                        .capabilities
                        .iter()
                        .map(|capability| capability.as_str().to_owned())
                        .collect(),
                    deployment,
                })
        })
        .collect()
}

/// The directory a shared AI config's relative paths resolve against.
fn provider_base(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// The app config's top-level sections — what [`migrate_legacy_settings`] keeps
/// when it splits a legacy unified file into the app half.
const APP_SECTIONS: [&str; 4] = ["paths", "generation", "lora", "runtime"];

/// The inference config's top-level sections.
const AI_SECTIONS: [&str; 2] = ["defaults", "providers"];

/// One-time migration, not a second runtime config format. A machine's pre-split
/// settings — the unified user `storyfella.toml`, or the older `app.toml` and
/// `providers.toml` — are split into the app file (`studio.toml`) and the shared
/// AI config (`clipforge.toml`). Originals remain for recovery, the new files are
/// authoritative once present, and an existing `clipforge.toml` is never
/// overwritten (its contents win).
fn migrate_legacy_settings(app: Option<&Path>, ai: Option<&Path>) -> Result<(), ConfigError> {
    let Some(app) = app else {
        return Ok(());
    };
    if app.exists() {
        return Ok(());
    }
    let Some(directory) = app.parent() else {
        return Ok(());
    };
    let old_app = directory.join("app.toml");
    let old_unified = directory.join("storyfella.toml");
    let old_providers = directory.join("providers.toml");
    if !old_app.exists() && !old_unified.exists() && !old_providers.exists() {
        return Ok(());
    }

    // The unified file is the newest, so it wins over the older split files
    // (`app.toml`, then `providers.toml`). Each legacy file may carry either kind
    // of section, so both halves are selected from all of them.
    let unified = read_config_value(&old_unified)?;
    let legacy_app = read_config_value(&old_app)?;
    let legacy_providers = read_config_value(&old_providers)?;

    let mut app_half = app_defaults();
    merge_toml(&mut app_half, select_sections(&legacy_app, &APP_SECTIONS));
    merge_toml(
        &mut app_half,
        select_sections(&legacy_providers, &APP_SECTIONS),
    );
    merge_toml(&mut app_half, select_sections(&unified, &APP_SECTIONS));

    let mut inference = select_sections(&legacy_app, &AI_SECTIONS);
    merge_toml(
        &mut inference,
        select_sections(&legacy_providers, &AI_SECTIONS),
    );
    merge_toml(&mut inference, select_sections(&unified, &AI_SECTIONS));

    write_migrated(app, &app_half)?;
    if let Some(ai) = ai {
        if !ai.exists() && !inference.as_table().is_some_and(toml::Table::is_empty) {
            write_migrated(ai, &inference)?;
        }
    }
    Ok(())
}

/// The top-level `keys` of `value`, as a new table.
fn select_sections(value: &toml::Value, keys: &[&str]) -> toml::Value {
    let mut selected = toml::Table::new();
    for key in keys {
        if let Some(section) = value.get(*key) {
            selected.insert((*key).into(), section.clone());
        }
    }
    toml::Value::Table(selected)
}

/// The app settings a machine starts from: the created platform `studio.toml`
/// carries them, and a migrated one keeps them beneath its legacy values. A key
/// not listed here falls back to the built-in code default.
const APP_DEFAULTS: &str = "[generation.sizes]\ncharacter = \"1024x1536\"\n";

/// Parses [`APP_DEFAULTS`].
fn app_defaults() -> toml::Value {
    toml::from_str(APP_DEFAULTS).expect("built-in app defaults are valid TOML")
}

/// Writes `contents` to `path` if no file is there yet, creating its directory
/// first. A file another process wrote is left alone.
fn write_new_config(path: &Path, contents: &str) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    atomic_write_config(path, contents, true)
}

/// Publishes a migrated file without replacing one another process wrote.
fn write_migrated(path: &Path, value: &toml::Value) -> Result<(), ConfigError> {
    let contents =
        toml::to_string_pretty(value).map_err(|source| ConfigError::Serialize { source })?;
    write_new_config(path, &contents)
}

/// Writes the app config with Storyfella's defaults the first time it is needed,
/// so a fresh machine keeps the portrait character canvas. An existing file is
/// never touched.
fn seed_app_config(path: &Path) -> Result<(), ConfigError> {
    if path.exists() {
        return Ok(());
    }
    let header = "\
# Storyfella Studio's app settings. The machine's shared inference configuration\n\
# — the provider registry — lives separately in clipforge.toml at the platform\n\
# config directory and managed in Clipforge.\n\
#\n\
# Qwen-Image-2.1 has no few-step turbo path: it is sampled with CFG at ~25\n\
# steps, and portrait characters use the full portrait canvas.\n";
    let defaults = app_defaults();
    let body =
        toml::to_string_pretty(&defaults).map_err(|source| ConfigError::Serialize { source })?;
    write_new_config(path, &format!("{header}{body}"))
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
    fn preview_preferences_persist_and_bound_numeric_values() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        fs::write(&app, "[paths]\ncache_dir = 'cache'\n").unwrap();
        fs::write(&ai, TWO_TEXT_PROVIDERS).unwrap();
        let config = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(config.preview, PreviewSettings::default());
        config
            .persist_preview_settings(PreviewSettings {
                autoplay_seconds: 12,
                text_percent: 125,
                contain_artwork: true,
                show_diagnostics: false,
            })
            .unwrap();
        let loaded = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(
            loaded.preview,
            PreviewSettings {
                autoplay_seconds: 12,
                text_percent: 125,
                contain_artwork: true,
                show_diagnostics: false
            }
        );
        loaded
            .persist_preview_settings(PreviewSettings {
                autoplay_seconds: 999,
                ..loaded.preview
            })
            .unwrap();
        assert_eq!(
            AppConfig::load_split(Some(app), Some(ai))
                .unwrap()
                .preview
                .autoplay_seconds,
            30
        );
        let mut blocked = loaded;
        let parent_file = directory.path().join("not-a-directory");
        fs::write(&parent_file, "blocked").unwrap();
        blocked.app_config_path = Some(parent_file.join("studio.toml"));
        assert!(blocked.persist_preview_settings(blocked.preview).is_err());
    }

    #[test]
    fn theme_and_vision_preferences_preserve_separate_provider_settings() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        fs::write(&app, "# keep app settings\n[paths]\ncache_dir = 'cache'\n").unwrap();
        fs::write(&ai, TWO_TEXT_PROVIDERS).unwrap();
        let config = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(config.theme, ThemePreference::Dark);
        assert!(
            config.separate_vision,
            "preserve an existing distinct Vision selection"
        );
        config.persist_separate_vision(false).unwrap();
        for theme in ThemePreference::ALL {
            config.persist_theme(theme).unwrap();
            let reloaded = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
            assert_eq!(reloaded.theme, theme);
            assert!(!reloaded.separate_vision);
            assert!(reloaded.vision.is_none());
            reloaded
                .persist_provider_selection(ChatCapability::Text, "deepseek")
                .unwrap();
            let selected = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
            assert!(selected.vision.is_none());
            assert_eq!(selected.vision_provider_id.as_deref(), Some("deepseek"));
        }
        config.persist_separate_vision(true).unwrap();
        let restored = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(restored.vision, config.vision);
        assert_eq!(fs::read_to_string(ai).unwrap(), TWO_TEXT_PROVIDERS);
        let saved = fs::read_to_string(&app).unwrap();
        assert!(saved.contains("# keep app settings"));
        assert!(saved.contains("cache_dir = 'cache'"));
        assert_eq!(saved.matches("theme =").count(), 1);
        assert_eq!(saved.matches("separate_vision =").count(), 1);
        let blocked = directory.path().join("missing-parent").join("parent-file");
        fs::write(blocked.parent().unwrap(), b"not a directory").unwrap();
        let mut config = restored;
        config.app_config_path = Some(blocked);
        assert!(config.persist_theme(ThemePreference::Light).is_err());
        assert!(config.persist_separate_vision(false).is_err());
    }

    #[test]
    fn vision_links_only_when_selected_and_preserves_an_unconfigured_capability() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        fs::write(&ai, "[defaults]\ntext = 'writer'\n[providers.writer]\nkind = 'openai'\ncapabilities = ['text']\nbase_url = 'http://localhost:8080/v1'\nmodel = 'writer'\n").unwrap();
        let config = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert!(config.separate_vision);
        assert!(config.vision.is_none());
        config.persist_separate_vision(false).unwrap();
        let linked = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert!(!linked.separate_vision);
        assert!(linked.vision.is_none());
        fs::write(&app, "[models]\nseparate_vision = true\n").unwrap();
        let separate = AppConfig::load_split(Some(app.clone()), Some(ai)).unwrap();
        assert!(separate.separate_vision);
        assert!(separate.vision.is_none());
        fs::write(&app, "[appearance]\ntheme = 'unknown'\n").unwrap();
        assert!(matches!(
            AppConfig::load_from(app),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn linked_vision_requires_the_selected_text_provider_capability() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        fs::create_dir_all(&scratch).unwrap();
        let directory = tempfile::tempdir_in(scratch).unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        fs::write(&app, "[models]\nseparate_vision = false\n").unwrap();
        let shared = TWO_TEXT_PROVIDERS.replace(
            "capabilities = [\"text\"]",
            "capabilities = [\"text\", \"vision\"]",
        );
        fs::write(&ai, &shared).unwrap();
        let config = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(config.vision, config.text);
        assert!(config
            .persist_provider_selection(ChatCapability::Text, "vision")
            .is_err());
        let text_only = shared.replace(
            "capabilities = [\"text\", \"vision\"]",
            "capabilities = [\"text\"]",
        );
        fs::write(&ai, text_only).unwrap();
        let reloaded = AppConfig::load_split(Some(app), Some(ai)).unwrap();
        assert!(reloaded.text.is_some());
        assert!(reloaded.vision.is_none());
    }

    /// A registry with two `text` providers and one `vision` provider, so a
    /// capability switch has somewhere to go.
    const TWO_TEXT_PROVIDERS: &str = "[defaults]\n\
         t2i = \"qwen-image\"\n\
         text = \"local\"\n\
         vision = \"vision\"\n\
         [providers.qwen-image]\n\
         kind = \"sdcpp\"\n\
         architecture = \"qwen-image-2.1\"\n\
         capabilities = [\"t2i\", \"edit\"]\n\
         base_url = \"http://127.0.0.1:7861\"\n\
         [providers.local]\n\
         kind = \"openai\"\n\
         capabilities = [\"text\"]\n\
         base_url = \"http://localhost:8080/v1\"\n\
         model = \"prism-ml/bonsai-27b\"\n\
         [providers.deepseek]\n\
         kind = \"openai\"\n\
         capabilities = [\"text\"]\n\
         base_url = \"https://api.deepseek.com/v1\"\n\
         model = \"deepseek-chat\"\n\
         [providers.vision]\n\
         kind = \"openai\"\n\
         capabilities = [\"vision\"]\n\
         base_url = \"http://localhost:8080/v1\"\n\
         model = \"google/gemma-4-e4b\"\n";

    #[test]
    fn studio_selection_uses_the_configured_provider_model() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        fs::create_dir_all(&scratch).unwrap();
        let directory = tempfile::tempdir_in(scratch).unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        fs::write(&app, "[models]\ntext_model = 'unconfigured'\n").unwrap();
        fs::write(&ai, TWO_TEXT_PROVIDERS).unwrap();
        let config = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        config
            .persist_provider_selection(ChatCapability::Text, "deepseek")
            .unwrap();
        config
            .persist_provider_selection(ChatCapability::Vision, "vision")
            .unwrap();
        assert!(config
            .persist_provider_selection(ChatCapability::Vision, "deepseek")
            .is_err());
        assert!(config
            .persist_provider_selection(ChatCapability::Text, "missing")
            .is_err());
        let selected = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(selected.text.as_ref().unwrap().model, "deepseek-chat");
        let updated = TWO_TEXT_PROVIDERS.replace("deepseek-chat", "new-model");
        fs::write(&ai, updated).unwrap();
        let refreshed = AppConfig::load_split(Some(app), Some(ai)).unwrap();
        assert_eq!(refreshed.text.as_ref().unwrap().model, "new-model");
    }

    #[test]
    fn studio_model_choices_are_per_capability_and_bound_to_their_provider() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        fs::create_dir_all(&scratch).unwrap();
        let directory = tempfile::tempdir_in(scratch).unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        fs::write(&ai, TWO_TEXT_PROVIDERS).unwrap();
        let load = || AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        load()
            .persist_model_selection(ChatCapability::Text, "new-text-model")
            .unwrap();
        load()
            .persist_model_selection(ChatCapability::Vision, "new-vision-model")
            .unwrap();
        let config = load();
        assert_eq!(config.text.as_ref().unwrap().model, "new-text-model");
        assert_eq!(config.vision.as_ref().unwrap().model, "new-vision-model");
        assert_eq!(fs::read_to_string(&ai).unwrap(), TWO_TEXT_PROVIDERS);
        config
            .persist_provider_selection(ChatCapability::Text, "deepseek")
            .unwrap();
        let other = load();
        assert_eq!(other.text.as_ref().unwrap().model, "deepseek-chat");
        assert_eq!(other.vision.as_ref().unwrap().model, "new-vision-model");
        other
            .persist_provider_selection(ChatCapability::Text, "local")
            .unwrap();
        assert_eq!(load().text.unwrap().model, "new-text-model");
        assert!(load()
            .persist_model_selection(ChatCapability::Text, " ")
            .is_err());
        let supports_both = TWO_TEXT_PROVIDERS.replacen(
            "capabilities = [\"text\"]",
            "capabilities = [\"text\", \"vision\"]",
            1,
        );
        fs::write(&ai, supports_both).unwrap();
        load().persist_separate_vision(false).unwrap();
        let linked = load();
        assert_eq!(linked.text.as_ref().unwrap().model, "new-text-model");
        assert_eq!(linked.vision.as_ref().unwrap().model, "new-text-model");
        linked.persist_separate_vision(true).unwrap();
        assert_eq!(load().vision.unwrap().model, "new-vision-model");
    }

    #[test]
    fn clipforge_edits_refresh_selected_text_and_vision_without_studio_writing_the_registry() {
        use agentkit_ai::deployments::NewChatConfig;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let app = directory.path().join("studio.toml");
        let shared = directory.path().join("clipforge.toml");
        let original = "[defaults]\ntext='local'\nvision='local'\n[providers.local]\nkind='llamacpp'\ncapabilities=['text','vision']\nbase_url='http://127.0.0.1:8080/v1'\nmodel='first'\nmodel_path='/models/first.gguf'\nmmproj='/models/first-projector.gguf'\n";
        fs::write(&shared, original).unwrap();
        let config = AppConfig::load_split(Some(app.clone()), Some(shared.clone())).unwrap();
        config
            .persist_provider_selection(ChatCapability::Text, "local")
            .unwrap();
        config
            .persist_provider_selection(ChatCapability::Vision, "local")
            .unwrap();
        assert_eq!(fs::read_to_string(&shared).unwrap(), original);

        let registry = Providers::load(&shared).unwrap();
        registry
            .update_chat_provider(
                &shared,
                &NewChatConfig {
                    id: "local".into(),
                    base_url: "http://127.0.0.1:8080/v1".into(),
                    model: "second".into(),
                    vision: true,
                    model_path: Some("/models/second.gguf".into()),
                    mmproj: Some("/models/second-projector.gguf".into()),
                    executable: None,
                    api_key_env: None,
                    api_key: None,
                },
            )
            .unwrap();
        let updated = AppConfig::load_split(Some(app), Some(shared)).unwrap();
        for deployment in [
            updated.text.as_ref().unwrap(),
            updated.vision.as_ref().unwrap(),
        ] {
            assert_eq!(deployment.model, "second");
            let server = deployment.server.as_ref().unwrap();
            assert_eq!(server.model_path, PathBuf::from("/models/second.gguf"));
            assert_eq!(
                server.mmproj.as_deref(),
                Some(Path::new("/models/second-projector.gguf"))
            );
        }
        assert_eq!(config.text.as_ref().unwrap().model, "first");
    }

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
    fn text_and_vision_deployments_come_from_the_providers_registry() {
        let dir = std::env::temp_dir().join(format!("svs-seed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let ai = dir.join("clipforge.toml");
        std::fs::write(&ai, TWO_TEXT_PROVIDERS).unwrap();
        let config = AppConfig::load_from(&ai).expect("seeded config loads");

        assert_eq!(config.text_provider_id.as_deref(), Some("local"));
        let text = config.text.as_ref().expect("text deployment");
        assert_eq!(text.base_url, "http://localhost:8080/v1");
        assert_eq!(text.model, "prism-ml/bonsai-27b");
        assert_eq!(config.vision_provider_id.as_deref(), Some("vision"));
        let vision = config.vision.as_ref().expect("vision deployment");
        assert_eq!(vision.base_url, "http://localhost:8080/v1");
        assert_eq!(vision.model, "google/gemma-4-e4b");
        // The image deployment resolves from the same registry.
        assert_eq!(
            config.native_image().unwrap().architecture,
            "qwen-image-2.1"
        );
        // The seeded model directory is absolute, never relative to the config.
        assert!(config.native_image().unwrap().model_dir.is_absolute());
        // The derived planner `LlmConfig` carries the same endpoint.
        assert_eq!(
            config.planner_config().expect("planner config").model,
            text.model
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn studio_selects_only_configured_local_image_providers_and_refreshes_paths() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        std::fs::write(&app, "").unwrap();
        std::fs::write(&ai, "[defaults]\nt2i='first'\n[providers.first]\nkind='sdcpp'\ncapabilities=['t2i']\nbase_url='http://127.0.0.1:7860'\narchitecture='qwen-image-2.1'\n[providers.second]\nkind='sdcpp'\ncapabilities=['t2i']\nbase_url='http://127.0.0.1:7860'\narchitecture='krea2-turbo'\n").unwrap();
        let config = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(config.image_providers.len(), 2);
        assert_eq!(config.image_provider_id.as_deref(), Some("first"));
        let shared_before = std::fs::read(&ai).unwrap();
        let registry = Providers::load(&ai).unwrap();
        assert_eq!(
            config.image.as_ref(),
            Some(
                &registry
                    .image_deployment_for("first", Capability::T2i)
                    .unwrap()
            )
        );
        assert_eq!(
            config.image.as_ref().unwrap().model_dir,
            directory.path().join("models")
        );
        assert!(config
            .persist_image_provider_selection(Capability::T2i, "missing")
            .is_err());
        config
            .persist_image_provider_selection(Capability::T2i, "second")
            .unwrap();
        assert_eq!(std::fs::read(&ai).unwrap(), shared_before);
        let selected = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(selected.image_provider_id.as_deref(), Some("second"));
        assert_eq!(
            selected.image.as_ref(),
            Some(
                &registry
                    .image_deployment_for("second", Capability::T2i)
                    .unwrap()
            )
        );
        std::fs::write(&ai, "[defaults]\nt2i='first'\n[providers.first]\nkind='sdcpp'\ncapabilities=['t2i']\nbase_url='http://127.0.0.1:7860'\narchitecture='qwen-image-2.1'\n").unwrap();
        let removed = AppConfig::load_split(Some(app), Some(ai)).unwrap();
        assert_eq!(removed.image_provider_id.as_deref(), Some("second"));
        assert!(removed.image.is_none());
    }

    #[test]
    fn studio_resolves_distinct_cloud_generation_and_edit_routes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        std::fs::write(&app, "").unwrap();
        std::fs::write(&ai, "[defaults]\nt2i='create'\nedit='edit'\n[providers.create]\nkind='fal'\ncapabilities=['t2i']\nbase_url='https://queue.fal.run'\nmodel='fal-ai/qwen-image-2512'\n[providers.edit]\nkind='fal'\ncapabilities=['edit']\nbase_url='https://queue.fal.run'\nmodel='fal-ai/qwen-image-edit-2511'\n").unwrap();
        let config = AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(config.image_provider_id.as_deref(), Some("create"));
        assert_eq!(config.image_edit_provider_id.as_deref(), Some("edit"));
        assert_eq!(config.image_providers.len(), 2);
        assert!(config
            .persist_image_provider_selection(Capability::Edit, "create")
            .is_err());
        config
            .persist_image_provider_selection(Capability::Edit, "edit")
            .unwrap();
        let reloaded = AppConfig::load_split(Some(app), Some(ai)).unwrap();
        assert_eq!(reloaded.image_edit_provider_id.as_deref(), Some("edit"));
    }

    #[test]
    fn shared_edit_default_wins_even_when_generation_provider_also_edits() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let path = directory.path().join("providers.toml");
        std::fs::write(&path, "[defaults]\nt2i='local'\nedit='cloud'\n[providers.local]\nkind='sdcpp'\ncapabilities=['t2i','edit']\narchitecture='qwen-image-2.1'\nbase_url='http://127.0.0.1:7860'\n[providers.cloud]\nkind='fal'\ncapabilities=['edit']\nbase_url='https://queue.fal.run'\nmodel='fal-ai/qwen-image-edit-2511'\n").unwrap();
        let config = AppConfig::load_from(path).unwrap();
        assert_eq!(config.image_provider_id.as_deref(), Some("local"));
        assert_eq!(config.image_edit_provider_id.as_deref(), Some("cloud"));
    }

    #[test]
    fn missing_or_empty_shared_registry_has_no_invented_providers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let app = directory.path().join("studio.toml");
        let shared = directory.path().join("clipforge.toml");
        for contents in [None, Some(""), Some("[providers]\n")] {
            if let Some(contents) = contents {
                fs::write(&shared, contents).unwrap();
            }
            let config = AppConfig::load_at(Some(app.clone()), Some(shared.clone())).unwrap();
            assert!(config.chat_providers.is_empty());
            assert!(config.image_providers.is_empty());
            assert!(config.text.is_none() && config.vision.is_none() && config.image.is_none());
            if contents.is_none() {
                assert!(!shared.exists());
            }
        }
    }

    #[tokio::test]
    async fn hosted_or_text_only_settings_keep_authoring_available_without_native_images() {
        use crate::planner::PlannerBackend;
        let hosted = r#"
t2i = 'replicate'
edit = 'replicate'
[providers.replicate]
kind = 'replicate'
capabilities = ['t2i', 'edit']
base_url = 'https://api.replicate.com/v1'
model = 'alibaba/qwen-image-3'
api_key = 'test-kept-key'
"#;
        for image_config in ["", hosted] {
            let directory = tempfile::tempdir().unwrap();
            let app = directory.path().join("studio.toml");
            let ai = directory.path().join("clipforge.toml");
            let text_config = "[providers.writer]\nkind='openai'\ncapabilities=['text']\nbase_url='http://127.0.0.1:1/v1'\nmodel='writer-test'\n";
            let contents = format!("[defaults]\ntext='writer'\n{image_config}\n{text_config}");
            std::fs::write(&ai, &contents).unwrap();
            std::fs::write(&app, "[runtime]\nprewarm=true\n").unwrap();
            let config = AppConfig::load_split(Some(app), Some(ai.clone())).unwrap();
            assert!(config.image.is_none());
            assert_eq!(
                config.image_provider_id.as_deref(),
                if image_config.is_empty() {
                    None
                } else {
                    Some("replicate")
                }
            );
            assert!(!format!("{config:?}").contains("test-kept-key"));
            assert_eq!(config.planner_config().unwrap().model, "writer-test");
            assert!(matches!(
                config.native_image(),
                Err(ConfigError::NativeImageUnavailable)
            ));
            assert_eq!(
                config.native_image().unwrap_err().code(),
                ErrorCode::ConfigNativeImageUnavailable
            );
            let backend = PlannerBackend::new(&config);
            let allocation = backend
                .planner_allocation()
                .await
                .unwrap()
                .expect("a hosted text provider needs no managed server");
            assert_eq!(allocation.config().model, "writer-test");
            assert_eq!(
                allocation.config().base_url,
                "http://127.0.0.1:1/v1",
                "a hosted provider is called at its configured address"
            );
            assert_eq!(std::fs::read_to_string(&ai).unwrap(), contents);
        }
    }

    #[test]
    fn malformed_provider_sections_are_rejected_instead_of_becoming_defaults() {
        for settings in ["providers = 'invalid'", "defaults = 'invalid'"] {
            let value = toml::from_str(settings).unwrap();
            assert!(matches!(
                providers_from_value(&value, Path::new(".")),
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
        std::fs::write(&base, TWO_TEXT_PROVIDERS).unwrap();
        let model_dir = directory.join("existing models");
        let encoded = toml::Value::String(model_dir.to_string_lossy().into_owned()).to_string();
        std::fs::write(&user, format!("# retain this comment\n[generation]\nsteps = 7\n[runtime]\nprewarm = true\n[providers.local]\napi_key_env = 'TEST_LOCAL_KEY'\nmodel = 'chosen-model'\n[providers.qwen-image]\nmodel_dir = {encoded}\nresidency = 'lowvram'\n")).unwrap();
        let config = AppConfig::load_layered(base.clone(), Some(user.clone())).unwrap();
        let image_path = config.native_image().unwrap().artifact_plan()[0]
            .path
            .clone();
        std::fs::create_dir_all(image_path.parent().unwrap()).unwrap();
        std::fs::write(&image_path, b"existing bytes stay in place").unwrap();
        let config = AppConfig::load_layered(base, Some(user.clone())).unwrap();
        assert_eq!(config.native_image().unwrap().model_dir, model_dir);
        assert_eq!(config.text.as_ref().unwrap().model, "chosen-model");
        assert_eq!(config.generation_defaults.steps, 7);
        assert_eq!(
            std::fs::read(&image_path).unwrap(),
            b"existing bytes stay in place"
        );
        let saved = std::fs::read_to_string(user).unwrap();
        assert!(saved.contains("retain this comment"));
        assert!(saved.contains("TEST_LOCAL_KEY"));
        assert!(saved.contains("prewarm = true"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migration_splits_old_settings_once_without_overwriting_or_deleting_sources() {
        let directory =
            std::env::temp_dir().join(format!("svs-migration-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let old_app = directory.join("app.toml");
        let old_providers = directory.join("providers.toml");
        let app = directory.join("studio.toml");
        let ai = directory.join("clipforge.toml");
        let original_app = "[generation]\nsteps = 9\n";
        let original_providers =
            "[providers.local]\nmodel = 'chosen'\napi_key = 'test-only-secret'\n";
        std::fs::write(&old_app, original_app).unwrap();
        std::fs::write(&old_providers, "invalid = [").unwrap();
        assert!(migrate_legacy_settings(Some(&app), Some(&ai)).is_err());
        assert!(!app.exists());
        std::fs::write(&old_providers, original_providers).unwrap();
        migrate_legacy_settings(Some(&app), Some(&ai)).unwrap();
        // The app half keeps the app sections plus the built-in defaults; the
        // inference half keeps providers.
        let app_value = read_config_value(&app).unwrap();
        assert_eq!(app_value["generation"]["steps"].as_integer(), Some(9));
        assert_eq!(
            app_value["generation"]["sizes"]["character"].as_str(),
            Some("1024x1536"),
            "the portrait canvas default survives migration: {app_value}"
        );
        assert!(app_value.get("providers").is_none(), "{app_value}");
        let ai_value = read_config_value(&ai).unwrap();
        assert_eq!(
            ai_value["providers"]["local"]["api_key"].as_str(),
            Some("test-only-secret")
        );
        assert!(ai_value.get("generation").is_none(), "{ai_value}");
        // Originals remain available for recovery.
        assert_eq!(std::fs::read_to_string(&old_app).unwrap(), original_app);
        assert_eq!(
            std::fs::read_to_string(&old_providers).unwrap(),
            original_providers
        );
        // The new app file is authoritative once present: a second run leaves it.
        std::fs::write(&app, "[generation]\nsteps = 12\n").unwrap();
        migrate_legacy_settings(Some(&app), Some(&ai)).unwrap();
        assert_eq!(
            read_config_value(&app).unwrap()["generation"]["steps"].as_integer(),
            Some(12)
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 4);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migration_splits_a_unified_user_file_into_app_and_inference_halves() {
        let directory = std::env::temp_dir().join(format!("svs-split-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let unified = directory.join("storyfella.toml");
        let app = directory.join("studio.toml");
        let ai = directory.join("clipforge.toml");
        std::fs::write(
            &unified,
            "[generation]\nsteps = 9\n\n[defaults]\ntext = \"local\"\n\n\
             [providers.local]\nkind = \"openai\"\ncapabilities = [\"text\"]\n\
             base_url = \"http://localhost:8080/v1\"\nmodel = \"chosen\"\n",
        )
        .unwrap();

        migrate_legacy_settings(Some(&app), Some(&ai)).unwrap();
        let app_value = read_config_value(&app).unwrap();
        assert_eq!(app_value["generation"]["steps"].as_integer(), Some(9));
        assert!(app_value.get("providers").is_none(), "{app_value}");
        let ai_value = read_config_value(&ai).unwrap();
        assert_eq!(ai_value["defaults"]["text"].as_str(), Some("local"));
        assert_eq!(
            ai_value["providers"]["local"]["model"].as_str(),
            Some("chosen")
        );
        // The unified original is never deleted or rewritten.
        assert!(unified.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migration_leaves_an_existing_shared_config_alone() {
        let directory = std::env::temp_dir().join(format!("svs-keep-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let unified = directory.join("storyfella.toml");
        let app = directory.join("studio.toml");
        let ai = directory.join("clipforge.toml");
        std::fs::write(&unified, "[generation]\nsteps = 9\n").unwrap();
        std::fs::write(&ai, "# already here\n[defaults]\ntext = 'kept'\n").unwrap();

        migrate_legacy_settings(Some(&app), Some(&ai)).unwrap();
        // The pre-existing shared config (perhaps created by another front end)
        // wins; migration only fills the app half.
        assert_eq!(
            std::fs::read_to_string(&ai).unwrap(),
            "# already here\n[defaults]\ntext = 'kept'\n"
        );
        assert!(app.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_legacy_shared_config_is_migrated_without_replacement() {
        let directory =
            std::env::temp_dir().join(format!("svs-shared-migrate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let legacy = directory.join("providers.toml");
        let contents = "[defaults]\ntext = \"local\"\n\
             [providers.local]\nkind = \"openai\"\ncapabilities = [\"text\"]\n\
             base_url = \"http://localhost:8080/v1\"\nmodel = \"bonsai-27b\"\n";
        std::fs::write(&legacy, contents).unwrap();
        let ai = directory.join("clipforge.toml");

        crate::ai_config::migrate_legacy_config_at(&ai);

        // The legacy registry is migrated without replacement.
        assert!(!legacy.exists(), "the legacy file was renamed");
        assert_eq!(std::fs::read_to_string(&ai).unwrap(), contents);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_split_loader_reads_app_settings_and_providers_from_their_own_files() {
        let directory = std::env::temp_dir().join(format!("svs-split-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let app = directory.join("studio.toml");
        let ai = directory.join("clipforge.toml");
        std::fs::write(&app, "[generation]\nsteps = 4\n").unwrap();
        std::fs::write(&ai, TWO_TEXT_PROVIDERS).unwrap();

        let config = AppConfig::load_split(Some(app), Some(ai.clone())).expect("split loads");
        assert_eq!(config.generation_defaults.steps, 4);
        assert_eq!(config.text_provider_id.as_deref(), Some("local"));
        assert_eq!(config.ai_config_path.as_deref(), Some(ai.as_path()));
        assert!(config
            .chat_providers
            .iter()
            .any(|provider| provider.id == "deepseek"));
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

        let providers = providers_from_value(&read_config_value(&base).unwrap(), &dir)
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
            "[defaults]\ntext = \"local\"\nvision = \"vision\"\n\
             [providers.local]\nkind = \"openai\"\ncapabilities = [\"text\"]\n\
             base_url = \"http://localhost:8080/v1\"\nmodel = \"prism-ml/bonsai-27b\"\n\
             [providers.vision]\nkind = \"openai\"\ncapabilities = [\"vision\"]\n\
             base_url = \"http://localhost:8080/v1\"\nmodel = \"google/gemma-4-e4b\"\n",
        )
        .expect("base fixture should write");
        std::fs::write(
            &user,
            "[providers.local]\nmodel = \"deepseek-chat\"\napi_key_env = \"DEEPSEEK_API_KEY\"\n",
        )
        .expect("user fixture should write");

        let mut value = read_config_value(&base).unwrap();
        merge_toml(&mut value, read_config_value(&user).unwrap());
        let providers = providers_from_value(&value, &dir).expect("providers should merge");
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
    fn chat_providers_are_listed_once_for_both_pickers() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("storyfella.toml");
        std::fs::write(&path, TWO_TEXT_PROVIDERS).expect("fixture should write");

        let config = AppConfig::load_from(&path).expect("config loads");
        let ids: Vec<&str> = config
            .chat_providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect();
        // One list for both pickers: every chat provider, `sdcpp` excluded.
        assert_eq!(
            ids,
            ["deepseek", "local", "vision"],
            "chat providers, in id order"
        );
        let deepseek = config
            .chat_providers
            .iter()
            .find(|provider| provider.id == "deepseek")
            .expect("deepseek listed");
        assert_eq!(deepseek.kind, ProviderKind::Openai);
        assert_eq!(deepseek.capabilities, ["text"]);
        assert_eq!(deepseek.deployment.model, "deepseek-chat");
        assert_eq!(deepseek.deployment.base_url, "https://api.deepseek.com/v1");
        assert_eq!(
            deepseek.label(),
            "deepseek · openai · https://api.deepseek.com/v1"
        );
    }

    #[test]
    fn existing_nested_lora_bindings_remain_available_to_cli_generation() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("app.toml");
        std::fs::write(
            &path,
            "[lora]\ncharacter = 'characters/mia-v2.safetensors'\n",
        )
        .expect("fixture should write");
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
        assert_eq!(config.app_config_path.as_deref(), Some(user.as_path()));
    }

    #[test]
    fn the_app_config_seed_sets_the_portrait_canvas_and_never_overwrites() {
        let directory = std::env::temp_dir().join(format!("svs-appseed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let app = directory.join("studio.toml");

        seed_app_config(&app).expect("seed writes");
        let config = AppConfig::load_from(&app).expect("seed loads");
        assert_eq!(
            config.generation_defaults.sizes.character,
            Some((1024, 1536))
        );
        assert_eq!(config.generation_defaults.steps, sd::DEFAULT_STEPS);

        // A hand-edited app config is never replaced.
        std::fs::write(&app, "# mine\n").unwrap();
        seed_app_config(&app).expect("seed is a no-op when the file exists");
        assert_eq!(std::fs::read_to_string(&app).unwrap(), "# mine\n");
        std::fs::remove_dir_all(directory).unwrap();
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
}
