use std::path::{Path, PathBuf};

use dioxus::prelude::*;

use storyfella_studio_core::config::AppConfig;
use storyfella_studio_core::errors::ReportedError;
use storyfella_studio_core::persistence::{ProjectDb, StoredProject};

use crate::{commands, state::AppState};

use super::{
    components::{ErrorDetailsButton, IconButton, RecoveryNotice},
    dialog::Dialog,
    icons::{Icon, IconName},
    inputs::{InputSize, SearchInput, TextInput},
};

/// A project row and its cached search index. Filtering never reopens databases
/// or reads source files on each keystroke.
#[derive(Clone, Debug, PartialEq)]
struct RecentProject {
    path: PathBuf,
    name: String,
    artifacts: usize,
    search_text: String,
}

impl RecentProject {
    fn from_stored(path: PathBuf, stored: StoredProject) -> Self {
        let mut search_text = format!(
            "{}\n{}\n{}",
            stored.name,
            path.display(),
            stored.story.source()
        );
        for artifact in &stored.registry.artifacts {
            search_text.push('\n');
            search_text.push_str(&artifact.name);
            search_text.push('\n');
            search_text.push_str(&artifact.description);
        }
        Self {
            path,
            name: stored.name,
            artifacts: stored.registry.artifacts.len(),
            search_text: search_text.to_lowercase(),
        }
    }

    fn matches_query(&self, query: &str) -> bool {
        query
            .to_lowercase()
            .split_whitespace()
            .all(|term| self.search_text.contains(term))
    }
}

