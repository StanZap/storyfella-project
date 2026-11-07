use std::time::Duration;

use dioxus::prelude::*;
use uuid::Uuid;

use agentkit_ai::deployments::chat::Client as ChatClient;
use storyfella_studio_core::config::{AppConfig, ChatCapability, ChatProvider, ThemePreference};
use storyfella_studio_core::errors::{CodedError, ReportedError};
use storyfella_studio_core::registry::backend::CreativeBackend;
use storyfella_studio_core::runtime::{
    check_all as check_services, CreativeRuntime, HealthStatus, ServiceHealth, ServiceId,
};

use super::{
    components::{
        ErrorDetails, SearchSelect, SectionHeading, SelectOption, SettingRow, StatusDot, Toggle,
    },
    icons::{Icon, IconName},
    AutosaveMode,
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum SettingsSection {
    General,
    Models,
    Storage,
}

#[derive(Clone, Debug, PartialEq)]
enum ModelListState {
    Empty,
    Loading(Uuid),
    Ready(Vec<String>),
    /// A managed `llama-server` that is not running: its models cannot be
    /// listed until the app starts it on demand.
    OnDemand,
    Failed(ReportedError),
}

#[component]
pub fn Settings(config: AppConfig, backend: Signal<CreativeBackend>) -> Element {
    let runtime = backend.read().runtime();
    let mut section = use_signal(|| SettingsSection::Models);
    let config = use_signal(|| config);
    let health = use_signal(Vec::<ServiceHealth>::new);
    let checking = use_signal(|| None::<Uuid>);
    let checked_at = use_signal(|| None::<String>);
    let text_models = use_signal(|| ModelListState::Empty);
    let vision_models = use_signal(|| ModelListState::Empty);
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let targets = ConfigTargets {
        config,
        backend,
        health,
        checking,
        checked_at,
        text_models,
        vision_models,
    };
    let initial_runtime = runtime.clone();
    let mut initialized = use_signal(|| false);
    use_effect(move || {
        if !initialized() {
            initialized.set(true);
            let current = config.read().clone();
            apply_config(current, initial_runtime.clone(), targets);
        }
    });
    let refresh_runtime = runtime.clone();
    let on_refresh = move |_| match AppConfig::load() {
        Ok(next) => apply_config(next, refresh_runtime.clone(), targets),
        Err(error) => error_report.set(Some(error.report().log())),
    };
    let select_runtime = runtime.clone();
    let on_select =
        move |(capability, provider, model): (ChatCapability, String, Option<String>)| {
            let result = config
                .read()
                .persist_model_selection(capability, &provider, model.as_deref())
                .and_then(|_| AppConfig::load());
            match result {
                Ok(next) => apply_config(next, select_runtime.clone(), targets),
                Err(error) => error_report.set(Some(error.report().log())),
            }
        };
    let vision_runtime = runtime.clone();
    let on_separate_vision = move |separate: bool| {
        let result = config
            .read()
            .persist_separate_vision(separate)
            .and_then(|_| AppConfig::load());
        match result {
            Ok(next) => apply_config(next, vision_runtime.clone(), targets),
            Err(error) => error_report.set(Some(error.report().log())),
        }
    };
    let on_fetch_text = move |_| fetch_models(&config.read(), ChatCapability::Text, text_models);
    let on_fetch_vision =
        move |_| fetch_models(&config.read(), ChatCapability::Vision, vision_models);
    rsx! {
        section { class: "grid h-full min-h-0 grid-cols-[210px_minmax(0,1fr)]",
            aside { class: "border-r border-white/[0.055] px-5 py-8",
                p { class: "mb-5 px-3 text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-600", "Settings" }
                nav { class: "space-y-1",
                    SettingsNav { label: "General", active: section() == SettingsSection::General, onclick: move |_| section.set(SettingsSection::General) }
                    SettingsNav { label: "Models", active: section() == SettingsSection::Models, onclick: move |_| section.set(SettingsSection::Models) }
                    SettingsNav { label: "Storage", active: section() == SettingsSection::Storage, onclick: move |_| section.set(SettingsSection::Storage) }
                }
            }
            div { class: "min-h-0 overflow-y-auto px-10 py-9",
                div { class: "mx-auto max-w-3xl",
                    div { hidden: section() != SettingsSection::General, GeneralSettings { config } }
                    div { hidden: section() != SettingsSection::Models, ModelsSettings { config: config(), health, checking, checked_at, text_models, vision_models, on_refresh, on_select, on_separate_vision, on_fetch_text, on_fetch_vision } }
                    div { hidden: section() != SettingsSection::Storage, StorageSettings { config: config() } }
                }
            }
        }
    }
}

fn run_checks(
    runtime: CreativeRuntime,
    config: AppConfig,
    mut health: Signal<Vec<ServiceHealth>>,
    mut checking: Signal<Option<Uuid>>,
    mut checked_at: Signal<Option<String>>,
) {
    let request = Uuid::new_v4();
    checking.set(Some(request));
    spawn(async move {
        let services = check_services(&config, &runtime).await;
        if *checking.peek() != Some(request) {
            return;
        }
        health.set(services);
        checked_at.set(Some(current_time_label()));
        checking.set(None);
    });
}

/// Discovers the model ids the provider serving `capability` offers. No
/// provider is an empty list, not a failure. A managed `llama-server` that is
/// not running cannot be listed, so it reports `OnDemand` rather than an error.
fn fetch_models(config: &AppConfig, capability: ChatCapability, mut state: Signal<ModelListState>) {
    let deployment = match capability {
        ChatCapability::Text => config.text.clone(),
        ChatCapability::Vision => config.vision.clone(),
    };
    let Some(mut deployment) = deployment else {
        state.set(ModelListState::Empty);
        return;
    };
    let managed = deployment.server.is_some();
    // Listing is a quick probe, not a generation: cap it well below the
    // deployment's (often generous) request timeout.
    deployment.timeout = Duration::from_secs(5);
    let request = Uuid::new_v4();
    state.set(ModelListState::Loading(request));
    spawn(async move {
        let result = ChatClient::new(deployment).list_models().await;
        if *state.peek() != ModelListState::Loading(request) {
            return;
        }
        match result {
            Ok(mut ids) => {
                ids.sort();
                ids.dedup();
                state.set(ModelListState::Ready(ids));
            }
            Err(_) if managed => state.set(ModelListState::OnDemand),
            Err(error) => state.set(ModelListState::Failed(error.report().log())),
        }
    });
}

/// The UI state a configuration reload refreshes: the effective config, the
/// planner client, the health check and each capability's model list.
#[derive(Clone, Copy)]
struct ConfigTargets {
    config: Signal<AppConfig>,
    backend: Signal<CreativeBackend>,
    health: Signal<Vec<ServiceHealth>>,
    checking: Signal<Option<Uuid>>,
    checked_at: Signal<Option<String>>,
    text_models: Signal<ModelListState>,
    vision_models: Signal<ModelListState>,
}

/// Applies a configuration reloaded from disk: replaces the effective config,
/// rebuilds the planner client, and re-probes the services and their models.
fn apply_config(next: AppConfig, runtime: CreativeRuntime, mut targets: ConfigTargets) {
    *targets.config.write() = next.clone();
    targets.backend.write().set_chat(&next);
    run_checks(
        runtime,
        next.clone(),
        targets.health,
        targets.checking,
        targets.checked_at,
    );
    fetch_models(&next, ChatCapability::Text, targets.text_models);
    fetch_models(&next, ChatCapability::Vision, targets.vision_models);
}

fn current_time_label() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let hour = (seconds / 3600) % 24;
    let minute = (seconds / 60) % 60;
    let second = seconds % 60;
    format!("{hour:02}:{minute:02}:{second:02} UTC")
}

