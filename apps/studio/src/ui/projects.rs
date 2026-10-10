use std::path::{Path, PathBuf};

use dioxus::prelude::*;

use storyfella_studio_core::config::AppConfig;
use storyfella_studio_core::errors::ReportedError;
use storyfella_studio_core::persistence::ProjectDb;

use crate::{commands, state::AppState};

use super::{
    components::IconButton,
    icons::{Icon, IconName},
};

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
    let (name_id, name_mount) = super::text_input::use_text_value(name(), None);
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let mut recent = use_signal(|| scan_projects(&config.project_dir));
    let mut scan_errors = use_signal(Vec::<String>::new);
    let opened = app_state.read().project_path.clone();
    let recent_dir = config.project_dir.clone();
    use_effect(use_reactive(&opened, move |path| {
        if let Some(path) = path {
            if let Err(error) = remember_project(&recent_dir, &path) {
                scan_errors.set(vec![error]);
            }
        }
        recent.set(scan_projects(&recent_dir));
    }));
    let refresh_dir = config.project_dir.clone();

    let (recent_list, load_errors) = recent.read().clone();
    let recent_items = recent_list.iter().cloned().map(|item| {
        rsx! {
            button {
                class: "group flex w-full items-center gap-5 border-y border-white/[0.06] py-5 text-left",
                title: item.path.display().to_string(),
                onclick: move |event| {
                    let path = item.path.clone();
                    match commands::open_project(&path, app_state) {
                        Ok(()) => {
                            on_open.call(event);
                        }
                        Err(error) => {
                            error_report.set(Some(error.log()));
                        }
                    }
                },
                div { class: "grid aspect-video w-32 shrink-0 place-items-center rounded-xl bg-[linear-gradient(145deg,rgba(124,58,237,.16),rgba(24,24,27,.7))] text-violet-300 ring-1 ring-inset ring-white/[0.07]",
                    Icon { name: IconName::Sparkles, class: "size-5" }
                }
                div { class: "min-w-0 flex-1",
                    h3 { class: "truncate text-sm font-medium text-zinc-200 group-hover:text-white", "{item.name}" }
                    p { class: "mt-1 text-[11px] text-zinc-600", "{item.artifacts} artifacts" }
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
                state.set_story_source(story);
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
                .add_filter("Storyfella project or story", &["db", "sf"])
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
                        h1 { class: "text-3xl font-semibold tracking-[-0.04em] text-zinc-100", "Your stories" }
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
                        div { class: "mb-4 flex items-center justify-between",
                            h2 { class: "text-sm font-medium text-zinc-300", "Recent" }
                            IconButton { label: "Refresh recent projects", icon: IconName::Refresh, onclick: move |_| { scan_errors.set(Vec::new()); recent.set(scan_projects(&refresh_dir)); } }
                        }
                        for message in load_errors.iter().chain(scan_errors.read().iter()) {
                            p { role: "alert", class: "mb-3 break-words text-xs text-amber-300", "{message}" }
                        }
                        if recent.read().0.is_empty() {
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
                        label { r#for: name_id.clone(), class: "mt-5 block text-xs text-zinc-400", "Story name" }
                        input {
                            aria_label: "Story name",
                            class: "mt-2 h-10 w-full border-b border-white/[0.1] bg-transparent text-sm text-zinc-100 outline-none transition placeholder:text-zinc-700 focus:border-violet-400/60",
                            placeholder: "Story name",
                            id: name_id,
                            onmounted: name_mount,
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
                            button {
                                class: "mt-4 flex h-9 w-full items-center justify-center gap-2 rounded-xl bg-violet-400/15 text-[11px] font-semibold text-violet-200 transition hover:bg-violet-400/25",
                                onclick: create_sample,
                                title: "Create a separate example story with characters, artwork and variants",
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
fn recent_paths(project_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let index = project_dir.join("recent-projects.json");
    if !index.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&index)
        .map_err(|error| format!("Cannot read {}: {error}", index.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Cannot read recent projects index: {error}"))
}

fn remember_project(project_dir: &Path, path: &Path) -> Result<(), String> {
    let mut paths = recent_paths(project_dir)?;
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    paths.retain(|old| old != &path);
    paths.insert(0, path);
    paths.truncate(30);
    std::fs::create_dir_all(project_dir).map_err(|error| error.to_string())?;
    let index = project_dir.join("recent-projects.json");
    let temporary = project_dir.join("recent-projects.json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&paths).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    std::fs::rename(temporary, index)
        .map_err(|error| format!("Could not remember the opened project: {error}"))
}

fn scan_projects(project_dir: &Path) -> (Vec<RecentProject>, Vec<String>) {
    let mut errors = Vec::new();
    let mut paths = recent_paths(project_dir).unwrap_or_else(|error| {
        errors.push(error);
        Vec::new()
    });
    match std::fs::read_dir(project_dir) {
        Ok(entries) => {
            let mut discovered = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) if entry.path().extension().is_some_and(|ext| ext == "db") => {
                        discovered.push(entry.path())
                    }
                    Ok(_) => {}
                    Err(error) => errors.push(format!("Could not read a project entry: {error}")),
                }
            }
            discovered.sort_by_key(|path| {
                std::cmp::Reverse(path.metadata().and_then(|meta| meta.modified()).ok())
            });
            for path in discovered {
                let path = path.canonicalize().unwrap_or(path);
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => errors.push(format!("Cannot read {}: {error}", project_dir.display())),
    }
    let projects = paths
        .into_iter()
        .filter_map(|path| {
            if !path.is_file() {
                errors.push(format!("Recent project is unavailable: {}", path.display()));
                return None;
            }
            match ProjectDb::open(&path).and_then(|db| db.load()) {
                Ok(stored) => Some(RecentProject {
                    path,
                    name: stored.name,
                    artifacts: stored.registry.artifacts.len(),
                }),
                Err(error) => {
                    errors.push(format!("Cannot load {}: {error}", path.display()));
                    None
                }
            }
        })
        .collect();
    (projects, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_opens_are_deduplicated_and_ordered_and_corrupt_index_is_preserved() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("recents-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let index_dir = directory.join("index");
        let first = directory.join("external-first.sf");
        let second = directory.join("external-second.sf");
        std::fs::write(&first, "scene first\n  end\n").unwrap();
        std::fs::write(&second, "scene second\n  end\n").unwrap();
        remember_project(&index_dir, &first).unwrap();
        remember_project(&index_dir, &second).unwrap();
        remember_project(&index_dir, &first).unwrap();
        assert_eq!(
            recent_paths(&index_dir).unwrap(),
            vec![
                first.canonicalize().unwrap(),
                second.canonicalize().unwrap()
            ]
        );
        let index = index_dir.join("recent-projects.json");
        std::fs::write(&index, "broken index").unwrap();
        assert!(remember_project(&index_dir, &first).is_err());
        assert_eq!(std::fs::read_to_string(index).unwrap(), "broken index");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
