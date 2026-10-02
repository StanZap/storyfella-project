use std::time::Duration;

use dioxus::prelude::*;

use svs_core::config::AppConfig;
use svs_core::errors::{CodedError, ReportedError};
use svs_core::llm::LlmClient;
use svs_core::registry::{
    backend::CreativeBackend, discover_loras, GenerationDefaults, LoraBinding,
};
use svs_core::runtime::{
    check_all as check_services, CreativeRuntime, HealthStatus, ServiceHealth, ServiceId,
};

use super::{
    components::{ErrorDetailsButton, SearchSelect, SectionHeading, SettingRow, StatusDot},
    icons::{Icon, IconName},
    AutosaveMode,
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum SettingsSection {
    Status,
    General,
    Intelligence,
    Generation,
    Storage,
}

#[derive(Clone, Debug, PartialEq)]
enum ModelListState {
    Empty,
    Loading,
    Ready(Vec<String>),
    Failed(ReportedError),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ServiceAction {
    Start,
    Stop,
    Restart,
}

impl ServiceAction {
    const fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
        }
    }
}

/// The controls a service offers at a given status. The resident generation
/// server can be started when it is down, and restarted or stopped while it is
/// up. External (text/vision) endpoints are never started or stopped here.
fn actions_for(service: ServiceId, status: HealthStatus) -> Vec<ServiceAction> {
    match (service, status) {
        (ServiceId::Llm | ServiceId::Vision, _) => Vec::new(),
        (ServiceId::ImageGeneration, HealthStatus::Online) => {
            vec![ServiceAction::Restart, ServiceAction::Stop]
        }
        (ServiceId::ImageGeneration, _) => vec![ServiceAction::Start],
    }
}

#[cfg(test)]
mod tests {
    use super::{actions_for, ServiceAction};
    use svs_core::runtime::{HealthStatus, ServiceId};

    #[test]
    fn external_services_have_no_controls() {
        assert!(actions_for(ServiceId::Llm, HealthStatus::Online).is_empty());
        assert!(actions_for(ServiceId::Vision, HealthStatus::Online).is_empty());
    }

    #[test]
    fn the_resident_runtime_offers_start_stop_and_restart() {
        for status in [
            HealthStatus::Idle,
            HealthStatus::Degraded,
            HealthStatus::Offline,
        ] {
            assert_eq!(
                actions_for(ServiceId::ImageGeneration, status),
                vec![ServiceAction::Start]
            );
        }
        assert_eq!(
            actions_for(ServiceId::ImageGeneration, HealthStatus::Online),
            vec![ServiceAction::Restart, ServiceAction::Stop]
        );
    }
}

