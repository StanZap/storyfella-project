//! A local writing workspace checkpoint, separate from accepted project saves.
use super::{
    writing::{WritingAction, WritingRequest},
    writing_review::WritingReview,
    RunStore,
};
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Upgrade stored checkpoints once, before decoding them into the current
/// review type. The original proposal supplies the pre-edit baseline, so old
/// manual edits and reset choices become protected rather than disappearing.
pub(super) fn migrate_review_edits(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    let snapshots = {
        let mut statement = transaction.prepare(
            "SELECT source_path, snapshot FROM writing_sessions
             WHERE CASE WHEN json_valid(snapshot) THEN
                 json_type(snapshot, '$.review') = 'object'
                 AND json_type(snapshot, '$.review.edit_base') IS NULL
             ELSE 0 END",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (path, json) in snapshots {
        let mut snapshot: serde_json::Value = serde_json::from_str(&json)?;
        let review = &mut snapshot["review"];
        let proposal: super::writing::WritingProposal =
            serde_json::from_value(review["proposal"].clone())?;
        let baseline = proposal.apply(
            &proposal.request.project,
            &proposal.request.source_path,
            &proposal.request.source,
        )?;
        review["edit_base"] = baseline.into();
        review["manual_edits"] = serde_json::json!([]);
        transaction.execute(
            "UPDATE writing_sessions SET snapshot=?2 WHERE source_path=?1",
            params![path, snapshot.to_string()],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritingSession {
    pub project: PathBuf,
    pub source_path: PathBuf,
    pub saved_source: String,
    pub draft: String,
    pub scope: String,
    pub action: WritingAction,
    pub instructions: String,
    pub preserve_facts: bool,
    pub request: Option<WritingRequest>,
    pub request_open: bool,
    pub review: Option<WritingReview>,
    pub generating: bool,
    pub streamed: String,
    pub phase: String,
    pub side_by_side: bool,
    pub selected: usize,
    pub marked: Vec<usize>,
}

impl WritingSession {
    pub fn check_owner(&self, project: &Path, source_path: &Path) -> Result<()> {
        ensure!(
            self.project == project && self.source_path == source_path,
            "The saved writing session belongs to another story."
        );
        Ok(())
    }

    pub fn can_restore(&self, current: &str) -> bool {
        !self.has_source_context() || current == self.saved_source || current == self.draft
    }

    /// Composer preferences alone must never replace a newer story.
    pub fn has_source_context(&self) -> bool {
        self.request.is_some()
            || self.review.is_some()
            || self.generating
            || self.draft != self.saved_source
    }
}

impl RunStore {
    pub fn writing_session(
        &self,
        project: &Path,
        source_path: &Path,
    ) -> Result<Option<WritingSession>> {
        let json: Option<String> = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT snapshot FROM writing_sessions WHERE source_path=?1",
                [source_path.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()?;
        let session = json
            .map(|json| serde_json::from_str::<WritingSession>(&json))
            .transpose()?;
        if let Some(session) = &session {
            session.check_owner(project, source_path)?;
        }
        Ok(session)
    }

    pub fn save_writing_session(&self, session: &WritingSession) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO writing_sessions (source_path, snapshot) VALUES (?1, ?2)
             ON CONFLICT(source_path) DO UPDATE SET snapshot=excluded.snapshot",
            params![
                session.source_path.to_string_lossy(),
                serde_json::to_string(session)?
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        registry::ArtifactRegistry,
        workflows::writing::{WritingProposal, WritingScope},
    };

    #[test]
    fn reopen_preserves_pending_manual_edits_and_unsaved_accepted_draft() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("workflows.sqlite3");
        let project = directory.path().join("story.db");
        let path = directory.path().join("story.sf");
        let source = "scene harbor\r\n  narrate: The sea is quiet.\r\n  end\r\n";
        let registry = ArtifactRegistry::new();
        let request =
            WritingRequest::capture(&project, &path, source, WritingScope::Scene(0), &registry)
                .unwrap();
        let mut review = WritingReview::new(
            WritingProposal {
                run_id: "recover".into(),
                request: request.clone(),
                replacement: source.replace("quiet", "restless"),
            },
            &registry,
        )
        .unwrap();
        review.draft = source.replace("quiet", "cálm 🌊");
        // Manual edits may be temporarily invalid. Recovery must retain them
        // exactly, and normal acceptance must still enforce compilation.
        review.edit("scene harbor\n  narrate: \"unfinished\n  end\n");
        let snapshot = WritingSession {
            project: project.clone(),
            source_path: path.clone(),
            saved_source: source.into(),
            draft: review.draft.clone(),
            scope: "scene".into(),
            action: WritingAction::Expand,
            instructions: "Keep the pause".into(),
            preserve_facts: false,
            request: Some(request),
            request_open: false,
            review: Some(review.clone()),
            generating: false,
            streamed: String::new(),
            phase: String::new(),
            side_by_side: true,
            selected: 2,
            marked: vec![0, 2],
        };
        RunStore::open(&database)
            .unwrap()
            .save_writing_session(&snapshot)
            .unwrap();
        let store = RunStore::open(&database).unwrap();
        let restored = store.writing_session(&project, &path).unwrap().unwrap();
        assert_eq!(restored, snapshot);
        assert!(restored.can_restore(source));
        assert!(restored.can_restore(&review.draft));
        assert!(!restored.can_restore("scene changed_elsewhere\n  end\n"));
        assert!(
            restored
                .review
                .unwrap()
                .acceptance(None)
                .unwrap()
                .review(&registry)
                .unwrap()
                .has_errors
        );
        assert!(store
            .writing_session(Path::new("another.db"), &path)
            .is_err());
        assert!(store
            .writing_session(&project, Path::new("another.sf"))
            .unwrap()
            .is_none());

        // Upgrade the checkpoint shape shipped before retry protection. The
        // user's current text, invalid intermediate edit and selection survive.
        let mut previous = serde_json::to_value(&snapshot).unwrap();
        previous["review"]
            .as_object_mut()
            .unwrap()
            .remove("edit_base");
        previous["review"]
            .as_object_mut()
            .unwrap()
            .remove("manual_edits");
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE writing_sessions SET snapshot=?1 WHERE source_path=?2",
                params![previous.to_string(), path.to_string_lossy()],
            )
            .unwrap();
        drop(store);
        for _ in 0..2 {
            let migrated = RunStore::open(&database)
                .unwrap()
                .writing_session(&project, &path)
                .unwrap()
                .unwrap();
            assert_eq!(migrated, snapshot);
        }
    }

    #[test]
    fn interrupted_response_survives_reopen_and_replaces_previous_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("workflows.sqlite3");
        let mut session = WritingSession {
            project: "story.db".into(),
            source_path: "story.sf".into(),
            saved_source: String::new(),
            draft: String::new(),
            scope: "beat".into(),
            action: WritingAction::Rewrite,
            instructions: "Make it chatty".into(),
            preserve_facts: true,
            request: None,
            request_open: true,
            review: None,
            generating: true,
            streamed: "    mara: \"Wait".into(),
            phase: "Writing a suggestion…".into(),
            side_by_side: false,
            selected: 0,
            marked: Vec::new(),
        };
        let store = RunStore::open(&database).unwrap();
        store.save_writing_session(&session).unwrap();
        session.streamed.push_str(" for me.\"");
        store.save_writing_session(&session).unwrap();
        drop(store);
        let restored = RunStore::open(&database)
            .unwrap()
            .writing_session(&session.project, &session.source_path)
            .unwrap()
            .unwrap();
        assert_eq!(restored, session);
        assert!(restored.generating);
        session.generating = false;
        assert!(!session.has_source_context());
        assert!(session.can_restore("a newer story"));
    }
}
