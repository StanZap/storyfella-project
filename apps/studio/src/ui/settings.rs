//! The Settings window: appearance and playback, the models Studio uses, and
//! where it keeps its files. Each change is written at once and adopted by
//! the Studio window.
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
};

use design_system::{
    ButtonKind, ComboBox, ComboOption, ComboSelection, NumericControl, NumericEvent, NumericSpec,
    Segment, SegmentedControl, Theme, TypeScale, Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::{
    config::{
        AppConfig, ChatCapability, ConfigError, InterfaceSize, ThemePreference, TEXT_CONTEXT_TOKENS,
    },
    errors::{CodedError, ErrorCode, ReportedError},
    runtime::{HealthStatus, ServiceHealth, ServiceId},
    workflows::{AssistantSettings, Thinking, MAX_OUTPUT_TOKENS, MAX_STEPS},
};

use super::{AutosaveMode, Studio, StudioEvent};

const SECTIONS: [Segment; 3] = [
    Segment::new("general", "General"),
    Segment::new("models", "Models"),
    Segment::new("storage", "Storage"),
];
const THEMES: [Segment; 3] = [
    Segment::new("dark", "Dark"),
    Segment::new("light", "Light"),
    Segment::new("system", "System"),
];
const INTERFACE_SIZES: [Segment; 3] = [
    Segment::new("small", "Small"),
    Segment::new("default", "Default"),
    Segment::new("large", "Large"),
];
const AUTOSAVE: [Segment; 3] = [
    Segment::new("on-change", "On change"),
    Segment::new("every-minute", "Every minute"),
    Segment::new("off", "Off"),
];
const THINKING: [Segment; 5] = [
    Segment::new("default", "Model default"),
    Segment::new("off", "Off"),
    Segment::new("low", "Low"),
    Segment::new("medium", "Medium"),
    Segment::new("high", "High"),
];
/// The reply limit a switched-on limit starts from.
const REPLY_LIMIT: u32 = 8192;
const ARTWORK: [Segment; 2] = [Segment::new("fill", "Fill"), Segment::new("fit", "Fit")];

/// A fingerprint of a file's contents, to notice when another app rewrites it.
pub fn file_stamp(path: Option<&Path>) -> Option<u64> {
    let bytes = std::fs::read(path?).ok()?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

/// One chat capability's provider and model pickers.
struct Pickers {
    capability: ChatCapability,
    provider: Entity<ComboBox>,
    model: Entity<ComboBox>,
    /// The ids behind `provider`'s options, and the names behind `model`'s.
    providers: Vec<String>,
    models: Vec<String>,
}

pub struct Settings {
    studio: WeakEntity<Studio>,
    window: AnyWindowHandle,
    section: usize,
    autoplay: Entity<NumericControl>,
    text_size: Entity<NumericControl>,
    text: Pickers,
    vision: Pickers,
    reply_limit: Entity<NumericControl>,
    context: Entity<NumericControl>,
    steps: Entity<NumericControl>,
    health: Option<Vec<ServiceHealth>>,
    checking: Option<Task<()>>,
    /// The last failure, indicated at the top of the window.
    error: Option<ReportedError>,
    /// Whether its details are open.
    showing_error: bool,
}

/// Opens the Settings window for `studio`.
pub fn open(studio: Entity<Studio>, cx: &mut App) -> Option<WindowHandle<Settings>> {
    let size = gpui_kit::size(px(620.), px(560.));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: Some(TitlebarOptions {
            title: Some("Settings".into()),
            appears_transparent: true,
            traffic_light_position: Some(super::traffic_lights()),
        }),
        focus: true,
        app_id: Some(super::APP_ID.into()),
        ..WindowOptions::default()
    };
    cx.open_window(options, |window, cx| {
        cx.new(|cx| Settings::new(studio, window, cx))
    })
    .map_err(|error| tracing::error!(%error, "open the Settings window"))
    .ok()
}

impl Settings {
    fn new(studio: Entity<Studio>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (config, theme) = {
            let studio = studio.read(cx);
            (studio.config().clone(), studio.theme())
        };
        cx.subscribe_in(
            &studio,
            window,
            |settings, _, event, window, cx| match event {
                StudioEvent::ConfigChanged => {
                    settings.apply_theme(cx);
                    settings.refresh_pickers(window, cx);
                }
            },
        )
        .detach();
        cx.observe(&studio, |_, _, cx| cx.notify()).detach();
        let preview = config.preview;
        let autoplay = cx.new(|cx| {
            NumericControl::new(
                NumericSpec::new("Autoplay delay", "s", 1..=30, 3),
                Some(preview.autoplay_seconds as i32),
                theme,
                window,
                cx,
            )
        });
        let text_size = cx.new(|cx| {
            NumericControl::new(
                NumericSpec::new("Text size", "%", 75..=150, 100),
                Some(preview.text_percent as i32),
                theme,
                window,
                cx,
            )
        });
        cx.subscribe_in(&autoplay, window, |settings, _, event, window, cx| {
            if let NumericEvent::Commit { value, .. } = event {
                let value = *value as u32;
                settings.update_preview(|preview| preview.autoplay_seconds = value, window, cx);
            }
        })
        .detach();
        cx.subscribe_in(&text_size, window, |settings, _, event, window, cx| {
            if let NumericEvent::Commit { value, .. } = event {
                let value = *value as u32;
                settings.update_preview(|preview| preview.text_percent = value, window, cx);
            }
        })
        .detach();
        let assistant = config.assistant;
        let numeric =
            |spec: NumericSpec, value: u32, window: &mut Window, cx: &mut Context<Self>| {
                cx.new(|cx| NumericControl::new(spec, Some(value as i32), theme, window, cx))
            };
        let spec = |label: &str, unit: &str, range: std::ops::RangeInclusive<u32>, default: u32| {
            NumericSpec::new(
                label.to_owned(),
                unit.to_owned(),
                *range.start() as i32..=*range.end() as i32,
                default as i32,
            )
        };
        let reply_limit = numeric(
            spec("Reply limit", "tokens", MAX_OUTPUT_TOKENS, REPLY_LIMIT),
            assistant.max_output_tokens.unwrap_or(REPLY_LIMIT),
            window,
            cx,
        );
        let default_context = agentkit_ai::native::llama::DEFAULT_CONTEXT_TOKENS;
        let context = numeric(
            spec("Context", "tokens", TEXT_CONTEXT_TOKENS, default_context),
            config.text_context_tokens().unwrap_or(default_context),
            window,
            cx,
        );
        let defaults = AssistantSettings::default();
        let steps = numeric(
            spec("Steps per request", "", MAX_STEPS, defaults.max_steps),
            assistant.max_steps,
            window,
            cx,
        );
        cx.subscribe_in(&reply_limit, window, |settings, _, event, window, cx| {
            if let NumericEvent::Commit { value, .. } = event {
                let value = *value as u32;
                settings.update_assistant(
                    |assistant| assistant.max_output_tokens = Some(value),
                    window,
                    cx,
                );
            }
        })
        .detach();
        cx.subscribe_in(&steps, window, |settings, _, event, window, cx| {
            if let NumericEvent::Commit { value, .. } = event {
                let value = *value as u32;
                settings.update_assistant(|assistant| assistant.max_steps = value, window, cx);
            }
        })
        .detach();
        cx.subscribe_in(&context, window, |settings, _, event, window, cx| {
            if let NumericEvent::Commit { value, .. } = event {
                let value = *value as u32;
                settings.persist(
                    |config| config.persist_text_context_tokens(Some(value)),
                    window,
                    cx,
                );
            }
        })
        .detach();
        let text = Pickers::new(ChatCapability::Text, theme, window, cx);
        let vision = Pickers::new(ChatCapability::Vision, theme, window, cx);
        let mut settings = Self {
            studio: studio.downgrade(),
            window: window.window_handle(),
            section: 0,
            autoplay,
            text_size,
            text,
            vision,
            reply_limit,
            context,
            steps,
            health: None,
            checking: None,
            error: None,
            showing_error: false,
        };
        settings.refresh_pickers(window, cx);
        settings.check_health(cx);
        settings
    }

    fn config(&self, cx: &App) -> Option<AppConfig> {
        self.studio
            .upgrade()
            .map(|studio| studio.read(cx).config().clone())
    }

    fn theme(&self, cx: &App) -> Theme {
        self.studio
            .upgrade()
            .map_or_else(Theme::dark, |studio| studio.read(cx).theme())
    }

    /// Writes a setting, then hands the reloaded configuration to Studio.
    fn persist(
        &mut self,
        write: impl FnOnce(&AppConfig) -> Result<(), ConfigError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.config(cx) else {
            return;
        };
        let result = write(&config).and_then(|()| AppConfig::load());
        match result {
            Ok(config) => {
                self.error = None;
                self.studio
                    .update(cx, |studio, cx| studio.set_config(config, window, cx))
                    .ok();
            }
            Err(error) => self.error = Some(error.report()),
        }
        cx.notify();
    }

    fn update_preview(
        &mut self,
        change: impl FnOnce(&mut storyfella_studio_core::config::PreviewSettings),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.config(cx) else {
            return;
        };
        let mut preview = config.preview;
        change(&mut preview);
        self.persist(
            |config| config.persist_preview_settings(preview),
            window,
            cx,
        );
    }

    fn update_assistant(
        &mut self,
        change: impl FnOnce(&mut AssistantSettings),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.config(cx) else {
            return;
        };
        let mut assistant = config.assistant;
        change(&mut assistant);
        self.persist(
            |config| config.persist_assistant_settings(assistant),
            window,
            cx,
        );
    }

    fn apply_theme(&mut self, cx: &mut Context<Self>) {
        let theme = self.theme(cx);
        for control in [
            &self.autoplay,
            &self.text_size,
            &self.reply_limit,
            &self.context,
            &self.steps,
        ] {
            control.update(cx, |control, cx| control.set_theme(theme, cx));
        }
        for picker in [
            &self.text.provider,
            &self.text.model,
            &self.vision.provider,
            &self.vision.model,
        ] {
            picker.update(cx, |picker, cx| picker.set_theme(theme, cx));
        }
    }

    /// Fills the pickers from the configuration and the last health check.
    fn refresh_pickers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.config(cx) else {
            return;
        };
        let context = config.text_context_tokens();
        self.context.update(cx, |control, cx| {
            if let Some(tokens) = context {
                control.set_value(Some(tokens as i32), cx);
            }
        });
        let health = self.health.clone();
        self.text.refresh(&config, health.as_deref(), window, cx);
        self.vision.refresh(&config, health.as_deref(), window, cx);
        cx.notify();
    }

    /// Asks each model endpoint whether it is up and what it serves. The
    /// checks only read; nothing is started.
    fn check_health(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config(cx) else {
            return;
        };
        let window = self.window;
        self.checking = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let health = cx
                    .background_spawn(async move {
                        tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map(|runtime| {
                                runtime
                                    .block_on(storyfella_studio_core::runtime::check_all(&config))
                            })
                            .ok()
                    })
                    .await;
                this.update(cx, |settings, cx| {
                    settings.checking = None;
                    settings.health = health;
                    cx.notify();
                })
                .ok();
                // The model lists come from the check, so the pickers follow it.
                window
                    .update(cx, |_, window, cx| {
                        this.update(cx, |settings, cx| settings.refresh_pickers(window, cx))
                            .ok();
                    })
                    .ok();
            }),
        );
        cx.notify();
    }

    fn select_provider(
        &mut self,
        capability: ChatCapability,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pickers = self.pickers(capability);
        let Some(id) = pickers.providers.get(index).cloned() else {
            return;
        };
        self.persist(
            |config| config.persist_provider_selection(capability, &id),
            window,
            cx,
        );
        self.check_health(cx);
    }

    fn select_model(
        &mut self,
        capability: ChatCapability,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = self.pickers(capability).models.get(index).cloned() else {
            return;
        };
        self.persist(
            |config| config.persist_model_selection(capability, &model),
            window,
            cx,
        );
    }

    fn pickers(&self, capability: ChatCapability) -> &Pickers {
        match capability {
            ChatCapability::Text => &self.text,
            ChatCapability::Vision => &self.vision,
        }
    }
}

