use std::{
    fs,
    path::{Path, PathBuf},
};

use clipforge_inference::{
    sd, Capability, ChatDeployment, ImageDeployment, ProviderError, ProviderKind, Providers,
};
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
    /// The capabilities the provider declares (`text`, `vision`). Selecting a
    /// provider for a new capability adds it (see
    /// [`AppConfig::with_text_provider`]).
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

    /// Whether this is an external endpoint the app calls but never starts
    /// (`openai`), rather than a managed native server (`llamacpp`). The user
    /// owns an external endpoint's URL; a managed server's is the app's.
    pub fn is_external(&self) -> bool {
        self.kind == ProviderKind::Openai
    }

    /// What the app launches for a managed server, or `None` for an external
    /// endpoint. The endpoint of a managed server is derived, not entered.
    pub fn managed_launch(&self) -> Option<ManagedLaunch> {
        let server = self.deployment.server.as_ref()?;
        Some(ManagedLaunch {
            executable: server.executable.display().to_string(),
            model_path: server.model_path.display().to_string(),
            mmproj: server
                .mmproj
                .as_ref()
                .map(|path| path.display().to_string()),
        })
    }
}

/// What the app launches for a managed chat server (a `llamacpp` provider).
#[derive(Clone, Debug, PartialEq)]
pub struct ManagedLaunch {
    pub executable: String,
    pub model_path: String,
    /// The vision projector, for a multimodal model.
    pub mmproj: Option<String>,
}

/// One editable field of a chat provider's endpoint — what the Settings
/// endpoint editor writes under `[providers.<id>]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatEndpointField {
    BaseUrl,
    Model,
    ApiKey,
}

impl ChatEndpointField {
    /// The TOML key this field writes.
    pub const fn key(self) -> &'static str {
        match self {
            Self::BaseUrl => "base_url",
            Self::Model => "model",
            Self::ApiKey => "api_key",
        }
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

/// How a chat provider created from Settings runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NewChatProviderKind {
    /// An external OpenAI-compatible endpoint the app calls but never starts.
    External,
    /// A native `llama.cpp` server the app starts and supervises.
    Managed,
}

impl NewChatProviderKind {
    /// The `kind` value written under `[providers.<id>]`.
    const fn as_str(self) -> &'static str {
        match self {
            Self::External => "openai",
            Self::Managed => "llamacpp",
        }
    }
}

