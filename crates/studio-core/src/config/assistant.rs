//! Saving the assistant's settings: the `[assistant]` section of Studio's
//! settings, and the context of a managed text model, which belongs to its
//! provider in the shared registry.

use std::ops::RangeInclusive;

use agentkit_ai::{
    deployments::{ProviderError, ProviderKind, Providers},
    native::llama::MIN_CONTEXT_TOKENS,
};

use super::{read_config_contents, write_config_file, write_key, AppConfig, ConfigError};
use crate::workflows::AssistantSettings;

/// The context sizes Settings offers a managed text model.
pub const TEXT_CONTEXT_TOKENS: RangeInclusive<u32> = MIN_CONTEXT_TOKENS..=1_048_576;

impl AppConfig {
    pub fn persist_assistant_settings(
        &self,
        settings: AssistantSettings,
    ) -> Result<(), ConfigError> {
        let path = self
            .app_config_path
            .as_deref()
            .ok_or(ConfigError::PlatformDirectories)?;
        let contents = read_config_contents(path)?;
        let mut lines: Vec<_> = contents.lines().map(str::to_owned).collect();
        let settings = settings.bounded();
        for (key, value) in [
            (
                "thinking",
                settings
                    .thinking
                    .map(|thinking| format!("{:?}", thinking.as_str())),
            ),
            (
                "max_output_tokens",
                settings.max_output_tokens.map(|tokens| tokens.to_string()),
            ),
            (
                "repair_attempts",
                Some(settings.repair_attempts.to_string()),
            ),
            (
                "whole_story_kib",
                Some(settings.whole_story_kib.to_string()),
            ),
        ] {
            write_key(&mut lines, "assistant", key, value);
        }
        write_config_file(path, lines.join("\n"))
    }

    /// The context the text model loads with, when Studio starts it; `None`
    /// when its server is run by someone else.
    pub fn text_context_tokens(&self) -> Option<u32> {
        let server = self.text.as_ref()?.server.as_ref()?;
        Some(server.context_tokens)
    }

    /// Sets the context the managed text model loads with; `None` returns it
    /// to the default. Takes effect the next time the model starts.
    pub fn persist_text_context_tokens(&self, tokens: Option<u32>) -> Result<(), ConfigError> {
        let path = self
            .ai_config_path
            .as_deref()
            .ok_or(ConfigError::PlatformDirectories)?;
        let id = self
            .text_provider_id
            .as_deref()
            .filter(|id| {
                self.chat_providers
                    .iter()
                    .any(|provider| provider.id == *id && provider.kind == ProviderKind::LlamaCpp)
            })
            .ok_or_else(|| {
                ConfigError::Providers(ProviderError::UnknownProvider {
                    id: self.text_provider_id.clone().unwrap_or_default(),
                })
            })?;
        Providers::load(path)
            .and_then(|registry| registry.set_context_tokens(path, id, tokens))
            .map_err(ConfigError::Providers)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use agentkit_ai::Thinking;

    use super::*;

    const LOCAL: &str = "[defaults]\ntext = 'local'\n[providers.local]\nkind = 'llamacpp'\ncapabilities = ['text']\nbase_url = 'http://127.0.0.1:8081/v1'\nmodel = 'qwen'\nmodel_path = '/models/qwen.gguf'\n";

    #[test]
    fn assistant_settings_persist_within_their_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        fs::write(&app, "[paths]\ncache_dir = 'cache'\n").unwrap();
        fs::write(&ai, LOCAL).unwrap();
        let load = || AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        let config = load();
        assert_eq!(config.assistant, AssistantSettings::default());
        config
            .persist_assistant_settings(AssistantSettings {
                thinking: Some(Thinking::Off),
                max_output_tokens: Some(4096),
                repair_attempts: 9,
                whole_story_kib: 64,
            })
            .unwrap();
        assert_eq!(
            load().assistant,
            AssistantSettings {
                thinking: Some(Thinking::Off),
                max_output_tokens: Some(4096),
                repair_attempts: 5,
                whole_story_kib: 64,
            }
        );
        load()
            .persist_assistant_settings(AssistantSettings::default())
            .unwrap();
        assert_eq!(load().assistant, AssistantSettings::default());
        assert!(
            fs::read_to_string(&app).unwrap().contains("cache_dir"),
            "other settings are kept"
        );
    }

    #[test]
    fn a_managed_text_models_context_is_saved_with_its_provider() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("studio.toml");
        let ai = directory.path().join("clipforge.toml");
        fs::write(&app, "").unwrap();
        fs::write(&ai, LOCAL).unwrap();
        let load = || AppConfig::load_split(Some(app.clone()), Some(ai.clone())).unwrap();
        assert_eq!(
            load().text_context_tokens(),
            Some(agentkit_ai::native::llama::DEFAULT_CONTEXT_TOKENS)
        );
        load().persist_text_context_tokens(Some(16_384)).unwrap();
        assert_eq!(load().text_context_tokens(), Some(16_384));
        assert!(load().persist_text_context_tokens(Some(10)).is_err());
        load().persist_text_context_tokens(None).unwrap();
        assert_eq!(
            load().text_context_tokens(),
            Some(agentkit_ai::native::llama::DEFAULT_CONTEXT_TOKENS)
        );
    }
}