impl Pickers {
    fn new(
        capability: ChatCapability,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Settings>,
    ) -> Self {
        let noun = match capability {
            ChatCapability::Text => "Text",
            ChatCapability::Vision => "Vision",
        };
        let provider = cx.new(|cx| {
            ComboBox::new(
                format!("{noun} provider"),
                "No provider",
                Vec::new(),
                theme,
                window,
                cx,
            )
        });
        let model = cx.new(|cx| {
            ComboBox::new(
                format!("{noun} model"),
                "No model",
                Vec::new(),
                theme,
                window,
                cx,
            )
        });
        cx.subscribe_in(
            &provider,
            window,
            move |settings, _, ComboSelection(index), window, cx| {
                settings.select_provider(capability, *index, window, cx)
            },
        )
        .detach();
        cx.subscribe_in(
            &model,
            window,
            move |settings, _, ComboSelection(index), window, cx| {
                settings.select_model(capability, *index, window, cx)
            },
        )
        .detach();
        Self {
            capability,
            provider,
            model,
            providers: Vec::new(),
            models: Vec::new(),
        }
    }

    /// The providers that serve this capability, and the models the chosen
    /// one reported, or only the configured model before it has answered.
    fn refresh(
        &mut self,
        config: &AppConfig,
        health: Option<&[ServiceHealth]>,
        window: &mut Window,
        cx: &mut Context<Settings>,
    ) {
        let (selected, deployment, service) = match self.capability {
            ChatCapability::Text => (&config.text_provider_id, &config.text, ServiceId::Llm),
            ChatCapability::Vision => (
                &config.vision_provider_id,
                &config.vision,
                ServiceId::Vision,
            ),
        };
        let serving: Vec<_> = config
            .chat_providers
            .iter()
            .filter(|provider| {
                provider
                    .capabilities
                    .iter()
                    .any(|item| item == self.capability.as_str())
            })
            .collect();
        self.providers = serving.iter().map(|provider| provider.id.clone()).collect();
        let options = serving
            .iter()
            .map(|provider| {
                ComboOption::new(
                    provider.id.clone(),
                    format!("{} · {}", provider.kind, provider.deployment.base_url),
                )
            })
            .collect();
        let selection = selected
            .as_ref()
            .and_then(|id| self.providers.iter().position(|item| item == id));
        self.provider.update(cx, |picker, cx| {
            picker.set_options(options, selection, window, cx)
        });

        let current = deployment
            .as_ref()
            .map(|deployment| deployment.model.clone());
        let reported = health
            .and_then(|health| health.iter().find(|item| item.id == service))
            .and_then(|item| item.models.clone());
        let mut models = reported.unwrap_or_default();
        if let Some(current) = &current {
            if !models.contains(current) {
                models.insert(0, current.clone());
            }
        }
        let selection = current
            .as_ref()
            .and_then(|model| models.iter().position(|item| item == model));
        let options = models
            .iter()
            .map(|model| ComboOption::new(model.clone(), ""))
            .collect();
        self.models = models;
        self.model.update(cx, |picker, cx| {
            picker.set_options(options, selection, window, cx)
        });
    }
}

