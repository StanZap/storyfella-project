mod persistence;

pub use persistence::{PersistenceError, ProjectStore};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::timeline::Timeline;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub timeline: Timeline,
    pub storyboard: Vec<StoryboardFrame>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Untitled Story".to_owned(),
            timeline: Timeline::default(),
            storyboard: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoryboardFrame {
    pub id: Uuid,
    pub prompt: String,
    pub asset_path: Option<String>,
    #[serde(default)]
    pub revisions: Vec<ImageRevision>,
    #[serde(default)]
    pub active_revision_id: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageRevision {
    pub id: Uuid,
    pub prompt: String,
    pub asset_path: Option<String>,
    pub status: RevisionStatus,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionStatus {
    #[default]
    Queued,
    Generating,
    Completed,
    Failed,
    Cancelled,
}

// TODO: This legacy storyboard model is transitional: SQLite
// (`src/persistence/`) carries versioned migrations and the GUI persists
// there now; the TOML `ProjectStore` is a one-time import path only. Delete
// `Project`/`StoryboardFrame`/`ProjectStore` when the canvas (roadmap item
// 4) replaces this model.
