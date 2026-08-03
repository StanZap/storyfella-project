use std::sync::Arc;

use parking_lot::RwLock;

use crate::models::Project;

#[derive(Clone, Debug)]
pub struct AppState {
    pub project_name: String,
    pub project: Arc<RwLock<Project>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            project_name: "Untitled Story".to_owned(),
            project: Arc::new(RwLock::new(Project::default())),
        }
    }
}

impl PartialEq for AppState {
    fn eq(&self, other: &Self) -> bool {
        self.project_name == other.project_name && *self.project.read() == *other.project.read()
    }
}
