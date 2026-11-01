//! Application-owned workflow persistence and execution. Credentials and model
//! bindings stay ephemeral; project context and results belong to the host.
#[cfg(test)]
mod tests;
pub mod writing;
pub mod writing_review;
pub mod writing_session;

use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use agentkit_workflow::{
    Runner, Workflow, WorkflowEvent, WorkflowRunController, WorkflowRunEvents, WorkflowRunIdentity,
    WorkflowRunState, WorkflowRunStatus, WorkflowRunStore,
};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub use agentkit_workflow as framework;
pub type Cancellation = Arc<AtomicBool>;

#[derive(Clone, Debug)]
pub enum Feedback {
    Event(WorkflowEvent),
    Text(String),
    /// A new writing phase; discard provisional text from the previous phase.
    Status(String),
}

/// One database beside the owning project (or under the app's own data root).
/// Independent of ProjectDb: normal project snapshot saves cannot erase runs.
pub struct RunStore(Mutex<Connection>);

impl RunStore {
    pub fn open(path: &Path) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS workflow_runs (
                id TEXT PRIMARY KEY, revision INTEGER NOT NULL,
                definition TEXT NOT NULL, context TEXT NOT NULL, state TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS workflow_events (
                run_id TEXT NOT NULL, sequence INTEGER NOT NULL, event TEXT NOT NULL,
                PRIMARY KEY (run_id, sequence));
            CREATE TABLE IF NOT EXISTS writing_sessions (
                source_path TEXT PRIMARY KEY, snapshot TEXT NOT NULL);",
        )?;
        writing_session::migrate_review_edits(&mut connection)?;
        Ok(Self(Mutex::new(connection)))
    }

    pub fn create(
        &self,
        workflow: &Workflow,
        context: &serde_json::Value,
    ) -> Result<WorkflowRunState> {
        let definition = serde_json::to_string(workflow)?;
        let state = WorkflowRunState::new(
            WorkflowRunIdentity {
                run_id: uuid::Uuid::new_v4().to_string(),
                workflow_id: workflow.meta.name.clone(),
                workflow_version: workflow.meta.version,
                workflow_hash: agentkit_workflow::content_id(
                    "workflow.json",
                    "application/json",
                    definition.as_bytes(),
                ),
            },
            timestamp(),
        );
        self.0.lock().unwrap().execute(
            "INSERT INTO workflow_runs (id, revision, definition, context, state) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![state.identity.run_id, i64::try_from(state.revision)?, definition, context.to_string(), state.to_json()?],
        )?;
        Ok(state)
    }

    pub fn context(&self, id: &str) -> Result<Option<serde_json::Value>> {
        let json: Option<String> = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT context FROM workflow_runs WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }
}

impl WorkflowRunStore for RunStore {
    type Error = anyhow::Error;
    fn load(&self, id: &str) -> Result<Option<WorkflowRunState>> {
        let json: Option<String> = self
            .0
            .lock()
            .unwrap()
            .query_row("SELECT state FROM workflow_runs WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        json.map(|json| WorkflowRunState::from_json(&json).map_err(Into::into))
            .transpose()
    }

    fn commit(
        &self,
        revision: u64,
        state: &WorkflowRunState,
        events: &[WorkflowEvent],
    ) -> Result<()> {
        ensure!(
            state.revision == revision + 1,
            "Workflow revision must advance once"
        );
        let mut connection = self.0.lock().unwrap();
        let transaction = connection.transaction()?;
        let old = transaction
            .query_row(
                "SELECT state FROM workflow_runs WHERE id=?1",
                [&state.identity.run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .context("Workflow run is missing")?;
        let old = WorkflowRunState::from_json(&old)?;
        ensure!(old.identity == state.identity, "Workflow identity changed");
        ensure!(old.revision == revision, "Workflow revision conflict");
        ensure!(
            state.event_sequence == old.event_sequence + events.len() as u64,
            "Workflow event sequence mismatch"
        );
        for (index, event) in events.iter().enumerate() {
            ensure!(
                event.run_id == state.identity.run_id
                    && event.sequence == old.event_sequence + index as u64 + 1,
                "Workflow event identity or sequence mismatch"
            );
            transaction.execute(
                "INSERT INTO workflow_events VALUES (?1, ?2, ?3)",
                params![
                    event.run_id,
                    i64::try_from(event.sequence)?,
                    serde_json::to_string(event)?
                ],
            )?;
        }
        transaction.execute(
            "UPDATE workflow_runs SET revision=?2, state=?3 WHERE id=?1",
            params![
                state.identity.run_id,
                i64::try_from(state.revision)?,
                state.to_json()?
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

impl WorkflowRunEvents for RunStore {
    fn events(&self, id: &str) -> Result<Vec<WorkflowEvent>> {
        let connection = self.0.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT event FROM workflow_events WHERE run_id=?1 ORDER BY sequence")?;
        let json = statement
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        json.into_iter()
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .collect()
    }
}

fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

/// The framework owns scheduling and terminal transitions. The host only
/// commits transitions and forwards feedback. Dropping an in-flight step on
/// cancellation prevents a delayed reply from becoming a published result.
pub async fn execute(
    runner: Runner,
    state: WorkflowRunState,
    store: &RunStore,
    cancellation: Cancellation,
    feedback: impl Fn(Feedback) + Send + Sync + 'static,
) -> Result<WorkflowRunState> {
    let feedback = Arc::new(feedback);
    let stream = feedback.clone();
    let mut controller = WorkflowRunController::new(runner, state, store).with_delta_observer(
        Arc::new(move |_, delta| {
            if let agentkit_ai::ChatStreamEvent::TextDelta { content } = delta {
                stream(Feedback::Text(content.clone()));
            }
        }),
    );
    let mut polling = tokio::time::interval(Duration::from_millis(25));
    while matches!(
        controller.state().status,
        WorkflowRunStatus::Ready | WorkflowRunStatus::Running
    ) {
        if cancellation.load(Ordering::Relaxed) {
            controller.cancellation_token().cancel();
            controller
                .cancel(timestamp())
                .map_err(|error| anyhow::anyhow!("Workflow cancellation: {error:?}"))?;
            break;
        }
        let transition = {
            let step = controller.step_transition(timestamp());
            tokio::pin!(step);
            loop {
                tokio::select! {
                    result = &mut step => break Some(result),
                    _ = polling.tick() => {
                        if cancellation.load(Ordering::Relaxed) { break None; }
                    }
                }
            }
        };
        match transition {
            Some(transition) => {
                let transition = transition
                    .map_err(|error| anyhow::anyhow!("Workflow storage/execution: {error:?}"))?;
                for event in transition.events {
                    feedback(Feedback::Event(event));
                }
            }
            None => {
                controller.cancellation_token().cancel();
                controller
                    .cancel(timestamp())
                    .map_err(|error| anyhow::anyhow!("Workflow cancellation: {error:?}"))?;
            }
        }
    }
    let state = controller.state().clone();
    match state.status {
        WorkflowRunStatus::Completed | WorkflowRunStatus::Cancelled => Ok(state),
        WorkflowRunStatus::Failed => bail!("Workflow failed: {:?}", state.outcome),
        _ => bail!("Workflow paused without an application handler"),
    }
}
