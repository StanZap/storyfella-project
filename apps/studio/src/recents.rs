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
    search_text: String,
}

impl RecentProject {
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

pub fn scan_projects(project_dir: &Path) -> (Vec<RecentProject>, Vec<String>) {
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
                        match ProjectFolder::locate_for_authoring(&entry.path()) {
                            Ok(Some(folder)) => discovered.push(folder.database()),
                            Ok(None) => {}
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
        .filter_map(|path| match inspect_project(&path) {
            Ok(stored) => Some(RecentProject::from_stored(path, stored)),
            Err(error) => {
                errors.push(format!("Cannot load {}: {error}", path.display()));
                None
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
}