#[component]
pub fn Settings(
    config: AppConfig,
    backend: Signal<CreativeBackend>,
    defaults: Signal<svs_core::registry::GenerationDefaults>,
) -> Element {
    let runtime = backend.read().runtime();
    let mut section = use_signal(|| SettingsSection::Status);
    let health = use_signal(Vec::<ServiceHealth>::new);
    let checking = use_signal(|| false);
    let checked_at = use_signal(|| None::<String>);
    let lm_model = use_signal(|| {
        backend.read().llm_model().unwrap_or_else(|| {
            config
                .text
                .as_ref()
                .map(|deployment| deployment.model.clone())
                .unwrap_or_default()
        })
    });
    let models_state = use_signal(|| ModelListState::Empty);
    let busy = use_signal(|| None::<ServiceId>);
    let operation_error = use_signal(|| None::<ReportedError>);

    let effect_runtime = runtime.clone();
    let effect_config = config.clone();
    let mut initialized = use_signal(|| false);
    use_effect(move || {
        if !initialized() {
            initialized.set(true);
            run_checks(
                effect_runtime.clone(),
                effect_config.clone(),
                lm_model,
                health,
                checking,
                checked_at,
            );
            fetch_models(effect_config.clone(), models_state);
        }
    });

    let check_runtime = runtime.clone();
    let check_config = config.clone();
    let on_check = move |_event: MouseEvent| {
        run_checks(
            check_runtime.clone(),
            check_config.clone(),
            lm_model,
            health,
            checking,
            checked_at,
        );
    };

    let operate_runtime = runtime.clone();
    let operate_config = config.clone();
    let on_operate = move |(service, action): (ServiceId, ServiceAction)| {
        operate_service(
            operate_runtime.clone(),
            operate_config.clone(),
            service,
            action,
            lm_model,
            busy,
            operation_error,
            health,
            checking,
            checked_at,
        );
    };

    rsx! {
        section { class: "grid h-full min-h-0 grid-cols-[210px_minmax(0,1fr)]",
            aside { class: "border-r border-white/[0.055] px-5 py-8",
                p { class: "mb-5 px-3 text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-600", "Settings" }
                nav { class: "space-y-1",
                    SettingsNav { label: "Status", active: section() == SettingsSection::Status, onclick: move |_| section.set(SettingsSection::Status) }
                    SettingsNav { label: "General", active: section() == SettingsSection::General, onclick: move |_| section.set(SettingsSection::General) }
                    SettingsNav { label: "Intelligence", active: section() == SettingsSection::Intelligence, onclick: move |_| section.set(SettingsSection::Intelligence) }
                    SettingsNav { label: "Image generation", active: section() == SettingsSection::Generation, onclick: move |_| section.set(SettingsSection::Generation) }
                    SettingsNav { label: "Storage", active: section() == SettingsSection::Storage, onclick: move |_| section.set(SettingsSection::Storage) }
                }
                div { class: "absolute bottom-7 px-3 text-[9px] leading-4 text-zinc-700", "Smart Visual Sequencer" br {} "Prototype 0.1.0" }
            }
            div { class: "min-h-0 overflow-y-auto px-10 py-9",
                div { class: "mx-auto max-w-3xl",
                    match section() {
                        SettingsSection::Status => rsx! { StatusSettings { health, checking, checked_at, busy, operation_error, on_check: on_check.clone(), on_operate: on_operate.clone() } },
                        SettingsSection::General => rsx! { GeneralSettings {} },
                        SettingsSection::Intelligence => rsx! { IntelligenceSettings { config: config.clone(), backend, health, lm_model, models_state, on_check: on_check.clone() } },
                        SettingsSection::Generation => rsx! { GenerationSettings { config: config.clone(), defaults } },
                        SettingsSection::Storage => rsx! { StorageSettings { config: config.clone() } },
                    }
                }
            }
        }
    }
}

fn run_checks(
    runtime: CreativeRuntime,
    config: AppConfig,
    lm_model: Signal<String>,
    mut health: Signal<Vec<ServiceHealth>>,
    mut checking: Signal<bool>,
    mut checked_at: Signal<Option<String>>,
) {
    if checking() {
        return;
    }
    checking.set(true);
    let mut check_config = config;
    if let Some(text) = check_config.text.as_mut() {
        text.model = lm_model();
    }
    spawn(async move {
        let services = check_services(&check_config, &runtime).await;
        health.set(services);
        checked_at.set(Some(current_time_label()));
        checking.set(false);
    });
}

fn fetch_models(config: AppConfig, mut state: Signal<ModelListState>) {
    if *state.read() == ModelListState::Loading {
        return;
    }
    let Some(mut lm_config) = config.planner_config() else {
        // No planner endpoint: nothing to discover, and no failure to report.
        state.set(ModelListState::Empty);
        return;
    };
    lm_config.timeout = Duration::from_secs(5);
    state.set(ModelListState::Loading);
    spawn(async move {
        let result = async { LlmClient::new(lm_config)?.list_models().await }.await;
        match result {
            Ok(models) => {
                let mut ids: Vec<String> = models.data.into_iter().map(|model| model.id).collect();
                ids.sort();
                ids.dedup();
                state.set(ModelListState::Ready(ids));
            }
            Err(error) => state.set(ModelListState::Failed(error.report().log())),
        }
    });
}