fn status_color(status: HealthStatus, c: Theme) -> Hsla {
    match status {
        HealthStatus::Online => c.success,
        HealthStatus::Degraded => c.warning,
        HealthStatus::Idle => c.text_muted,
        HealthStatus::Offline => c.error,
    }
}

/// A labelled row: the name on the left, its control on the right.
fn row(c: Theme, label: &str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .min_h(c.metrics.control + px(8.))
        .child(
            div()
                .w(px(150.))
                .flex_none()
                .text_color(c.text_muted)
                .child(label.to_owned()),
        )
        .child(div().flex_1().min_w_0().child(control))
}

impl Settings {
    fn render_general(&self, config: &AppConfig, cx: &mut Context<Self>) -> Div {
        let c = self.theme(cx);
        let theme_index = ThemePreference::ALL
            .iter()
            .position(|item| *item == config.theme)
            .unwrap_or(0);
        let size_index = InterfaceSize::ALL
            .iter()
            .position(|item| *item == config.interface_size)
            .unwrap_or(1);
        let autosave = self
            .studio
            .upgrade()
            .map_or(AutosaveMode::OnChange, |studio| studio.read(cx).autosave());
        let autosave_index = AutosaveMode::ALL
            .iter()
            .position(|mode| *mode == autosave)
            .unwrap_or(0);
        let preview = config.preview;
        let this = cx.entity().downgrade();
        let on_theme = this.clone();
        let on_size = this.clone();
        let on_autosave = this.clone();
        let on_artwork = this.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(row(
                c,
                "Appearance",
                SegmentedControl::new(
                    "theme",
                    "Appearance",
                    &THEMES,
                    theme_index,
                    c,
                    move |index, window, cx| {
                        let theme = ThemePreference::ALL[index];
                        on_theme
                            .update(cx, |settings, cx| {
                                settings.persist(|config| config.persist_theme(theme), window, cx)
                            })
                            .ok();
                    },
                ),
            ))
            .child(row(
                c,
                "Interface size",
                SegmentedControl::new(
                    "interface-size",
                    "Interface size",
                    &INTERFACE_SIZES,
                    size_index,
                    c,
                    move |index, window, cx| {
                        let size = InterfaceSize::ALL[index];
                        on_size
                            .update(cx, |settings, cx| {
                                settings.persist(
                                    |config| config.persist_interface_size(size),
                                    window,
                                    cx,
                                )
                            })
                            .ok();
                    },
                ),
            ))
            .child(row(
                c,
                "Autosave",
                SegmentedControl::new(
                    "autosave",
                    "Autosave",
                    &AUTOSAVE,
                    autosave_index,
                    c,
                    move |index, _, cx| {
                        let mode = AutosaveMode::ALL[index];
                        on_autosave
                            .update(cx, |settings, cx| {
                                settings
                                    .studio
                                    .update(cx, |studio, cx| studio.set_autosave(mode, cx))
                                    .ok();
                            })
                            .ok();
                    },
                ),
            ))
            .child(
                div()
                    .pt(px(12.))
                    .pb(px(4.))
                    .type_scale(TypeScale::Small)
                    .text_color(c.text_muted)
                    .child("Playback"),
            )
            // Numeric controls carry their own labels above the field.
            .child(row(c, "", self.autoplay.clone()))
            .child(row(c, "", self.text_size.clone()))
            .child(row(
                c,
                "Artwork",
                SegmentedControl::new(
                    "artwork",
                    "Artwork",
                    &ARTWORK,
                    usize::from(preview.contain_artwork),
                    c,
                    move |index, window, cx| {
                        on_artwork
                            .update(cx, |settings, cx| {
                                settings.update_preview(
                                    |preview| preview.contain_artwork = index == 1,
                                    window,
                                    cx,
                                )
                            })
                            .ok();
                    },
                ),
            ))
            .child(row(
                c,
                "Diagnostics",
                c.switch(
                    "diagnostics",
                    "Show story diagnostics",
                    preview.show_diagnostics,
                )
                .on_click(cx.listener(move |settings, _, window, cx| {
                    settings.update_preview(
                        |preview| preview.show_diagnostics = !preview.show_diagnostics,
                        window,
                        cx,
                    )
                })),
            ))
    }

