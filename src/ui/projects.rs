use std::path::{Path, PathBuf};

use dioxus::prelude::*;

use crate::{
    app::AppConfig, models::ProjectStore, persistence::ProjectDb, registry::ArtifactRegistry,
    state::AppState,
};

use super::icons::{Icon, IconName};

/// One row of the Recent list: a `.svs-project.db` file plus the headline
/// counts read from it.
#[derive(Clone, Debug, PartialEq)]
struct RecentProject {
    path: PathBuf,
    name: String,
    beats: usize,
    artifacts: usize,
}

#[component]
pub fn Projects(
    config: AppConfig,
    app_state: Signal<AppState>,
    on_open: EventHandler<MouseEvent>,
) -> Element {
    let mut name = use_signal(String::new);
    let status = use_signal(|| None::<String>);
    let recent = use_signal(|| scan_projects(&config.project_dir));

    let recent_list = recent.read().clone();
    let recent_items = recent_list.iter().cloned().map(|item| {
        let mut status = status;
        let mut app_state = app_state;
        rsx! {
            button {
                class: "group flex w-full items-center gap-5 border-y border-white/[0.06] py-5 text-left",
                onclick: move |event| {
                    let path = item.path.clone();
                    match ProjectDb::open(&path).and_then(|db| db.load()) {
                        Ok(stored) => {
                            app_state.write().open_project(stored, path);
                            status.set(None);
                            on_open.call(event);
                        }
                        Err(error) => status.set(Some(format!(
                            "could not open {}: {error}",
                            path.display()
                        ))),
                    }
                },
                div { class: "grid aspect-video w-32 shrink-0 place-items-center rounded-xl bg-[linear-gradient(145deg,rgba(124,58,237,.16),rgba(24,24,27,.7))] text-violet-300 ring-1 ring-inset ring-white/[0.07]",
                    Icon { name: IconName::Sparkles, class: "size-5" }
                }
                div { class: "min-w-0 flex-1",
                    h3 { class: "truncate text-sm font-medium text-zinc-200 group-hover:text-white", "{item.name}" }
                    p { class: "mt-1 text-[11px] text-zinc-600", "{item.beats} beats · {item.artifacts} artifacts" }
                    p { class: "mt-0.5 truncate text-[10px] text-zinc-700", "{item.path.display()}" }
                }
                Icon { name: IconName::Arrow, class: "size-4 text-zinc-700 transition group-hover:translate-x-1 group-hover:text-zinc-300" }
            }
        }
    });

    let create = {
        let project_dir = config.project_dir.clone();
        let mut status = status;
        let mut name = name;
        move |event: MouseEvent| {
            let story_name = name();
            if story_name.trim().is_empty() {
                return;
            }
            match create_project_file(&project_dir, &story_name, app_state) {
                Ok(()) => {
                    name.set(String::new());
                    status.set(None);
                    on_open.call(event);
                }
                Err(error) => status.set(Some(format!(
                    "could not create {}: {error}",
                    project_dir.display()
                ))),
            }
        }
    };

    let import = {
        let project_dir = config.project_dir.clone();
        let mut status = status;
        move |event: MouseEvent| {
            let Some(picked) = rfd::FileDialog::new()
                .set_title("Import project")
                .add_filter("Project", &["toml", "json", "db"])
                .pick_file()
            else {
                return;
            };
            match import_project_file(&project_dir, &picked, app_state) {
                Ok(()) => {
                    status.set(None);
                    on_open.call(event);
                }
                Err(error) => status.set(Some(error)),
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
                        onclick: import,
                        Icon { name: IconName::Arrow, class: "size-3.5 rotate-180" }
                        "Import"
                    }
                }

                if let Some(error) = status() {
                    p { class: "mb-6 rounded-lg bg-red-500/10 px-4 py-3 text-xs leading-5 text-red-300/90 ring-1 ring-inset ring-red-400/20", "{error}" }
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
                    name: stored.project.name,
                    beats: stored.project.storyboard.len(),
                    artifacts: stored.registry.artifacts.len(),
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    projects.sort_by(|a, b| b.name.cmp(&a.name));
    projects
}

/// Creates the database file for a new story (unique slug path) and saves
/// the fresh project into it before the screen switches to Studio.
fn create_project_file(
    project_dir: &Path,
    story_name: &str,
    mut app_state: Signal<AppState>,
) -> Result<(), String> {
    std::fs::create_dir_all(project_dir).map_err(|error| error.to_string())?;
    let path = unique_project_path(project_dir, story_name);
    let db = ProjectDb::open(&path).map_err(|error| error.to_string())?;
    app_state.write().create_project(story_name);
    let (project, registry) = {
        let state = app_state.read();
        (state.project.clone(), state.registry.clone())
    };
    db.save_project(&project, &registry)
        .map_err(|error| error.to_string())?;
    app_state.write().project_path = Some(path);
    app_state.write().has_unsaved_changes = false;
    Ok(())
}

/// One-time migration into the projects directory: a legacy TOML project
/// (`ProjectStore`) becomes a new database carrying the storyboard; a
/// legacy JSON registry file (`svs import` format) becomes a database with
/// the registry; a `.db` file is opened in place.
fn import_project_file(
    project_dir: &Path,
    legacy_path: &Path,
    mut app_state: Signal<AppState>,
) -> Result<(), String> {
    match legacy_path.extension().and_then(|ext| ext.to_str()) {
        Some("toml") => {
            let project = ProjectStore::load(legacy_path)
                .map_err(|error| format!("could not read legacy project: {error}"))?;
            std::fs::create_dir_all(project_dir).map_err(|error| error.to_string())?;
            let path = unique_project_path(project_dir, &project.name);
            let db = ProjectDb::open(&path).map_err(|error| error.to_string())?;
            db.save_project(&project, &ArtifactRegistry::default())
                .map_err(|error| error.to_string())?;
            let stored = db.load().map_err(|error| error.to_string())?;
            app_state.write().open_project(stored, path);
            Ok(())
        }
        Some("json") => {
            let stem = legacy_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("imported");
            std::fs::create_dir_all(project_dir).map_err(|error| error.to_string())?;
            let path = unique_project_path(project_dir, stem);
            let db = ProjectDb::open(&path).map_err(|error| error.to_string())?;
            db.import_json(legacy_path)
                .map_err(|error| format!("could not import legacy project: {error}"))?;
            let stored = db.load().map_err(|error| error.to_string())?;
            app_state.write().open_project(stored, path);
            Ok(())
        }
        Some("db") => {
            let stored = ProjectDb::open(legacy_path)
                .and_then(|db| db.load())
                .map_err(|error| format!("could not open project: {error}"))?;
            app_state
                .write()
                .open_project(stored, legacy_path.to_path_buf());
            Ok(())
        }
        _ => Err(format!(
            "{} is not a project file (expected .toml, .json, or .db)",
            legacy_path.display()
        )),
    }
}

/// `"My Story"` → `my-story.svs-project.db`; an existing file gets a `-2`,
/// `-3`, … suffix.
fn unique_project_path(project_dir: &Path, name: &str) -> PathBuf {
    let slug: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_owned();
    let slug = if slug.is_empty() {
        "story".to_owned()
    } else {
        slug
    };
    let mut candidate = project_dir.join(format!("{slug}.svs-project.db"));
    let mut index = 2;
    while candidate.exists() {
        candidate = project_dir.join(format!("{slug}-{index}.svs-project.db"));
        index += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::unique_project_path;

    #[test]
    fn slugs_are_safe_filename_components() {
        let dir = std::path::Path::new("/tmp/projects");
        assert_eq!(
            unique_project_path(dir, "The Lighthouse!"),
            dir.join("the-lighthouse.svs-project.db")
        );
        assert_eq!(
            unique_project_path(dir, "   "),
            dir.join("story.svs-project.db")
        );
    }

    #[test]
    fn existing_files_get_a_suffix() {
        let dir = std::env::temp_dir().join(format!("svs-slug-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("story.svs-project.db"), b"x").unwrap();
        assert_eq!(
            unique_project_path(&dir, "Story"),
            dir.join("story-2.svs-project.db")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