fn operate_service(
    runtime: CreativeRuntime,
    config: AppConfig,
    service: ServiceId,
    action: ServiceAction,
    lm_model: Signal<String>,
    mut busy: Signal<Option<ServiceId>>,
    mut operation_error: Signal<Option<ReportedError>>,
    health: Signal<Vec<ServiceHealth>>,
    checking: Signal<bool>,
    checked_at: Signal<Option<String>>,
) {
    if busy().is_some() {
        return;
    }
    busy.set(Some(service));
    operation_error.set(None);
    spawn(async move {
        let result = match (service, action) {
            (ServiceId::ImageGeneration, ServiceAction::Start) => runtime.ensure_ready().await,
            (ServiceId::ImageGeneration, ServiceAction::Stop) => {
                runtime.stop_generation_runtime().await
            }
            (ServiceId::ImageGeneration, ServiceAction::Restart) => {
                runtime.restart_generation_runtime().await
            }
            (ServiceId::Llm | ServiceId::Vision, _) => Ok(()),
        };
        busy.set(None);
        match result {
            Ok(()) => {}
            Err(error) => operation_error.set(Some(error.report().log())),
        }
        run_checks(runtime, config, lm_model, health, checking, checked_at);
    });
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
fn StatusSettings(
    health: Signal<Vec<ServiceHealth>>,
    checking: Signal<bool>,
    checked_at: Signal<Option<String>>,
    busy: Signal<Option<ServiceId>>,
    operation_error: Signal<Option<ReportedError>>,
    on_check: EventHandler<MouseEvent>,
    on_operate: EventHandler<(ServiceId, ServiceAction)>,
) -> Element {
    let services = health.read().clone();
    let last_checked = checked_at();
    let is_checking = checking();
    let is_busy = busy().is_some();
    let busy_service = busy();
    rsx! {
        SectionHeading {
            eyebrow: "Dependencies",
            title: "Status",
            description: "Probe the services the app depends on. Local runtimes can be started and stopped here; checks themselves are read-only."
        }
        div { class: "mb-6 flex items-center justify-between",
            p { class: "text-[10px] text-zinc-600",
                if let Some(at) = last_checked { "Last checked {at}" } else { "Not checked yet" }
            }
            button {
                class: "flex h-8 items-center gap-2 rounded-lg bg-white/[0.055] px-3 text-[10px] font-medium text-zinc-400 transition hover:bg-white/[0.08] hover:text-white disabled:cursor-not-allowed disabled:opacity-40",
                disabled: is_checking,
                onclick: move |event| on_check.call(event),
                if is_checking {
                    span { class: "size-3 animate-spin rounded-full border border-zinc-600 border-t-white" }
                    "Checking…"
                } else {
                    Icon { name: IconName::Refresh, class: "size-3" }
                    "Check now"
                }
            }
        }
        if let Some(error) = operation_error() {
            div { class: "mb-4 flex items-start gap-3 rounded-lg bg-rose-400/[0.06] px-3 py-2 ring-1 ring-inset ring-rose-300/[0.12]",
                span { class: "mt-1 size-1.5 shrink-0 rounded-full bg-rose-400" }
                div { class: "min-w-0 flex-1",
                    p { class: "font-mono text-[9px] text-rose-300/70", "{error.code.code()}" }
                    p { class: "mt-0.5 text-[10px] leading-4 text-rose-200/80", "{error.code.message()}" }
                }
                ErrorDetailsButton { error }
            }
        }
        if services.is_empty() {
            div { class: "grid min-h-40 place-items-center rounded-xl border border-dashed border-white/[0.07]",
                p { class: "text-xs text-zinc-700", if is_checking { "Probing services…" } else { "Press Check now to probe the services." } }
            }
        } else {
            div { class: "border-y border-white/[0.06]",
                for (index, service) in services.iter().enumerate() {
                    SettingRow {
                        title: service.id.label().to_owned(),
                        description: service.detail.clone(),
                        last: index + 1 == services.len(),
                        div { class: "flex items-center justify-end gap-3",
                            if busy_service == Some(service.id) {
                                span { class: "size-3 animate-spin rounded-full border border-zinc-600 border-t-white" }
                            } else {
                                for action in actions_for(service.id, service.status) {
                                    button {
                                        class: "h-7 rounded-md bg-white/[0.06] px-2.5 text-[10px] font-medium text-zinc-400 transition hover:bg-white/[0.1] hover:text-white disabled:cursor-not-allowed disabled:opacity-40",
                                        disabled: is_busy,
                                        onclick: {
                                            let id = service.id;
                                            move |_| on_operate.call((id, action))
                                        },
                                        "{action.label()}"
                                    }
                                }
                            }
                            StatusDot { status: service.status }
                            span { class: "text-[10px] text-zinc-500", "{service.status.label()}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn GeneralSettings() -> Element {
    let autosave = use_context::<Signal<AutosaveMode>>();
    let mut autosave = autosave;
    rsx! {
        SectionHeading {
            eyebrow: "Application",
            title: "General",
            description: "Keep the workspace quiet and predictable while long-running creative work happens in the background."
        }
        div { class: "border-y border-white/[0.06]",
            SettingRow { title: "Appearance", description: "Interface theme used throughout the app.",
                select { class: "setting-control", option { "Dark" } }
            }
            SettingRow { title: "Background generation", description: "Continue queued jobs while you read or present other scenes.",
                Toggle { enabled: true }
            }
            SettingRow { title: "Autosave", description: "Save project changes locally as you work.", last: true,
                select {
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
fn IntelligenceSettings(
    config: AppConfig,
    backend: Signal<CreativeBackend>,
    health: Signal<Vec<ServiceHealth>>,
    lm_model: Signal<String>,
    models_state: Signal<ModelListState>,
    on_check: EventHandler<MouseEvent>,
) -> Element {
    let llm = health
        .read()
        .iter()
        .find(|service| service.id == ServiceId::Llm)
        .cloned();
    let (status, detail) = match llm {
        Some(service) => (service.status, service.detail),
        None => (HealthStatus::Idle, "Not checked yet".to_owned()),
    };
    // Applying a pick: swap the live client, persist to the user config,
    // and re-check so the Status screen reflects the new model.
    let persist_config = config.clone();
    let on_select = move |model: String| {
        if backend.read().llm_model().as_deref() == Some(model.as_str()) {
            return;
        }
        lm_model.set(model.clone());
        backend.write().set_llm_model(model.clone());
        if let Err(error) = persist_config.persist_text_model(&model) {
            tracing::error!("could not persist planner model to config: {error}");
        }
    };
    // Where a machine-local secret (an API key, a different endpoint) belongs:
    // the user config layered over the checked-in `config/app.toml`.
    let user_config = config
        .user_config_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "platform config directory unavailable".to_owned());
    let text_endpoint = config
        .text
        .as_ref()
        .map(|deployment| deployment.base_url.clone())
        .unwrap_or_else(|| "not configured".to_owned());
    let vision_endpoint = config
        .vision
        .as_ref()
        .map(|deployment| deployment.base_url.clone())
        .unwrap_or_else(|| "not configured".to_owned());
    let text_timeout = config
        .text
        .as_ref()
        .map(|deployment| deployment.timeout.as_secs().to_string())
        .unwrap_or_else(|| "—".to_owned());
    rsx! {
        SectionHeading {
            eyebrow: "Planner & vision",
            title: "Intelligence",
            description: "Planning (`text`) and visual understanding (`vision`) resolve from providers in providers.toml. An external endpoint is never bundled or started by the app; a managed native server is started on demand."
        }
        div { class: "mb-8 flex items-center justify-between border-y border-white/[0.06] py-4",
            div { class: "flex items-center gap-3",
                span { class: "grid size-9 place-items-center rounded-xl bg-white/[0.035]", StatusDot { status } }
                div { p { class: "text-[12px] font-medium text-zinc-200", "LLM endpoint" } p { class: "mt-0.5 text-[10px] text-zinc-600", "{detail}" } }
            }
            button { class: "rounded-lg bg-white/[0.05] px-3 py-2 text-[10px] font-medium text-zinc-400 transition hover:text-white", onclick: move |event| on_check.call(event), "Test connection" }
        }
        div { class: "border-y border-white/[0.06]",
            SettingRow { title: "Base URL", description: "OpenAI-compatible API endpoint serving the `text` provider.",
                input { class: "setting-control", value: "{text_endpoint}", readonly: true }
            }
            SettingRow { title: "Planner model", description: "Model identifier loaded in the endpoint. Picking one applies it immediately and saves it to the user providers file.",
                ModelPicker { config: config.clone(), lm_model, models_state, on_select }
            }
            SettingRow { title: "Vision endpoint", description: "OpenAI-compatible API endpoint serving the `vision` provider (`svs eval`).",
                input { class: "setting-control", value: "{vision_endpoint}", readonly: true }
            }
            SettingRow { title: "Request timeout", description: "Maximum wait for planning and visual analysis.",
                div { class: "flex items-center gap-3", input { class: "setting-control", value: "{text_timeout}", readonly: true } span { class: "text-[10px] text-zinc-700", "seconds" } }
            }
            SettingRow { title: "User config", description: "Layered over config/app.toml. Put an api_key or a per-machine override here — it is not checked into the repository.", last: true,
                input { class: "setting-control", value: "{user_config}", readonly: true }
            }
        }
        p { class: "mt-4 text-[10px] leading-5 text-zinc-700", "Model discovery connects to the configured Base URL. Model, generation, and secret choices are saved to the user config and apply immediately — no restart needed." }
    }
}

#[component]
fn ModelPicker(
    config: AppConfig,
    lm_model: Signal<String>,
    models_state: Signal<ModelListState>,
    on_select: EventHandler<String>,
) -> Element {
    let refresh_config = config.clone();
    rsx! {
        div { class: "flex min-w-0 items-center justify-end gap-2",
            match &*models_state.read() {
                ModelListState::Loading => rsx! {
                    span { class: "size-3 animate-spin rounded-full border border-zinc-600 border-t-white" }
                    span { class: "text-[10px] text-zinc-600", "Loading models…" }
                },
                ModelListState::Failed(error) => rsx! {
                    span { class: "max-w-52 truncate font-mono text-[9px] text-rose-300/70", title: "{error.details}", "{error.code.code()} · {error.code.message()}" }
                    ErrorDetailsButton { error: error.clone() }
                    button { class: "shrink-0 rounded-lg bg-white/[0.05] px-2.5 py-1.5 text-[9px] text-zinc-500 transition hover:text-white", onclick: move |_| fetch_models(refresh_config.clone(), models_state), "Retry" }
                },
                ModelListState::Ready(models) => {
                    let current = lm_model();
                    let has_current = models.iter().any(|model| model == &current);
                    // The loaded model may not be one the endpoint reports (a
                    // renamed or already-unloaded id), so it is offered as well
                    // — the control's value must never go blank just because
                    // discovery did not name it.
                    let mut options = models.clone();
                    if !has_current {
                        options.insert(0, current.clone());
                    }
                    rsx! {
                        if !has_current {
                            span { class: "shrink-0 text-[9px] text-amber-300/70", "not in the endpoint's list" }
                        }
                        SearchSelect {
                            options: options,
                            value: current.clone(),
                            on_select: on_select,
                        }
                        button {
                            class: "grid size-8 shrink-0 place-items-center rounded-lg bg-white/[0.05] text-zinc-500 transition hover:text-white",
                            aria_label: "Refresh model list",
                            title: "Refresh model list",
                            onclick: move |_| fetch_models(refresh_config.clone(), models_state),
                            Icon { name: IconName::Refresh, class: "size-3" }
                        }
                    }
                }
                ModelListState::Empty => rsx! {
                    SearchSelect {
                        options: Vec::new(),
                        value: lm_model(),
                        empty_message: "No models discovered yet — test the connection.".to_owned(),
                        on_select: on_select,
                    }
                    button {
                        class: "grid size-8 shrink-0 place-items-center rounded-lg bg-white/[0.05] text-zinc-500 transition hover:text-white",
                        aria_label: "Refresh model list",
                        title: "Refresh model list",
                        onclick: move |_| fetch_models(refresh_config.clone(), models_state),
                        Icon { name: IconName::Refresh, class: "size-3" }
                    }
                },
            }
        }
    }
}

#[component]
fn GenerationSettings(config: AppConfig, mut defaults: Signal<GenerationDefaults>) -> Element {
    let mut prewarm = use_context::<Signal<bool>>();
    let sizes = defaults.read().sizes;
    let steps = defaults.read().steps;
    let cfg_scale = defaults.read().cfg_scale;
    let size_error = use_signal(|| None::<String>);
    let mut steps_error = use_signal(|| None::<String>);
    // The lora dir shown here and the bindings share the same defaults.
    let lora_dir = config.image.lora_dir.clone();
    // Discovered adapters, reloadable: symlinked or newly dropped files
    // appear after pressing the refresh icon next to the directory.
    let mut discovered = use_signal(|| discover_loras(&lora_dir));
    let scan_dir = lora_dir.clone();
    let on_rescan = move |_event: MouseEvent| discovered.set(discover_loras(&scan_dir));
    rsx! {
        SectionHeading {
            eyebrow: "Local image engine",
            title: "Image generation",
            description: "Qwen-Image-2.1 runs locally through the native resident stable-diffusion.cpp server, with no ComfyUI dependency. The profile is designed for a 24 GB memory budget."
        }
        div { class: "border-y border-white/[0.06]",
            SettingRow { title: "Profile", description: "The resident generation model.",
                input { class: "setting-control", value: "{config.image.architecture}", readonly: true }
            }
            SettingRow { title: "Runtime endpoint", description: "Resident stable-diffusion.cpp service.",
                input { class: "setting-control", value: "http://{config.image.host}:{config.image.port}", readonly: true }
            }
            SettingRow { title: "Steps per image", description: "Quality vs. speed of every generation. The default is 25 at CFG {cfg_scale:.1} (Qwen-Image-2.1); fewer steps is faster and rougher.",
                div { class: "flex items-center gap-3",
                    if let Some(error) = steps_error() {
                        p { class: "truncate text-[9px] text-rose-300/80", "{error}" }
                    }
                    input {
                        class: "setting-control w-20",
                        r#type: "number",
                        min: 1,
                        max: 50,
                        value: "{steps}",
                        onchange: {
                            let mut defaults = defaults;
                            move |event| {
                                let parsed = event.value().parse::<u32>().ok().filter(|steps| (1..=50).contains(steps));
                                match parsed {
                                    Some(steps) => {
                                        steps_error.set(None);
                                        defaults.write().steps = steps;
                                        persist_defaults(defaults);
                                    }
                                    None => steps_error.set(Some("1–50".to_owned())),
                                }
                            }
                        },
                    }
                }
            }
            SettingRow { title: "Character size", description: "Default canvas for character images (WxH, multiples of 32).",
                SizeField { label: "512×768", value: sizes.character, error: size_error, defaults, kind: "character" }
            }
            SettingRow { title: "Environment size", description: "Default canvas for environment images.",
                SizeField { label: "1024×576", value: sizes.environment, error: size_error, defaults, kind: "environment" }
            }
            SettingRow { title: "Object size", description: "Default canvas for object images.",
                SizeField { label: "768×768", value: sizes.object, error: size_error, defaults, kind: "object" }
            }
            SettingRow { title: "Scene / beat size", description: "Default canvas for scene and beat images.",
                SizeField { label: "1024×576", value: sizes.scene, error: size_error, defaults, kind: "scene" }
            }
            SettingRow { title: "LoRA directory", description: "Adapters are discovered here (subfolders and symlinks included).",
                div { class: "flex items-center justify-between gap-3",
                    span { class: "truncate text-[11px] text-zinc-500", "{lora_dir.display()}" }
                    div { class: "flex shrink-0 items-center gap-2",
                        span { class: "rounded-md bg-violet-500/10 px-2 py-1 text-[9px] text-violet-300", "lora_dir" }
                        button {
                            class: "grid size-7 shrink-0 place-items-center rounded-lg bg-white/[0.05] text-zinc-500 transition hover:bg-white/[0.09] hover:text-white",
                            aria_label: "Rescan the LoRA directory",
                            title: "Rescan the LoRA directory",
                            onclick: on_rescan,
                            Icon { name: IconName::Refresh, class: "size-3" }
                        }
                    }
                }
            }
            SettingRow { title: "Prewarm on project open", description: "Start the resident server as soon as a project opens, so the first generation is not a multi-minute weight load. Leave it off while another local model shares the card.",
                Toggle { enabled: prewarm(), onchange: move |enabled| {
                    prewarm.set(enabled);
                    if let Err(error) = config.persist_prewarm(enabled) {
                        tracing::error!("could not persist prewarm: {error}");
                    }
                }
                }
            }
            SettingRow { title: "Residency policy", description: "Avoid unloading between interactive requests.", last: true,
                select { class: "setting-control", option { "Keep active model loaded" } option { "Release when idle" } }
            }
        }
        LoRASettings { defaults, discovered }
        div { class: "mt-5 flex items-start gap-3 rounded-xl bg-amber-400/[0.035] px-4 py-3 ring-1 ring-inset ring-amber-300/[0.08]",
            Icon { name: IconName::Sparkles, class: "mt-0.5 size-3.5 shrink-0 text-amber-200/50" }
            p { class: "text-[10px] leading-5 text-zinc-600", "Qwen-Image-2.1 peaks around 23.4 GiB (diffusion + text encoder + VAE + a reference encode), so leave headroom on a 24 GB card. Size and steps changes apply to the next generation immediately." }
        }
    }
}

/// Writes the live defaults back to the user config (line-preserving).
fn persist_defaults(defaults: Signal<GenerationDefaults>) {
    let snapshot = defaults.read().clone();
    let path = match svs_core::paths::user_config_path() {
        Some(path) => path,
        None => {
            tracing::error!("could not resolve the user config path; defaults not saved");
            return;
        }
    };
    if let Err(error) = AppConfig::persist_generation_defaults_to(path, &snapshot) {
        tracing::error!("could not persist generation defaults: {error}");
    }
}

/// The `WxH` size input for one artifact kind. Commits on Enter/blur; an
/// invalid value shows a hint and keeps the last good default.
#[component]
fn SizeField(
    label: String,
    value: Option<(u32, u32)>,
    error: Signal<Option<String>>,
    mut defaults: Signal<GenerationDefaults>,
    kind: &'static str,
) -> Element {
    let current = value
        .map(|(width, height)| format!("{width}x{height}"))
        .unwrap_or_default();
    let kind_label = kind.to_owned();
    rsx! {
        div { class: "flex items-center gap-3",
            if let Some(message) = error() {
                p { class: "truncate text-[9px] text-rose-300/80", "{message}" }
            }
            input {
                class: "setting-control w-24",
                placeholder: "{label}",
                value: "{current}",
                onchange: {
                    let mut defaults = defaults;
                    move |event| {
                        let value = event.value();
                        let parsed = value.trim().replace('×', "x");
                        let size = parsed
                            .split_once('x')
                            .and_then(|(width, height)| {
                                let width = width.trim().parse::<u32>().ok()?;
                                let height = height.trim().parse::<u32>().ok()?;
                                svs_core::registry::pipeline::is_valid_size(width, height)
                                    .then_some((width, height))
                            });
                        match size {
                            Some((width, height)) => {
                                error.set(None);
                                let mut settings = defaults.write();
                                match kind_label.as_str() {
                                    "character" => settings.sizes.character = Some((width, height)),
                                    "environment" => settings.sizes.environment = Some((width, height)),
                                    "object" => settings.sizes.object = Some((width, height)),
                                    "scene" => settings.sizes.scene = Some((width, height)),
                                    "beat" => settings.sizes.beat = Some((width, height)),
                                    _ => {}
                                }
                                drop(settings);
                                persist_defaults(defaults);
                            }
                            None => error.set(Some("WxH, multiples of 32".to_owned())),
                        }
                    }
                },
            }
        }
    }
}

/// Config-level adapter bindings: discovered files from `lora_dir` bound
/// to per-kind defaults at one multiplier. The `discovered` signal is owned
/// by [`GenerationSettings`] — the refresh action lives next to the
/// directory row.
#[component]
fn LoRASettings(
    mut defaults: Signal<GenerationDefaults>,
    discovered: Signal<Vec<String>>,
) -> Element {
    let bindings = defaults.read().loras.clone();
    let multiplier = bindings.multiplier;
    let files = discovered();
    let mut binding_error = use_signal(|| None::<String>);
    rsx! {
        SectionHeading {
            eyebrow: "Adapters",
            title: "LoRAs",
            description: "Bind adapters from the lora directory to kinds of work. Project bindings (/attach-lora on an artifact) always win over these defaults."
        }
        div { class: "mb-4 rounded-lg bg-white/[0.025] px-3 py-2.5 text-[10px] leading-4 text-zinc-600 ring-1 ring-inset ring-white/[0.05]",
            {
                let bound = bindings.kinds.character.is_some()
                    || bindings.kinds.environment.is_some()
                    || bindings.kinds.object.is_some();
                let summary = if files.is_empty() {
                    "No .safetensors adapters found in the LoRA directory. Drop adapter files there (or configure lora_dir in config/app.toml) and they will appear here."
                        .to_owned()
                } else {
                    format!(
                        "Found {} adapter{}{}",
                        files.len(),
                        if files.len() == 1 { "" } else { "s" },
                        if bound { " — bound below" } else { " — none bound yet" }
                    )
                };
                rsx! { p { "{summary}" } }
            }
        }
        div { class: "border-y border-white/[0.06]",
            LoraBindingRow { slot: LoraSlot::Character, current: bindings.kinds.character.clone(), files: files.clone(), defaults }
            LoraBindingRow { slot: LoraSlot::Environment, current: bindings.kinds.environment.clone(), files: files.clone(), defaults }
            LoraBindingRow { slot: LoraSlot::Object, current: bindings.kinds.object.clone(), files: files.clone(), defaults }
            SettingRow { title: "Binding weight", description: "Multiplier for the adapters above (per-artifact bindings carry their own).", last: true,
                div { class: "flex items-center gap-3",
                    if let Some(error) = binding_error() {
                        p { class: "truncate text-[9px] text-rose-300/80", "{error}" }
                    }
                    input {
                        class: "setting-control w-20",
                        r#type: "number",
                        step: "0.1",
                        min: -2,
                        max: 2,
                        value: "{multiplier}",
                        onchange: {
                            let mut defaults = defaults;
                            move |event| {
                                let parsed = event.value().parse::<f32>().ok().filter(|m| (-2.0..=2.0).contains(m));
                                match parsed {
                                    Some(multiplier) => {
                                        binding_error.set(None);
                                        defaults.write().loras.multiplier = multiplier;
                                        persist_defaults(defaults);
                                    }
                                    None => binding_error.set(Some("between -2 and 2".to_owned())),
                                }
                            }
                        },
                    }
                }
            }
        }
    }
}

/// Which use-case a LoRA binding row edits.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LoraSlot {
    Character,
    Environment,
    Object,
}

impl LoraSlot {
    fn title(self) -> &'static str {
        match self {
            Self::Character => "Characters",
            Self::Environment => "Environments",
            Self::Object => "Objects",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Character => "Applied to every character generation without a project binding.",
            Self::Environment => {
                "Applied to every environment generation without a project binding."
            }
            Self::Object => "Applied to every object generation without a project binding.",
        }
    }
}

/// One use-case row: a select of the discovered adapters plus "none".
#[component]
fn LoraBindingRow(
    slot: LoraSlot,
    current: Option<LoraBinding>,
    files: Vec<String>,
    mut defaults: Signal<GenerationDefaults>,
) -> Element {
    let current_file = current.map(|binding| binding.file).unwrap_or_default();
    let title = slot.title().to_owned();
    let description = slot.description().to_owned();
    rsx! {
        SettingRow { title, description,
            select {
                class: "setting-control",
                // `oninput` — `onchange` never fires on <select> in Dioxus,
                // and without a committed value the bound option reverts on
                // the next render (the same pattern the model picker uses).
                // The current binding is marked `selected` on the option
                // itself — the select `value` property is unreliable on
                // remount, so the option-level attribute is the source of
                // truth for what is shown.
                oninput: {
                    let mut defaults = defaults;
                    let slot = slot;
                    move |event| {
                        let value = event.value();
                        let binding = if value.is_empty() {
                            None
                        } else {
                            Some(LoraBinding { file: value })
                        };
                        let mut settings = defaults.write();
                        match slot {
                            LoraSlot::Character => settings.loras.kinds.character = binding,
                            LoraSlot::Environment => settings.loras.kinds.environment = binding,
                            LoraSlot::Object => settings.loras.kinds.object = binding,
                        }
                        drop(settings);
                        persist_defaults(defaults);
                    }
                },
                option {
                    selected: current_file.is_empty(),
                    value: "",
                    "— none —"
                }
                for file in files.iter() {
                    option {
                        selected: current_file == *file,
                        value: "{file}",
                        "{file}"
                    }
                }
            }
        }
    }
}

#[component]
fn StorageSettings(config: AppConfig) -> Element {
    rsx! {
        SectionHeading {
            eyebrow: "Files & models",
            title: "Storage",
            description: "Platform-appropriate directories keep generated assets, model weights, and disposable cache data separate."
        }
        div { class: "border-y border-white/[0.06]",
            PathRow { title: "Models", path: config.image.model_dir.display().to_string() }
            PathRow { title: "Generated assets", path: config.asset_dir.display().to_string() }
            PathRow { title: "Cache", path: config.cache_dir.display().to_string(), last: true }
        }
    }
}

#[component]
fn PathRow(title: String, path: String, #[props(default)] last: bool) -> Element {
    rsx! {
        SettingRow { title, description: "Resolved local path", last,
            div { class: "flex min-w-0 items-center gap-3",
                code { class: "min-w-0 flex-1 truncate text-[10px] text-zinc-500", title: "{path}", "{path}" }
                button { class: "shrink-0 rounded-lg bg-white/[0.05] px-2.5 py-1.5 text-[9px] text-zinc-500 hover:text-white", "Reveal" }
            }
        }
    }
}

#[component]
fn Toggle(enabled: bool, #[props(default)] onchange: Option<EventHandler<bool>>) -> Element {
    let track = if enabled {
        "bg-violet-500/70"
    } else {
        "bg-zinc-800"
    };
    let knob = if enabled {
        "translate-x-[18px]"
    } else {
        "translate-x-0.5"
    };
    rsx! {
        button {
            class: "relative ml-auto h-5 w-10 rounded-full {track} transition disabled:cursor-not-allowed disabled:opacity-40",
            role: "switch",
            aria_checked: "{enabled}",
            onclick: move |_| {
                if let Some(handler) = &onchange {
                    handler.call(!enabled);
                }
            },
            span { class: "absolute left-0 top-0.5 size-4 rounded-full bg-white shadow-sm transition {knob}" }
        }
    }
}