#[component]
fn SettingsNav(label: String, active: bool, onclick: EventHandler<MouseEvent>) -> Element {
    let class = if active {
        "flex h-9 w-full items-center rounded-lg bg-white/[0.055] px-3 text-left text-[12px] font-medium text-zinc-100"
    } else {
        "flex h-9 w-full items-center rounded-lg px-3 text-left text-[12px] text-zinc-600 transition hover:bg-white/[0.03] hover:text-zinc-300"
    };
    rsx! { button { class, onclick, "{label}" } }
}

#[component]
fn GeneralSettings(mut config: Signal<AppConfig>) -> Element {
    let autosave = use_context::<Signal<AutosaveMode>>();
    let mut autosave = autosave;
    let mut theme = use_context::<Signal<ThemePreference>>();
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let theme_id = use_hook(|| format!("theme-{}", Uuid::new_v4()));
    let restore_theme_id = theme_id.clone();
    rsx! {
        SectionHeading {
            eyebrow: "Application",
            title: "General",
            description: "Keep the workspace quiet and predictable while long-running creative work happens in the background."
        }
        div { class: "border-y border-white/[0.06]",
            SettingRow { title: "Appearance", description: "Interface theme used throughout the app.",
                select {
                    id: theme_id.clone(),
                    class: "setting-control", aria_label: "Interface theme",
                    oninput: move |event| {
                        let Some(next) = ThemePreference::ALL.into_iter()
                            .find(|theme| theme.as_str() == event.value()) else { return; };
                        let current = config.read().clone();
                        match current.persist_theme(next) {
                            Ok(()) => { config.write().theme = next; theme.set(next); }
                            Err(error) => {
                                document::eval(&format!(
                                    "document.getElementById('{restore_theme_id}').value = {:?};",
                                    current.theme.as_str(),
                                ));
                                error_report.set(Some(error.report().log()));
                            }
                        }
                    },
                    for choice in ThemePreference::ALL {
                        option { value: choice.as_str(), selected: theme() == choice, "{choice.label()}" }
                    }
                }
            }
            SettingRow { title: "Autosave", description: "Save project changes locally as you work.", last: true,
                select {
                    aria_label: "Autosave for this session",
                    class: "setting-control",
                    // oninput + option-level selected — the pattern that
                    // actually commits on <select> in Dioxus.
                    oninput: move |event| autosave.set(AutosaveMode::from_value(&event.value())),
                    option { selected: autosave() == AutosaveMode::OnChange, value: "on_change", "After every change" }
                    option { selected: autosave() == AutosaveMode::EveryMinute, value: "every_minute", "Every minute" }
                    option { selected: autosave() == AutosaveMode::Off, value: "off", "Off" }
                }
            }
        }
    }
}

