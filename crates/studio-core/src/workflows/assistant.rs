//! The story assistant's conversation: one per project, saved beside the
//! project's workflow runs so it survives closing Studio.
//!
//! A turn records what was asked, the steps the assistant took, and how it
//! ended: what it said and which files it changed. The changes themselves
//! are in the story already; reviewing them is the editor's.
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{agent::AgentRequest, RunStore};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub turns: Vec<Turn>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Turn {
    Asked(Asked),
    Answered(Answer),
}

/// What the person asked, and of which passage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asked {
    pub text: String,
    /// The passage, as the person should read it: "Scene harbor in story.sf".
    pub scope: String,
    /// Where the passage began, to go back to; none for the whole project.
    pub place: Option<Place>,
}

/// A position in one of the story's files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub file: String,
    /// A byte offset into the file as it was when the request was sent.
    pub offset: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub steps: Vec<Step>,
    pub outcome: Outcome,
}

/// One step the assistant took, such as editing a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub label: String,
    /// How long it took; `None` while it runs.
    pub millis: Option<u64>,
    /// What the phase found or is waiting on, such as the model loading or
    /// the problem a check found.
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    #[default]
    Working,
    /// The request ran to its end; what it changed is in the story already.
    Done(Done),
    Failed(String),
    /// A model call's answer stopped at the output limit.
    CutOff(String),
    /// The writer stopped it; what it changed by then stays in the story.
    Stopped,
    /// Studio closed while the reply was being written.
    Interrupted,
}

/// How a request that ran to its end left the story.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    /// What the assistant said: what it changed, or its answer.
    pub reply: String,
    /// The files it changed.
    pub files: Vec<FileChange>,
    /// It used every step it was allowed and may not have finished.
    pub out_of_steps: bool,
    /// The request to send to carry on, when it ran out of steps.
    pub continuation: Option<Box<AgentRequest>>,
}

/// One file a request changed, and by how many lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// Relative to the entry story's folder.
    pub file: String,
    pub added: usize,
    pub removed: usize,
}

impl Conversation {
    /// The conversation as reopened: a reply still being written when Studio
    /// closed is marked interrupted; nothing is sent again.
    fn reopened(mut self) -> Self {
        for turn in &mut self.turns {
            if let Turn::Answered(answer) = turn {
                if answer.outcome == Outcome::Working {
                    answer.outcome = Outcome::Interrupted;
                    for step in &mut answer.steps {
                        step.millis.get_or_insert(0);
                    }
                }
            }
        }
        self
    }
}

/// Files the assistant changed and the writer has not decided on, each as it
/// was before the assistant's first change: `None` for a file it created.
pub type Undecided = std::collections::BTreeMap<String, Option<String>>;

impl RunStore {
    /// The conversation saved for `project`, empty when there is none.
    pub fn conversation(&self, project: &Path) -> Result<Conversation> {
        let saved: Option<String> = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT conversation FROM assistant_conversations WHERE project = ?1",
                params![project.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match saved {
            Some(saved) => serde_json::from_str::<Conversation>(&saved)?.reopened(),
            None => Conversation::default(),
        })
    }

    pub fn save_conversation(&self, project: &Path, conversation: &Conversation) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO assistant_conversations (project, conversation) VALUES (?1, ?2)
             ON CONFLICT(project) DO UPDATE SET conversation = excluded.conversation",
            params![
                project.to_string_lossy(),
                serde_json::to_string(conversation)?
            ],
        )?;
        Ok(())
    }

    /// Each file the assistant changed in `project` that the writer has not
    /// decided on, as it was before: `None` for a file it created.
    pub fn undecided(&self, project: &Path) -> Result<Undecided> {
        let saved: Option<String> = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT files FROM assistant_undecided WHERE project = ?1",
                params![project.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match saved {
            Some(saved) => serde_json::from_str(&saved)?,
            None => Undecided::new(),
        })
    }

    pub fn save_undecided(&self, project: &Path, files: &Undecided) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO assistant_undecided (project, files) VALUES (?1, ?2)
             ON CONFLICT(project) DO UPDATE SET files = excluded.files",
            params![project.to_string_lossy(), serde_json::to_string(files)?],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undecided_changes_survive_reopening_the_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("story.workflows.sqlite3");
        let project = directory.path().join("story.db");
        let store = RunStore::open(&path).unwrap();
        assert!(store.undecided(&project).unwrap().is_empty());
        let files = Undecided::from([
            (
                "story.sf".to_owned(),
                Some("scene harbor\n  end\n".to_owned()),
            ),
            ("coda.sf".to_owned(), None),
        ]);
        store.save_undecided(&project, &files).unwrap();
        drop(store);
        let store = RunStore::open(&path).unwrap();
        assert_eq!(store.undecided(&project).unwrap(), files);
        store.save_undecided(&project, &Undecided::new()).unwrap();
        assert!(store.undecided(&project).unwrap().is_empty());
    }

    #[test]
    fn a_conversation_survives_reopening_and_an_unfinished_reply_is_interrupted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("story.workflows.sqlite3");
        let project = directory.path().join("story.db");
        let store = RunStore::open(&path).unwrap();
        assert_eq!(
            store.conversation(&project).unwrap(),
            Conversation::default()
        );
        let conversation = Conversation {
            turns: vec![
                Turn::Asked(Asked {
                    text: "Give the reunion more weight".into(),
                    scope: "Scene harbor in story.sf".into(),
                    place: Some(Place {
                        file: "story.sf".into(),
                        offset: 0,
                    }),
                }),
                Turn::Answered(Answer {
                    steps: vec![Step {
                        label: "Writing".into(),
                        millis: None,
                        detail: Some("1,200 characters".into()),
                    }],
                    outcome: Outcome::Working,
                }),
            ],
        };
        store.save_conversation(&project, &conversation).unwrap();
        drop(store);
        let reopened = RunStore::open(&path)
            .unwrap()
            .conversation(&project)
            .unwrap();
        let Turn::Answered(answer) = &reopened.turns[1] else {
            panic!("an answer");
        };
        assert_eq!(answer.outcome, Outcome::Interrupted);
        assert_eq!(answer.steps[0].millis, Some(0));
        assert_eq!(reopened.turns[0], conversation.turns[0]);
    }
}