/// A chat provider to create from the Settings UI, written under
/// `[providers.<id>]` in the user overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct NewChatProvider {
    pub id: String,
    pub kind: NewChatProviderKind,
    pub capabilities: Vec<ChatCapability>,
    pub base_url: String,
    pub model: String,
    /// The inline bearer token for an external endpoint, when one is needed.
    pub api_key: Option<String>,
    /// The `llama-server` executable, for a managed provider.
    pub executable: Option<String>,
    /// The model file a managed server loads.
    pub model_path: Option<String>,
}

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
    /// Every configured provider serving `text`, in id order. The Settings
    /// provider picker switches `[defaults] text` among these, so moving
    /// Every configured provider that serves a chat capability — `openai` or
    /// `llamacpp` — in id order, with the endpoint each resolves to. Both the
    /// text and vision pickers choose from this one list: any chat provider
    /// can be assigned to either capability, and the assignment records the
    /// capability (see [`AppConfig::with_text_provider`]).
    pub chat_providers: Vec<ChatProvider>,
    /// The provider ids the user overlay defines (see [`paths::user_config_path`]).
    /// Settings may remove only these: a provider from the checked-in
    /// `config/storyfella.toml` is not the UI's to delete.
    pub user_providers: Vec<String>,
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
        let mut user_providers = Vec::new();
        if let Some(overlay) = user.as_deref() {
            if overlay != path {
                let overlay_value = read_config_value(overlay)?;
                if let Some(providers) = overlay_value
                    .get("providers")
                    .and_then(toml::Value::as_table)
                {
                    user_providers = providers.keys().cloned().collect();
                }
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
        let chat_providers = chat_providers(&providers);

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
            chat_providers,
            user_providers,
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

    /// Persists a chat provider's `capabilities` array to the user overlay —
    /// used when a provider is assigned to a capability it did not declare.
    pub fn persist_provider_capabilities(
        &self,
        provider_id: &str,
        capabilities: &[String],
    ) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        Self::persist_provider_capabilities_to(path, provider_id, capabilities)
    }

    /// Testable variant of [`AppConfig::persist_provider_capabilities`].
    pub fn persist_provider_capabilities_to(
        path: impl AsRef<Path>,
        provider_id: &str,
        capabilities: &[String],
    ) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let contents = read_config_contents(path)?;
        let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();
        let section = format!("providers.{provider_id}");
        let value = format!(
            "[{}]",
            capabilities
                .iter()
                .map(|capability| format!("{capability:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        write_key(&mut lines, &section, "capabilities", Some(value));
        write_config_file(path, lines.join("\n"))
    }

    /// Persists which configured provider serves the planner (`text`) by
    /// writing `[defaults] text = "<provider_id>"` to the user config (see
    /// [`paths::user_config_path`]); line-preserving like the other settings,
    /// so the choice survives restarts without a file edit.
    pub fn persist_text_provider(&self, provider_id: &str) -> Result<(), ConfigError> {
        self.persist_default_provider(Capability::Text, provider_id)
    }

    /// A copy with the planner (`text`) re-pointed at `provider_id` — the
    /// Settings provider switch applied in memory. `None` when no provider
    /// serving `text` carries that id. It keeps the selected id and the
    /// resolved deployment in step, so callers never have to.
    pub fn with_text_provider(&self, provider_id: &str) -> Option<Self> {
        self.with_chat_provider(Capability::Text, provider_id)
    }

    /// A copy with `vision` re-pointed at `provider_id` (see
    /// [`AppConfig::with_text_provider`]).
    pub fn with_vision_provider(&self, provider_id: &str) -> Option<Self> {
        self.with_chat_provider(Capability::Vision, provider_id)
    }

    fn with_chat_provider(&self, capability: Capability, provider_id: &str) -> Option<Self> {
        if !matches!(capability, Capability::Text | Capability::Vision) {
            return None;
        }
        let deployment = self
            .chat_providers
            .iter()
            .find(|provider| provider.id == provider_id)?
            .deployment
            .clone();
        let mut next = self.clone();
        // `[defaults]` may only name a provider that declares the capability, so
        // assigning a provider to a new capability records it there (the user
        // decides which provider serves what).
        if let Some(provider) = next
            .chat_providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)
        {
            let capability = capability.as_str().to_owned();
            if !provider.capabilities.contains(&capability) {
                provider.capabilities.push(capability);
            }
        }
        if capability == Capability::Text {
            next.text = Some(deployment);
            next.text_provider_id = Some(provider_id.to_owned());
        } else {
            next.vision = Some(deployment);
            next.vision_provider_id = Some(provider_id.to_owned());
        }
        Some(next)
    }

    /// A copy with one endpoint field of the named chat provider changed in
    /// memory — the Settings endpoint editor. It updates the provider list
    /// entry and, when that provider serves a capability, the resolved
    /// deployment, so the two never drift. Editing a managed provider's base
    /// URL also moves the `llama-server` it launches, since that address is
    /// derived from the URL. `None` when no chat provider carries the id, when
    /// an empty value would clear a field that must hold one (only the API key
    /// may be cleared), or when a managed base URL cannot be parsed. The edit is
    /// persisted separately (see [`AppConfig::persist_chat_endpoint`]).
    pub fn with_chat_endpoint(
        &self,
        provider_id: &str,
        field: ChatEndpointField,
        value: &str,
    ) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() && field != ChatEndpointField::ApiKey {
            return None;
        }
        let managed = self
            .chat_providers
            .iter()
            .find(|provider| provider.id == provider_id)?
            .deployment
            .server
            .is_some();
        let launch = if managed && field == ChatEndpointField::BaseUrl {
            Some(sd::parse_base_url(value).ok()?)
        } else {
            None
        };
        self.with_chat_provider_edited(provider_id, |deployment| match field {
            ChatEndpointField::BaseUrl => {
                deployment.base_url = value.to_owned();
                if let (Some(server), Some((host, port))) =
                    (deployment.server.as_mut(), launch.clone())
                {
                    server.host = host;
                    server.port = port;
                }
            }
            ChatEndpointField::Model => deployment.model = value.to_owned(),
            // An empty key clears the inline secret and falls back to any
            // `api_key_env` the provider names.
            ChatEndpointField::ApiKey => {
                deployment.api_key = (!value.is_empty()).then(|| value.to_owned());
            }
        })
    }

    fn with_chat_provider_edited(
        &self,
        provider_id: &str,
        edit: impl Fn(&mut ChatDeployment),
    ) -> Option<Self> {
        let mut next = self.clone();
        let provider = next
            .chat_providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)?;
        edit(&mut provider.deployment);
        next.text = active_deployment(&next.chat_providers, &next.text_provider_id);
        next.vision = active_deployment(&next.chat_providers, &next.vision_provider_id);
        Some(next)
    }

    /// Persists which configured provider serves `vision` (see
    /// [`AppConfig::persist_text_provider`]).
    pub fn persist_vision_provider(&self, provider_id: &str) -> Result<(), ConfigError> {
        self.persist_default_provider(Capability::Vision, provider_id)
    }

    /// Writes `[defaults] <capability> = "<provider_id>"` to the user config.
    fn persist_default_provider(
        &self,
        capability: Capability,
        provider_id: &str,
    ) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        Self::persist_default_provider_to(path, capability, provider_id)
    }

    /// Testable variant of [`AppConfig::persist_default_provider`]; creates the
    /// file when it does not exist yet.
    pub fn persist_default_provider_to(
        path: impl AsRef<Path>,
        capability: Capability,
        provider_id: &str,
    ) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let contents = read_config_contents(path)?;
        let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();
        write_key(
            &mut lines,
            "defaults",
            capability.as_str(),
            Some(format!("{provider_id:?}")),
        );
        write_config_file(path, lines.join("\n"))
    }

    /// Persists one endpoint field of the named chat provider to the user
    /// settings file (see [`paths::user_config_path`]) — the Settings endpoint
    /// editor writes here so an edit survives restarts without touching the
    /// checked-in `storyfella.toml`. Line-preserving.
    pub fn persist_chat_endpoint(
        &self,
        provider_id: &str,
        field: ChatEndpointField,
        value: &str,
    ) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        Self::persist_chat_endpoint_to(path, provider_id, field, value)
    }

    /// Testable variant of [`AppConfig::persist_chat_endpoint`] that targets an
    /// arbitrary path; creates the file when it does not exist yet. The write
    /// is line-preserving: only the touched key under `[providers.<provider_id>]`
    /// changes, so comments and key order elsewhere survive.
    pub fn persist_chat_endpoint_to(
        path: impl AsRef<Path>,
        provider_id: &str,
        field: ChatEndpointField,
        value: &str,
    ) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let contents = read_config_contents(path)?;
        let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();
        let section = format!("providers.{provider_id}");
        // An empty value (a cleared API key) removes the key rather than
        // writing `= ""`, so a provider's `api_key_env` can apply again.
        let encoded = (!value.is_empty()).then(|| format!("{value:?}"));
        write_key(&mut lines, &section, field.key(), encoded);
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

    /// Adds a chat provider to the user overlay (see
    /// [`paths::user_config_path`]) — the Settings "Add provider" action. The
    /// candidate is validated against the merged configuration before it is
    /// written, so a registry the app cannot load is never persisted.
    pub fn persist_provider_added(&self, provider: &NewChatProvider) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        Self::persist_provider_added_to(path, &paths::config_path(), provider)
    }

    /// Testable variant of [`AppConfig::persist_provider_added`]; `base` is the
    /// checked-in configuration the overlay layers over.
    pub fn persist_provider_added_to(
        path: &Path,
        base: &Path,
        provider: &NewChatProvider,
    ) -> Result<(), ConfigError> {
        let contents = read_config_contents(path)?;
        let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();
        while lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(provider_section_lines(provider));
        let updated = lines.join("\n");
        validate_providers_overlay(base, path, &updated)?;
        write_config_file(path, updated)
    }

    /// Removes a chat provider the user overlay defines, and any `[defaults]`
    /// entry naming it (so the base file's default applies again). A provider
    /// from the checked-in file cannot be removed this way.
    pub fn persist_provider_removed(&self, id: &str) -> Result<(), ConfigError> {
        let path = self
            .user_config_path
            .as_ref()
            .ok_or(ConfigError::PlatformDirectories)?;
        Self::persist_provider_removed_to(path, &paths::config_path(), id)
    }

    /// Testable variant of [`AppConfig::persist_provider_removed`].
    pub fn persist_provider_removed_to(
        path: &Path,
        base: &Path,
        id: &str,
    ) -> Result<(), ConfigError> {
        let contents = read_config_contents(path)?;
        let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();
        remove_provider_section(&mut lines, id);
        remove_default_provider(&mut lines, id);
        let updated = lines.join("\n");
        validate_providers_overlay(base, path, &updated)?;
        write_config_file(path, updated)
    }
}