#[component]
fn ModelsSettings(
    config: AppConfig,
    health: Signal<Vec<ServiceHealth>>,
    checking: Signal<Option<Uuid>>,
    checked_at: Signal<Option<String>>,
    text_models: Signal<ModelListState>,
    vision_models: Signal<ModelListState>,
    on_refresh: EventHandler<MouseEvent>,
    on_select: EventHandler<(ChatCapability, String, Option<String>)>,
    on_separate_vision: EventHandler<bool>,
    on_fetch_text: EventHandler<()>,
    on_fetch_vision: EventHandler<()>,
) -> Element {
    let services = health.read().clone();
    let (text_status, text_detail) = service_of(&services, ServiceId::Llm);
    let (vision_status, vision_detail) = service_of(&services, ServiceId::Vision);
    let providers = provider_options(&config.chat_providers);
    let text_provider = config.text_provider_id.clone().unwrap_or_default();
    let vision_provider = config.vision_provider_id.clone().unwrap_or_default();
    let text_model = config
        .text
        .as_ref()
        .map(|d| d.model.clone())
        .unwrap_or_default();
    let vision_model = config
        .vision
        .as_ref()
        .map(|d| d.model.clone())
        .unwrap_or_default();
    let text_id = text_provider.clone();
    let vision_id = vision_provider.clone();
    let mut launch_error = use_signal(|| None::<String>);
    rsx! {
        div { class: "flex flex-wrap items-end justify-between gap-x-4",
            SectionHeading { eyebrow: "Capabilities", title: "Models", description: "" }
            CheckBar { checking, checked_at, on_check: on_refresh }
        }
        ChatEndpointGroup {
            title: "Text (LLM)".to_owned(),
            providers: providers.clone(), provider_id: text_provider, model: text_model,
            status: text_status, detail: text_detail, models: text_models, on_fetch: on_fetch_text,
            on_select_provider: move |id| on_select.call((ChatCapability::Text, id, None)),
            on_select_model: move |model| on_select.call((ChatCapability::Text, text_id.clone(), Some(model))),
        }
        ChatEndpointGroup {
            title: "Vision (LLM)".to_owned(), show_fields: config.separate_vision,
            providers, provider_id: vision_provider, model: vision_model,
            status: vision_status, detail: vision_detail, models: vision_models, on_fetch: on_fetch_vision,
            on_select_provider: move |id| on_select.call((ChatCapability::Vision, id, None)),
            on_select_model: move |model| on_select.call((ChatCapability::Vision, vision_id.clone(), Some(model))),
            control: rsx! {
                div { class: "flex shrink-0 items-center gap-3",
                    span { class: "text-xs text-zinc-400", "Use a separate Vision LLM" }
                    Toggle {
                        label: "Use a separate Vision LLM", enabled: config.separate_vision,
                        title: "When off, Vision uses the Text model, which must support images",
                        onchange: on_separate_vision,
                    }
                }
            },
        }
        button {
            class: "mt-3 rounded bg-white/10 px-3 py-2 text-xs",
            title: "Manage providers, credentials, models and services in Clipforge",
            onclick: move |_| launch_error.set(open_clipforge().err()),
            "Manage providers in Clipforge"
        }
        if let Some(error) = launch_error() { p { role: "alert", class: "mt-2 text-xs text-amber-300", "{error}" } }
    }
}

