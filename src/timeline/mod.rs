use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Uuid,
    pub label: String,
    pub start_seconds: f64,
    pub duration_seconds: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub clips: Vec<Clip>,
}

impl Timeline {
    pub fn push(&mut self, label: impl Into<String>, start_seconds: f64, duration_seconds: f64) {
        self.clips.push(Clip {
            id: Uuid::new_v4(),
            label: label.into(),
            start_seconds,
            duration_seconds,
        });
    }
}