/// The `[providers.<id>]` section for a provider being added: one line per
/// field, with an empty optional field omitted rather than written blank.
fn provider_section_lines(provider: &NewChatProvider) -> Vec<String> {
    let mut lines = vec![format!("[providers.{}]", provider.id)];
    lines.push(format!("kind = {:?}", provider.kind.as_str()));
    let capabilities = provider
        .capabilities
        .iter()
        .map(|capability| format!("{:?}", capability.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!("capabilities = [{capabilities}]"));
    lines.push(format!("base_url = {:?}", provider.base_url));
    lines.push(format!("model = {:?}", provider.model));
    for (key, value) in [
        ("api_key", &provider.api_key),
        ("executable", &provider.executable),
        ("model_path", &provider.model_path),
    ] {
        let value = value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(value) = value {
            lines.push(format!("{key} = {value:?}"));
        }
    }
    lines
}

/// Removes `[providers.<id>]` and any `[providers.<id>.*]` subsection from a
/// line-preserving config file.
fn remove_provider_section(lines: &mut Vec<String>, id: &str) {
    let header = format!("[providers.{id}]");
    let prefix = format!("[providers.{id}.");
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed == header || trimmed.starts_with(&prefix) {
            lines.remove(index);
            while index < lines.len() && !lines[index].trim_start().starts_with('[') {
                lines.remove(index);
            }
            continue;
        }
        index += 1;
    }
}