fn open_clipforge() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open")
            .args(["-b", "com.clipforge.desktop"])
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(
                "Could not open Clipforge. Install its desktop app, then retry.".to_owned(),
            );
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "windows")]
        let executable = "clipforge-desktop.exe";
        #[cfg(not(target_os = "windows"))]
        let executable = "clipforge-desktop";
        std::process::Command::new(executable)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The shared check bar: when the services were last probed, and the action
/// that re-probes them all.
#[component]
fn CheckBar(
    checking: Signal<Option<Uuid>>,
    checked_at: Signal<Option<String>>,
    on_check: EventHandler<MouseEvent>,
) -> Element {
    let is_checking = checking().is_some();
    rsx! {
        div { class: "mb-7 flex justify-end",
            button {
                class: "flex h-8 items-center gap-2 rounded-lg bg-white/[0.055] px-3 text-[10px] font-medium text-zinc-400 transition hover:bg-white/[0.08] hover:text-white disabled:cursor-not-allowed disabled:opacity-40",
                title: if let Some(at) = checked_at() { format!("Reload providers and refresh model availability · Last checked {at}") } else { "Reload providers and refresh model availability".to_owned() },
                disabled: is_checking,
                onclick: move |event| on_check.call(event),
                if is_checking {
                    span { class: "size-3 animate-spin rounded-full border border-zinc-600 border-t-white" }
                    "Checking…"
                } else {
                    Icon { name: IconName::Refresh, class: "size-3" }
                    "Refresh providers"
                }
            }
        }
    }
}

/// One capability's provider and model selection.
#[component]
fn ChatEndpointGroup(
    title: String,
    #[props(default = true)] show_fields: bool,
    #[props(default = rsx! {})] control: Element,
    providers: Vec<SelectOption>,
    provider_id: String,
    model: String,
    status: HealthStatus,
    detail: String,
    models: Signal<ModelListState>,
    on_select_provider: EventHandler<String>,
    on_select_model: EventHandler<String>,
    on_fetch: EventHandler<()>,
) -> Element {
    let provider_label = format!("{title} provider");
    rsx! {
        div { class: "flex flex-wrap items-center justify-between gap-3",
            div { class: "flex min-w-0 items-center gap-3",
                span { class: "grid size-8 shrink-0 place-items-center rounded-xl bg-white/[0.035]", title: detail.clone(), role: "img", aria_label: detail, StatusDot { status } }
                h3 { class: "text-[15px] font-semibold text-zinc-100", "{title}" }
            }
            {control}
        }
        if show_fields { div { class: "mt-4 border-y border-white/[0.06]",
            if !providers.is_empty() {
                SettingRow { title: "Provider".to_owned(), description: String::new(),
                    SearchSelect { label: provider_label, options: providers, value: provider_id.clone(), placeholder: "Select a provider".to_owned(), on_select: on_select_provider }
                }
            }
            if provider_id.is_empty() {
                p { class: "py-4 text-xs text-zinc-400", "Set up a provider in Clipforge, then refresh." }
            } else {
                SettingRow { title: "Model".to_owned(), description: String::new(), last: true,
                    ModelSelect { models, value: model, on_select: on_select_model, on_fetch }
                }
            }
        } }
        div { class: "h-8" }
    }
}

