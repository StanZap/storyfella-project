//! The story assistant's conversations: any number per project, saved beside
//! the project's workflow runs so they survive closing Studio.
//!
//! A turn records what was asked, the steps the assistant took, and how it
//! ended: what it said and which files it changed. The changes themselves
//! are in the story already; reviewing them is the editor's.
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{agent::AgentRequest, RunStore};

/// What a conversation is called until its first question names it.
pub const UNTITLED: &str = "New conversation";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    /// Milliseconds since the epoch.
    pub created: u64,
    pub updated: u64,
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

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

impl Conversation {
    /// An empty conversation, created now.
    pub fn new() -> Self {
        let now = now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: UNTITLED.into(),
            created: now,
            updated: now,
            turns: Vec::new(),
        }
    }

    /// Adds what the person asked and an empty reply to fill in, returning
    /// the reply's turn. The first question names the conversation.
    pub fn ask(&mut self, asked: Asked) -> usize {
        if self.turns.is_empty() {
            self.title = title_from(&asked.text);
        }
        self.turns.push(Turn::Asked(asked));
        self.turns.push(Turn::Answered(Answer::default()));
        self.touch();
        self.turns.len() - 1
    }

    pub fn touch(&mut self) {
        self.updated = now();
    }

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
    /// The conversations saved for `project`, most recently changed first.
    pub fn conversations(&self, project: &Path) -> Result<Vec<Conversation>> {
        let connection = self.0.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT conversation FROM assistant_chats WHERE project = ?1
             ORDER BY updated DESC, rowid DESC",
        )?;
        let saved = statement
            .query_map(params![project.to_string_lossy()], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        saved
            .iter()
            .map(|saved| Ok(serde_json::from_str::<Conversation>(saved)?.reopened()))
            .collect()
    }

    pub fn save_conversation(&self, project: &Path, conversation: &Conversation) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO assistant_chats (id, project, updated, conversation)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                updated = excluded.updated, conversation = excluded.conversation",
            params![
                conversation.id,
                project.to_string_lossy(),
                i64::try_from(conversation.updated)?,
                serde_json::to_string(conversation)?
            ],
        )?;
        Ok(())
    }

    pub fn delete_conversation(&self, project: &Path, id: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "DELETE FROM assistant_chats WHERE project = ?1 AND id = ?2",
            params![project.to_string_lossy(), id],
        )?;
        Ok(())
    }

    /// Deletes every conversation saved for `project`.
    pub fn clear_conversations(&self, project: &Path) -> Result<()> {
        self.0.lock().unwrap().execute(
            "DELETE FROM assistant_chats WHERE project = ?1",
            params![project.to_string_lossy()],
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

/// The first line of a question, short enough for a list.
fn title_from(text: &str) -> String {
    const LONGEST: usize = 48;
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if line.is_empty() {
        return UNTITLED.into();
    }
    if line.chars().count() <= LONGEST {
        return line.to_owned();
    }
    let cut: String = line.chars().take(LONGEST).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{}…", cut.trim_end())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(text: &str) -> Asked {
        Asked {
            text: text.into(),
            scope: "Project · 1 file".into(),
            place: None,
        }
    }

    #[test]
    fn the_first_question_names_the_conversation() {
        let mut conversation = Conversation::new();
        assert_eq!(conversation.title, UNTITLED);
        let reply = conversation.ask(asked(
            "\nGive the harbor reunion more weight, and let Ivo hesitate before he speaks",
        ));
        assert_eq!(reply, 1);
        assert_eq!(
            conversation.title,
            "Give the harbor reunion more weight, and let…"
        );
        conversation.ask(asked("Shorter"));
        assert_eq!(
            conversation.title,
            "Give the harbor reunion more weight, and let…"
        );
        assert_eq!(title_from("  \n "), UNTITLED);
    }

    /// A conversation with one finished exchange, changed at `updated`.
    fn finished(text: &str, updated: u64) -> Conversation {
        let mut conversation = Conversation::new();
        let reply = conversation.ask(asked(text));
        conversation.turns[reply] = Turn::Answered(Answer {
            steps: Vec::new(),
            outcome: Outcome::Stopped,
        });
        conversation.updated = updated;
        conversation
    }

    #[test]
    fn a_projects_conversations_are_listed_newest_first_and_kept_apart() {
        let directory = tempfile::tempdir().unwrap();
        let store = RunStore::open(&directory.path().join("story.workflows.sqlite3")).unwrap();
        let project = directory.path().join("story.db");
        let other = directory.path().join("other.db");
        let mut older = finished("First", 100);
        let newer = finished("Second", 200);
        let elsewhere = finished("Elsewhere", 100);
        store.save_conversation(&project, &older).unwrap();
        store.save_conversation(&project, &newer).unwrap();
        store.save_conversation(&other, &elsewhere).unwrap();
        assert_eq!(
            store.conversations(&project).unwrap(),
            [newer.clone(), older.clone()]
        );

        // Saving again replaces it in place; it moves up once it changes.
        older.title = "Renamed".into();
        older.updated = 300;
        store.save_conversation(&project, &older).unwrap();
        assert_eq!(
            store.conversations(&project).unwrap(),
            [older.clone(), newer.clone()]
        );

        store.delete_conversation(&project, &older.id).unwrap();
        assert_eq!(store.conversations(&project).unwrap(), [newer]);
        store.clear_conversations(&project).unwrap();
        assert!(store.conversations(&project).unwrap().is_empty());
        assert_eq!(store.conversations(&other).unwrap(), [elsewhere]);
    }

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
        assert!(store.conversations(&project).unwrap().is_empty());
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
            ..Conversation::new()
        };
        store.save_conversation(&project, &conversation).unwrap();
        drop(store);
        let [reopened] = RunStore::open(&path)
            .unwrap()
            .conversations(&project)
            .unwrap()
            .try_into()
            .unwrap();
        let Turn::Answered(answer) = &reopened.turns[1] else {
            panic!("an answer");
        };
        assert_eq!(answer.outcome, Outcome::Interrupted);
        assert_eq!(answer.steps[0].millis, Some(0));
        assert_eq!(reopened.turns[0], conversation.turns[0]);
    }
}