    fn render_models(&self, config: &AppConfig, cx: &mut Context<Self>) -> Div {
        let c = self.theme(cx);
        let separate = config.separate_vision;
        let status = |service: ServiceId| -> AnyElement {
            let (label, detail, color) = match &self.health {
                None if self.checking.is_some() => {
                    ("Checking…".into(), String::new(), c.text_muted)
                }
                None => ("Not checked".into(), String::new(), c.text_muted),
                Some(health) => match health.iter().find(|item| item.id == service) {
                    Some(item) => (
                        item.status.label().to_owned(),
                        item.detail.clone(),
                        status_color(item.status, c),
                    ),
                    None => ("Unknown".into(), String::new(), c.text_muted),
                },
            };
            div()
                .flex()
                .gap(px(8.))
                .type_scale(TypeScale::Small)
                .child(div().flex_none().text_color(color).child(label))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(c.text_muted)
                        .child(detail),
                )
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .when(config.chat_providers.is_empty(), |section| {
                section.child(
                    div()
                        .pb(px(8.))
                        .text_color(c.text_muted)
                        .child("No model providers are configured. Add one in Clipforge."),
                )
            })
            .child(row(c, "Text provider", self.text.provider.clone()))
            .child(row(c, "Text model", self.text.model.clone()))
            .child(row(c, "Text status", status(ServiceId::Llm)))
            .child(row(
                c,
                "Vision",
                c.switch("separate-vision", "Use a separate vision model", separate)
                    .on_click(cx.listener(move |settings, _, window, cx| {
                        settings.persist(
                            |config| config.persist_separate_vision(!separate),
                            window,
                            cx,
                        );
                        settings.check_health(cx);
                    })),
            ))
            .when(separate, |section| {
                section
                    .child(row(c, "Vision provider", self.vision.provider.clone()))
                    .child(row(c, "Vision model", self.vision.model.clone()))
                    .child(row(c, "Vision status", status(ServiceId::Vision)))
            })
            .child(self.render_assistant(config, cx))
            .child(
                div()
                    .pt(px(12.))
                    .flex()
                    .gap(px(8.))
                    .child(
                        c.button("refresh-models", "Check again", ButtonKind::Secondary)
                            .disabled(self.checking.is_some())
                            .on_click(cx.listener(|settings, _, _, cx| settings.check_health(cx))),
                    )
                    .child(
                        c.button(
                            "clipforge",
                            "Manage providers in Clipforge",
                            ButtonKind::Quiet,
                        )
                        .on_click(cx.listener(|settings, _, _, cx| {
                            if let Err(error) =
                                storyfella_studio_core::ai_config::open_provider_manager()
                            {
                                settings.error = Some(
                                    ReportedError::plain(
                                        ErrorCode::AppProviderManager,
                                        error.to_string(),
                                    )
                                    .log(),
                                );
                                cx.notify();
                            }
                        })),
                    ),
            )
    }