/// The model dropdown: the ids the endpoint serves, with a refetch action.
#[component]
fn ModelSelect(
    models: Signal<ModelListState>,
    value: String,
    on_select: EventHandler<String>,
    on_fetch: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "flex min-w-0 items-center justify-end gap-2",
            match &*models.read() {
                ModelListState::Loading(_) => rsx! {
                    span { class: "text-[10px] text-zinc-600", "Checking available models…" }
                },
                ModelListState::Failed(error) => rsx! {
                    ModelErrorCode { error: error.clone() }
                },
                // Only the ids the endpoint actually serves are ever offered:
                // the saved model shows as selected only when it is one of
                // them, and reads as unselected otherwise.
                ModelListState::Ready(ids) if ids.is_empty() => rsx! {
                    span { class: "text-[10px] text-zinc-600", "Provider lists no models" }
                },
                ModelListState::Ready(ids) => {
                    let options: Vec<SelectOption> = ids
                        .iter()
                        .map(|id| SelectOption { value: id.clone(), label: id.clone() })
                        .collect();
                    let served = ids.iter().any(|id| id == &value);
                    rsx! {
                        if !served {
                            span { class: "max-w-40 truncate text-[9px] text-amber-300/80", title: "The saved model is not in the endpoint's list", "saved model not served" }
                        }
                        SearchSelect {
                            label: "Model",
                            options,
                            value: if served { value.clone() } else { String::new() },
                            placeholder: "Pick a served model".to_owned(),
                            on_select,
                        }
                    }
                }
                ModelListState::OnDemand => rsx! {
                    span { class: "text-[10px] text-zinc-600", "Managed server starts on demand" }
                },
                ModelListState::Empty => rsx! {
                    span { class: "text-[10px] text-zinc-600", "No provider serves this capability." }
                },
            }
            button {
                class: "grid size-8 shrink-0 place-items-center rounded-lg bg-white/[0.05] text-zinc-500 transition hover:text-white",
                aria_label: "Refetch models",
                disabled: matches!(*models.read(), ModelListState::Loading(_)),
                title: "Refetch models",
                onclick: move |_| on_fetch.call(()),
                Icon { name: IconName::Refresh, class: "size-3" }
            }
        }
    }
}

#[component]
fn ModelErrorCode(error: ReportedError) -> Element {
    let mut details = use_context::<ErrorDetails>().0;
    rsx! {
        button {
            class: "font-mono text-[10px] text-rose-300 underline decoration-rose-300/50 underline-offset-4 hover:text-rose-200",
            title: "Show error details",
            aria_label: "Show model error details",
            onclick: move |_| details.set(Some(error.clone())),
            "{error.code.code()}"
        }
    }
}

fn provider_options(providers: &[ChatProvider]) -> Vec<SelectOption> {
    providers
        .iter()
        .map(|provider| SelectOption {
            value: provider.id.clone(),
            label: provider.label(),
        })
        .collect()
}

/// The health of one service, or its not-yet-checked state.
fn service_of(health: &[ServiceHealth], id: ServiceId) -> (HealthStatus, String) {
    health
        .iter()
        .find(|service| service.id == id)
        .map(|service| (service.status, service.detail.clone()))
        .unwrap_or((HealthStatus::Idle, "Not checked yet".to_owned()))
}

