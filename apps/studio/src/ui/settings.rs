use std::time::Duration;

use dioxus::prelude::*;
use uuid::Uuid;

use storyfella_studio_core::config::{AppConfig, ChatCapability, ChatProvider, ThemePreference};
use storyfella_studio_core::errors::{CodedError, ReportedError};
use storyfella_studio_core::planner::PlannerBackend;
use storyfella_studio_core::runtime::{
    check_all as check_services, HealthStatus, ServiceHealth, ServiceId,
};

use super::{
    components::{SearchSelect, SectionHeading, SelectOption, SettingRow, StatusDot, Toggle},
    icons::{Icon, IconName},
    AutosaveMode,
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum SettingsSection {
    General,
    Models,
    Storage,
}

#[component]
pub fn Settings(config: Signal<AppConfig>, backend: Signal<PlannerBackend>) -> Element {
    let mut section = use_signal(|| SettingsSection::Models);
    let health = use_signal(Vec::<ServiceHealth>::new);
    let checking = use_signal(|| None::<Uuid>);
    let checked_at = use_signal(|| None::<String>);
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let targets = ConfigTargets {
        config,
        backend,
        health,
        checking,
        checked_at,
    };
    let mut initialized = use_signal(|| false);
    use_effect(move || {
        if !initialized() {
            initialized.set(true);
            let current = config.read().clone();
            apply_config(current, targets);
        }
    });
    // Clipforge replaces the shared configuration atomically. Detect content
    // changes even when the Models page is hidden, and update the live backend.
    use_future(move || async move {
        let path = config.peek().ai_config_path.clone();
        let mut last = shared_config_stamp(path.as_deref());
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        interval.tick().await;
        loop {
            interval.tick().await;
            let next = shared_config_stamp(path.as_deref());
            if next != last {
                match AppConfig::load() {
                    Ok(next_config) => {
                        apply_config(next_config, targets);
                        last = next;
                    }
                    Err(error) => {
                        error_report.set(Some(error.report().log()));
                        last = next;
                    }
                }
            }
        }
    });
    let on_refresh = move |_| match AppConfig::load() {
        Ok(next) => apply_config(next, targets),
        Err(error) => error_report.set(Some(error.report().log())),
    };
    let on_select = move |(capability, provider): (ChatCapability, String)| {
        let result = config
            .read()
            .persist_provider_selection(capability, &provider)
            .and_then(|_| AppConfig::load());
        match result {
            Ok(next) => apply_config(next, targets),
            Err(error) => error_report.set(Some(error.report().log())),
        }
    };
    let on_separate_vision = move |separate: bool| {
        let result = config
            .read()
            .persist_separate_vision(separate)
            .and_then(|_| AppConfig::load());
        match result {
            Ok(next) => apply_config(next, targets),
            Err(error) => error_report.set(Some(error.report().log())),
        }
    };
    let on_select_model = move |(capability, model): (ChatCapability, String)| {
        let result = config
            .read()
            .persist_model_selection(capability, &model)
            .and_then(|_| AppConfig::load());
        match result {
            Ok(next) => apply_config(next, targets),
            Err(error) => error_report.set(Some(error.report().log())),
        }
    };
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
                    div { hidden: section() != SettingsSection::Models, ModelsSettings { config: config(), health, checking, checked_at, on_refresh, on_select, on_select_model, on_separate_vision } }
                    div { hidden: section() != SettingsSection::Storage, StorageSettings { config: config() } }
                }
            }
        }
    }
}

fn shared_config_stamp(path: Option<&std::path::Path>) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let bytes = std::fs::read(path?).ok()?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    Some(hash.finish())
}

fn run_checks(
    config: AppConfig,
    mut health: Signal<Vec<ServiceHealth>>,
    mut checking: Signal<Option<Uuid>>,
    mut checked_at: Signal<Option<String>>,
) {
    let request = Uuid::new_v4();
    checking.set(Some(request));
    spawn(async move {
        let services = check_services(&config).await;
        if *checking.peek() != Some(request) {
            return;
        }
        health.set(services);
        checked_at.set(Some(current_time_label()));
        checking.set(None);
    });
}