    /// How the assistant asks the text model, and how hard it tries.
    fn render_assistant(&self, config: &AppConfig, cx: &mut Context<Self>) -> Div {
        let c = self.theme(cx);
        let assistant = config.assistant;
        let thinking_index = assistant
            .thinking
            .and_then(|thinking| Thinking::ALL.iter().position(|item| *item == thinking))
            .map_or(0, |index| index + 1);
        let limited = assistant.max_output_tokens.is_some();
        let this = cx.entity().downgrade();
        let heading = |label: &'static str| {
            div()
                .pt(px(12.))
                .pb(px(4.))
                .type_scale(TypeScale::Small)
                .text_color(c.text_muted)
                .child(label)
        };
        let note = |text: &'static str| {
            div()
                .type_scale(TypeScale::Small)
                .text_color(c.text_muted)
                .child(text)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(heading("Assistant"))
            .child(row(
                c,
                "Thinking",
                SegmentedControl::new(
                    "thinking",
                    "Thinking",
                    &THINKING,
                    thinking_index,
                    c,
                    move |index, window, cx| {
                        let thinking = index.checked_sub(1).map(|index| Thinking::ALL[index]);
                        this.update(cx, |settings, cx| {
                            settings.update_assistant(
                                |assistant| assistant.thinking = thinking,
                                window,
                                cx,
                            )
                        })
                        .ok();
                    },
                ),
            ))
            .child(row(
                c,
                "Reply length",
                c.switch("reply-limit", "Limit reply length", limited).on_click(
                    cx.listener(move |settings, _, window, cx| {
                        let limit = settings
                            .reply_limit
                            .read(cx)
                            .value()
                            .map_or(REPLY_LIMIT, |value| value as u32);
                        settings.update_assistant(
                            |assistant| {
                                assistant.max_output_tokens = (!limited).then_some(limit)
                            },
                            window,
                            cx,
                        )
                    }),
                ),
            ))
            // Numeric controls carry their own labels above the field.
            .when(limited, |section| {
                section.child(row(c, "", self.reply_limit.clone()))
            })
            .child(match config.text_context_tokens() {
                Some(_) => row(c, "", self.context.clone()),
                None => row(
                    c,
                    "Context",
                    note("Set by the text provider's own server. Studio sizes the context only of the models it starts."),
                ),
            })
            .child(row(c, "", self.steps.clone()))
            .child(row(
                c,
                "",
                note("A Project request sends every file's text while the story is smaller than this, and only its files' paths and scenes beyond it."),
            ))
    }

    fn render_storage(&self, config: &AppConfig, cx: &mut Context<Self>) -> Div {
        let c = self.theme(cx);
        let paths: [(&str, Option<PathBuf>); 6] = [
            ("Projects", Some(config.project_dir.clone())),
            ("Artwork", Some(config.asset_dir.clone())),
            ("Cache", Some(config.cache_dir.clone())),
            ("Model state", Some(config.state_dir.clone())),
            ("Studio settings", config.app_config_path.clone()),
            ("Shared providers", config.ai_config_path.clone()),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(paths.into_iter().enumerate().map(|(index, (label, path))| {
                let shown = path
                    .as_ref()
                    .map_or_else(|| "Not set".to_owned(), |path| path.display().to_string());
                row(
                    c,
                    label,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().min_w_0().truncate().child(shown))
                        .when_some(path, |line, path| {
                            line.child(
                                c.button(("reveal", index), "Reveal", ButtonKind::Quiet)
                                    .on_click(move |_, _, cx| cx.reveal_path(&path)),
                            )
                        }),
                )
            }))
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme(cx);
        let Some(config) = self.config(cx) else {
            return div().size_full().bg(c.surface_deep).into_any_element();
        };
        window.set_rem_size(super::rem_size(config.interface_size));
        let body = match self.section {
            1 => self.render_models(&config, cx),
            2 => self.render_storage(&config, cx),
            _ => self.render_general(&config, cx),
        };
        let this = cx.entity().downgrade();
        div()
            .id("settings")
            .size_full()
            .flex()
            .flex_col()
            .bg(c.surface_deep)
            .text_color(c.text)
            .type_scale(TypeScale::Body)
            .child(
                div()
                    .h(super::title_bar_height())
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_b_1()
                    .border_color(c.hairline)
                    .bg(c.surface)
                    .child(SegmentedControl::new(
                        "sections",
                        "Settings section",
                        &SECTIONS,
                        self.section,
                        c,
                        move |index, _, cx| {
                            this.update(cx, |settings, cx| {
                                settings.section = index;
                                cx.notify();
                            })
                            .ok();
                        },
                    )),
            )
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .when_some(self.error.clone(), |body, error| {
                        body.child(super::details::failure_indication(
                            "settings-error",
                            c,
                            &error,
                            {
                                let settings = cx.entity().downgrade();
                                move |_, cx| {
                                    settings
                                        .update(cx, |settings, cx| {
                                            settings.showing_error = true;
                                            cx.notify();
                                        })
                                        .ok();
                                }
                            },
                        ))
                    })
                    .child(body),
            )
            .child({
                let settings = cx.entity().downgrade();
                super::details::details_dialog(
                    c,
                    self.error.as_ref().filter(|_| self.showing_error),
                    move |cx| {
                        settings
                            .update(cx, |settings, cx| {
                                // Read once, the failure has been seen.
                                settings.showing_error = false;
                                settings.error = None;
                                cx.notify();
                            })
                            .ok();
                    },
                )
            })
            .into_any_element()
    }
}