#[component]
fn StorageSettings(config: AppConfig) -> Element {
    let application = config
        .app_config_path
        .clone()
        .or_else(storyfella_studio_core::paths::config_path)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "not configured".to_owned());
    let inference = config
        .ai_config_path
        .clone()
        .or_else(storyfella_studio_core::paths::clipforge_config_path)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "not configured".to_owned());
    rsx! {
        SectionHeading {
            eyebrow: "Files",
            title: "Storage",
            description: "Platform-appropriate directories keep generated assets, model weights, and disposable cache data separate."
        }
        div { class: "border-y border-white/[0.06]",
            if let Some(image) = &config.image {
                PathRow { title: "Models", description: "The image engine's weight tree.", path: image.model_dir.display().to_string() }
            }
            PathRow { title: "Generated assets", description: "Imported generations.", path: config.asset_dir.display().to_string() }
            PathRow { title: "Projects", description: "Where new project databases are created.", path: config.project_dir.display().to_string() }
            PathRow { title: "Cache", description: "Disposable data; safe to clear.", path: config.cache_dir.display().to_string() }
            PathRow { title: "Inference state", description: "Server markers and logs, shared by Studio, Atelier and Clipforge.", path: config.state_dir.display().to_string() }
            PathRow { title: "Application configuration", description: "This app's settings.", path: application }
            PathRow { title: "Inference configuration", description: "Providers shared by Studio, Atelier and Clipforge.", path: inference, last: true }
        }
    }
}

#[component]
fn PathRow(
    title: String,
    description: String,
    path: String,
    #[props(default)] last: bool,
) -> Element {
    let mut error = use_signal(|| None::<String>);
    let reveal_path = path.clone();
    rsx! {
        SettingRow { title, description, last,
            div { class: "flex min-w-0 items-center gap-3",
                code { class: "min-w-0 flex-1 truncate text-[10px] text-zinc-500", title: "{path}", "{path}" }
                button { class: "shrink-0 rounded-lg bg-white/[0.05] px-2.5 py-1.5 text-[9px] text-zinc-500 hover:text-white", onclick: move |_| error.set(super::components::open_path(std::path::Path::new(&reveal_path)).err()), "Reveal" }
                if let Some(message) = error() { p { role: "alert", class: "text-xs text-rose-300", "{message}" } }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyfella_studio_core::errors::ErrorCode;

    #[component]
    fn SettingsProbe(config: AppConfig) -> Element {
        use_context_provider(|| Signal::new(None::<ReportedError>));
        use_context_provider(|| Signal::new(AutosaveMode::OnChange));
        use_context_provider(|| Signal::new(ThemePreference::System));
        let backend = use_signal(|| CreativeBackend::new(&config));
        rsx! { Settings { config, backend } }
    }

    #[component]
    fn OfflineModelProbe(value: String) -> Element {
        use_context_provider(|| ErrorDetails(Signal::new(None)));
        let models = use_signal(|| {
            ModelListState::Failed(ReportedError::plain(ErrorCode::LlmRequest, "offline"))
        });
        rsx! { ModelSelect { models, value, on_select: |_| {}, on_fetch: |_| {} } }
    }

    #[test]
    fn unavailable_provider_shows_an_actionable_error_instead_of_a_model() {
        let mut dom = VirtualDom::new_with_props(
            OfflineModelProbe,
            OfflineModelProbeProps {
                value: "saved-model".to_owned(),
            },
        );
        let rendered = format!("{:?}", dom.rebuild_to_vec());
        assert!(rendered.contains("SVS-LLM-102"));
        assert!(!rendered.contains("saved-model"));
    }

    #[tokio::test]
    async fn opening_settings_runs_initial_refresh_without_borrowing_config_twice() {
        let config = AppConfig::load_from(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/studio-startup-test-missing.toml"),
        )
        .unwrap();
        let mut dom = VirtualDom::new_with_props(SettingsProbe, SettingsProbeProps { config });
        dom.rebuild_in_place();
        tokio::time::timeout(std::time::Duration::from_secs(1), dom.wait_for_work())
            .await
            .expect("initial Settings effect should schedule work");
        dom.render_immediate(&mut dioxus::core::NoOpMutations);
    }
}
