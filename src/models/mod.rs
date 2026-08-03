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
}

// TODO: Add explicit schema versions and migration support before the first public release.
