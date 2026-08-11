use std::path::{Path, PathBuf};

use dioxus::prelude::*;

use crate::{
    app::AppConfig,
    commands,
    errors::{CodedError, ReportedError},
    persistence::ProjectDb,
    state::AppState,
};

use super::icons::{Icon, IconName};

/// One row of the Recent list: a `.svs-project.db` file plus the headline
/// counts read from it.
#[derive(Clone, Debug, PartialEq)]
struct RecentProject {
    path: PathBuf,
    name: String,
    artifacts: usize,
}

#[component]
pub fn Projects(
    config: AppConfig,
    app_state: Signal<AppState>,
    on_open: EventHandler<MouseEvent>,
) -> Element {
    let mut name = use_signal(String::new);
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let recent = use_signal(|| scan_projects(&config.project_dir));

    let recent_list = recent.read().clone();
    let recent_items = recent_list.iter().cloned().map(|item| {
        let mut app_state = app_state;
        rsx! {
            button {
                class: "group flex w-full items-center gap-5 border-y border-white/[0.06] py-5 text-left",
                onclick: move |event| {
                    let path = item.path.clone();
                    match ProjectDb::open(&path).and_then(|db| db.load()) {
                        Ok(stored) => {
                            app_state.write().open_project(stored, path);
                            on_open.call(event);
                        }
                        Err(error) => {
                            error_report.set(Some(error.report().log()));
                        }
                    }
                },
                div { class: "grid aspect-video w-32 shrink-0 place-items-center rounded-xl bg-[linear-gradient(145deg,rgba(124,58,237,.16),rgba(24,24,27,.7))] text-violet-300 ring-1 ring-inset ring-white/[0.07]",
                    Icon { name: IconName::Sparkles, class: "size-5" }
                }
                div { class: "min-w-0 flex-1",
                    h3 { class: "truncate text-sm font-medium text-zinc-200 group-hover:text-white", "{item.name}" }
                    p { class: "mt-1 text-[11px] text-zinc-600", "{item.artifacts} artifacts" }
                    p { class: "mt-0.5 truncate text-[10px] text-zinc-700", "{item.path.display()}" }
                }
                Icon { name: IconName::Arrow, class: "size-4 text-zinc-700 transition group-hover:translate-x-1 group-hover:text-zinc-300" }
            }
        }
    });

    let create = {
        let project_dir = config.project_dir.clone();
        let mut name = name;
        move |event: MouseEvent| {
            let story_name = name();
            if story_name.trim().is_empty() {
                return;
            }
            match commands::create_project(&project_dir, &story_name, app_state) {
                Ok(_) => {
                    name.set(String::new());
                    on_open.call(event);
                }
                Err(report) => error_report.set(Some(report.log())),
            }
        }
    };

    // A populated project to look at the layouts with. Creates the project
    // the normal way, then fills the registry and story before the first
    // save so it lands on disk like any other.
    let create_sample = {
        let project_dir = config.project_dir.clone();
        let asset_dir = config.asset_dir.clone();
        move |event: MouseEvent| match commands::create_project(
            &project_dir,
            "Keeper of the Low Light",
            app_state,
        ) {
            Ok(_) => {
                let (registry, story) = crate::app::sample::project(&asset_dir);
                let mut state = app_state.write();
                state.registry = registry;
                state.story_source = story;
                state.has_unsaved_changes = true;
                drop(state);
                on_open.call(event);
            }
            Err(report) => error_report.set(Some(report.log())),
        }
    };

    let open_dialog = {
        move |event: MouseEvent| {
            let Some(picked) = rfd::FileDialog::new()
                .set_title("Open project")
                .add_filter("Project", &["db"])
                .pick_file()
            else {
                return;
            };
            match commands::open_project(&picked, app_state) {
                Ok(()) => {
                    on_open.call(event);
                }
                Err(report) => error_report.set(Some(report.log())),
            }
        }
    };

    rsx! {
        section { class: "h-full overflow-y-auto px-10 py-10",
            div { class: "mx-auto max-w-5xl",
                header { class: "mb-12 flex items-end justify-between",
                    div {
                        p { class: "mb-2 text-[10px] font-semibold uppercase tracking-[0.18em] text-violet-400", "Workspace" }
                        h1 { class: "text-3xl font-semibold tracking-[-0.04em] text-zinc-100", "Your stories" }
                        p { class: "mt-2 text-sm text-zinc-600", "Create, continue, and shape visual sequences." }
                    }
                    button {
                        class: "flex h-9 items-center gap-2 rounded-xl bg-white/[0.05] px-4 text-xs font-medium text-zinc-300 transition hover:bg-white/[0.09] hover:text-white",
                        onclick: open_dialog,
                        Icon { name: IconName::Arrow, class: "size-3.5 rotate-180" }
                        "Open…"
                    }
                }

                div { class: "grid grid-cols-[minmax(0,1.4fr)_minmax(260px,.8fr)] gap-10",
                    div {
                        h2 { class: "mb-4 text-[11px] font-medium text-zinc-500", "Recent" }
                        if recent.read().is_empty() {
                            div { class: "grid min-h-44 place-items-center border-y border-white/[0.05]",
                                div { class: "text-center",
                                    Icon { name: IconName::Sparkles, class: "mx-auto mb-3 size-6 text-zinc-700" }
                                    p { class: "text-xs text-zinc-600", "No projects yet — start a new story or import one." }
                                }
                            }
                        } else {
                            {recent_items}
                        }
                    }

                    aside { class: "border-l border-white/[0.06] pl-10",
                        div { class: "mb-5 grid size-10 place-items-center rounded-xl bg-white/[0.045] text-zinc-400", Icon { name: IconName::Add, class: "size-4" } }
                        h2 { class: "text-sm font-medium text-zinc-200", "Start a new story" }
                        p { class: "mt-2 text-xs leading-5 text-zinc-600", "Begin with an idea. Structure and visuals can evolve as you work." }
                        input {
                            class: "mt-6 h-10 w-full border-b border-white/[0.1] bg-transparent text-sm text-zinc-100 outline-none transition placeholder:text-zinc-700 focus:border-violet-400/60",
                            placeholder: "Story name",
                            value: "{name}",
                            oninput: move |event| name.set(event.value()),
                        }
                        button {
                            class: "mt-5 flex h-10 w-full items-center justify-center gap-2 rounded-xl bg-zinc-100 text-xs font-semibold text-zinc-950 transition hover:bg-white disabled:opacity-35",
                            disabled: name.read().trim().is_empty(),
                            onclick: create,
                            "Create story"
                            Icon { name: IconName::Arrow, class: "size-3.5" }
                        }

                        div { class: "mt-8 border-t border-white/[0.06] pt-5",
                            p { class: "text-xs font-medium text-zinc-400", "Or open the sample" }
                            p { class: "mt-1.5 text-[11px] leading-5 text-zinc-600",
                                "Twelve artifacts with variants, and a story that names three that do not exist yet — enough to see how the screens behave when they are full."
                            }
                            button {
                                class: "mt-4 flex h-9 w-full items-center justify-center gap-2 rounded-xl bg-violet-400/15 text-[11px] font-semibold text-violet-200 transition hover:bg-violet-400/25",
                                onclick: create_sample,
                                "Create sample project"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Opens every `.svs-project.db` in the projects directory to read its name
/// and headline counts, newest first.
fn scan_projects(project_dir: &Path) -> Vec<RecentProject> {
    let mut projects: Vec<RecentProject> = match std::fs::read_dir(project_dir) {
        Ok(entries) => entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("db") {
                    return None;
                }
                let stored = ProjectDb::open(&path).ok()?.load().ok()?;
                Some(RecentProject {
                    path,
                    name: stored.name,
                    artifacts: stored.registry.artifacts.len(),
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    projects.sort_by(|a, b| b.name.cmp(&a.name));
    projects
}