#[component]
pub fn Projects(
    config: AppConfig,
    app_state: Signal<AppState>,
    on_open: EventHandler<()>,
) -> Element {
    let mut searching = use_signal(|| false);
    let mut query = use_signal(String::new);
    let mut show_form = use_signal(|| false);
    let mut name = use_signal(String::new);
    let name_id = use_hook(|| format!("project-name-{}", uuid::Uuid::new_v4()));
    let mut form_error = use_signal(|| None::<ReportedError>);
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let mut recent = use_signal(|| scan_projects(&config.project_dir));
    let mut scan_errors = use_signal(Vec::<String>::new);
    let opened = app_state.read().project_path.clone();
    let screen = use_context::<Signal<super::AppScreen>>();
    let active = screen() == super::AppScreen::Projects;
    let standalone_project = opened.as_ref().is_some_and(|path| {
        !path.parent().is_some_and(|parent| {
            parent
                .join(storyfella_studio_core::project::MANIFEST)
                .exists()
        })
    });
    let remember_dir = config.project_dir.clone();
    use_effect(use_reactive(&opened, move |path| {
        if let Some(path) = path {
            if let Err(error) = remember_project(&remember_dir, &path) {
                scan_errors.set(vec![error]);
            }
        }
    }));
    let recent_dir = config.project_dir.clone();
    use_effect(use_reactive(&(opened, active), move |(_, active)| {
        if active {
            recent.set(scan_projects(&recent_dir));
        }
    }));
    let refresh_dir = config.project_dir.clone();

    let recent_data = recent.read();
    let (recent_list, load_errors) = &*recent_data;
    let query_text = query();
    let filtered: Vec<_> = recent_list
        .iter()
        .filter(|item| item.matches_query(&query_text))
        .collect();
    let result_count = filtered.len();
    let recent_items = filtered.into_iter().map(|item| {
        let path = item.path.clone();
        let title = path.display().to_string();
        let name = item.name.clone();
        let artifacts = item.artifacts;
        rsx! {
            button {
                class: "group flex w-full items-center gap-5 rounded-xl border-y border-white/[0.06] px-3 py-5 text-left transition-colors hover:border-white/15 hover:bg-white/[0.04] focus-visible:bg-white/[0.04]",
                title,
                onclick: move |_| {
                    match commands::open_project(&path, app_state) {
                        Ok(()) => {
                            on_open.call(());
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
                    h3 { class: "truncate text-sm font-medium text-zinc-200 group-hover:text-white", "{name}" }
                    p { class: "mt-1 text-[11px] text-zinc-600", "{artifacts} artifacts" }
                }
                Icon { name: IconName::Arrow, class: "size-4 text-zinc-700 transition group-hover:translate-x-1 group-hover:text-zinc-300" }
            }
        }
    });

    let create = {
        let project_dir = config.project_dir.clone();
        let mut name = name;
        move |event: FormEvent| {
            event.prevent_default();
            let story_name = name();
            if story_name.trim().is_empty() {
                return;
            }
            match commands::create_project(&project_dir, &story_name, app_state) {
                Ok(_) => {
                    name.set(String::new());
                    show_form.set(false);
                    form_error.set(None);
                    on_open.call(());
                }
                Err(report) => form_error.set(Some(report.log())),
            }
        }
    };

    let open_dialog = {
        move |_| {
            let Some(picked) = rfd::FileDialog::new()
                .set_title("Open project")
                .add_filter(
                    "Storyfella project, manifest or story",
                    &["toml", "db", "sf"],
                )
                .pick_file()
            else {
                return;
            };
            match commands::open_project(&picked, app_state) {
                Ok(()) => {
                    show_form.set(false);
                    form_error.set(None);
                    on_open.call(());
                }
                Err(report) => form_error.set(Some(report.log())),
            }
        }
    };

    let dismiss_search = EventHandler::new(move |_| {
        searching.set(false);
        document::eval("requestAnimationFrame(() => document.querySelector('[aria-label=\"Search projects\"]')?.focus());");
    });
    rsx! {
        section { class: "flex h-full min-h-0 flex-col", onkeydown: move |event| {
            if searching() && event.key() == Key::Escape {
                event.prevent_default();
                event.stop_propagation();
                query.set(String::new());
                dismiss_search.call(());
            }
        },
            div { class: "min-h-0 flex-1 overflow-y-auto px-6 py-10 sm:px-10",
                div { class: "mx-auto max-w-5xl",
                    div { class: "mb-8 flex h-12 items-center gap-3",
                        if searching() {
                            SearchInput {
                                value: query(), label: "Search projects", controls: "studio-project-list",
                                size: InputSize::Large, autofocus: true,
                                placeholder: "Search names, story content or assets… (Esc to cancel)",
                                title: "Search project names, locations, story content and asset descriptions. Esc cancels.",
                                oninput: move |value| query.set(value), oncancel: dismiss_search,
                            }
                        } else {
                            h1 { class: "min-w-0 flex-1 text-3xl font-semibold tracking-[-0.04em] text-zinc-100", "Your stories" }
                            div { role: "group", aria_label: "Project actions", class: "flex shrink-0 items-center gap-1",
                                IconButton { label: "Search projects", icon: IconName::Search, onclick: move |_| searching.set(true) }
                                IconButton { label: "Refresh projects", icon: IconName::Refresh, onclick: move |_| { scan_errors.set(Vec::new()); recent.set(scan_projects(&refresh_dir)); } }
                                IconButton { label: "New story", icon: IconName::Add, expanded: show_form(), onclick: move |_| show_form.set(true) }
                            }
                        }
                    }
                    for message in load_errors.iter().chain(scan_errors.read().iter()) {
                        p { role: "alert", class: "mb-3 break-words text-xs text-amber-300", "{message}" }
                    }
                    if searching() {
                        span { role: "status", class: "sr-only", "{result_count} matching projects" }
                    }
                    div { id: "studio-project-list",
                        if recent_list.is_empty() {
                            p { class: "py-10 text-sm text-zinc-400", "No stories yet. Use + to create or open one." }
                        } else if result_count == 0 {
                            p { class: "py-10 text-sm text-zinc-400", "No projects match your search." }
                        } else {
                            {recent_items}
                        }
                    }
                }
            }
        }
        if show_form() {
            Dialog { title: "New story", on_close: move |_| show_form.set(false),
                form { class: "studio-task-form", onsubmit: create,
                    header { class: "flex items-center gap-3",
                        h2 { class: "flex-1 text-xl font-semibold", "New story" }
                        IconButton { label: "Close story form", icon: IconName::Close, onclick: move |_| show_form.set(false) }
                    }
                    div { class: "studio-field",
                        label { r#for: name_id.clone(), "Story name" }
                        TextInput { id: name_id.clone(), value: name(), label: "Story name", autofocus: true, placeholder: "Story name", oninput: move |value| name.set(value) }
                    }
                    if let Some(report) = form_error() {
                        RecoveryNotice { title: report.code.message(), message: "Your input is retained. Correct it or try again.",
                            ErrorDetailsButton { error: report }
                        }
                    }
                    footer { class: "flex flex-wrap justify-end gap-2",
                        button { r#type: "button", class: "studio-button", title: "Open an existing project, manifest or story file", onclick: open_dialog, "Open existing…" }
                        button { r#type: "submit", class: "studio-button studio-button-primary", disabled: name.read().trim().is_empty(), title: "Create a story project", "Create story" }
                    }
                    if standalone_project {
                        button { r#type: "button", class: "studio-button", title: "Copy the open story and artwork into a portable project folder",
                            onclick: move |_| {
                                if let Some(parent) = rfd::FileDialog::new().set_title("Choose a parent folder for the project copy").pick_folder() {
                                    match commands::copy_to_project_folder(&parent, app_state) {
                                        Ok(()) => { show_form.set(false); form_error.set(None); on_open.call(()); }
                                        Err(report) => form_error.set(Some(report.log())),
                                    }
                                }
                            },
                            "Copy current story to project folder…"
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
                    Ok(entry)
                        if entry
                            .path()
                            .join(storyfella_studio_core::project::MANIFEST)
                            .is_file() =>
                    {
                        match storyfella_studio_core::project::ProjectFolder::open(&entry.path()) {
                            Ok(folder) => discovered.push(folder.database()),
                            Err(error) => errors
                                .push(format!("Cannot load {}: {error:#}", entry.path().display())),
                        }
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
                Ok(stored) => Some(RecentProject::from_stored(path, stored)),
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
    fn filtering_searches_saved_story_asset_metadata_and_location_without_rereading_on_input() {
        use storyfella_studio_core::{
            catalog::{self, CatalogEdit},
            project::ProjectFolder,
            registry::{ArtifactKind, ArtifactRegistry},
        };
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("project-search-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut registry = ArtifactRegistry::default();
        catalog::apply(
            &mut registry,
            CatalogEdit::Create {
                kind: ArtifactKind::Object,
                name: "brass_lantern".into(),
                description: "The captain's weathered lamp".into(),
                variant_of: None,
            },
        )
        .unwrap();
        let folder = ProjectFolder::create(
            &directory.join("lighthouse"),
            "The Lighthouse",
            &registry,
            "scene harbor\n  narrate: Nightwatch at the café during a storm.\n  end\n",
        )
        .unwrap();
        ProjectFolder::create(
            &directory.join("school"),
            "The School",
            &ArtifactRegistry::default(),
            "scene classroom\n  end\n",
        )
        .unwrap();
        let (projects, errors) = scan_projects(&directory);
        assert!(errors.is_empty());
        assert_eq!(projects.len(), 2);
        for query in [
            "LIGHTHOUSE",
            "NIGHTWATCH",
            "CAFÉ",
            "brass_lantern",
            "weathered",
            "lighthouse captain",
            "lighthouse/story.db",
        ] {
            let matched: Vec<_> = projects
                .iter()
                .filter(|item| item.matches_query(query))
                .collect();
            assert_eq!(matched.len(), 1, "{query}");
            assert_eq!(matched[0].name, "The Lighthouse");
        }
        assert_eq!(
            projects
                .iter()
                .filter(|item| item.matches_query("  "))
                .count(),
            2
        );
        assert!(!projects
            .iter()
            .any(|item| item.matches_query("lighthouse classroom")));
        assert!(!projects
            .iter()
            .any(|item| item.matches_query("unknown words")));
        std::fs::write(
            folder.root().join("assets/story.sf"),
            "scene harbor\n  narrate: An unexpected sunrise.\n  end\n",
        )
        .unwrap();
        assert!(
            !projects.iter().any(|item| item.matches_query("sunrise")),
            "typing uses the cached scan"
        );
        let (refreshed, errors) = scan_projects(&directory);
        assert!(errors.is_empty());
        assert_eq!(
            refreshed
                .iter()
                .filter(|item| item.matches_query("sunrise"))
                .count(),
            1
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

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