/// The UI state a configuration reload refreshes: the effective config, the
/// planner client, the health check and each capability's model list.
#[derive(Clone, Copy)]
struct ConfigTargets {
    config: Signal<AppConfig>,
    backend: Signal<PlannerBackend>,
    health: Signal<Vec<ServiceHealth>>,
    checking: Signal<Option<Uuid>>,
    checked_at: Signal<Option<String>>,
}

/// Applies a configuration reloaded from disk: replaces the effective config,
/// rebuilds the planner client, and re-probes the services and their models.
fn apply_config(next: AppConfig, mut targets: ConfigTargets) {
    *targets.config.write() = next.clone();
    targets.backend.write().set_config(&next);
    run_checks(
        next.clone(),
        targets.health,
        targets.checking,
        targets.checked_at,
    );
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
    rsx! { button { class, title: "{label}", onclick, "{label}" } }
}

#[component]
fn GeneralSettings(mut config: Signal<AppConfig>) -> Element {
    let autosave = use_context::<Signal<AutosaveMode>>();
    let mut autosave = autosave;
    let mut theme = use_context::<Signal<ThemePreference>>();
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let preview = use_context::<Signal<storyfella_studio_core::config::PreviewSettings>>();
    let theme_id = use_hook(|| format!("theme-{}", Uuid::new_v4()));
    let restore_theme_id = theme_id.clone();
    let preview_id = use_hook(|| format!("preview-{}", Uuid::new_v4()));
    let seconds_id = format!("{preview_id}-seconds");
    let text_id = format!("{preview_id}-text");
    let fit_id = format!("{preview_id}-fit");
    let diagnostics_id = format!("{preview_id}-diagnostics");
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
            SettingRow { title: "Autosave", description: "Save project changes locally as you work.",
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
            SettingRow { title: "Autoplay interval", description: "Seconds between passages during automatic playback.",
                select { id: seconds_id.clone(), class: "setting-control", aria_label: "Autoplay interval", value: "{preview().autoplay_seconds}",
                    oninput: move |event| { if let Ok(value) = event.value().parse::<u32>() { let old = preview(); let mut next = old; next.autoplay_seconds = value; save_preview(config, preview, error_report, next, &seconds_id, old.autoplay_seconds.to_string()); } },
                    for seconds in 1..=30 { option { value: "{seconds}", selected: preview().autoplay_seconds == seconds, "{seconds} seconds" } }
                }
            }
            SettingRow { title: "Preview text size", description: "Scale the story text in Preview.",
                select { id: text_id.clone(), class: "setting-control", aria_label: "Preview text size", value: "{preview().text_percent}",
                    oninput: move |event| { if let Ok(value) = event.value().parse::<u32>() { let old = preview(); let mut next = old; next.text_percent = value; save_preview(config, preview, error_report, next, &text_id, old.text_percent.to_string()); } },
                    for percent in 75..=150 { option { value: "{percent}", selected: preview().text_percent == percent, "{percent}%" } }
                }
            }
            SettingRow { title: "Preview artwork fit", description: "Choose how location artwork fits the stage.",
                select { id: fit_id.clone(), class: "setting-control", aria_label: "Preview artwork fit", value: "{preview().contain_artwork}",
                    oninput: move |event| { let old = preview(); let mut next = old; next.contain_artwork = event.value() == "true"; save_preview(config, preview, error_report, next, &fit_id, old.contain_artwork.to_string()); },
                    option { value: "false", selected: !preview().contain_artwork, "Cover stage" } option { value: "true", selected: preview().contain_artwork, "Show entire image" }
                }
            }
            SettingRow { title: "Preview diagnostics", description: "Show missing-artwork placeholders and unsupported-direction notices.", last: true,
                select { id: diagnostics_id.clone(), class: "setting-control", aria_label: "Preview diagnostics", value: "{preview().show_diagnostics}",
                    oninput: move |event| { let old = preview(); let mut next = old; next.show_diagnostics = event.value() == "true"; save_preview(config, preview, error_report, next, &diagnostics_id, old.show_diagnostics.to_string()); },
                    option { value: "true", selected: preview().show_diagnostics, "Show" } option { value: "false", selected: !preview().show_diagnostics, "Hide" }
                }
            }
        }
    }
}

