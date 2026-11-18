//! The projects Home lists: those opened before, newest first, then any
//! others in the projects directory.
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use storyfella_studio_core::persistence::{ProjectDb, StoredProject};
use storyfella_studio_core::project::ProjectFolder;

use crate::status::Problems;

/// A project row and its cached search index. Filtering never reopens databases
/// or reads source files on each keystroke.
#[derive(Clone, Debug, PartialEq)]
pub struct RecentProject {
    pub path: PathBuf,
    pub name: String,
    pub assets: usize,
    /// When the story or its database last changed on disk.
    pub edited: Option<SystemTime>,
    /// Whether the saved story compiles; an error when it cannot be read.
    pub problems: Result<Problems, String>,
    /// Why the project could not be opened at all, when it cannot.
    pub trouble: Option<Trouble>,
    search_text: String,
}

/// A listed project Home cannot open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trouble {
    pub message: String,
    /// Nothing is at the path any more.
    pub missing: bool,
    /// The path comes from the recent-projects index, so it can be forgotten.
    pub remembered: bool,
}

impl RecentProject {
    fn unreadable(path: PathBuf, message: String, remembered: bool) -> Self {
        let name = path
            .file_stem()
            .filter(|_| !path.ends_with("story.db"))
            .or_else(|| path.parent().and_then(Path::file_name))
            .map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
        let search_text = format!("{name}\n{}", path.display()).to_lowercase();
        Self {
            trouble: Some(Trouble {
                missing: !path.exists(),
                message,
                remembered,
            }),
            problems: Ok(Problems::default()),
            assets: 0,
            edited: None,
            name,
            path,
            search_text,
        }
    }

    fn from_stored(path: PathBuf, stored: StoredProject) -> Self {
        let source = stored.story.path().map(Path::to_path_buf);
        let edited = [Some(path.as_path()), source.as_deref()]
            .into_iter()
            .flatten()
            .filter_map(|file| file.metadata().and_then(|meta| meta.modified()).ok())
            .max();
        let problems = match &source {
            Some(entry) => {
                storyfella_studio_core::story::load_source_workspace(entry, &Default::default())
                    .map_err(|error| error.to_string())
                    .and_then(|workspace| Problems::of(&workspace, &stored.registry))
            }
            None => Err("This project has no story file".into()),
        };
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
            assets: stored.registry.artifacts.len(),
            edited,
            problems,
            trouble: None,
            search_text: search_text.to_lowercase(),
        }
    }

    pub fn matches_query(&self, query: &str) -> bool {
        query
            .to_lowercase()
            .split_whitespace()
            .all(|term| self.search_text.contains(term))
    }
}

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

pub fn remember_project(project_dir: &Path, path: &Path) -> Result<(), String> {
    let mut paths = recent_paths(project_dir)?;
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    paths.retain(|old| old != &path);
    paths.insert(0, path);
    paths.truncate(30);
    write_index(project_dir, &paths)
}

fn write_index(project_dir: &Path, paths: &[PathBuf]) -> Result<(), String> {
    std::fs::create_dir_all(project_dir).map_err(|error| error.to_string())?;
    let index = project_dir.join("recent-projects.json");
    let temporary = project_dir.join("recent-projects.json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(paths).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    std::fs::rename(temporary, index)
        .map_err(|error| format!("Could not update the recent projects: {error}"))
}

/// Drops `path` from the recent-projects index. Nothing on disk is touched.
pub fn forget_project(project_dir: &Path, path: &Path) -> Result<(), String> {
    let mut paths = recent_paths(project_dir)?;
    paths.retain(|old| old != path);
    write_index(project_dir, &paths)
}

/// The project a path belongs to: a project folder for its database,
/// otherwise the path itself. Two paths with one root are one project.
fn project_root(path: &Path) -> &Path {
    match path.parent() {
        Some(folder)
            if path.ends_with("story.db")
                && folder
                    .join(storyfella_studio_core::project::MANIFEST)
                    .exists() =>
        {
            folder
        }
        _ => path,
    }
}

/// Every listed project, readable or not, and any failure to read the
/// listing itself.
pub fn scan_projects(project_dir: &Path) -> (Vec<RecentProject>, Vec<String>) {
    let mut errors = Vec::new();
    let mut paths = recent_paths(project_dir).unwrap_or_else(|error| {
        errors.push(error);
        Vec::new()
    });
    let remembered = paths.clone();
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
                        match ProjectFolder::locate_for_authoring(&entry.path()) {
                            Ok(Some(folder)) => discovered.push(folder.database()),
                            Ok(None) => {}
                            // Listed as unreadable when inspected below.
                            Err(_) => discovered.push(entry.path()),
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
    let mut seen = Vec::new();
    let projects = paths
        .into_iter()
        .filter(|path| {
            let root = project_root(path).to_path_buf();
            let fresh = !seen.contains(&root);
            seen.push(root);
            fresh
        })
        .map(|path| match inspect_project(&path) {
            Ok(stored) => RecentProject::from_stored(path, stored),
            Err(error) => {
                let listed = remembered.contains(&path);
                RecentProject::unreadable(path, format!("{error:#}"), listed)
            }
        })
        .collect();
    (projects, errors)
}

fn inspect_project(path: &Path) -> anyhow::Result<StoredProject> {
    if let Some(folder) = ProjectFolder::locate_for_authoring(path)? {
        let database = folder.database();
        let stored = ProjectDb::inspect_existing(&database)?;
        anyhow::ensure!(
            stored.id == folder.id(),
            "Manifest and database project identities differ"
        );
        return Ok(stored);
    }
    Ok(ProjectDb::open_existing(path)?.load()?)
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

    #[test]
    fn unreadable_and_missing_projects_are_listed_once_and_can_be_forgotten() {
        let directory = tempfile::tempdir().unwrap();
        let projects = directory.path().join("projects");
        // A folder whose manifest is from an older format, known both by its
        // folder and by its database.
        let broken = projects.join("old-format");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(
            broken.join(storyfella_studio_core::project::MANIFEST),
            "project_id = \"3d57d310-b51a-483a-953c-6aa815b4b327\"\n",
        )
        .unwrap();
        std::fs::write(broken.join("story.db"), "").unwrap();
        let gone = directory.path().join("gone/story.db");
        write_index(
            &projects,
            &[
                gone.clone(),
                broken.join("story.db").canonicalize().unwrap(),
            ],
        )
        .unwrap();

        let (listed, errors) = scan_projects(&projects);
        assert!(errors.is_empty(), "{errors:?}");
        let names: Vec<_> = listed.iter().map(|project| project.name.as_str()).collect();
        assert_eq!(names, ["gone", "old-format"]);
        let missing = listed[0].trouble.as_ref().unwrap();
        assert!(missing.missing && missing.remembered);
        let old = listed[1].trouble.as_ref().unwrap();
        assert!(!old.missing);
        assert!(old.message.contains("version"), "{}", old.message);

        forget_project(&projects, &gone).unwrap();
        let (listed, _) = scan_projects(&projects);
        assert_eq!(listed.len(), 1);
    }
}
