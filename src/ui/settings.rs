use std::time::Duration;

use dioxus::prelude::*;

use crate::{
    app::AppConfig,
    llm::LmStudioClient,
    runtime::{
        check_all as check_services, CreativeRuntime, HealthStatus, KreaQuantization,
        ServiceHealth, ServiceId,
    },
};

use super::{
    components::{SectionHeading, SettingRow, StatusDot},
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
    Failed(String),
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

fn action_for(service: ServiceId, status: HealthStatus) -> Option<ServiceAction> {
    match (service, status) {
        (ServiceId::LmStudio, _) | (ServiceId::Segmentation, _) => None,
        (ServiceId::VisionRuntime, HealthStatus::Online) => Some(ServiceAction::Stop),
        (ServiceId::VisionRuntime, HealthStatus::Offline) => Some(ServiceAction::Restart),
        (ServiceId::VisionRuntime, _) => Some(ServiceAction::Start),
        (ServiceId::ImageGeneration, HealthStatus::Online) => Some(ServiceAction::Stop),
        (ServiceId::ImageGeneration, _) => Some(ServiceAction::Start),
    }
}

#[cfg(test)]
mod tests {
    use super::{action_for, ServiceAction};
    use crate::runtime::{HealthStatus, ServiceId};

    #[test]
    fn external_and_following_services_have_no_controls() {
        assert_eq!(action_for(ServiceId::LmStudio, HealthStatus::Online), None);
        assert_eq!(
            action_for(ServiceId::Segmentation, HealthStatus::Online),
            None
        );
    }

    #[test]
    fn resident_runtimes_offer_start_stop_by_state() {
        assert_eq!(
            action_for(ServiceId::VisionRuntime, HealthStatus::Idle),
            Some(ServiceAction::Start)
        );
        assert_eq!(
            action_for(ServiceId::VisionRuntime, HealthStatus::Online),
            Some(ServiceAction::Stop)
        );
        assert_eq!(
            action_for(ServiceId::VisionRuntime, HealthStatus::Offline),
            Some(ServiceAction::Restart)
        );
        assert_eq!(
            action_for(ServiceId::ImageGeneration, HealthStatus::Idle),
            Some(ServiceAction::Start)
        );
        assert_eq!(
            action_for(ServiceId::ImageGeneration, HealthStatus::Degraded),
            Some(ServiceAction::Start)
        );
        assert_eq!(
            action_for(ServiceId::ImageGeneration, HealthStatus::Online),
            Some(ServiceAction::Stop)
        );
    }
}

#[component]
pub fn Settings(config: AppConfig, runtime: CreativeRuntime) -> Element {
    let mut section = use_signal(|| SettingsSection::Status);
    let profile = use_signal(|| config.generation.profile);
    let health = use_signal(Vec::<ServiceHealth>::new);
    let checking = use_signal(|| false);
    let checked_at = use_signal(|| None::<String>);
    let lm_model = use_signal(|| config.lm_studio.model.clone());
    let models_state = use_signal(|| ModelListState::Empty);
    let busy = use_signal(|| None::<ServiceId>);
    let operation_error = use_signal(|| None::<String>);

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
                        SettingsSection::Intelligence => rsx! { IntelligenceSettings { config: config.clone(), health, lm_model, models_state, on_check: on_check.clone() } },
                        SettingsSection::Generation => rsx! { GenerationSettings { config: config.clone(), profile } },
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
    check_config.lm_studio.model = lm_model();
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
    state.set(ModelListState::Loading);
    spawn(async move {
        let result = async {
            let mut lm_config = config.lm_studio.clone();
            lm_config.timeout = Duration::from_secs(5);
            LmStudioClient::new(lm_config)?.list_models().await
        }
        .await;
        match result {
            Ok(models) => {
                let mut ids: Vec<String> = models.data.into_iter().map(|model| model.id).collect();
                ids.sort();
                ids.dedup();
                state.set(ModelListState::Ready(ids));
            }
            Err(error) => state.set(ModelListState::Failed(error.to_string())),
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
    mut operation_error: Signal<Option<String>>,
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
            (ServiceId::VisionRuntime, ServiceAction::Start) => {
                runtime.start_vision_runtime().await
            }
            (ServiceId::VisionRuntime, ServiceAction::Stop) => runtime.stop_vision_runtime().await,
            (ServiceId::VisionRuntime, ServiceAction::Restart) => {
                runtime.restart_vision_runtime().await
            }
            (ServiceId::ImageGeneration, ServiceAction::Start) => {
                runtime.start_generation_runtime().await
            }
            (ServiceId::ImageGeneration, ServiceAction::Stop) => {
                runtime.stop_generation_runtime().await
            }
            (ServiceId::ImageGeneration, ServiceAction::Restart) => {
                let _ = runtime.stop_generation_runtime().await;
                runtime.start_generation_runtime().await
            }
            (ServiceId::LmStudio | ServiceId::Segmentation, _) => Ok(()),
        };
        busy.set(None);
        match result {
            Ok(()) => {}
            Err(error) => operation_error.set(Some(error.to_string())),
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
    operation_error: Signal<Option<String>>,
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
            p { class: "mb-4 rounded-lg bg-rose-400/[0.06] px-3 py-2 text-[10px] leading-4 text-rose-200/80 ring-1 ring-inset ring-rose-300/[0.12]", "{error}" }
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
                            } else if let Some(action) = action_for(service.id, service.status) {
                                button {
                                    class: "h-7 rounded-md bg-white/[0.06] px-2.5 text-[10px] font-medium text-zinc-400 transition hover:bg-white/[0.1] hover:text-white disabled:cursor-not-allowed disabled:opacity-40",
                                    disabled: is_busy,
                                    onclick: {
                                        let service = service.id;
                                        move |_| on_operate.call((service, action))
                                    },
                                    "{action.label()}"
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
            SettingRow { title: "Keep models resident", description: "Favor low latency by retaining active models in memory.",
                Toggle { enabled: true }
            }
            SettingRow { title: "Autosave", description: "Save project changes locally as you work.", last: true,
                select {
                    class: "setting-control",
                    value: autosave().as_str(),
                    onchange: move |event| autosave.set(AutosaveMode::from_value(&event.value())),
                    option { value: "on_change", "After every change" }
                    option { value: "every_minute", "Every minute" }
                    option { value: "off", "Off" }
                }
            }
        }
    }
}

#[component]
fn IntelligenceSettings(
    config: AppConfig,
    health: Signal<Vec<ServiceHealth>>,
    lm_model: Signal<String>,
    models_state: Signal<ModelListState>,
    on_check: EventHandler<MouseEvent>,
) -> Element {
    let lm_studio = health
        .read()
        .iter()
        .find(|service| service.id == ServiceId::LmStudio)
        .cloned();
    let (status, detail) = match lm_studio {
        Some(service) => (service.status, service.detail),
        None => (HealthStatus::Idle, "Not checked yet".to_owned()),
    };
    rsx! {
        SectionHeading {
            eyebrow: "Planner & vision",
            title: "Intelligence",
            description: "LM Studio handles planning and visual understanding. It remains an external service and is never bundled with the app."
        }
        div { class: "mb-8 flex items-center justify-between border-y border-white/[0.06] py-4",
            div { class: "flex items-center gap-3",
                span { class: "grid size-9 place-items-center rounded-xl bg-white/[0.035]", StatusDot { status } }
                div { p { class: "text-[12px] font-medium text-zinc-200", "LM Studio" } p { class: "mt-0.5 text-[10px] text-zinc-600", "{detail}" } }
            }
            button { class: "rounded-lg bg-white/[0.05] px-3 py-2 text-[10px] font-medium text-zinc-400 transition hover:text-white", onclick: move |event| on_check.call(event), "Test connection" }
        }
        div { class: "border-y border-white/[0.06]",
            SettingRow { title: "Base URL", description: "OpenAI-compatible API endpoint.",
                input { class: "setting-control", value: "{config.lm_studio.base_url}", readonly: true }
            }
            SettingRow { title: "Planner model", description: "Model identifier loaded in LM Studio. The selection applies for this session; config/app.toml holds the default.",
                ModelPicker { config: config.clone(), lm_model, models_state }
            }
            SettingRow { title: "Request timeout", description: "Maximum wait for planning and visual analysis.", last: true,
                div { class: "flex items-center gap-3", input { class: "setting-control", value: "{config.lm_studio.timeout.as_secs()}", readonly: true } span { class: "text-[10px] text-zinc-700", "seconds" } }
            }
        }
        p { class: "mt-4 text-[10px] leading-5 text-zinc-700", "Model discovery connects to the configured Base URL. To make a selection permanent, set model in config/app.toml." }
    }
}

#[component]
fn ModelPicker(
    config: AppConfig,
    mut lm_model: Signal<String>,
    models_state: Signal<ModelListState>,
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
                    span { class: "truncate text-[10px] text-zinc-600", title: "{error}", "Could not reach LM Studio" }
                    button { class: "shrink-0 rounded-lg bg-white/[0.05] px-2.5 py-1.5 text-[9px] text-zinc-500 transition hover:text-white", onclick: move |_| fetch_models(refresh_config.clone(), models_state), "Retry" }
                },
                ModelListState::Ready(models) => {
                    let current = lm_model();
                    let has_current = models.iter().any(|model| model == &current);
                    rsx! {
                        select {
                            class: "setting-control",
                            value: "{lm_model}",
                            oninput: move |event| lm_model.set(event.value()),
                            if !has_current {
                                option { value: "{current}", "{current} (not loaded)" }
                            }
                            for model in models.iter() {
                                option { value: "{model}", "{model}" }
                            }
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
                    select {
                        class: "setting-control",
                        value: "{lm_model}",
                        oninput: move |event| lm_model.set(event.value()),
                        option { value: "{lm_model}", "{lm_model}" }
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
fn GenerationSettings(config: AppConfig, mut profile: Signal<KreaQuantization>) -> Element {
    rsx! {
        SectionHeading {
            eyebrow: "Local image engine",
            title: "Image generation",
            description: "Krea 2 Turbo runs locally through the native resident runtime, with no ComfyUI dependency. Profiles are designed for a 24 GB memory budget."
        }
        div { class: "mb-8 grid grid-cols-2 gap-3",
            ProfileChoice {
                name: "Q2 · Fast draft",
                detail: "Lower memory · fastest iteration",
                selected: profile() == KreaQuantization::Q2,
                onclick: move |_| profile.set(KreaQuantization::Q2)
            }
            ProfileChoice {
                name: "Q4 · Final quality",
                detail: "Higher fidelity · more memory",
                selected: profile() == KreaQuantization::Q4,
                onclick: move |_| profile.set(KreaQuantization::Q4)
            }
        }
        div { class: "border-y border-white/[0.06]",
            SettingRow { title: "Runtime endpoint", description: "Resident stable-diffusion.cpp service.",
                input { class: "setting-control", value: "{config.generation.base_url}", readonly: true }
            }
            SettingRow { title: "Text encoder", description: "Shared quantized encoder kept resident with Krea.",
                div { class: "flex items-center justify-between", span { class: "text-xs text-zinc-300", "Qwen3-VL 4B" } span { class: "rounded-md bg-violet-500/10 px-2 py-1 text-[9px] text-violet-300", "Q4 GGUF" } }
            }
            SettingRow { title: "LoRA support", description: "Apply up to eight adapters per generation request.",
                div { class: "flex items-center justify-between", span { class: "truncate text-[11px] text-zinc-500", "{config.generation.lora_dir.display()}" } Toggle { enabled: true } }
            }
            SettingRow { title: "Residency policy", description: "Avoid unloading between interactive requests.", last: true,
                select { class: "setting-control", option { "Keep active model loaded" } option { "Release when idle" } }
            }
        }
        div { class: "mt-5 flex items-start gap-3 rounded-xl bg-amber-400/[0.035] px-4 py-3 ring-1 ring-inset ring-amber-300/[0.08]",
            Icon { name: IconName::Sparkles, class: "mt-0.5 size-3.5 shrink-0 text-amber-200/50" }
            p { class: "text-[10px] leading-5 text-zinc-600", "Changing quantization requires switching the resident generation process. Queued work will finish before the switch in the production scheduler." }
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
            PathRow { title: "Models", path: config.model_dir.display().to_string() }
            PathRow { title: "Generated assets", path: config.asset_dir.display().to_string() }
            PathRow { title: "Cache", path: config.cache_dir.display().to_string() }
            PathRow { title: "Python runtime", path: config.python_runtime.display().to_string(), last: true }
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
fn ProfileChoice(
    name: String,
    detail: String,
    selected: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let class = if selected {
        "relative p-4 text-left ring-1 ring-inset ring-violet-400/35 bg-violet-500/[0.055] rounded-xl"
    } else {
        "relative p-4 text-left ring-1 ring-inset ring-white/[0.07] hover:ring-white/[0.13] rounded-xl transition"
    };
    rsx! {
        button { class, onclick,
            div { class: "flex items-center justify-between", p { class: "text-[12px] font-medium text-zinc-200", "{name}" } if selected { span { class: "size-1.5 rounded-full bg-violet-400" } } }
            p { class: "mt-1.5 text-[10px] text-zinc-600", "{detail}" }
        }
    }
}

#[component]
fn Toggle(enabled: bool) -> Element {
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
        button { class: "relative ml-auto h-5 w-10 rounded-full {track} transition", role: "switch", aria_checked: "{enabled}",
            span { class: "absolute left-0 top-0.5 size-4 rounded-full bg-white shadow-sm transition {knob}" }
        }
    }
}