fn save_preview(
    config: Signal<AppConfig>,
    mut preview: Signal<storyfella_studio_core::config::PreviewSettings>,
    mut error_report: Signal<Option<ReportedError>>,
    next: storyfella_studio_core::config::PreviewSettings,
    select_id: &str,
    previous: String,
) {
    match config.read().persist_preview_settings(next) {
        Ok(()) => preview.set(next),
        Err(error) => {
            let id = serde_json::to_string(select_id).expect("serialize select id");
            let value = serde_json::to_string(&previous).expect("serialize select value");
            let _ = document::eval(&format!("document.getElementById({id}).value = {value};"));
            error_report.set(Some(error.report().log()));
        }
    }
}

#[component]
fn ModelsSettings(
    config: AppConfig,
    health: Signal<Vec<ServiceHealth>>,
    checking: Signal<Option<Uuid>>,
    checked_at: Signal<Option<String>>,
    on_refresh: EventHandler<MouseEvent>,
    on_select: EventHandler<(ChatCapability, String)>,
    on_select_model: EventHandler<(ChatCapability, String)>,
    on_separate_vision: EventHandler<bool>,
) -> Element {
    let services = health.read().clone();
    let (text_status, text_detail) = service_of(&services, ServiceId::Llm);
    let (vision_status, vision_detail) = service_of(&services, ServiceId::Vision);
    let text_providers = provider_options(&config.chat_providers, ChatCapability::Text);
    let vision_providers = provider_options(&config.chat_providers, ChatCapability::Vision);
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
    let text_known = config.chat_providers.iter().any(|p| p.id == text_provider);
    let vision_known = config
        .chat_providers
        .iter()
        .any(|p| p.id == vision_provider);
    let models_for = |id| {
        services
            .iter()
            .find(|service| service.id == id)
            .and_then(|service| service.models.clone())
    };
    let text_models = models_for(ServiceId::Llm);
    let vision_models = models_for(ServiceId::Vision);
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let vision_warning = "The selected Text provider does not support Vision. Choose a Vision-capable Text provider or use a separate Vision provider.";
    rsx! {
        div { class: "flex flex-wrap items-end justify-between gap-x-4",
            SectionHeading { eyebrow: "Capabilities", title: "Models", description: "" }
            CheckBar { checking, checked_at, on_check: on_refresh }
        }
        ChatEndpointGroup {
            title: "Text (LLM)".to_owned(),
            providers: text_providers, provider_id: text_provider, model: text_model,
            provider_known: text_known, models: text_models, checking_models: checking().is_some(),
            status: text_status, detail: text_detail,
            on_select_provider: move |id| on_select.call((ChatCapability::Text, id)),
            on_select_model: move |id| on_select_model.call((ChatCapability::Text, id)),
        }
        ChatEndpointGroup {
            title: "Vision (LLM)".to_owned(), show_fields: config.separate_vision,
            providers: vision_providers, provider_id: vision_provider, model: vision_model,
            provider_known: vision_known, models: vision_models, checking_models: checking().is_some(),
            status: vision_status, detail: vision_detail,
            on_select_provider: move |id| on_select.call((ChatCapability::Vision, id)),
            on_select_model: move |id| on_select_model.call((ChatCapability::Vision, id)),
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
        if !config.separate_vision && config.text.is_some() && config.vision.is_none() {
            p { class: "mb-6 text-xs text-amber-300", role: "status", "{vision_warning}" }
        }
        button {
            class: "mt-3 rounded bg-white/10 px-3 py-2 text-xs",
            title: "Manage providers, credentials, models and services in Clipforge",
            onclick: move |_| if let Err(error) = storyfella_studio_core::ai_config::open_provider_manager() { error_report.set(Some(ReportedError::plain(storyfella_studio_core::errors::ErrorCode::AppCommandInvalid, error.to_string()).log())); },
            "Manage providers in Clipforge"
        }
    }
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
                title: if let Some(at) = checked_at() { format!("Refresh providers · Last checked {at}") } else { "Refresh providers".to_owned() },
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
    provider_known: bool,
    models: Option<Vec<String>>,
    checking_models: bool,
    status: HealthStatus,
    detail: String,
    on_select_provider: EventHandler<String>,
    on_select_model: EventHandler<String>,
) -> Element {
    let provider_label = format!("{title} provider");
    let provider_available = providers.iter().any(|option| option.value == provider_id);
    let has_providers = !providers.is_empty();
    let model_label = format!("{title} model");
    let model_available = models
        .as_ref()
        .is_some_and(|models| models.contains(&model));
    let model_options = models
        .as_ref()
        .map(|models| {
            models
                .iter()
                .map(|model| SelectOption {
                    value: model.clone(),
                    label: model.clone(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
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
            } else if !provider_available {
                p { class: "py-4 text-xs text-amber-300", role: "status",
                    if provider_known { "The saved {title} provider \"{provider_id}\" does not declare this capability in Clipforge." }
                    else { "The saved {title} provider \"{provider_id}\" is missing from Clipforge." }
                    if has_providers { " Choose an available provider above." } else { " Configure a provider for this capability in Clipforge, then refresh." }
                }
            } else {
                SettingRow { title: "Model".to_owned(), description: "Models reported by this provider; check the model supports this capability.".to_owned(), last: true,
                    if checking_models { span { class: "text-xs text-zinc-400", "Checking models…" } }
                    else if models.is_none() { span { class: "text-xs text-zinc-400", "Model list unavailable. Refresh providers." } }
                    else if model_options.is_empty() { span { class: "text-xs text-zinc-400", "No models reported by this provider." } }
                    else { SearchSelect { label: model_label, options: model_options, value: model.clone(), placeholder: "Select an available model".to_owned(), on_select: on_select_model } }
                }
                if !checking_models && models.is_some() && !model.is_empty() && !model_available { p { class: "pb-3 text-xs text-amber-300", role: "status", "Saved {title} model \"{model}\" was not reported by this provider." } }
            }
        } }
        div { class: "h-8" }
    }
}

fn provider_options(providers: &[ChatProvider], capability: ChatCapability) -> Vec<SelectOption> {
    providers
        .iter()
        .filter(|provider| {
            provider
                .capabilities
                .iter()
                .any(|item| item == capability.as_str())
        })
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
            description: "Application data and provider settings."
        }
        div { class: "border-y border-white/[0.06]",
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
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let reveal_path = path.clone();
    rsx! {
        SettingRow { title, description, last,
            div { class: "flex min-w-0 items-center gap-3",
                code { class: "min-w-0 flex-1 truncate text-[10px] text-zinc-500", title: "{path}", "{path}" }
                button { class: "shrink-0 rounded-lg bg-white/[0.05] px-2.5 py-1.5 text-[9px] text-zinc-500 hover:text-white", title: "Reveal location", onclick: move |_| if let Err(error) = super::components::open_path(std::path::Path::new(&reveal_path)) { error_report.set(Some(ReportedError::plain(storyfella_studio_core::errors::ErrorCode::AppCommandInvalid, error).log())); }, "Reveal" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_config_stamp_detects_changes_and_removal() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let path = directory.path().join("clipforge.toml");
        assert_eq!(shared_config_stamp(Some(&path)), None);
        std::fs::write(&path, "first").unwrap();
        let first = shared_config_stamp(Some(&path));
        std::fs::write(&path, "other").unwrap();
        assert_ne!(first, shared_config_stamp(Some(&path)));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(shared_config_stamp(Some(&path)), None);
    }

    #[test]
    fn provider_choices_only_include_the_requested_capability() {
        use agentkit_ai::deployments::Providers;
        let registry = Providers::from_toml(
            "[providers.text]\nkind='openai'\ncapabilities=['text']\nbase_url='http://localhost:8080/v1'\nmodel='text'\n\
             [providers.vision]\nkind='openai'\ncapabilities=['vision']\nbase_url='http://localhost:8081/v1'\nmodel='vision'\n\
             [providers.both]\nkind='openai'\ncapabilities=['text','vision']\nbase_url='http://localhost:8082/v1'\nmodel='both'\n",
            std::path::Path::new("."),
        ).unwrap();
        let providers: Vec<ChatProvider> = registry
            .list()
            .map(|(id, provider)| ChatProvider {
                id: id.into(),
                kind: provider.kind,
                capabilities: provider
                    .capabilities
                    .iter()
                    .map(|c| c.as_str().into())
                    .collect(),
                deployment: registry
                    .chat_deployment_for(id, provider.capabilities[0])
                    .unwrap(),
            })
            .collect();
        let ids = |capability| {
            provider_options(&providers, capability)
                .into_iter()
                .map(|option| option.value)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(ChatCapability::Text), ["both", "text"]);
        assert_eq!(ids(ChatCapability::Vision), ["both", "vision"]);
    }

    #[component]
    fn ModelPickerProbe() -> Element {
        rsx! { ChatEndpointGroup {
            title: "Text (LLM)".to_owned(),
            providers: vec![SelectOption { value: "local".into(), label: "Local".into() }],
            provider_id: "local".to_owned(), provider_known: true,
            model: "model-b".to_owned(), models: Some(vec!["model-a".into(), "model-b".into()]),
            checking_models: false, status: HealthStatus::Online, detail: "online".to_owned(),
            on_select_provider: |_| {}, on_select_model: |_| {},
        } }
    }

    #[component]
    fn VisionCapabilityWarningProbe() -> Element {
        rsx! { ChatEndpointGroup {
            title: "Vision (LLM)".to_owned(), providers: Vec::new(),
            provider_id: "lmstudio".to_owned(), provider_known: true,
            model: String::new(), models: None, checking_models: false,
            status: HealthStatus::Idle, detail: "not configured".to_owned(),
            on_select_provider: |_| {}, on_select_model: |_| {},
        } }
    }

    #[test]
    fn model_picker_shows_selected_model_and_vision_warning_names_capability() {
        let mut models = VirtualDom::new(ModelPickerProbe);
        let rendered = format!("{:?}", models.rebuild_to_vec());
        assert!(rendered.contains("model-b"));
        assert!(rendered.contains("Text (LLM) model"));
        let mut vision = VirtualDom::new(VisionCapabilityWarningProbe);
        let rendered = format!("{:?}", vision.rebuild_to_vec());
        assert!(rendered.contains("Vision (LLM)"));
        assert!(rendered.contains("does not declare this capability"));
        assert!(rendered.contains("lmstudio"));
    }

    #[component]
    fn SettingsProbe(config: AppConfig) -> Element {
        use_context_provider(|| Signal::new(config.preview));
        use_context_provider(|| Signal::new(None::<ReportedError>));
        use_context_provider(|| Signal::new(AutosaveMode::OnChange));
        use_context_provider(|| Signal::new(ThemePreference::System));
        let backend = use_signal(|| PlannerBackend::new(&config));
        let config = use_signal(|| config);
        rsx! { Settings { config, backend } }
    }

    #[test]
    fn removed_provider_shows_its_saved_id_and_recovery() {
        let mut config = AppConfig::load_from(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/studio-startup-test-missing.toml"),
        )
        .unwrap();
        config.text_provider_id = Some("removed-provider".into());
        config.text = None;
        let mut dom = VirtualDom::new_with_props(SettingsProbe, SettingsProbeProps { config });
        let rendered = format!("{:?}", dom.rebuild_to_vec());
        assert!(rendered.contains("removed-provider"));
        assert!(rendered.contains("is missing from Clipforge"));
    }

    #[test]
    fn settings_only_offer_studio_model_capabilities() {
        let config = AppConfig::load_from(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/studio-startup-test-missing.toml"),
        )
        .unwrap();
        let mut dom = VirtualDom::new_with_props(SettingsProbe, SettingsProbeProps { config });
        let rendered = format!("{:?}", dom.rebuild_to_vec());
        assert!(rendered.contains("Text (LLM)"));
        assert!(rendered.contains("Vision (LLM)"));
        assert!(!rendered.contains("Image generation"));
        assert!(!rendered.contains("Image editing"));
    }

    #[test]
    fn linked_text_without_vision_shows_recovery() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(root).unwrap();
        let app = dir.path().join("studio.toml");
        std::fs::write(&app, "[models]\nseparate_vision = false\n[defaults]\ntext='writer'\n[providers.writer]\nkind='openai'\ncapabilities=['text']\nbase_url='http://localhost:8080/v1'\nmodel='writer'\n").unwrap();
        let config = AppConfig::load_from(app).unwrap();
        assert!(config.text.is_some());
        assert!(config.vision.is_none());
        assert!(!config.separate_vision);
        let mut dom = VirtualDom::new_with_props(SettingsProbe, SettingsProbeProps { config });
        let rendered = format!("{:?}", dom.rebuild_to_vec());
        assert!(rendered.contains("does not support Vision"));
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
