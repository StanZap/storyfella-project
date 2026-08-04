use std::{fs, path::Path};

use thiserror::Error;

use super::Project;

pub struct ProjectStore;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("failed to read project {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to write project {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse project {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("failed to serialize project: {0}")]
    Serialize(#[source] toml::ser::Error),
}

impl ProjectStore {
    pub fn load(path: impl AsRef<Path>) -> Result<Project, PersistenceError> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(|source| PersistenceError::Read {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&contents).map_err(|source| PersistenceError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    pub fn save(project: &Project, path: impl AsRef<Path>) -> Result<(), PersistenceError> {
        let path = path.as_ref();
        let contents = toml::to_string_pretty(project).map_err(PersistenceError::Serialize)?;
        fs::write(path, contents).map_err(|source| PersistenceError::Write {
            path: path.display().to_string(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{
        models::{ImageRevision, Project, RevisionStatus, StoryboardFrame},
        timeline::{Clip, Timeline},
    };

    use super::ProjectStore;

    #[test]
    fn project_round_trips_through_pretty_toml() {
        let project = Project {
            id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            name: "The Lighthouse".to_owned(),
            timeline: Timeline {
                clips: vec![Clip {
                    id: Uuid::parse_str("17e29460-a0f4-47ab-a1d6-c22c60e2f078").unwrap(),
                    label: "Beat 1".to_owned(),
                    start_seconds: 0.0,
                    duration_seconds: 5.0,
                }],
            },
            storyboard: vec![StoryboardFrame {
                id: Uuid::parse_str("0e5d0c53-4ce8-42a4-a11f-1d0e5f5ac001").unwrap(),
                prompt: "A quiet lighthouse above a silver sea at dusk".to_owned(),
                asset_path: Some("assets/generated/frame.png".to_owned()),
                revisions: vec![ImageRevision {
                    id: Uuid::parse_str("c11f3a5e-4b57-4b3b-9f13-2d8a9c0e7f01").unwrap(),
                    prompt: "make the light warmer".to_owned(),
                    asset_path: Some("assets/generated/frame.png".to_owned()),
                    status: RevisionStatus::Completed,
                    error: None,
                }],
                active_revision_id: Some(
                    Uuid::parse_str("c11f3a5e-4b57-4b3b-9f13-2d8a9c0e7f01").unwrap(),
                ),
            }],
        };

        let toml = toml::to_string_pretty(&project).expect("project should serialize");

        assert_eq!(
            toml,
            r#"id = "550e8400-e29b-41d4-a716-446655440000"
name = "The Lighthouse"

[[timeline.clips]]
id = "17e29460-a0f4-47ab-a1d6-c22c60e2f078"
label = "Beat 1"
start_seconds = 0.0
duration_seconds = 5.0

[[storyboard]]
id = "0e5d0c53-4ce8-42a4-a11f-1d0e5f5ac001"
prompt = "A quiet lighthouse above a silver sea at dusk"
asset_path = "assets/generated/frame.png"
active_revision_id = "c11f3a5e-4b57-4b3b-9f13-2d8a9c0e7f01"

[[storyboard.revisions]]
id = "c11f3a5e-4b57-4b3b-9f13-2d8a9c0e7f01"
prompt = "make the light warmer"
asset_path = "assets/generated/frame.png"
status = "completed"
"#
        );

        let loaded = toml::from_str::<Project>(&toml).expect("project should parse");
        assert_eq!(loaded, project);
    }

    #[test]
    fn save_and_load_are_symmetric() {
        let path = std::env::temp_dir().join(format!("svs-project-test-{}.toml", Uuid::new_v4()));
        let project = Project {
            id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            name: "Untitled Story".to_owned(),
            timeline: Timeline::default(),
            storyboard: Vec::new(),
        };

        ProjectStore::save(&project, &path).expect("save should succeed");
        let loaded = ProjectStore::load(&path).expect("load should succeed");
        let _ = std::fs::remove_file(&path);

        assert_eq!(loaded, project);
    }
}
