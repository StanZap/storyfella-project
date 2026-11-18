//! The story assistant's conversation: one per project, saved beside the
//! project's workflow runs so it survives closing Studio.
//!
//! A turn records what was asked, the phases the writing workflow went
//! through, and how it ended. A suggestion keeps its full proposal — one
//! passage's, or the whole project's — so a review interrupted by closing
//! Studio can be picked up again.
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{novel::NovelProposal, writing::WritingProposal, RunStore};

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

/// One phase of the writing workflow, such as checking the suggestion.
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
    Proposed(Box<Proposed>),
    /// The request was not an edit; the model answered in prose.
    Replied(String),
    /// The suggestion matched the passage as it stood.
    Unchanged,
    Failed(String),
    /// A model call's answer stopped at the output limit, even when asked
    /// for again without thinking.
    CutOff(String),
    /// The project workflow's plan still did not fit the story after its
    /// repairs.
    UnusablePlan(String),
    Stopped,
    /// Studio closed while the reply was being written.
    Interrupted,
}

/// A suggested edit and what became of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposed {
    pub suggestion: Suggestion,
    /// The files it changes, in the order they are reviewed.
    pub files: Vec<FileChange>,
    pub decision: Decision,
}

/// What a suggestion was asked of: one passage, or the whole project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Suggestion {
    Passage {
        /// The file, relative to the entry story's folder.
        file: String,
        proposal: WritingProposal,
    },
    Project(NovelProposal),
}

/// One file a suggestion changes, and by how many lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// Relative to the entry story's folder.
    pub file: String,
    pub added: usize,
    pub removed: usize,
}

impl Suggestion {
    /// What the model said about its edit.
    pub fn note(&self) -> Option<&str> {
        match self {
            Self::Passage { proposal, .. } => proposal.note.as_deref(),
            Self::Project(proposal) => proposal.note.as_deref(),
        }
    }

    /// Why the run ended before its files were finished and checked; such
    /// a suggestion may not compile.
    pub fn unfinished(&self) -> Option<&str> {
        match self {
            Self::Passage { .. } => None,
            Self::Project(proposal) => proposal.unfinished.as_deref(),
        }
    }

    /// The files it changes, each as it was asked about and as suggested.
    pub fn texts(&self) -> Vec<(String, String, String)> {
        match self {
            Self::Passage { file, proposal } => {
                let request = &proposal.request;
                let mut after = request.source.clone();
                after.replace_range(
                    request.range.start..request.range.end,
                    &proposal.replacement,
                );
                vec![(file.clone(), request.source.clone(), after)]
            }
            Self::Project(proposal) => proposal
                .files
                .iter()
                .map(|(file, after)| {
                    let before = proposal
                        .request
                        .workspace
                        .files
                        .get(file)
                        .cloned()
                        .unwrap_or_default();
                    (file.clone(), before, after.clone())
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    #[default]
    Pending,
    /// Some or all of its changes were kept.
    Kept {
        kept: usize,
        of: usize,
    },
    Discarded,
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
