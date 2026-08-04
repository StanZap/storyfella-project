use dioxus::prelude::*;

use crate::{app::AppConfig, runtime::KreaQuantization};

use super::{
    components::{SectionHeading, SettingRow, StatusDot},
    icons::{Icon, IconName},
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum SettingsSection {
    General,
    Intelligence,
    Generation,
    Storage,
}

#[component]
pub fn Settings(config: AppConfig) -> Element {
    let mut section = use_signal(|| SettingsSection::General);
    let profile = use_signal(|| config.generation.profile);

    rsx! {
        section { class: "grid h-full min-h-0 grid-cols-[210px_minmax(0,1fr)]",
            aside { class: "border-r border-white/[0.055] px-5 py-8",
                p { class: "mb-5 px-3 text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-600", "Settings" }
                nav { class: "space-y-1",
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
                        SettingsSection::General => rsx! { GeneralSettings {} },
                        SettingsSection::Intelligence => rsx! { IntelligenceSettings { config: config.clone() } },
                        SettingsSection::Generation => rsx! { GenerationSettings { config: config.clone(), profile } },
                        SettingsSection::Storage => rsx! { StorageSettings { config: config.clone() } },
                    }
                }
            }
        }
    }
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
                select { class: "setting-control", option { "After every change" } option { "Every minute" } }
            }
        }
    }
}

#[component]
fn IntelligenceSettings(config: AppConfig) -> Element {
    rsx! {
        SectionHeading {
            eyebrow: "Planner & vision",
            title: "Intelligence",
            description: "LM Studio handles planning and visual understanding. It remains an external service and is never bundled with the app."
        }
        div { class: "mb-8 flex items-center justify-between border-y border-white/[0.06] py-4",
            div { class: "flex items-center gap-3",
                span { class: "grid size-9 place-items-center rounded-xl bg-white/[0.035]", StatusDot { online: false } }
                div { p { class: "text-[12px] font-medium text-zinc-200", "LM Studio" } p { class: "mt-0.5 text-[10px] text-zinc-600", "Configured · connection checked when used" } }
            }
            button { class: "rounded-lg bg-white/[0.05] px-3 py-2 text-[10px] font-medium text-zinc-400 transition hover:text-white", "Test connection" }
        }
        div { class: "border-y border-white/[0.06]",
            SettingRow { title: "Base URL", description: "OpenAI-compatible API endpoint.",
                input { class: "setting-control", value: "{config.lm_studio.base_url}", readonly: true }
            }
            SettingRow { title: "Planner model", description: "Model identifier currently configured in LM Studio.",
                input { class: "setting-control", value: "{config.lm_studio.model}", readonly: true }
            }
            SettingRow { title: "Request timeout", description: "Maximum wait for planning and visual analysis.", last: true,
                div { class: "flex items-center gap-3", input { class: "setting-control", value: "{config.lm_studio.timeout.as_secs()}", readonly: true } span { class: "text-[10px] text-zinc-700", "seconds" } }
            }
        }
        p { class: "mt-4 text-[10px] leading-5 text-zinc-700", "Configuration editing and model discovery will be enabled after the runtime lifecycle is connected. Current values come from config/app.toml." }
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