/// Removes every `[defaults] key = "<id>"` entry naming `id`, so removing a
/// provider does not leave a default pointing at a provider that is gone.
fn remove_default_provider(lines: &mut Vec<String>, id: &str) {
    let double = format!("{id:?}");
    let single = format!("'{id}'");
    let mut in_defaults = false;
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with('[') {
            in_defaults = trimmed == "[defaults]";
            index += 1;
            continue;
        }
        if in_defaults {
            if let Some((_, value)) = trimmed.split_once('=') {
                if value.trim() == double || value.trim() == single {
                    lines.remove(index);
                    continue;
                }
            }
        }
        index += 1;
    }
}

/// Parses a candidate overlay, merges it over the base configuration and
/// resolves the provider registry, so a broken provider is rejected before it
/// is written.
fn validate_providers_overlay(
    base: &Path,
    overlay_path: &Path,
    overlay: &str,
) -> Result<(), ConfigError> {
    let overlay = if overlay.trim().is_empty() {
        toml::Value::Table(toml::Table::new())
    } else {
        toml::from_str(overlay).map_err(|source| ConfigError::Parse {
            path: overlay_path.to_path_buf(),
            source,
        })?
    };
    let mut value = read_config_value(base)?;
    merge_toml(&mut value, overlay);
    providers_from_value(&value).map(|_| ())
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

/// The deployment the selected provider id resolves to, for re-syncing an
/// edited provider's active deployment.
fn active_deployment(providers: &[ChatProvider], id: &Option<String>) -> Option<ChatDeployment> {
    let id = id.as_deref()?;
    providers
        .iter()
        .find(|provider| provider.id == id)
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
            providers
                .chat_deployment_for(id, Capability::Text)
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

        assert_eq!(config.text_provider_id.as_deref(), Some("local"));
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
            "# retain this comment\n[providers.local]\napi_key_env = 'TEST_LOCAL_KEY'\n",
        )
        .unwrap();
        AppConfig::persist_chat_endpoint_to(
            &user,
            "local",
            ChatEndpointField::Model,
            "chosen-model",
        )
        .unwrap();
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
        assert!(saved.contains("TEST_LOCAL_KEY"));
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
            "[providers.local]\nmodel = 'chosen'\napi_key = 'test-only-secret'\n";
        std::fs::write(&old_app, original_app).unwrap();
        std::fs::write(&old_providers, "invalid = [").unwrap();
        assert!(migrate_user_config(&destination).is_err());
        assert!(!destination.exists());
        std::fs::write(&old_providers, original_providers).unwrap();
        migrate_user_config(&destination).unwrap();
        let value = read_config_value(&destination).unwrap();
        assert_eq!(value["generation"]["steps"].as_integer(), Some(9));
        assert_eq!(
            value["providers"]["local"]["api_key"].as_str(),
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
    fn selecting_a_provider_repoints_the_deployment_in_memory() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("storyfella.toml");
        std::fs::write(&path, TWO_TEXT_PROVIDERS).expect("fixture should write");

        let config = AppConfig::load_from(&path).expect("config loads");
        assert_eq!(config.text.as_ref().unwrap().model, "prism-ml/bonsai-27b");

        let switched = config
            .with_text_provider("deepseek")
            .expect("deepseek serves text");
        assert_eq!(switched.text_provider_id.as_deref(), Some("deepseek"));
        let text = switched.text.as_ref().expect("text deployment");
        assert_eq!(text.model, "deepseek-chat");
        assert_eq!(text.base_url, "https://api.deepseek.com/v1");
        // The original is untouched. Assigning a provider to a capability it
        // did not declare records the capability on it.
        assert_eq!(config.text_provider_id.as_deref(), Some("local"));
        let repointed = config
            .with_text_provider("vision")
            .expect("every chat provider can be assigned");
        assert_eq!(repointed.text_provider_id.as_deref(), Some("vision"));
        let vision = repointed
            .chat_providers
            .iter()
            .find(|provider| provider.id == "vision")
            .unwrap();
        assert!(vision.capabilities.contains(&"text".to_owned()));
        assert!(config.with_text_provider("nope").is_none());
    }

    #[test]
    fn editing_a_chat_endpoint_field_updates_the_provider_and_the_active_deployment() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("storyfella.toml");
        std::fs::write(&path, TWO_TEXT_PROVIDERS).expect("fixture should write");
        let config = AppConfig::load_from(&path).expect("config loads");

        // `local` is the active `text` provider; editing it re-syncs the
        // resolved deployment as well as the provider list entry.
        let moved = config
            .with_chat_endpoint(
                "local",
                ChatEndpointField::BaseUrl,
                "http://127.0.0.1:9999/v1",
            )
            .expect("local is a chat provider");
        assert_eq!(
            moved.text.as_ref().unwrap().base_url,
            "http://127.0.0.1:9999/v1"
        );
        assert_eq!(
            moved
                .chat_providers
                .iter()
                .find(|provider| provider.id == "local")
                .unwrap()
                .deployment
                .base_url,
            "http://127.0.0.1:9999/v1"
        );

        // Editing an inactive provider changes it without activating it.
        let renamed = moved
            .with_chat_endpoint("deepseek", ChatEndpointField::Model, "deepseek-reasoner")
            .expect("deepseek is a chat provider");
        assert_eq!(renamed.text.as_ref().unwrap().model, "prism-ml/bonsai-27b");
        assert_eq!(
            renamed
                .chat_providers
                .iter()
                .find(|provider| provider.id == "deepseek")
                .unwrap()
                .deployment
                .model,
            "deepseek-reasoner"
        );

        // An unknown id changes nothing; an empty base URL or model is rejected.
        assert!(config
            .with_chat_endpoint("nope", ChatEndpointField::Model, "x")
            .is_none());
        assert!(config
            .with_chat_endpoint("local", ChatEndpointField::BaseUrl, "   ")
            .is_none());
    }

    #[test]
    fn clearing_an_api_key_removes_the_key_so_api_key_env_can_apply() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("providers.toml");
        std::fs::write(
            &path,
            "[providers.local]\nbase_url = \"http://localhost:8080/v1\"\n\
             api_key = \"sk-old\"\napi_key_env = \"LOCAL_KEY\"\n",
        )
        .expect("fixture should write");

        AppConfig::persist_chat_endpoint_to(&path, "local", ChatEndpointField::ApiKey, "")
            .expect("persist should write");
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(!contents.contains("sk-old"), "{contents}");
        assert!(!contents.contains("api_key ="), "{contents}");
        assert!(
            contents.contains("api_key_env = \"LOCAL_KEY\""),
            "the env fallback survives: {contents}"
        );
        assert!(
            contents.contains("base_url"),
            "unrelated keys survive: {contents}"
        );
    }

    #[test]
    fn a_managed_provider_reports_its_launch_and_an_external_one_does_not() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("storyfella.toml");
        std::fs::write(
            &path,
            "[defaults]\n\
             t2i = \"qwen-image\"\n\
             text = \"remote\"\n\
             vision = \"managed\"\n\
             [providers.qwen-image]\n\
             kind = \"sdcpp\"\n\
             architecture = \"qwen-image-2.1\"\n\
             capabilities = [\"t2i\"]\n\
             base_url = \"http://127.0.0.1:7861\"\n\
             [providers.remote]\n\
             kind = \"openai\"\n\
             capabilities = [\"text\"]\n\
             base_url = \"https://api.deepseek.com/v1\"\n\
             model = \"deepseek-chat\"\n\
             [providers.managed]\n\
             kind = \"llamacpp\"\n\
             capabilities = [\"vision\"]\n\
             base_url = \"http://127.0.0.1:8081/v1\"\n\
             model = \"gemma-4-e4b\"\n\
             executable = \"/opt/llama-server\"\n\
             model_path = \"/models/gemma.gguf\"\n\
             mmproj = \"/models/mmproj.gguf\"\n",
        )
        .expect("fixture should write");
        let config = AppConfig::load_from(&path).expect("config loads");

        // An external endpoint has no launch the app owns.
        let remote = config
            .chat_providers
            .iter()
            .find(|provider| provider.id == "remote")
            .expect("remote listed");
        assert!(remote.is_external());
        assert!(remote.managed_launch().is_none());

        // A managed server reports what the app launches.
        let managed = config
            .chat_providers
            .iter()
            .find(|provider| provider.id == "managed")
            .expect("managed listed");
        assert!(!managed.is_external());
        let launch = managed.managed_launch().expect("managed launch");
        assert_eq!(launch.executable, "/opt/llama-server");
        assert_eq!(launch.model_path, "/models/gemma.gguf");
        assert_eq!(launch.mmproj.as_deref(), Some("/models/mmproj.gguf"));
    }

    #[test]
    fn editing_a_managed_provider_base_url_moves_the_server_it_launches() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("storyfella.toml");
        std::fs::write(
            &path,
            "[defaults]\n\
             t2i = \"qwen-image\"\n\
             vision = \"managed\"\n\
             [providers.qwen-image]\n\
             kind = \"sdcpp\"\n\
             architecture = \"qwen-image-2.1\"\n\
             capabilities = [\"t2i\"]\n\
             base_url = \"http://127.0.0.1:7861\"\n\
             [providers.managed]\n\
             kind = \"llamacpp\"\n\
             capabilities = [\"vision\"]\n\
             base_url = \"http://127.0.0.1:8081/v1\"\n\
             model = \"gemma-4-e4b\"\n\
             executable = \"/opt/llama-server\"\n\
             model_path = \"/models/gemma.gguf\"\n",
        )
        .expect("fixture should write");
        let config = AppConfig::load_from(&path).expect("config loads");
        let before = config
            .vision
            .as_ref()
            .and_then(|deployment| deployment.server.as_ref())
            .expect("managed vision server");
        assert_eq!((before.host.as_str(), before.port), ("127.0.0.1", 8081));

        let moved = config
            .with_chat_endpoint(
                "managed",
                ChatEndpointField::BaseUrl,
                "http://127.0.0.1:9090/v1",
            )
            .expect("base URL edit applies");
        let vision = moved.vision.as_ref().expect("vision deployment");
        assert_eq!(vision.base_url, "http://127.0.0.1:9090/v1");
        let server = vision.server.as_ref().expect("managed server");
        assert_eq!(
            (server.host.as_str(), server.port),
            ("127.0.0.1", 9090),
            "the launch address follows the client URL"
        );

        // An unparseable base URL is rejected rather than desyncing the two.
        assert!(config
            .with_chat_endpoint("managed", ChatEndpointField::BaseUrl, "not a url")
            .is_none());
    }

    #[test]
    fn switching_the_default_provider_persists_and_takes_effect() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let base = dir.join("base.toml");
        let user = dir.join("storyfella.toml");
        std::fs::write(&base, TWO_TEXT_PROVIDERS).expect("base fixture should write");

        // The checked-in default is the local endpoint...
        let config = AppConfig::load_layered(base.clone(), Some(user.clone())).unwrap();
        assert_eq!(config.text_provider_id.as_deref(), Some("local"));
        assert_eq!(config.text.as_ref().unwrap().model, "prism-ml/bonsai-27b");

        // ...until the picker switches to deepseek, which is written under
        // `[defaults]` in the user overlay and wins on the next load.
        AppConfig::persist_default_provider_to(&user, Capability::Text, "deepseek")
            .expect("persist should write");
        let config = AppConfig::load_layered(base, Some(user.clone())).unwrap();
        assert_eq!(config.text_provider_id.as_deref(), Some("deepseek"));
        let text = config.text.as_ref().expect("text deployment");
        assert_eq!(text.model, "deepseek-chat");
        assert_eq!(text.base_url, "https://api.deepseek.com/v1");
        let saved = std::fs::read_to_string(&user).unwrap();
        assert!(saved.contains("text = \"deepseek\""), "{saved}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persist_default_provider_rewrites_only_the_defaults_line() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("storyfella.toml");
        std::fs::write(
            &path,
            "# keep me\n[defaults]\ntext = \"local\"\nvision = \"vision\"\n\n\
             [providers.local]\nmodel = \"prism-ml/bonsai-27b\"\n",
        )
        .expect("fixture should write");

        AppConfig::persist_default_provider_to(&path, Capability::Text, "deepseek")
            .expect("persist should write");
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(contents.contains("# keep me"), "{contents}");
        assert!(contents.contains("text = \"deepseek\""), "{contents}");
        assert!(contents.contains("vision = \"vision\""), "{contents}");
        assert!(
            contents.contains("model = \"prism-ml/bonsai-27b\""),
            "{contents}"
        );
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
    fn persist_chat_endpoint_writes_under_the_provider_section_line_preservingly() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("providers.toml");
        std::fs::write(
            &path,
            "# keep me\n[providers.local]\nkind = \"openai\"\nmodel = \"old-model\"\n\n\
             [providers.vision]\nmodel = \"google/gemma-4-e4b\"\n",
        )
        .expect("fixture should write");

        AppConfig::persist_chat_endpoint_to(
            &path,
            "local",
            ChatEndpointField::Model,
            "deepseek-chat",
        )
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
            contents.find("[providers.local]") < contents.find("[providers.vision]"),
            "section order must survive: {contents}"
        );
    }

    #[test]
    fn persist_chat_endpoint_adds_the_provider_section_when_missing() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let path = dir.join("providers.toml");
        std::fs::write(&path, "[defaults]\ntext = \"local\"\n").expect("fixture should write");

        AppConfig::persist_chat_endpoint_to(&path, "local", ChatEndpointField::Model, "new-model")
            .expect("persist should write");
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(contents.contains("[providers.local]"), "{contents}");
        assert!(contents.contains("model = \"new-model\""), "{contents}");
        assert!(
            contents.contains("text = \"local\""),
            "defaults must survive: {contents}"
        );
    }

    #[test]
    fn adding_a_provider_persists_it_and_marks_it_user_owned() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let base = dir.join("base.toml");
        let user = dir.join("user.toml");
        std::fs::write(&base, TWO_TEXT_PROVIDERS).expect("base fixture should write");
        std::fs::write(&user, "# my settings\n").expect("user fixture should write");

        let provider = NewChatProvider {
            id: "lmstudio".to_owned(),
            kind: NewChatProviderKind::External,
            capabilities: vec![ChatCapability::Text],
            base_url: "http://localhost:1234/v1".to_owned(),
            model: "qwen2.5-7b-instruct".to_owned(),
            api_key: None,
            executable: None,
            model_path: None,
        };
        AppConfig::persist_provider_added_to(&user, &base, &provider)
            .expect("persist should write");

        let contents = std::fs::read_to_string(&user).expect("file should read");
        assert!(contents.contains("[providers.lmstudio]"), "{contents}");
        assert!(
            contents.contains("# my settings"),
            "comments survive: {contents}"
        );

        let config = AppConfig::load_layered(base, Some(user)).expect("layered config should load");
        assert_eq!(config.user_providers, vec!["lmstudio".to_owned()]);
        assert!(
            config
                .chat_providers
                .iter()
                .any(|provider| provider.id == "lmstudio"),
            "the new provider is selectable"
        );
    }

    #[test]
    fn a_broken_provider_is_rejected_before_it_is_written() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let base = dir.join("base.toml");
        let user = dir.join("user.toml");
        std::fs::write(&base, TWO_TEXT_PROVIDERS).expect("base fixture should write");
        std::fs::write(&user, "").expect("user fixture should write");

        // A managed provider needs a model file; validation must refuse it.
        let provider = NewChatProvider {
            id: "broken".to_owned(),
            kind: NewChatProviderKind::Managed,
            capabilities: vec![ChatCapability::Text],
            base_url: "http://127.0.0.1:8090".to_owned(),
            model: "local".to_owned(),
            api_key: None,
            executable: None,
            model_path: None,
        };
        assert!(
            AppConfig::persist_provider_added_to(&user, &base, &provider).is_err(),
            "a managed provider without a model file must be rejected"
        );
        assert_eq!(
            std::fs::read_to_string(&user).expect("file should read"),
            "",
            "a rejected provider leaves the file untouched"
        );
    }

    #[test]
    fn removing_a_user_provider_drops_it_and_its_default() {
        let dir = std::env::temp_dir().join(format!("svs-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let base = dir.join("base.toml");
        let user = dir.join("user.toml");
        std::fs::write(&base, TWO_TEXT_PROVIDERS).expect("base fixture should write");
        std::fs::write(
            &user,
            "[defaults]\ntext = \"mine\"\n\n\
             [providers.mine]\nkind = \"openai\"\ncapabilities = [\"text\"]\n\
             base_url = \"http://localhost:1234/v1\"\nmodel = \"local-model\"\n",
        )
        .expect("user fixture should write");

        // The provider is a default before removal; it must not be left dangling.
        let before =
            AppConfig::load_layered(base.clone(), Some(user.clone())).expect("config should load");
        assert_eq!(before.text_provider_id.as_deref(), Some("mine"));

        AppConfig::persist_provider_removed_to(&user, &base, "mine").expect("persist should write");
        let contents = std::fs::read_to_string(&user).expect("file should read");
        assert!(!contents.contains("[providers.mine]"), "{contents}");
        assert!(!contents.contains("text = \"mine\""), "{contents}");

        let after = AppConfig::load_layered(base, Some(user)).expect("config should still load");
        assert_eq!(after.text_provider_id.as_deref(), Some("local"));
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

        AppConfig::persist_chat_endpoint_to(
            &path,
            "local",
            ChatEndpointField::Model,
            "deepseek-chat",
        )
        .expect("persist should create the directory and write");
        let contents = std::fs::read_to_string(&path).expect("file should read");
        assert!(contents.contains("[providers.local]"), "{contents}");
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
