use std::time::Duration;

use dioxus::prelude::*;

use storyfella_studio_core::config::{
    AppConfig, ChatCapability, ChatEndpointField, ChatProvider, NewChatProvider,
    NewChatProviderKind,
};
use storyfella_studio_core::errors::{CodedError, ReportedError};
use storyfella_studio_core::llm::{LlmClient, LlmConfig};
use storyfella_studio_core::registry::{
    backend::CreativeBackend, discover_loras, GenerationDefaults, LoraBinding,
};
use storyfella_studio_core::runtime::{
    check_all as check_services, image_artifacts, provision_image_model, CreativeRuntime,
    HealthStatus, ModelArtifactStatus, ServiceHealth, ServiceId, ServiceRecovery,
};

use super::{
    components::{
        ErrorDetailsButton, SearchSelect, SectionHeading, SelectOption, SettingRow, StatusDot,
    },
    icons::{Icon, IconName},
    AutosaveMode,
};

#[derive(Clone, Copy)]
pub struct PersistedDefaults(pub Signal<GenerationDefaults>);
#[derive(Clone, Copy)]
struct RefreshHealth(EventHandler<()>);

#[derive(Clone, Copy, Debug, PartialEq)]
enum SettingsSection {
    General,
    Models,
    Services,
    Storage,
}

#[derive(Clone, Debug, PartialEq)]
enum ModelListState {
    Empty,
    Loading,
    Ready(Vec<String>),
    /// A managed `llama-server` that is not running: its models cannot be
    /// listed until the app starts it on demand.
    OnDemand,
    Failed(ReportedError),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ServiceAction {
    Start,
    Stop,
    Restart,
}

/// The state of an in-app model download.
#[derive(Clone, Debug, PartialEq)]
enum ModelPrepState {
    Idle,
    Running {
        filename: String,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Done,
    Failed(ReportedError),
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

/// The controls a service offers at a given status. The image engine shares one
/// server with the other front ends through the shared lifecycle owner, so it
/// only needs **Start** (ensure the configured model is resident, replacing a
/// server the shared owner started) and **Stop**; a foreign server is reported
/// rather than replaced here. Missing files need setup first. External
/// (text/vision) endpoints are never started or stopped here — a managed
/// `llama-server` has its own Start/Stop/Restart.
fn actions_for(
    service: ServiceId,
    status: HealthStatus,
    recovery: Option<ServiceRecovery>,
) -> Vec<ServiceAction> {
    match (service, status, recovery) {
        (ServiceId::Llm | ServiceId::Vision, _, _) => Vec::new(),
        (ServiceId::ImageGeneration, _, Some(ServiceRecovery::ConfigureImageFiles)) => Vec::new(),
        (ServiceId::ImageGeneration, HealthStatus::Online, None) => vec![ServiceAction::Stop],
        (ServiceId::ImageGeneration, _, None) => vec![ServiceAction::Start],
    }
}

#[cfg(test)]
mod tests {
    use super::{actions_for, ServiceAction};
    use storyfella_studio_core::runtime::{HealthStatus, ServiceId, ServiceRecovery};

    #[test]
    fn external_services_have_no_controls() {
        assert!(actions_for(ServiceId::Llm, HealthStatus::Online, None).is_empty());
        assert!(actions_for(ServiceId::Vision, HealthStatus::Online, None).is_empty());
    }

    #[test]
    fn the_image_engine_offers_start_stop_only() {
        for status in [
            HealthStatus::Idle,
            HealthStatus::Degraded,
            HealthStatus::Offline,
        ] {
            assert_eq!(
                actions_for(ServiceId::ImageGeneration, status, None),
                vec![ServiceAction::Start]
            );
        }
        assert_eq!(
            actions_for(ServiceId::ImageGeneration, HealthStatus::Online, None),
            vec![ServiceAction::Stop]
        );
        assert!(actions_for(
            ServiceId::ImageGeneration,
            HealthStatus::Offline,
            Some(ServiceRecovery::ConfigureImageFiles)
        )
        .is_empty());
    }
}

#[component]
pub fn Settings(
    config: AppConfig,
    backend: Signal<CreativeBackend>,
    defaults: Signal<storyfella_studio_core::registry::GenerationDefaults>,
) -> Element {
    let runtime = backend.read().runtime();
    let mut section = use_signal(|| SettingsSection::Models);
    // The effective config, updated in place when a provider picker switches the
    // planner or vision endpoint, so the rows, checks and model list below track
    // the live choice without a restart.
    let mut config = use_signal(|| config);
    let health = use_signal(Vec::<ServiceHealth>::new);
    let checking = use_signal(|| false);
    let checked_at = use_signal(|| None::<String>);
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let text_models = use_signal(|| ModelListState::Empty);
    let vision_models = use_signal(|| ModelListState::Empty);
    let busy = use_signal(|| None::<ServiceId>);
    let mut operation_error = use_signal(|| None::<ReportedError>);
    // The managed chat servers start/stop/restart from Services; the running set
    // is refreshed after each operation, on Check now, and when the page opens.
    let mut chat_servers = use_signal(Vec::<String>::new);
    let mut chat_busy = use_signal(|| None::<String>);

    let effect_runtime = runtime.clone();
    let mut initialized = use_signal(|| false);
    use_effect(move || {
        if !initialized() {
            initialized.set(true);
            let current = config.read().clone();
            run_checks(
                effect_runtime.clone(),
                current.clone(),
                health,
                checking,
                checked_at,
            );
            fetch_models(&current, ChatCapability::Text, text_models);
            fetch_models(&current, ChatCapability::Vision, vision_models);
        }
    });

    // Keep the Services chat-server rows in step with the page being shown and
    // with every Check now.
    let sync_backend = backend;
    use_effect(move || {
        if section() == SettingsSection::Services {
            chat_servers.set(sync_backend.read().running_chat_servers());
        }
        let _ = checked_at();
    });

    let check_runtime = runtime.clone();
    let on_check = EventHandler::new(move |_event: MouseEvent| {
        let current = config.read().clone();
        run_checks(
            check_runtime.clone(),
            current.clone(),
            health,
            checking,
            checked_at,
        );
        // A check re-probes the endpoints, so their model lists are refreshed
        // too: a server started since the last check shows its ids at once.
        fetch_models(&current, ChatCapability::Text, text_models);
        fetch_models(&current, ChatCapability::Vision, vision_models);
    });

    let refresh_runtime = runtime.clone();
    use_context_provider(|| {
        RefreshHealth(EventHandler::new(move |_| {
            run_checks(
                refresh_runtime.clone(),
                config.read().clone(),
                health,
                checking,
                checked_at,
            );
        }))
    });
    let operate_runtime = runtime.clone();
    let on_operate = move |(service, action): (ServiceId, ServiceAction)| {
        operate_service(
            operate_runtime.clone(),
            config.read().clone(),
            service,
            action,
            ServiceSignals {
                busy,
                operation_error,
                health,
                checking,
                checked_at,
            },
        );
    };

    // A managed chat server starts, stops and restarts from Services; the
    // running set and the health probe are refreshed afterwards.
    let chat_runtime = runtime.clone();
    let on_chat_operate = move |(id, action): (String, ServiceAction)| {
        if (chat_busy)().is_some() {
            return;
        }
        chat_busy.set(Some(id.clone()));
        operation_error.set(None);
        let runtime = chat_runtime.clone();
        spawn(async move {
            let engine = backend.read().clone();
            let result = match action {
                ServiceAction::Start => engine.start_chat_server(&id).await,
                ServiceAction::Stop => engine.stop_chat_server(&id).await,
                ServiceAction::Restart => engine.restart_chat_server(&id).await,
            };
            chat_busy.set(None);
            if let Err(error) = result {
                operation_error.set(Some(error.report().log()));
            }
            chat_servers.set(engine.running_chat_servers());
            let current = config.read().clone();
            // A start/stop changes what the endpoint serves, so re-list the ids.
            fetch_models(&current, ChatCapability::Text, text_models);
            fetch_models(&current, ChatCapability::Vision, vision_models);
            run_checks(runtime, current, health, checking, checked_at);
        });
    };

    // Switching a capability to a different configured provider persists the
    // choice under `[defaults]`, then applies it: the effective config is
    // updated in place, the planner client rebuilt, and health/discovery re-run.
    let switch_text_runtime = runtime.clone();
    let on_text_provider = move |id: String| {
        if config.read().text_provider_id.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(next) = config.read().with_text_provider(&id) else {
            return;
        };
        if let Err(error) = config.read().persist_text_provider(&id) {
            error_report.set(Some(error.report().log()));
            return;
        }
        if let Some(provider) = next
            .chat_providers
            .iter()
            .find(|provider| provider.id == id)
        {
            let capabilities = provider.capabilities.clone();
            if let Err(error) = config
                .read()
                .persist_provider_capabilities(&id, &capabilities)
            {
                error_report.set(Some(error.report().log()));
                return;
            }
        }
        *config.write() = next.clone();
        backend.write().set_chat(&next);
        run_checks(
            switch_text_runtime.clone(),
            next.clone(),
            health,
            checking,
            checked_at,
        );
        fetch_models(&next, ChatCapability::Text, text_models);
    };

    let switch_vision_runtime = runtime.clone();
    let on_vision_provider = move |id: String| {
        if config.read().vision_provider_id.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(next) = config.read().with_vision_provider(&id) else {
            return;
        };
        if let Err(error) = config.read().persist_vision_provider(&id) {
            error_report.set(Some(error.report().log()));
            return;
        }
        if let Some(provider) = next
            .chat_providers
            .iter()
            .find(|provider| provider.id == id)
        {
            let capabilities = provider.capabilities.clone();
            if let Err(error) = config
                .read()
                .persist_provider_capabilities(&id, &capabilities)
            {
                error_report.set(Some(error.report().log()));
                return;
            }
        }
        *config.write() = next.clone();
        backend.write().set_chat(&next);
        run_checks(
            switch_vision_runtime.clone(),
            next.clone(),
            health,
            checking,
            checked_at,
        );
        fetch_models(&next, ChatCapability::Vision, vision_models);
    };

    // Editing an endpoint field persists it, applies it in memory, rebuilds the
    // planner client (so the canvas uses the new endpoint at once) and re-checks
    // the services the change affects.
    let edit_runtime = runtime.clone();
    let on_edit_endpoint =
        move |(provider_id, field, value): (String, ChatEndpointField, String)| {
            let value = value.trim().to_owned();
            // An empty base URL or model is not a usable endpoint; an empty API
            // key clears the inline secret.
            if value.is_empty() && field != ChatEndpointField::ApiKey {
                return;
            }
            let Some(next) = config
                .read()
                .with_chat_endpoint(&provider_id, field, &value)
            else {
                return;
            };
            if let Err(error) = config
                .read()
                .persist_chat_endpoint(&provider_id, field, &value)
            {
                error_report.set(Some(error.report().log()));
                return;
            }
            let text_changed = next.text_provider_id.as_deref() == Some(provider_id.as_str());
            let vision_changed = next.vision_provider_id.as_deref() == Some(provider_id.as_str());
            *config.write() = next.clone();
            backend.write().set_chat(&next);
            run_checks(
                edit_runtime.clone(),
                next.clone(),
                health,
                checking,
                checked_at,
            );
            if text_changed {
                fetch_models(&next, ChatCapability::Text, text_models);
            }
            if vision_changed {
                fetch_models(&next, ChatCapability::Vision, vision_models);
            }
        };

    let on_fetch_text = move |_| {
        fetch_models(&config.read(), ChatCapability::Text, text_models);
    };
    let on_fetch_vision = move |_| {
        fetch_models(&config.read(), ChatCapability::Vision, vision_models);
    };

    // Adding or removing a provider rewrites the user overlay; the effective
    // config is reloaded from disk so the whole registry is validated and the
    // pickers, health and model lists all track it.
    let targets = ConfigTargets {
        config,
        backend,
        health,
        checking,
        checked_at,
        text_models,
        vision_models,
    };
    let provider_runtime = runtime.clone();
    let on_add_provider = move |provider: NewChatProvider| {
        let current = targets.config.read().clone();
        if let Err(error) = current.persist_provider_added(&provider) {
            error_report.set(Some(error.report().log()));
            return;
        }
        match AppConfig::load() {
            Ok(next) => apply_config(next, provider_runtime.clone(), targets),
            Err(error) => error_report.set(Some(error.report().log())),
        }
    };
    let remove_runtime = runtime.clone();
    let on_remove_provider = move |id: String| {
        let current = targets.config.read().clone();
        if let Err(error) = current.persist_provider_removed(&id) {
            error_report.set(Some(error.report().log()));
            return;
        }
        match AppConfig::load() {
            Ok(next) => apply_config(next, remove_runtime.clone(), targets),
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
                    SettingsNav { label: "Services", active: section() == SettingsSection::Services, onclick: move |_| section.set(SettingsSection::Services) }
                    SettingsNav { label: "Storage", active: section() == SettingsSection::Storage, onclick: move |_| section.set(SettingsSection::Storage) }
                }
                div { class: "absolute bottom-7 px-3 text-[9px] leading-4 text-zinc-700", "Storyfella Studio" br {} "Prototype 0.1.0" }
            }
            div { class: "min-h-0 overflow-y-auto px-10 py-9",
                div { class: "mx-auto max-w-3xl",
                    div { hidden: section() != SettingsSection::General, GeneralSettings {} }
                    div { hidden: section() != SettingsSection::Models, ModelsSettings { config: config(), defaults, health, checking, checked_at, text_models, vision_models, on_check, on_text_provider, on_vision_provider, on_edit_endpoint, on_fetch_text, on_fetch_vision, on_add_provider, on_remove_provider } }
                    div { hidden: section() != SettingsSection::Services, ServicesSettings { config: config(), backend, health, checking, checked_at, busy, operation_error, chat_servers, chat_busy, on_check, on_operate, on_chat_operate } }
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
    mut checking: Signal<bool>,
    mut checked_at: Signal<Option<String>>,
) {
    if checking() {
        return;
    }
    checking.set(true);
    spawn(async move {
        let services = check_services(&config, &runtime).await;
        health.set(services);
        checked_at.set(Some(current_time_label()));
        checking.set(false);
    });
}

/// Discovers the model ids the provider serving `capability` offers. No
/// provider is an empty list, not a failure. A managed `llama-server` that is
/// not running cannot be listed, so it reports `OnDemand` rather than an error.
fn fetch_models(config: &AppConfig, capability: ChatCapability, mut state: Signal<ModelListState>) {
    if *state.read() == ModelListState::Loading {
        return;
    }
    let deployment = match capability {
        ChatCapability::Text => config.text.as_ref(),
        ChatCapability::Vision => config.vision.as_ref(),
    };
    let Some(deployment) = deployment else {
        state.set(ModelListState::Empty);
        return;
    };
    let managed = deployment.server.is_some();
    let mut lm_config = LlmConfig::from(deployment);
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
    checking: Signal<bool>,
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

/// The UI state updated together by a service operation and its health check.
#[derive(Clone, Copy)]
struct ServiceSignals {
    busy: Signal<Option<ServiceId>>,
    operation_error: Signal<Option<ReportedError>>,
    health: Signal<Vec<ServiceHealth>>,
    checking: Signal<bool>,
    checked_at: Signal<Option<String>>,
}

fn operate_service(
    runtime: CreativeRuntime,
    config: AppConfig,
    service: ServiceId,
    action: ServiceAction,
    mut signals: ServiceSignals,
) {
    if (signals.busy)().is_some() {
        return;
    }
    signals.busy.set(Some(service));
    signals.operation_error.set(None);
    spawn(async move {
        let result = match (service, action) {
            (ServiceId::ImageGeneration, ServiceAction::Start) => runtime.ensure_ready().await,
            (ServiceId::ImageGeneration, ServiceAction::Stop) => {
                runtime.stop_generation_runtime().await
            }
            _ => Ok(()),
        };
        signals.busy.set(None);
        match result {
            Ok(()) => {}
            Err(error) => signals.operation_error.set(Some(error.report().log())),
        }
        run_checks(
            runtime,
            config,
            signals.health,
            signals.checking,
            signals.checked_at,
        );
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
                p { class: "text-sm text-zinc-300", "Dark theme" }
            }
            SettingRow { title: "Background generation", description: "Continue queued jobs while you read or present other scenes.",
                p { class: "text-sm text-zinc-300", "Jobs continue while you use other screens." }
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
    defaults: Signal<GenerationDefaults>,
    health: Signal<Vec<ServiceHealth>>,
    checking: Signal<bool>,
    checked_at: Signal<Option<String>>,
    text_models: Signal<ModelListState>,
    vision_models: Signal<ModelListState>,
    on_check: EventHandler<MouseEvent>,
    on_text_provider: EventHandler<String>,
    on_vision_provider: EventHandler<String>,
    on_edit_endpoint: EventHandler<(String, ChatEndpointField, String)>,
    on_fetch_text: EventHandler<()>,
    on_fetch_vision: EventHandler<()>,
    on_add_provider: EventHandler<NewChatProvider>,
    on_remove_provider: EventHandler<String>,
) -> Element {
    let services = health.read().clone();
    let (text_status, text_detail) = service_of(&services, ServiceId::Llm);
    let (vision_status, vision_detail) = service_of(&services, ServiceId::Vision);
    let text = config.text.clone();
    let vision = config.vision.clone();
    // Text and vision are the same kind of OpenAI-compatible endpoint — the
    // same provider list, the same fields. Whether the app also starts the
    // server (a `llamacpp` provider) is an internal detail, not a card shape.
    let chat_providers = provider_options(&config.chat_providers);
    rsx! {
        SectionHeading {
            eyebrow: "Capabilities",
            title: "Models",
            description: "Choose the provider and model that serves each capability."
        }
        CheckBar { checking, checked_at, on_check }
        ChatEndpointGroup {
            title: "Text (LLM)".to_owned(),
            description: "Plans operation stacks and grounds edits.".to_owned(),
            providers: chat_providers.clone(),
            provider_id: config.text_provider_id.clone().unwrap_or_default(),
            base_url: text.as_ref().map(|d| d.base_url.clone()).unwrap_or_default(),
            model: text.as_ref().map(|d| d.model.clone()).unwrap_or_default(),
            api_key_set: text.as_ref().is_some_and(|d| d.api_key.is_some()),
            status: text_status,
            detail: text_detail,
            models: text_models,
            on_select_provider: on_text_provider,
            on_commit: on_edit_endpoint,
            on_fetch: on_fetch_text,
        }
        ChatEndpointGroup {
            title: "Vision (LLM)".to_owned(),
            description: "Reads images for `svs eval` and plan `expect_vision` steps.".to_owned(),
            providers: chat_providers,
            provider_id: config.vision_provider_id.clone().unwrap_or_default(),
            base_url: vision.as_ref().map(|d| d.base_url.clone()).unwrap_or_default(),
            model: vision.as_ref().map(|d| d.model.clone()).unwrap_or_default(),
            api_key_set: vision.as_ref().is_some_and(|d| d.api_key.is_some()),
            status: vision_status,
            detail: vision_detail,
            models: vision_models,
            on_select_provider: on_vision_provider,
            on_commit: on_edit_endpoint,
            on_fetch: on_fetch_vision,
        }
        ImageModelGroup { config: config.clone(), defaults, services }
        ProvidersSection { config: config.clone(), on_add: on_add_provider, on_remove: on_remove_provider }
    }
}

/// The shared check bar: when the services were last probed, and the action
/// that re-probes them all.
#[component]
fn CheckBar(
    checking: Signal<bool>,
    checked_at: Signal<Option<String>>,
    on_check: EventHandler<MouseEvent>,
) -> Element {
    let is_checking = checking();
    rsx! {
        div { class: "mb-6 flex items-center justify-between",
            p { class: "text-[10px] text-zinc-600",
                if let Some(at) = checked_at() { "Last checked {at}" } else { "Not checked yet" }
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
    }
}

/// A failed service operation, with the shared Details action.
#[component]
fn OperationErrorBanner(error: ReportedError) -> Element {
    rsx! {
        div { class: "mb-4 flex items-start gap-3 rounded-lg bg-rose-400/[0.06] px-3 py-2 ring-1 ring-inset ring-rose-300/[0.12]",
            span { class: "mt-1 size-1.5 shrink-0 rounded-full bg-rose-400" }
            div { class: "min-w-0 flex-1",
                p { class: "font-mono text-[9px] text-rose-300/70", "{error.code.code()}" }
                p { class: "mt-0.5 text-[10px] leading-4 text-rose-200/80", "{error.code.message()}" }
            }
            ErrorDetailsButton { error }
        }
    }
}

/// How the app runs its backends: the servers it launches, the files they load,
/// and where those files live. Choosing which model serves a capability is on
/// the Models page.
#[component]
fn ServicesSettings(
    config: AppConfig,
    backend: Signal<CreativeBackend>,
    health: Signal<Vec<ServiceHealth>>,
    checking: Signal<bool>,
    checked_at: Signal<Option<String>>,
    busy: Signal<Option<ServiceId>>,
    operation_error: Signal<Option<ReportedError>>,
    chat_servers: Signal<Vec<String>>,
    chat_busy: Signal<Option<String>>,
    on_check: EventHandler<MouseEvent>,
    on_operate: EventHandler<(ServiceId, ServiceAction)>,
    on_chat_operate: EventHandler<(String, ServiceAction)>,
) -> Element {
    let services = health.read().clone();
    rsx! {
        SectionHeading {
            eyebrow: "Runtime",
            title: "Services",
            description: "The servers the app runs and the files they load."
        }
        CheckBar { checking, checked_at, on_check }
        if let Some(error) = operation_error() {
            OperationErrorBanner { error }
        }
        ImageServiceCard { config: config.clone(), backend, services: services.clone(), busy, on_operate }
        div { id: "image-models", ModelFiles { config: config.clone() } }
        ChatServers { config: config.clone(), running: chat_servers, busy: chat_busy, on_operate: on_chat_operate }
    }
}

/// The chat servers the app can start: a `llamacpp` provider's `llama-server`,
/// started on demand and stopped before a generation. Start/stop/restart live
/// here; the Models page only picks the provider and model.
#[component]
fn ChatServers(
    config: AppConfig,
    running: Signal<Vec<String>>,
    busy: Signal<Option<String>>,
    on_operate: EventHandler<(String, ServiceAction)>,
) -> Element {
    let rows: Vec<(String, bool, String)> = config
        .chat_providers
        .iter()
        .filter(|provider| !provider.is_external())
        .map(|provider| {
            (
                provider.id.clone(),
                running().contains(&provider.id),
                chat_server_description(provider),
            )
        })
        .collect();
    let last = rows.len().saturating_sub(1);
    rsx! {
        SectionHeading {
            eyebrow: "Managed",
            title: "Chat servers",
            description: "The llama.cpp servers the app starts for a `llamacpp` provider."
        }
        if rows.is_empty() {
            p { class: "text-[11px] leading-5 text-zinc-600", "No managed chat provider. A provider with kind `llamacpp` appears here with Start, Stop and Restart." }
        } else {
            div { class: "border-y border-white/[0.06]",
                for (index, (id, is_running, description)) in rows.iter().enumerate() {
                    SettingRow {
                        title: id.clone(),
                        description: description.clone(),
                        last: index == last,
                        div { class: "flex items-center justify-end gap-3",
                            if busy() == Some(id.clone()) {
                                span { class: "size-3 animate-spin rounded-full border border-zinc-600 border-t-white" }
                            } else {
                                span { class: "text-[10px] text-zinc-600", if *is_running { "Running" } else { "Stopped" } }
                            }
                            div { class: "flex shrink-0 items-center gap-2",
                                for action in chat_actions(*is_running) {
                                    ServiceActionButton {
                                        label: action.label().to_owned(),
                                        disabled: busy().is_some(),
                                        onclick: {
                                            let id = id.clone();
                                            move |_| on_operate.call((id.clone(), action))
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A service action button (`Start`/`Stop`/`Restart`), shared by the image
/// engine and the chat servers.
#[component]
fn ServiceActionButton(
    label: String,
    disabled: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            class: "rounded-md bg-white/[0.06] px-2.5 py-1.5 text-[10px] font-medium text-zinc-400 transition hover:bg-white/[0.1] hover:text-white disabled:cursor-not-allowed disabled:opacity-40",
            disabled,
            onclick: move |event| onclick.call(event),
            "{label}"
        }
    }
}

/// The controls a managed chat server offers: start when it is down, restart or
/// stop while it is up.
fn chat_actions(running: bool) -> Vec<ServiceAction> {
    if running {
        vec![ServiceAction::Restart, ServiceAction::Stop]
    } else {
        vec![ServiceAction::Start]
    }
}

/// A chat server's model file, for the Services row: the short name the app
/// loads, not the full path.
fn chat_server_description(provider: &ChatProvider) -> String {
    match provider.managed_launch() {
        Some(launch) => {
            let model = std::path::Path::new(&launch.model_path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or(launch.model_path);
            format!("llama.cpp · {model}")
        }
        None => "llama.cpp".to_owned(),
    }
}

/// The configured chat providers: what each serves, which the user may remove,
/// and the action that adds a new one to the user overlay.
#[component]
fn ProvidersSection(
    config: AppConfig,
    on_add: EventHandler<NewChatProvider>,
    on_remove: EventHandler<String>,
) -> Element {
    let mut adding = use_signal(|| false);
    let providers = config.chat_providers.clone();
    let user_providers = config.user_providers.clone();
    let existing: Vec<String> = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect();
    let last = providers.len().saturating_sub(1);
    rsx! {
        SectionHeading {
            eyebrow: "Configured",
            title: "Providers",
            description: "The endpoints the app can use. Add one, or remove one you added."
        }
        if providers.is_empty() {
            p { class: "text-[11px] leading-5 text-zinc-600", "No chat provider is configured." }
        } else {
            div { class: "border-y border-white/[0.06]",
                for (index, provider) in providers.iter().enumerate() {
                    SettingRow {
                        title: provider.id.clone(),
                        description: provider_summary(provider),
                        last: index == last,
                        div { class: "flex items-center justify-end gap-3",
                            if user_providers.contains(&provider.id) {
                                button {
                                    class: "rounded-md bg-white/[0.06] px-2.5 py-1.5 text-[10px] font-medium text-zinc-400 transition hover:bg-rose-400/[0.12] hover:text-rose-200",
                                    onclick: {
                                        let id = provider.id.clone();
                                        move |_| on_remove.call(id.clone())
                                    },
                                    "Remove"
                                }
                            } else {
                                span { class: "text-[10px] text-zinc-700", "from configuration" }
                            }
                        }
                    }
                }
            }
        }
        div { class: "mt-5 flex justify-end",
            button {
                class: "rounded-lg bg-white/[0.06] px-3.5 py-2 text-[10px] font-medium text-zinc-300 transition hover:bg-white/[0.1] hover:text-white",
                onclick: move |_| adding.set(true),
                "Add provider"
            }
        }
        if adding() {
            ProviderDialog {
                existing,
                on_close: move |_| adding.set(false),
                on_save: move |provider: NewChatProvider| {
                    on_add.call(provider);
                    adding.set(false);
                },
            }
        }
    }
}

/// One provider's kind, endpoint and capabilities, for the Providers list.
fn provider_summary(provider: &ChatProvider) -> String {
    let capabilities = provider.capabilities.join(", ");
    if provider.is_external() {
        format!(
            "external · {} · {capabilities}",
            provider.deployment.base_url
        )
    } else {
        format!("managed llama.cpp · {capabilities}")
    }
}

/// The "Add provider" form. It validates before enabling Save; the parent
/// persists the provider to the user overlay and reloads.
#[component]
fn ProviderDialog(
    existing: Vec<String>,
    on_close: EventHandler<()>,
    on_save: EventHandler<NewChatProvider>,
) -> Element {
    let id = use_signal(String::new);
    let mut managed = use_signal(|| false);
    let mut text_capability = use_signal(|| true);
    let mut vision_capability = use_signal(|| false);
    let base_url = use_signal(String::new);
    let model = use_signal(String::new);
    let api_key = use_signal(String::new);
    let executable = use_signal(String::new);
    let model_path = use_signal(String::new);

    let id_value = id();
    let id_valid = !id_value.is_empty()
        && id_value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
        && !existing.contains(&id_value);
    let capabilities_valid = text_capability() || vision_capability();
    let base_url_valid = !base_url().trim().is_empty();
    let model_valid = !model().trim().is_empty();
    let model_path_valid = !managed() || !model_path().trim().is_empty();
    let valid = id_valid && capabilities_valid && base_url_valid && model_valid && model_path_valid;
    let hint = if !id_value.is_empty() && !id_valid {
        "An id uses letters, digits, - or _ and must be new."
    } else if !capabilities_valid {
        "Pick at least one capability."
    } else if !base_url_valid {
        "A base URL is required."
    } else if !model_valid {
        "A model id is required."
    } else if !model_path_valid {
        "A managed provider needs a model file."
    } else {
        ""
    };
    rsx! {
        super::dialog::Dialog { title: "Add provider".to_owned(), on_close,
            div { class: "w-[30rem] max-w-full rounded-2xl bg-zinc-950 p-6 shadow-[0_30px_90px_rgba(0,0,0,.5)] ring-1 ring-white/[0.09]",
                h3 { class: "text-sm font-semibold text-zinc-100", "Add provider" }
                div { class: "mt-4 border-y border-white/[0.06]",
                    DraftField { label: "Provider id".to_owned(), value: id, placeholder: "e.g. lmstudio".to_owned() }
                    SettingRow { title: "Kind".to_owned(), description: String::new(),
                        select {
                            class: "setting-control",
                            aria_label: "Provider kind",
                            oninput: move |event| managed.set(event.value() == "managed"),
                            option { selected: !managed(), value: "external", "OpenAI-compatible endpoint" }
                            option { selected: managed(), value: "managed", "Managed llama.cpp server" }
                        }
                    }
                    SettingRow { title: "Capabilities".to_owned(), description: String::new(),
                        div { class: "flex items-center justify-end gap-4",
                            Toggle { label: ChatCapability::Text.label().to_owned(), enabled: text_capability(), onchange: move |value| text_capability.set(value) }
                            Toggle { label: ChatCapability::Vision.label().to_owned(), enabled: vision_capability(), onchange: move |value| vision_capability.set(value) }
                        }
                    }
                    DraftField { label: "Base URL".to_owned(), value: base_url, placeholder: "http://localhost:1234/v1".to_owned() }
                    DraftField { label: "Model".to_owned(), value: model, placeholder: "the model id the endpoint serves".to_owned() }
                    if managed() {
                        DraftField { label: "Model file".to_owned(), value: model_path, placeholder: "/path/to/model.gguf".to_owned() }
                        DraftField { label: "Executable".to_owned(), value: executable, placeholder: "llama-server (optional)".to_owned(), last: true }
                    } else {
                        DraftField { label: "API key".to_owned(), value: api_key, placeholder: "optional".to_owned(), last: true }
                    }
                }
                div { class: "mt-4 flex items-center justify-between gap-3",
                    p { class: "text-[10px] text-rose-300/80", "{hint}" }
                    div { class: "flex shrink-0 items-center gap-2",
                        button {
                            class: "rounded-lg bg-white/[0.06] px-3.5 py-2 text-[10px] font-medium text-zinc-300 transition hover:bg-white/[0.1] hover:text-white",
                            onclick: move |_| on_close.call(()),
                            "Cancel"
                        }
                        button {
                            class: "rounded-lg bg-violet-500/80 px-3.5 py-2 text-[10px] font-medium text-white transition hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-40",
                            disabled: !valid,
                            onclick: move |_| on_save.call(NewChatProvider {
                                id: id().trim().to_owned(),
                                kind: if managed() { NewChatProviderKind::Managed } else { NewChatProviderKind::External },
                                capabilities: [ChatCapability::Text, ChatCapability::Vision]
                                    .into_iter()
                                    .filter(|capability| match capability {
                                        ChatCapability::Text => text_capability(),
                                        ChatCapability::Vision => vision_capability(),
                                    })
                                    .collect(),
                                base_url: base_url().trim().to_owned(),
                                model: model().trim().to_owned(),
                                api_key: (!api_key().trim().is_empty()).then(|| api_key().trim().to_owned()),
                                executable: managed().then(|| executable().trim().to_owned()).filter(|value| !value.is_empty()),
                                model_path: managed().then(|| model_path().trim().to_owned()).filter(|value| !value.is_empty()),
                            }),
                            "Add provider"
                        }
                    }
                }
            }
        }
    }
}

/// A live text field for the provider form. The dialog owns the state, so a
/// controlled input is enough; `use_text_value` keeps typing intact.
#[component]
fn DraftField(
    label: String,
    value: Signal<String>,
    #[props(default)] placeholder: String,
    #[props(default)] last: bool,
) -> Element {
    let mut value = value;
    let (id, mount) = super::text_input::use_text_value(value(), None);
    rsx! {
        SettingRow { title: label.clone(), description: String::new(), last,
            input {
                class: "setting-control",
                aria_label: label,
                placeholder,
                id,
                onmounted: mount,
                oninput: move |event| value.set(event.value()),
            }
        }
    }
}

/// One capability's editable endpoint: its provider, base URL, model and API
/// key, plus the models the endpoint currently serves. Text and vision share
/// this shape; a managed `llama-server` is not a different kind of card.
#[component]
fn ChatEndpointGroup(
    title: String,
    description: String,
    providers: Vec<SelectOption>,
    provider_id: String,
    base_url: String,
    model: String,
    api_key_set: bool,
    status: HealthStatus,
    detail: String,
    models: Signal<ModelListState>,
    on_select_provider: EventHandler<String>,
    on_commit: EventHandler<(String, ChatEndpointField, String)>,
    on_fetch: EventHandler<()>,
) -> Element {
    let base_id = provider_id.clone();
    let model_id = provider_id.clone();
    let key_id = provider_id.clone();
    let provider_label = format!("{title} provider");
    rsx! {
        div { class: "flex items-start gap-3",
            span { class: "mt-1 grid size-8 shrink-0 place-items-center rounded-xl bg-white/[0.035]", StatusDot { status } }
            div { class: "min-w-0",
                h3 { class: "text-[15px] font-semibold text-zinc-100", "{title}" }
                p { class: "mt-0.5 text-[11px] leading-5 text-zinc-500", "{description}" }
                p { class: "mt-0.5 text-[10px] text-zinc-600", "{detail}" }
            }
        }
        div { class: "mt-4 border-y border-white/[0.06]",
            if providers.len() > 1 {
                SettingRow { title: "Provider".to_owned(), description: String::new(),
                    SearchSelect { label: provider_label, options: providers, value: provider_id.clone(), placeholder: "Select a provider".to_owned(), on_select: on_select_provider }
                }
            }
            EndpointField {
                label: "Base URL".to_owned(),
                description: String::new(),
                value: base_url,
                placeholder: "http://localhost:8080/v1".to_owned(),
                masked: false,
                on_commit: move |value| on_commit.call((base_id.clone(), ChatEndpointField::BaseUrl, value)),
            }
            SettingRow { title: "Model".to_owned(), description: String::new(),
                ModelSelect {
                    models,
                    value: model,
                    on_select: move |value| on_commit.call((model_id.clone(), ChatEndpointField::Model, value)),
                    on_fetch,
                }
            }
            EndpointField {
                label: "API key".to_owned(),
                description: String::new(),
                value: String::new(),
                placeholder: if api_key_set { "•••••• (set)".to_owned() } else { "none".to_owned() },
                masked: true,
                last: true,
                on_commit: move |value| on_commit.call((key_id.clone(), ChatEndpointField::ApiKey, value)),
            }
        }
        div { class: "h-8" }
    }
}

/// One editable endpoint value. Native typing is preserved; the value commits
/// on Enter or blur, and app-driven changes sync back into the field.
#[component]
fn EndpointField(
    label: String,
    description: String,
    value: String,
    placeholder: String,
    masked: bool,
    #[props(default)] last: bool,
    on_commit: EventHandler<String>,
) -> Element {
    let (id, mount) = super::text_input::use_text_value(value, None);
    rsx! {
        SettingRow { title: label.clone(), description, last,
            input {
                class: "setting-control",
                r#type: if masked { "password" } else { "text" },
                placeholder,
                aria_label: label,
                id,
                onmounted: mount,
                onchange: move |event| on_commit.call(event.value()),
            }
        }
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
                ModelListState::Loading => rsx! {
                    span { class: "text-[10px] text-zinc-600", "Loading models…" }
                },
                ModelListState::Failed(error) => rsx! {
                    span { class: "max-w-40 truncate font-mono text-[9px] text-rose-300/70", title: "{error.details}", "{error.code.code()}" }
                    ErrorDetailsButton { error: error.clone() }
                },
                // Only the ids the endpoint actually serves are ever offered:
                // the saved model shows as selected only when it is one of
                // them, and reads as unselected otherwise.
                ModelListState::Ready(ids) if ids.is_empty() => rsx! {
                    span { class: "text-[10px] text-zinc-600", "The endpoint lists no models." }
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
                    span { class: "text-[10px] text-zinc-600", "Managed server is not running — start it in Services." }
                },
                ModelListState::Empty => rsx! {
                    span { class: "text-[10px] text-zinc-600", "No provider serves this capability." }
                },
            }
            button {
                class: "grid size-8 shrink-0 place-items-center rounded-lg bg-white/[0.05] text-zinc-500 transition hover:text-white",
                aria_label: "Refetch models",
                title: "Refetch models",
                onclick: move |_| on_fetch.call(()),
                Icon { name: IconName::Refresh, class: "size-3" }
            }
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

/// The image model's identity and output: the served profile, the sampling
/// steps, the per-kind canvas, and the adapters bound to it. Running the engine
/// — its files, downloads, residency and start/stop — lives on the Services
/// page.
#[component]
fn ImageModelGroup(
    config: AppConfig,
    mut defaults: Signal<GenerationDefaults>,
    services: Vec<ServiceHealth>,
) -> Element {
    let (status, detail) = service_of(&services, ServiceId::ImageGeneration);
    let sizes = defaults.read().sizes;
    let steps = defaults.read().steps;
    let cfg_scale = defaults.read().cfg_scale;
    let mut steps_error = use_signal(|| None::<String>);
    let mut steps_draft = use_signal(|| steps.to_string());
    let (steps_id, steps_mount) = super::text_input::use_text_value(steps_draft(), None);
    // The lora dir shown here and the bindings share the same defaults.
    let lora_dir = config.image.lora_dir.clone();
    // Discovered adapters, reloadable: symlinked or newly dropped files
    // appear after pressing the refresh icon next to the directory.
    let mut discovered = use_signal(|| discover_loras(&lora_dir));
    let scan_dir = lora_dir.clone();
    let on_rescan = move |_event: MouseEvent| discovered.set(discover_loras(&scan_dir));
    rsx! {
        div { class: "flex items-start gap-3",
            span { class: "mt-1 grid size-8 shrink-0 place-items-center rounded-xl bg-white/[0.035]", StatusDot { status } }
            div { class: "min-w-0 flex-1",
                h3 { class: "text-[15px] font-semibold text-zinc-100", "Image generation (t2i + edit)" }
                p { class: "mt-0.5 text-[11px] leading-5 text-zinc-500", "Qwen-Image-2.1 — the profile, quality and adapters." }
                p { class: "mt-0.5 text-[10px] text-zinc-600", "{detail}" }
            }
        }
        div { class: "mt-4 border-y border-white/[0.06]",
            SettingRow { title: "Model", description: String::new(),
                input { class: "setting-control", value: "{config.image.architecture}", readonly: true }
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
                        aria_label: "Steps per image",
                        aria_invalid: steps_error().is_some(),
                        id: steps_id,
                        onmounted: steps_mount,
                        oninput: move |event| steps_draft.set(event.value()),
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
                SizeField { label: "512×768", value: sizes.character, defaults, kind: "character" }
            }
            SettingRow { title: "Environment size", description: String::new(),
                SizeField { label: "1024×576", value: sizes.environment, defaults, kind: "environment" }
            }
            SettingRow { title: "Object size", description: String::new(),
                SizeField { label: "768×768", value: sizes.object, defaults, kind: "object" }
            }
            SettingRow { title: "Scene size", description: String::new(),
                SizeField { label: "1024×576", value: sizes.scene, defaults, kind: "scene" }
            }
            SettingRow { title: "Beat size", description: String::new(),
                SizeField { label: "1024×576", value: sizes.beat, defaults, kind: "beat" }
            }
            SettingRow { title: "LoRA directory", description: "Adapters are discovered here (subfolders and symlinks included).", last: true,
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
        }
        LoRASettings { defaults, discovered }
    }
}

/// The resident image engine: its status and actions, the files it runs, and
/// the launch-time choices. Its model, quality and adapters are on the Models
/// page.
#[component]
fn ImageServiceCard(
    config: AppConfig,
    backend: Signal<CreativeBackend>,
    services: Vec<ServiceHealth>,
    busy: Signal<Option<ServiceId>>,
    on_operate: EventHandler<(ServiceId, ServiceAction)>,
) -> Element {
    let service = services
        .iter()
        .find(|service| service.id == ServiceId::ImageGeneration)
        .cloned();
    let (status, detail, recovery) = match service {
        Some(service) => (service.status, service.detail, service.recovery),
        None => (HealthStatus::Idle, "Not checked yet".to_owned(), None),
    };
    let is_busy = busy().is_some();
    let busy_here = busy() == Some(ServiceId::ImageGeneration);
    let actions = actions_for(ServiceId::ImageGeneration, status, recovery);
    let needs_setup = recovery == Some(ServiceRecovery::ConfigureImageFiles);
    let on_setup = move |_| {
        document::eval(
            "document.getElementById('image-models')?.scrollIntoView({behavior: 'smooth', block: 'start'})",
        );
    };
    let mut prewarm = use_context::<Signal<bool>>();
    let mut residency = use_context::<Signal<String>>();
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let refresh = use_context::<RefreshHealth>().0;
    let persist_config = config.clone();
    let on_prewarm = move |enabled: bool| {
        if let Err(error) = persist_config.persist_prewarm(enabled) {
            error_report.set(Some(error.report().log()));
            return;
        }
        prewarm.set(enabled);
    };
    // Changing residency is a launch-flag change: persist it, and restart the
    // shared server so the new mode takes effect now. The shared lifecycle
    // owner performs the restart; the app does not own the process.
    let residency_config = config.clone();
    let mut residency_backend = backend;
    let on_residency = EventHandler::new(move |value: String| {
        if let Err(error) = residency_config.persist_image_residency(&value) {
            error_report.set(Some(error.report().log()));
            return;
        }
        residency.set(value.clone());
        spawn(async move {
            let before = residency_backend.read().runtime();
            let was_running = before.is_running().await;
            if was_running {
                if let Err(error) = before.stop_generation_runtime().await {
                    error_report.set(Some(error.report().log()));
                    refresh.call(());
                    return;
                }
            }
            if !residency_backend.write().set_image_residency(&value) {
                tracing::error!("unknown image residency {value:?}");
            }
            if was_running {
                // Re-read: the backend now serves the new residency, and the
                // restart must start the server in the new mode.
                let runtime = residency_backend.read().runtime();
                if let Err(error) = runtime.ensure_ready().await {
                    error_report.set(Some(error.report().log()));
                }
            }
            refresh.call(());
        });
    });
    let executable = config.image.executable.display().to_string();
    rsx! {
        div { class: "flex items-start gap-3",
            span { class: "mt-1 grid size-8 shrink-0 place-items-center rounded-xl bg-white/[0.035]", StatusDot { status } }
            div { class: "min-w-0 flex-1",
                h3 { class: "text-[15px] font-semibold text-zinc-100", "Image engine" }
                p { class: "mt-0.5 text-[11px] leading-5 text-zinc-500", "The native stable-diffusion.cpp server the shared owner starts, reused by Studio, Atelier and Clipforge." }
                p { class: "mt-0.5 text-[10px] text-zinc-600", "{detail}" }
            }
            div { class: "flex shrink-0 items-center gap-2",
                if busy_here {
                    span { class: "size-3 animate-spin rounded-full border border-zinc-600 border-t-white" }
                } else if needs_setup {
                    button {
                        class: "rounded-md bg-white/[0.06] px-2.5 py-1.5 text-[10px] font-medium text-zinc-300 transition hover:bg-white/[0.1] hover:text-white",
                        onclick: on_setup,
                        "Set up image engine"
                    }
                } else {
                    for action in actions {
                        ServiceActionButton {
                            label: action.label().to_owned(),
                            disabled: is_busy,
                            onclick: move |_| on_operate.call((ServiceId::ImageGeneration, action)),
                        }
                    }
                }
            }
        }
        div { class: "mt-4 border-y border-white/[0.06]",
            SettingRow { title: "Executable", description: String::new(),
                input { class: "setting-control", value: "{executable}", readonly: true }
            }
            SettingRow { title: "Runtime endpoint", description: String::new(),
                input { class: "setting-control", value: "http://{config.image.host}:{config.image.port}", readonly: true }
            }
            SettingRow { title: "Prewarm on project open", description: "Start the resident server as soon as a project opens, so the first generation is not a multi-minute weight load. Leave it off while another local model shares the card.",
                Toggle { label: "Prewarm on project open", enabled: prewarm(), onchange: on_prewarm }
            }
            SettingRow { title: "Residency", description: "How the weights are held on the GPU. `resident` is fastest (measured peak ≈23 GiB); `lowvram` pages them from system RAM (`--offload-to-cpu`) for ≈9 GiB at t2i and ≈16 GiB at an edit, at +7% / +0.5% sampling time. A running server restarts into the new mode.", last: true,
                select {
                    aria_label: "Image residency",
                    class: "setting-control",
                    oninput: move |event| on_residency.call(event.value()),
                    option { selected: residency() == "resident", value: "resident", "Resident (fastest)" }
                    option { selected: residency() == "lowvram", value: "lowvram", "Low VRAM (offload)" }
                }
            }
        }
        div { class: "mt-5 flex items-start gap-3 rounded-xl bg-amber-400/[0.035] px-4 py-3 ring-1 ring-inset ring-amber-300/[0.08]",
            Icon { name: IconName::Sparkles, class: "mt-0.5 size-3.5 shrink-0 text-amber-200/50" }
            p { class: "text-[10px] leading-5 text-zinc-600", "Qwen-Image-2.1 peaks around 23.4 GiB (diffusion + text encoder + VAE + a reference encode), so leave headroom on a 24 GB card." }
        }
    }
}

/// Writes the live defaults back to the user config (line-preserving).
fn persist_defaults(mut defaults: Signal<GenerationDefaults>) {
    let mut baseline = consume_context::<PersistedDefaults>().0;
    let mut error_report = consume_context::<Signal<Option<ReportedError>>>();
    let snapshot = defaults.read().clone();
    let path = match storyfella_studio_core::paths::config_path() {
        Some(path) => path,
        None => {
            defaults.set(baseline());
            error_report.set(Some(ReportedError::plain(
                storyfella_studio_core::errors::ErrorCode::ConfigWrite,
                "Could not resolve the app config path. Settings were not changed.",
            )));
            return;
        }
    };
    if let Err(error) = AppConfig::persist_generation_defaults_to(path, &snapshot) {
        defaults.set(baseline());
        error_report.set(Some(error.report().log()));
    } else {
        baseline.set(snapshot);
    }
}

/// The `WxH` size input for one artifact kind. Commits on Enter/blur; an
/// invalid value shows a hint and keeps the last good default.
#[component]
fn SizeField(
    label: String,
    value: Option<(u32, u32)>,
    mut defaults: Signal<GenerationDefaults>,
    kind: &'static str,
) -> Element {
    let current = value
        .map(|(width, height)| format!("{width}x{height}"))
        .unwrap_or_default();
    let kind_label = kind.to_owned();
    let mut error = use_signal(|| None::<String>);
    let mut draft = use_signal(|| current);
    let (draft_id, draft_mount) = super::text_input::use_text_value(draft(), None);
    rsx! {
        div { class: "flex items-center gap-3",
            if let Some(message) = error() {
                p { class: "truncate text-[9px] text-rose-300/80", "{message}" }
            }
            input {
                aria_label: "{kind} image size",
                aria_invalid: error().is_some(),
                class: "setting-control w-24",
                placeholder: "{label}",
                id: draft_id,
                onmounted: draft_mount,
                oninput: move |event| draft.set(event.value()),
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
                                storyfella_studio_core::registry::pipeline::is_valid_size(width, height)
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

/// A byte count in binary units (`1.5 GiB`).
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Where one artifact is loaded from, for the model-files list.
fn artifact_detail(status: &ModelArtifactStatus) -> String {
    match &status.local_override {
        Some(path) => format!("Local override · {}", path.display()),
        None => status.path.display().to_string(),
    }
}

/// The image profile's weight files and the in-app download that fills the
/// gaps, so a first run does not need a terminal.
#[component]
fn ModelFiles(config: AppConfig) -> Element {
    let mut repair_error = use_signal(|| None::<String>);
    let mut pending_directory = use_signal(|| None::<std::path::PathBuf>);
    let change_config = config.clone();
    let change_location = move |_| {
        let Some(directory) = rfd::FileDialog::new()
            .set_title("Choose model folder")
            .set_directory(&change_config.image.model_dir)
            .pick_folder()
        else {
            return;
        };
        match change_config.persist_model_dir(&directory) {
            Ok(()) => {
                repair_error.set(None);
                pending_directory.set(Some(directory));
            }
            Err(error) => repair_error.set(Some(error.to_string())),
        }
    };
    let model_directory = config.image.model_dir.clone();
    let configuration_path = config
        .ai_config_path
        .clone()
        .or_else(storyfella_studio_core::paths::clipforge_config_path)
        .or_else(storyfella_studio_core::paths::config_path)
        .unwrap_or_default();
    let artifacts = image_artifacts(&config);
    let mut state = use_signal(|| ModelPrepState::Idle);
    let current = state();
    let running = matches!(current, ModelPrepState::Running { .. });
    let missing = artifacts
        .iter()
        .filter(|status| status.needs_download())
        .count();
    let blocked_overrides = artifacts
        .iter()
        .filter(|status| status.local_override.is_some() && !status.is_present())
        .count();
    let executable_ready = config.image.executable.is_file();
    let bytes_to_download: u64 = artifacts
        .iter()
        .filter(|status| status.needs_download())
        .map(|status| status.size_bytes)
        .sum();

    let prepare_config = config.clone();
    let on_prepare = move |_event: MouseEvent| {
        if matches!(state(), ModelPrepState::Running { .. }) {
            return;
        }
        let config = prepare_config.clone();
        state.set(ModelPrepState::Running {
            filename: String::new(),
            downloaded_bytes: 0,
            total_bytes: 0,
        });
        spawn(async move {
            let mut progress_signal = state;
            let result = provision_image_model(&config, move |progress| {
                progress_signal.set(ModelPrepState::Running {
                    filename: progress.filename,
                    downloaded_bytes: progress.downloaded_bytes,
                    total_bytes: progress.total_bytes,
                });
            })
            .await;
            match result {
                Ok(()) => state.set(ModelPrepState::Done),
                Err(error) => state.set(ModelPrepState::Failed(error.report().log())),
            }
        });
    };

    rsx! {
        SectionHeading {
            eyebrow: "Weights",
            title: "Model files",
            description: "Qwen-Image-2.1 loads three weight files. Provision them once here — they are several gigabytes and verified by size and SHA-256."
        }
        div { class: "border-y border-white/[0.06]",
            for (index, status) in artifacts.iter().enumerate() {
                SettingRow {
                    title: status.filename.clone(),
                    description: artifact_detail(status),
                    last: index + 1 == artifacts.len(),
                    div { class: "flex items-center gap-3",
                        span { class: "text-[10px] text-zinc-600", "{human_bytes(status.size_bytes)}" }
                        if status.is_present() {
                            span { class: "rounded-md bg-emerald-400/10 px-2 py-1 text-[9px] text-emerald-300", "ready" }
                        } else {
                            span { class: "rounded-md bg-amber-400/10 px-2 py-1 text-[9px] text-amber-300", "missing" }
                        }
                    }
                }
            }
        }
        div { class: "mt-5 flex items-start justify-between gap-4",
            div { class: "min-w-0 flex-1",
                match current {
                    ModelPrepState::Running { filename, downloaded_bytes, total_bytes } => {
                        let percent = downloaded_bytes
                            .saturating_mul(100)
                            .checked_div(total_bytes)
                            .unwrap_or(0)
                            .min(100);
                        rsx! {
                            div { class: "mb-2 h-1 w-full overflow-hidden rounded-full bg-white/[0.06]",
                                div { class: "h-full rounded-full bg-violet-400 transition-all", style: "width: {percent}%" }
                            }
                            p { class: "truncate text-[10px] text-zinc-500",
                                if filename.is_empty() {
                                    "Starting…"
                                } else {
                                    "{filename} · {human_bytes(downloaded_bytes)} / {human_bytes(total_bytes)}"
                                }
                            }
                        }
                    }
                    ModelPrepState::Done => rsx! {
                        p { class: "text-[10px] text-emerald-300/80", "Download finished. Review all file states and check runtime status before generating." }
                    },
                    ModelPrepState::Failed(error) => rsx! {
                        div { class: "flex items-start gap-3",
                            div { class: "min-w-0 flex-1",
                                p { class: "font-mono text-[9px] text-rose-300/70", "{error.code.code()}" }
                                p { class: "mt-0.5 text-[10px] leading-4 text-rose-200/80", "{error.code.message()}" }
                            }
                            ErrorDetailsButton { error: error.clone() }
                        }
                    },
                    ModelPrepState::Idle => rsx! {
                        p { class: "text-[10px] text-zinc-600",
                            if missing == 0 {
                                "No downloadable files are missing."
                            } else {
                                "{missing} of {artifacts.len()} files still to download."
                            }
                        }
                    },
                }
            }
            button {
                class: "shrink-0 rounded-lg bg-white/[0.06] px-3.5 py-2 text-[10px] font-medium text-zinc-300 transition hover:bg-white/[0.1] hover:text-white disabled:cursor-not-allowed disabled:opacity-40",
                disabled: running || missing == 0 || pending_directory().is_some(),
                onclick: on_prepare,
                if running {
                    "Preparing…"
                } else if missing == 0 {
                    "No downloads needed"
                } else {
                    "Prepare model"
                }
            }
        }
        p { class: "mt-3 text-xs text-zinc-300", "Download: {human_bytes(bytes_to_download)} · Destination: {config.image.model_dir.display()}" }
        div { class: "mt-3 flex flex-wrap items-center gap-3",
            button { disabled: running, title: if running { "Wait for this download before changing its destination" } else { "Choose an existing model tree or a new download destination" }, class: "rounded bg-white/10 px-3 py-2", onclick: change_location, "Change location…" }
            button { class: "rounded bg-white/10 px-3 py-2", onclick: move |_| {
                let directory = pending_directory().unwrap_or_else(|| model_directory.clone());
                let result = std::fs::create_dir_all(&directory).map_err(|error| error.to_string())
                    .and_then(|_| super::components::open_path(&directory));
                repair_error.set(result.err());
            }, "Open folder" }
        }
        if let Some(directory) = pending_directory() {
            p { role: "status", class: "mt-3 text-xs text-violet-200", "Saved location: {directory.display()}. Restart Studio to use it. Existing files stay in place; choosing their folder reuses downloads. Preparation is paused until restart." }
        }
        if blocked_overrides > 0 {
            p { role: "alert", class: "mt-3 text-xs text-amber-300", "{blocked_overrides} configured local weight files are missing. Repair their paths in clipforge.toml; downloading cannot replace a local override." }
        }
        if !executable_ready {
            p { role: "alert", class: "mt-3 text-xs text-amber-300", "The generation executable is missing: {config.image.executable.display()}. Configure an installed sd-server executable in clipforge.toml." }
        }
        if blocked_overrides > 0 || !executable_ready {
            button { class: "mt-3 rounded bg-white/10 px-3 py-2", onclick: move |_| repair_error.set(super::components::open_path(&configuration_path).err()), "Open configuration" }
        }
        if let Some(message) = repair_error() { p { role: "alert", "{message}" } }
        p { class: "mt-2 text-xs text-zinc-400", "Downloads continue across screens. Quitting stops this app’s download; verified files are reused when you prepare again." }
    }
}

/// Config-level adapter bindings: discovered files from `lora_dir` bound
/// to per-kind defaults at one multiplier. The `discovered` signal is owned
/// by [`ImageServiceCard`] — the refresh action lives next to the
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
                    "No .safetensors adapters found in the LoRA directory. Drop adapter files there (or configure lora_dir in clipforge.toml) and they will appear here."
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
            PathRow { title: "Models", description: "The image engine's weight tree.", path: config.image.model_dir.display().to_string() }
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

#[component]
fn Toggle(label: String, enabled: bool, onchange: EventHandler<bool>) -> Element {
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
            aria_label: label,
            aria_checked: "{enabled}",
            onclick: move |_| onchange.call(!enabled),
            span { class: "absolute left-0 top-0.5 size-4 rounded-full bg-white shadow-sm transition {knob}" }
        }
    }
}
