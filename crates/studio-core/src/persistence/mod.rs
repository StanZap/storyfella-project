//! SQLite persistence for the artifact registry (`docs/ROADMAP.md` §10).
//!
//! The registry stays the in-memory source of truth for operations; the
//! database is a snapshot written after each applied operation. Each save
//! replaces the snapshot rows in one transaction (with
//! `PRAGMA defer_foreign_keys` so the circular `artifacts ↔ revisions`
//! references can be written in any order). `lora_registry` carries the
//! project's LoRA adapters (roadmap §12 item 6).
//!
//! Deviations from the §10 sketch, all deliberate:
//! - Timestamps are unix epoch integers (`strftime('%s','now')`), not
//!   ISO-8601 text — no chrono dependency; `created_at`/`updated_at` are
//!   database metadata only (the in-memory model has no timestamps).
//! - `operation_log.id` is TEXT (the model uses `Uuid`), not
//!   AUTOINCREMENT; `op` holds the serde tag name and `args` the full
//!   serialized `Operation` JSON.
//! - `artifacts.composition_mode` (beat-only) and `artifacts.drafts_json`
//!   (artifact-scoped generated text proposals) are not in the
//!   sketch; `revisions.error` and `revisions.seed`/`model` round out the
//!   model fields.
//!
//! Schema history: v1 (§10 tables), v2 carried the legacy storyboard as
//! `projects.project_json`, v3 dropped it — the canvas (roadmap §12 item 4)
//! replaced the storyboard model, so the column went with it.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::errors::{CodedError, ErrorCode};
use crate::registry::{
    ops::{Operation, OperationLogEntry},
    Artifact, ArtifactRegistry, ArtifactRevision, BeatComposition, CompositionMode, Layer,
    LoraEntry, StoryDraft,
};
use crate::story::{export_draft, included_edit_path, source_path, StoryDocument, StoryFileError};

/// The `projects` row for a database, plus the full registry snapshot.
///
/// `id`/`name` mirror the `projects` row (stable per database file); the
/// registry is the whole project — the storyboard model it replaced was
/// removed with schema v3.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredProject {
    pub registry_version: i64,
    pub id: Uuid,
    pub name: String,
    pub registry: ArtifactRegistry,
    /// The authoritative `.sf` file beside this database.
    pub story: StoryDocument,
}

/// SQLite-backed storage for one project (one registry per database).
///
/// Synchronous, single writer connection behind a mutex — the access shape
/// the desktop app needs (no async pool, no tokio involvement). WAL mode;
/// full source/catalog reads briefly serialize with source publication.
pub struct ProjectDb {
    conn: Mutex<Connection>,
    path: PathBuf,
}

enum SnapshotMode {
    Discovery,
    ReadAuthoring,
    Recover,
}

struct SourceEdit<'a> {
    document: &'a StoryDocument,
    include: Option<(&'a StoryDocument, &'a str)>,
}

struct PendingSource {
    file: Option<String>,
    before: String,
    after: String,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("the asset catalog changed during saving; your work is retained, refresh and retry")]
    Conflict,
    #[error("the project has an interrupted source save that requires recovery")]
    PendingSource,
    #[error("could not access shared project resources: {0:#}")]
    Resources(#[from] anyhow::Error),
    #[error(transparent)]
    Story(#[from] StoryFileError),
    #[error("could not resolve project database path {path}: {source}")]
    Path {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not open project database {path}: {source}")]
    Open {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("could not configure project database {path}: {source}")]
    Configure {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("could not migrate project database {path}: {source}")]
    Migrate {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("could not save registry to {path}: {source}")]
    Save {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("could not serialize a registry value for {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("could not load registry from {path}: {source}")]
    Load {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("corrupt value in {path}: {column} = {value:?}")]
    Corrupt {
        path: PathBuf,
        column: String,
        value: String,
    },
}

impl CodedError for StoreError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Resources(_) | Self::Conflict | Self::PendingSource => ErrorCode::StoreSave,
            Self::Story(error) => error.code(),
            Self::Open { .. } | Self::Path { .. } => ErrorCode::StoreOpen,
            Self::Configure { .. } => ErrorCode::StoreConfigure,
            Self::Migrate { .. } => ErrorCode::StoreMigrate,
            Self::Save { .. } => ErrorCode::StoreSave,
            Self::Serialize { .. } => ErrorCode::StoreSerialize,
            Self::Load { .. } => ErrorCode::StoreLoad,
            Self::Corrupt { .. } => ErrorCode::StoreCorrupt,
        }
    }
}

impl ProjectDb {
    /// Opens (creating if necessary) the project database at `path`,
    /// migrates it to the latest schema, and seeds the single `projects`
    /// row on first creation.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with_flags(path.as_ref(), rusqlite::OpenFlags::default())
    }

    /// Background refresh and saves must not recreate a moved/deleted database.
    pub fn open_existing(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with_flags(
            path.as_ref(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
    }

    fn open_with_flags(path: &Path, flags: rusqlite::OpenFlags) -> Result<Self, StoreError> {
        let db = Self::connect(path, flags)?;
        db.configure()?;
        db.migrate()?;
        db.seed_project_row()?;
        Ok(db)
    }

    fn connect(path: &Path, flags: rusqlite::OpenFlags) -> Result<Self, StoreError> {
        let path = path.to_path_buf();
        let conn =
            Connection::open_with_flags(&path, flags).map_err(|source| StoreError::Open {
                path: path.clone(),
                source,
            })?;
        let path = path.canonicalize().map_err(|source| StoreError::Path {
            path: path.clone(),
            source,
        })?;
        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    /// Inspect saved files for discovery. During a pending source write, the
    /// catalog can be ahead of the source on disk. Opening for authoring
    /// handles recovery and migrations before treating the pair as a snapshot.
    pub fn inspect_existing(path: &Path) -> Result<StoredProject, StoreError> {
        Self::connect(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?
        .load_snapshot(SnapshotMode::Discovery)
    }

    /// Serialize the source and catalog read with cooperating publishers.
    /// An interrupted save is reported without applying recovery or migrations.
    pub fn read_authoring_snapshot(path: &Path) -> Result<StoredProject, StoreError> {
        Self::connect(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?
        .load_snapshot(SnapshotMode::ReadAuthoring)
    }

    /// Replaces the stored snapshot with `registry` in one transaction,
    /// leaving the `projects` row untouched.
    pub fn save_registry(&self, registry: &ArtifactRegistry) -> Result<(), StoreError> {
        self.persist_registry(registry, None).map(|_| ())
    }

    /// Optimistic snapshot commit: another editor's catalog changes must never
    /// be silently replaced. The comparison and write share one transaction.
    pub fn save_registry_checked(
        &self,
        registry: &ArtifactRegistry,
        version: i64,
    ) -> Result<i64, StoreError> {
        self.persist_registry(registry, Some(version))
    }

    /// Commit artwork and a recoverable source intent together. A failed source
    /// publish leaves the exact old/new text in SQLite; load/retry completes it
    /// only if disk still matches either side. No source rewrite precedes a
    /// successful catalog commit.
    pub fn save_authoring_checked(
        &self,
        registry: &ArtifactRegistry,
        version: i64,
        story: &mut StoryDocument,
    ) -> Result<i64, StoreError> {
        self.save_source_checked(registry, version, story, None)
    }

    /// Save one included file with its reconciled catalog, while checking
    /// the entry snapshot used to validate the edit.
    pub fn save_included_authoring_checked(
        &self,
        registry: &ArtifactRegistry,
        version: i64,
        entry: &StoryDocument,
        file: &str,
        story: &mut StoryDocument,
    ) -> Result<i64, StoreError> {
        self.save_source_checked(registry, version, story, Some((entry, file)))
    }

    fn save_source_checked(
        &self,
        registry: &ArtifactRegistry,
        version: i64,
        story: &mut StoryDocument,
        include: Option<(&StoryDocument, &str)>,
    ) -> Result<i64, StoreError> {
        self.recover_source()?;
        let registry = self.prepare_registry(registry)?;
        let version = self.write_snapshot(
            &registry,
            Some(version),
            Some(SourceEdit {
                document: story,
                include,
            }),
        )?;
        self.recover_source()?;
        story.reload()?;
        Ok(version)
    }

    fn recover_source(&self) -> Result<(), StoreError> {
        let mut conn = self.conn.lock();
        if self.pending_source(&conn)?.is_none() {
            return Ok(());
        }
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;
        self.recover_source_in(&tx)?;
        tx.commit().map_err(|source| StoreError::Save {
            path: self.path.clone(),
            source,
        })
    }

    fn pending_source(&self, conn: &Connection) -> Result<Option<PendingSource>, StoreError> {
        conn.query_row(
            "SELECT source_file, before_source, after_source FROM source_write WHERE id = 1",
            [],
            |row| {
                Ok(PendingSource {
                    file: row.get(0)?,
                    before: row.get(1)?,
                    after: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|source| StoreError::Load {
            path: self.path.clone(),
            source,
        })
    }

    fn open_pending_source(&self, pending: &PendingSource) -> Result<StoryDocument, StoreError> {
        let entry = source_path(&self.path);
        let Some(file) = &pending.file else {
            return Ok(StoryDocument::open(&entry)?);
        };
        let path = included_edit_path(&entry, file)?;
        let story = StoryDocument::open(&path)?;
        if story.path() != Some(path.as_path()) {
            return Err(StoryFileError::Conflict { path }.into());
        }
        Ok(story)
    }

    fn recover_source_in(&self, conn: &Connection) -> Result<(), StoreError> {
        if let Some(pending) = self.pending_source(conn)? {
            self.validate_identity(conn)?;
            let mut story = self.open_pending_source(&pending)?;
            if story.source() != pending.after {
                if story.source() != pending.before {
                    return Err(StoreError::Story(StoryFileError::Conflict {
                        path: story.path().ok_or(StoryFileError::Unbound)?.to_owned(),
                    }));
                }
                story.set_source(pending.after);
                story.save()?;
            }
            conn.execute("DELETE FROM source_write WHERE id = 1", [])
                .map_err(|source| StoreError::Save {
                    path: self.path.clone(),
                    source,
                })?;
        }
        Ok(())
    }

    /// Explicit recovery when an external source edit interrupted a paired
    /// save. Preserve those exact bytes beside the source before completing
    /// the already committed intent. Normal load never chooses this policy.
    pub fn preserve_source_and_recover(&self) -> Result<Option<PathBuf>, StoreError> {
        let mut conn = self.conn.lock();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;
        let mut preserved = None;
        if let Some(pending) = self.pending_source(&tx)? {
            self.validate_identity(&tx)?;
            let mut story = self.open_pending_source(&pending)?;
            if story.source() != pending.before && story.source() != pending.after {
                let copy = self
                    .path
                    .with_extension(format!("conflict-{}.sf", Uuid::new_v4()));
                story.save_copy(&copy)?;
                preserved = Some(copy);
                story.set_source(pending.after);
                story.save()?;
            }
        }
        self.recover_source_in(&tx)?;
        tx.commit().map_err(|source| StoreError::Save {
            path: self.path.clone(),
            source,
        })?;
        Ok(preserved)
    }

    fn persist_registry(
        &self,
        registry: &ArtifactRegistry,
        expected: Option<i64>,
    ) -> Result<i64, StoreError> {
        self.recover_source()?;
        self.write_snapshot(&self.prepare_registry(registry)?, expected, None)
    }

    fn validate_identity(&self, conn: &Connection) -> Result<(), StoreError> {
        if let Some(folder) = crate::project::for_database(&self.path)? {
            if single_project_id(conn)? != folder.id().to_string() {
                return Err(StoreError::Corrupt {
                    path: self.path.clone(),
                    column: "projects.id".into(),
                    value: "Manifest and database project identities differ".into(),
                });
            }
        }
        Ok(())
    }

    fn prepare_registry(
        &self,
        registry: &ArtifactRegistry,
    ) -> Result<ArtifactRegistry, StoreError> {
        self.validate_identity(&self.conn.lock())?;
        Ok(crate::project::prepare_registry(&self.path, registry)?)
    }

    /// Used only when adopting a saved project into a new portable folder.
    pub(crate) fn set_project_id(&self, id: Uuid) -> Result<(), StoreError> {
        self.conn
            .lock()
            .execute("UPDATE projects SET id = ?1", [id.to_string()])
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;
        Ok(())
    }

    /// Renames the `projects` row (used at creation, and by any future
    /// rename UI).
    pub fn rename_project(&self, name: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE projects SET name = ?1, updated_at = ?2",
            params![name, unix_now()],
        )
        .map_err(|source| StoreError::Save {
            path: self.path.clone(),
            source,
        })?;
        Ok(())
    }

    fn write_snapshot(
        &self,
        registry: &ArtifactRegistry,
        expected: Option<i64>,
        source_edit: Option<SourceEdit<'_>>,
    ) -> Result<i64, StoreError> {
        crate::authoring::validate_associations(registry).map_err(|value| StoreError::Corrupt {
            path: self.path.clone(),
            column: "projects.story_scenes_json".into(),
            value,
        })?;
        let story_scenes_json =
            serde_json::to_string(&registry.story_scenes).map_err(|source| {
                StoreError::Serialize {
                    path: self.path.clone(),
                    source,
                }
            })?;
        let mut conn = self.conn.lock();
        let project_id = single_project_id(&conn)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;
        let changed = tx.execute(
            "UPDATE projects SET registry_version = registry_version + 1 WHERE ?1 IS NULL OR registry_version = ?1",
            params![expected],
        ).map_err(|source| StoreError::Save { path: self.path.clone(), source })?;
        if changed != 1 {
            return Err(StoreError::Conflict);
        }
        let version = tx
            .query_row("SELECT registry_version FROM projects", [], |row| {
                row.get(0)
            })
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;
        // FKs are checked at commit, so the circular artifacts ↔ revisions
        // references can be inserted in any order within this transaction.
        tx.execute_batch("PRAGMA defer_foreign_keys = ON;")
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;
        for table in [
            "layers",
            "revisions",
            "artifacts",
            "lora_registry",
            "operation_log",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])
                .map_err(|source| StoreError::Save {
                    path: self.path.clone(),
                    source,
                })?;
        }
        let now = unix_now();
        tx.execute(
            "UPDATE projects SET updated_at = ?1, story_scenes_json = ?2",
            params![now, story_scenes_json],
        )
        .map_err(|source| StoreError::Save {
            path: self.path.clone(),
            source,
        })?;

        for artifact in &registry.artifacts {
            let drafts_json = serde_json::to_string(&artifact.drafts).map_err(|source| {
                StoreError::Serialize {
                    path: self.path.clone(),
                    source,
                }
            })?;
            tx.execute(
                "INSERT INTO artifacts (id, project_id, kind, name, description, \
                 variant_of_id, variant_axis, default_width, default_height, \
                 default_steps, default_seed, parent_id, \
                 composition_mode, active_revision_id, drafts_json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    artifact.id.to_string(),
                    project_id,
                    enum_to_db(artifact.kind),
                    artifact.name,
                    artifact.description,
                    artifact.variant_of.map(|id| id.to_string()),
                    artifact.variant_axis.map(enum_to_db),
                    artifact.default_size.map(|(width, _)| width as i64),
                    artifact.default_size.map(|(_, height)| height as i64),
                    artifact.default_steps.map(|steps| steps as i64),
                    artifact.default_seed.map(|seed| seed as i64),
                    artifact.parent_id.map(|id| id.to_string()),
                    artifact.composition.as_ref().map(|c| enum_to_db(c.mode)),
                    artifact.active_revision_id.map(|id| id.to_string()),
                    drafts_json,
                    now,
                    now,
                ],
            )
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;

            for revision in &artifact.revisions {
                tx.execute(
                    "INSERT INTO revisions (id, artifact_id, prompt, asset_path, status, \
                     seed, model, error, origin, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        revision.id.to_string(),
                        artifact.id.to_string(),
                        revision.prompt,
                        revision.asset_path,
                        enum_to_db(revision.status),
                        revision.seed.map(|seed| seed as i64),
                        revision.model,
                        revision.error,
                        enum_to_db(revision.origin),
                        now,
                    ],
                )
                .map_err(|source| StoreError::Save {
                    path: self.path.clone(),
                    source,
                })?;
            }

            if let Some(composition) = &artifact.composition {
                for layer in &composition.layers {
                    tx.execute(
                        "INSERT INTO layers (id, beat_id, position, artifact_ref, variant_ref, \
                         role, anchor) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            layer.id.to_string(),
                            artifact.id.to_string(),
                            layer.position as i64,
                            layer.artifact_ref.to_string(),
                            layer.variant_ref.map(|id| id.to_string()),
                            enum_to_db(layer.role),
                            layer.anchor,
                        ],
                    )
                    .map_err(|source| StoreError::Save {
                        path: self.path.clone(),
                        source,
                    })?;
                }
            }
        }

        for entry in &registry.log {
            let args =
                serde_json::to_string(&entry.op).map_err(|source| StoreError::Serialize {
                    path: self.path.clone(),
                    source,
                })?;
            tx.execute(
                "INSERT INTO operation_log (id, project_id, artifact_id, op, args, origin, \
                 status, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    entry.id.to_string(),
                    project_id,
                    entry.artifact_id.map(|id| id.to_string()),
                    op_name(&entry.op),
                    args,
                    enum_to_db(entry.origin),
                    enum_to_db(entry.status),
                    now,
                ],
            )
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;
        }

        for lora in &registry.loras {
            tx.execute(
                "INSERT INTO lora_registry (id, project_id, name, path, base_model, \
                 multiplier, artifact_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    lora.id.to_string(),
                    project_id,
                    lora.name,
                    lora.path,
                    lora.base_model,
                    lora.multiplier as f64,
                    lora.artifact_id.map(|id| id.to_string()),
                ],
            )
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;
        }

        if let Some(SourceEdit {
            document: story,
            include,
        }) = source_edit
        {
            let entry_path = source_path(&self.path);
            let (target, file) = match include {
                Some((entry, file)) if entry.path() == Some(entry_path.as_path()) => {
                    entry.check_disk()?;
                    (included_edit_path(&entry_path, file)?, Some(file))
                }
                Some(_) => return Err(StoryFileError::Unbound.into()),
                None => (entry_path, None),
            };
            if story.path() != Some(target.as_path()) {
                return Err(StoryFileError::Unbound.into());
            }
            story.check_disk()?;
            if story.is_dirty() {
                tx.execute(
                    "INSERT INTO source_write (id, before_source, after_source, source_file) VALUES (1, ?1, ?2, ?3)",
                    params![story.saved_source(), story.source(), file],
                )
                .map_err(|source| StoreError::Save {
                    path: self.path.clone(),
                    source,
                })?;
            }
        }
        tx.commit().map_err(|source| StoreError::Save {
            path: self.path.clone(),
            source,
        })?;
        Ok(version)
    }

    /// Loads the project row and the full registry snapshot.
    pub fn load(&self) -> Result<StoredProject, StoreError> {
        self.load_snapshot(SnapshotMode::Recover)
    }

    fn load_snapshot(&self, mode: SnapshotMode) -> Result<StoredProject, StoreError> {
        let mut connection = self.conn.lock();
        let behavior = match mode {
            SnapshotMode::Discovery => rusqlite::TransactionBehavior::Deferred,
            SnapshotMode::ReadAuthoring | SnapshotMode::Recover => {
                rusqlite::TransactionBehavior::Immediate
            }
        };
        let conn = connection
            // Source lives beside SQLite. Serialize its short snapshot read
            // with cooperating source publishers, including pending recovery.
            .transaction_with_behavior(behavior)
            .map_err(|source| StoreError::Load {
                path: self.path.clone(),
                source,
            })?;
        match mode {
            SnapshotMode::Recover => self.recover_source_in(&conn)?,
            SnapshotMode::ReadAuthoring if self.pending_source(&conn)?.is_some() => {
                return Err(StoreError::PendingSource);
            }
            SnapshotMode::ReadAuthoring | SnapshotMode::Discovery => {}
        }
        let (row_id, row_name, registry_version) = conn
            .query_row(
                "SELECT id, name, registry_version FROM projects ORDER BY rowid LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|source| StoreError::Load {
                path: self.path.clone(),
                source,
            })?
            .ok_or_else(|| StoreError::Corrupt {
                path: self.path.clone(),
                column: "projects".to_owned(),
                value: "<no project row>".to_owned(),
            })?;
        let project_id = uuid_from_db(&self.path, "projects.id", &row_id)?;

        let mut registry = ArtifactRegistry::default();

        // Artifacts, in creation order. Revisions and layers are attached
        // below once every row is in hand.
        let mut artifacts: Vec<Artifact> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, name, description, variant_of_id, variant_axis, \
                     default_width, default_height, default_steps, default_seed, \
                     parent_id, composition_mode, \
                     active_revision_id, drafts_json \
                     FROM artifacts ORDER BY rowid",
                )
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(ArtifactRow {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        description: row.get(3)?,
                        variant_of_id: row.get(4)?,
                        variant_axis: row.get(5)?,
                        default_width: row.get(6)?,
                        default_height: row.get(7)?,
                        default_steps: row.get(8)?,
                        default_seed: row.get(9)?,
                        parent_id: row.get(10)?,
                        composition_mode: row.get(11)?,
                        active_revision_id: row.get(12)?,
                        drafts_json: row.get(13)?,
                    })
                })
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            let mut artifacts = Vec::new();
            for row in rows {
                let row = row.map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
                let drafts: Vec<StoryDraft> =
                    serde_json::from_str(&row.drafts_json).map_err(|_source| {
                        StoreError::Corrupt {
                            path: self.path.clone(),
                            column: "artifacts.drafts_json".to_owned(),
                            value: row.drafts_json.clone(),
                        }
                    })?;
                artifacts.push(Artifact {
                    id: uuid_from_db(&self.path, "artifacts.id", &row.id)?,
                    kind: enum_from_db(&self.path, "artifacts.kind", &row.kind)?,
                    name: row.name,
                    description: row.description,
                    variant_of: row
                        .variant_of_id
                        .as_deref()
                        .map(|id| uuid_from_db(&self.path, "artifacts.variant_of_id", id))
                        .transpose()?,
                    variant_axis: row
                        .variant_axis
                        .as_deref()
                        .map(|axis| enum_from_db(&self.path, "artifacts.variant_axis", axis))
                        .transpose()?,
                    default_size: match (row.default_width, row.default_height) {
                        (Some(width), Some(height)) => Some((width as u32, height as u32)),
                        _ => None,
                    },
                    default_steps: row.default_steps.map(|steps| steps as u32),
                    default_seed: row.default_seed.map(|seed| seed as u64),
                    parent_id: row
                        .parent_id
                        .as_deref()
                        .map(|id| uuid_from_db(&self.path, "artifacts.parent_id", id))
                        .transpose()?,
                    composition: row
                        .composition_mode
                        .as_deref()
                        .map(|mode| {
                            Ok::<_, StoreError>(BeatComposition {
                                mode: enum_from_db(&self.path, "artifacts.composition_mode", mode)?,
                                layers: Vec::new(),
                            })
                        })
                        .transpose()?,
                    active_revision_id: row
                        .active_revision_id
                        .as_deref()
                        .map(|id| uuid_from_db(&self.path, "artifacts.active_revision_id", id))
                        .transpose()?,
                    revisions: Vec::new(),
                    drafts,
                });
            }
            artifacts
        };

        // Revisions, then layers grouped per revision.
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, artifact_id, prompt, asset_path, status, seed, model, error, \
                     origin FROM revisions ORDER BY rowid",
                )
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(RevisionRow {
                        id: row.get(0)?,
                        artifact_id: row.get(1)?,
                        prompt: row.get(2)?,
                        asset_path: row.get(3)?,
                        status: row.get(4)?,
                        seed: row.get(5)?,
                        model: row.get(6)?,
                        error: row.get(7)?,
                        origin: row.get(8)?,
                    })
                })
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            let mut revisions: Vec<RevisionRow> = Vec::new();
            for row in rows {
                revisions.push(row.map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?);
            }

            let artifact_index = index_by_id(&artifacts);
            for revision in &revisions {
                let revision_artifact_id =
                    uuid_from_db(&self.path, "revisions.artifact_id", &revision.artifact_id)?;
                let Some(artifact_index) = artifact_index.get(&revision_artifact_id) else {
                    return Err(StoreError::Corrupt {
                        path: self.path.clone(),
                        column: "revisions.artifact_id".to_owned(),
                        value: revision.artifact_id.clone(),
                    });
                };
                let artifact = &mut artifacts[*artifact_index];
                artifact.revisions.push(ArtifactRevision {
                    id: uuid_from_db(&self.path, "revisions.id", &revision.id)?,
                    prompt: revision.prompt.clone(),
                    asset_path: revision.asset_path.clone(),
                    status: enum_from_db(&self.path, "revisions.status", &revision.status)?,
                    seed: revision.seed.map(|seed| seed as u64),
                    model: revision.model.clone(),
                    error: revision.error.clone(),
                    origin: enum_from_db(&self.path, "revisions.origin", &revision.origin)?,
                });
            }
        }

        // Layers attach to beat compositions, ordered by position.
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, beat_id, position, artifact_ref, variant_ref, role, anchor \
                     FROM layers ORDER BY position, rowid",
                )
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(LayerRow {
                        id: row.get(0)?,
                        beat_id: row.get(1)?,
                        position: row.get(2)?,
                        artifact_ref: row.get(3)?,
                        variant_ref: row.get(4)?,
                        role: row.get(5)?,
                        anchor: row.get(6)?,
                    })
                })
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            let mut layers: Vec<LayerRow> = Vec::new();
            for row in rows {
                layers.push(row.map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?);
            }

            let artifact_index = index_by_id(&artifacts);
            for layer in &layers {
                let layer_beat_id = uuid_from_db(&self.path, "layers.beat_id", &layer.beat_id)?;
                let Some(index) = artifact_index.get(&layer_beat_id) else {
                    return Err(StoreError::Corrupt {
                        path: self.path.clone(),
                        column: "layers.beat_id".to_owned(),
                        value: layer.beat_id.clone(),
                    });
                };
                let composition =
                    artifacts[*index]
                        .composition
                        .get_or_insert_with(|| BeatComposition {
                            mode: CompositionMode::Baked,
                            layers: Vec::new(),
                        });
                composition.layers.push(Layer {
                    id: uuid_from_db(&self.path, "layers.id", &layer.id)?,
                    position: layer.position as u32,
                    artifact_ref: uuid_from_db(
                        &self.path,
                        "layers.artifact_ref",
                        &layer.artifact_ref,
                    )?,
                    variant_ref: layer
                        .variant_ref
                        .as_deref()
                        .map(|id| uuid_from_db(&self.path, "layers.variant_ref", id))
                        .transpose()?,
                    role: enum_from_db(&self.path, "layers.role", &layer.role)?,
                    anchor: layer.anchor.clone(),
                });
            }
        }

        // Operation log, in apply order.
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, artifact_id, args, origin, status \
                     FROM operation_log ORDER BY rowid",
                )
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(LogRow {
                        id: row.get(0)?,
                        artifact_id: row.get(1)?,
                        args: row.get(2)?,
                        origin: row.get(3)?,
                        status: row.get(4)?,
                    })
                })
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            for row in rows {
                let row = row.map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
                let operation: Operation =
                    serde_json::from_str(&row.args).map_err(|_| StoreError::Corrupt {
                        path: self.path.clone(),
                        column: "operation_log.args".to_owned(),
                        value: row.args.clone(),
                    })?;
                registry.log.push(OperationLogEntry {
                    id: uuid_from_db(&self.path, "operation_log.id", &row.id)?,
                    artifact_id: row
                        .artifact_id
                        .as_deref()
                        .map(|id| uuid_from_db(&self.path, "operation_log.artifact_id", id))
                        .transpose()?,
                    op: operation,
                    origin: enum_from_db(&self.path, "operation_log.origin", &row.origin)?,
                    status: enum_from_db(&self.path, "operation_log.status", &row.status)?,
                });
            }
        }

        // LoRA adapters (`lora_registry`), in attach order.
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, path, base_model, multiplier, artifact_id \
                     FROM lora_registry ORDER BY rowid",
                )
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(LoraRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        path: row.get(2)?,
                        base_model: row.get(3)?,
                        multiplier: row.get(4)?,
                        artifact_id: row.get(5)?,
                    })
                })
                .map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
            for row in rows {
                let row = row.map_err(|source| StoreError::Load {
                    path: self.path.clone(),
                    source,
                })?;
                registry.loras.push(LoraEntry {
                    id: uuid_from_db(&self.path, "lora_registry.id", &row.id)?,
                    name: row.name,
                    path: row.path,
                    base_model: row.base_model,
                    multiplier: row.multiplier as f32,
                    artifact_id: row
                        .artifact_id
                        .as_deref()
                        .map(|id| uuid_from_db(&self.path, "lora_registry.artifact_id", id))
                        .transpose()?,
                });
            }
        }

        registry.artifacts = artifacts;
        let story_scenes_json: String = conn
            .query_row("SELECT story_scenes_json FROM projects", [], |row| {
                row.get(0)
            })
            .map_err(|source| StoreError::Load {
                path: self.path.clone(),
                source,
            })?;
        registry.story_scenes =
            serde_json::from_str(&story_scenes_json).map_err(|error| StoreError::Corrupt {
                path: self.path.clone(),
                column: "projects.story_scenes_json".into(),
                value: error.to_string(),
            })?;
        crate::authoring::validate_associations(&registry).map_err(|value| {
            StoreError::Corrupt {
                path: self.path.clone(),
                column: "projects.story_scenes_json".into(),
                value,
            }
        })?;
        crate::project::resolve_registry(&self.path, project_id, &mut registry)?;
        let stored = StoredProject {
            registry_version,
            story: StoryDocument::open(&source_path(&self.path))?,
            id: project_id,
            name: row_name,
            registry,
        };
        conn.commit().map_err(|source| StoreError::Load {
            path: self.path.clone(),
            source,
        })?;
        Ok(stored)
    }

    fn configure(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| StoreError::Configure {
                path: self.path.clone(),
                source,
            })?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|source| StoreError::Configure {
                path: self.path.clone(),
                source,
            })?;
        conn.pragma_update(None, "busy_timeout", 5_000)
            .map_err(|source| StoreError::Configure {
                path: self.path.clone(),
                source,
            })?;
        Ok(())
    }

    /// Applies every migration with a version above the current schema
    /// version, recording each in `schema_meta`.
    fn migrate(&self) -> Result<(), StoreError> {
        let mut conn = self.conn.lock();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (\
               version INTEGER NOT NULL,\
               applied_at INTEGER NOT NULL\
             );",
        )
        .map_err(|source| StoreError::Migrate {
            path: self.path.clone(),
            source,
        })?;
        let current: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_meta",
                [],
                |row| row.get(0),
            )
            .map_err(|source| StoreError::Migrate {
                path: self.path.clone(),
                source,
            })?;
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            let version = (index + 1) as i64;
            if version <= current {
                continue;
            }
            // SQLite requires disabling FK enforcement outside the transaction
            // when rebuilding a referenced table. Validate all links before
            // committing; an error drops this unopened database connection.
            if version == 11 {
                conn.pragma_update(None, "foreign_keys", false)
                    .map_err(|source| StoreError::Migrate {
                        path: self.path.clone(),
                        source,
                    })?;
            }
            let tx = conn.transaction().map_err(|source| StoreError::Migrate {
                path: self.path.clone(),
                source,
            })?;
            if version == 9 {
                let draft: Option<String> = tx
                    .query_row(
                        "SELECT story_source FROM projects ORDER BY rowid LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|source| StoreError::Migrate {
                        path: self.path.clone(),
                        source,
                    })?;
                export_draft(
                    &source_path(&self.path),
                    draft.as_deref().unwrap_or_default(),
                )?;
            }
            tx.execute_batch(migration)
                .map_err(|source| StoreError::Migrate {
                    path: self.path.clone(),
                    source,
                })?;
            if version == 11 {
                let violation: Option<String> = tx
                    .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
                    .optional()
                    .map_err(|source| StoreError::Migrate {
                        path: self.path.clone(),
                        source,
                    })?;
                if let Some(table) = violation {
                    return Err(StoreError::Corrupt {
                        path: self.path.clone(),
                        column: table,
                        value: "Invalid foreign key during catalog migration".into(),
                    });
                }
            }
            tx.execute(
                "INSERT INTO schema_meta (version, applied_at) VALUES (?1, ?2)",
                params![version, unix_now()],
            )
            .map_err(|source| StoreError::Migrate {
                path: self.path.clone(),
                source,
            })?;
            tx.commit().map_err(|source| StoreError::Migrate {
                path: self.path.clone(),
                source,
            })?;
            if version == 11 {
                conn.pragma_update(None, "foreign_keys", true)
                    .map_err(|source| StoreError::Migrate {
                        path: self.path.clone(),
                        source,
                    })?;
            }
        }
        Ok(())
    }

    /// Ensures the database holds exactly one `projects` row (the schema
    /// requires `project_id` on every artifact).
    fn seed_project_row(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .map_err(|source| StoreError::Load {
                path: self.path.clone(),
                source,
            })?;
        if count == 0 {
            let name = self
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Untitled Story")
                .to_owned();
            let now = unix_now();
            conn.execute(
                "INSERT INTO projects (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params![Uuid::new_v4().to_string(), name, now, now],
            )
            .map_err(|source| StoreError::Save {
                path: self.path.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- schema v1

/// Schema version 1 — the §10 tables. Appending to `MIGRATIONS` with a new
/// `CREATE TABLE IF NOT EXISTS`/`ALTER TABLE` script migrates existing
/// databases; `schema_meta` records what has been applied.
const MIGRATIONS: &[&str] = &[
    SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8,
    SCHEMA_V9, SCHEMA_V10, SCHEMA_V11, SCHEMA_V12, SCHEMA_V13,
];

// A NULL path identifies the entry, including any pending v12 save.
const SCHEMA_V13: &str = "ALTER TABLE source_write ADD COLUMN source_file TEXT;";

const SCHEMA_V12: &str = "
CREATE TABLE source_write (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  before_source TEXT NOT NULL,
  after_source TEXT NOT NULL
);
";

/// Schema version 2 — the GUI carried the legacy `Project` model
/// (storyboard + timeline) as JSON on the `projects` row until the canvas
/// replaced it.
const SCHEMA_V2: &str = "ALTER TABLE projects ADD COLUMN project_json TEXT NOT NULL DEFAULT '';";

/// Schema version 3 — the canvas (roadmap §12 item 4) replaced the legacy
/// storyboard model; the column that carried it is dropped. This is the
/// sanctioned one-time loss: the format evolves in place, and no legacy
/// formats are parsed.
const SCHEMA_V3: &str = "ALTER TABLE projects DROP COLUMN project_json;";

/// Schema version 4 — artifacts carry an optional default generation size
/// (explicit `/create … --size WxH`, or `NULL` = the kind default).
const SCHEMA_V4: &str = "\
ALTER TABLE artifacts ADD COLUMN default_width INTEGER;\n\
ALTER TABLE artifacts ADD COLUMN default_height INTEGER;";

/// Schema version 5 — artifact-level generation defaults reachable from the
/// inspector: a step count and a pinned seed (`NULL` = reroll each run).
const SCHEMA_V5: &str = "\
ALTER TABLE artifacts ADD COLUMN default_steps INTEGER;\n\
ALTER TABLE artifacts ADD COLUMN default_seed INTEGER;";

/// Schema version 6 — a revision records whether it renders the artifact's
/// description or an experiment against it. Existing rows default to
/// `canon`, which is what they were before experiments were distinguished.
const SCHEMA_V6: &str = "\
ALTER TABLE revisions ADD COLUMN origin TEXT NOT NULL DEFAULT 'canon' \
  CHECK (origin IN ('canon','experiment'));";

/// Schema version 7 — the story source the Writer edits.
///
/// Version 9 exports this text to `.sf` and removes the column.
const SCHEMA_V7: &str = "\
ALTER TABLE projects ADD COLUMN story_source TEXT NOT NULL DEFAULT '';";

/// Schema version 8 — the mask/inpaint/composite edit path is gone; the mask
/// table and its index are dropped. Revisions no longer carry masks.
const SCHEMA_V8: &str = "\
DROP INDEX IF EXISTS idx_masks_revision;\n\
DROP TABLE IF EXISTS masks;";

/// The draft is exported by `migrate` before this SQL runs.
const SCHEMA_V9: &str = "ALTER TABLE projects DROP COLUMN story_source;";

/// Ordered compiler-name → artwork UUID associations, never a second source.
const SCHEMA_V10: &str =
    "ALTER TABLE projects ADD COLUMN story_scenes_json TEXT NOT NULL DEFAULT '[]';";

/// Shared catalog snapshots and independent illustrations. Rebuild rather than
/// altering SQLite's schema text, preserving every existing row and identity.
const SCHEMA_V11: &str = "
ALTER TABLE projects ADD COLUMN registry_version INTEGER NOT NULL DEFAULT 0 CHECK (registry_version >= 0);
CREATE TABLE artifacts_next (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  kind TEXT NOT NULL CHECK (kind IN ('story','scene','beat','character','environment','object','illustration')),
  name TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  variant_of_id TEXT REFERENCES artifacts(id),
  variant_axis TEXT CHECK (variant_axis IN ('outfit','age','body','hair','expression','time-of-day','weather','season','mood')),
  parent_id TEXT REFERENCES artifacts(id),
  composition_mode TEXT CHECK (composition_mode IN ('baked','layered','hybrid')),
  active_revision_id TEXT REFERENCES revisions(id),
  drafts_json TEXT NOT NULL DEFAULT '[]',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  default_width INTEGER,
  default_height INTEGER,
  default_steps INTEGER,
  default_seed INTEGER
);
INSERT INTO artifacts_next SELECT * FROM artifacts ORDER BY rowid;
DROP TABLE artifacts;
ALTER TABLE artifacts_next RENAME TO artifacts;
";

const SCHEMA_V1: &str = "
CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE artifacts (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  kind TEXT NOT NULL CHECK (kind IN
    ('story','scene','beat','character','environment','object')),
  name TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  variant_of_id TEXT REFERENCES artifacts(id),
  variant_axis TEXT CHECK (variant_axis IN
    ('outfit','age','body','hair','expression','time-of-day','weather','season','mood')),
  parent_id TEXT REFERENCES artifacts(id),
  composition_mode TEXT CHECK (composition_mode IN ('baked','layered','hybrid')),
  active_revision_id TEXT REFERENCES revisions(id),
  drafts_json TEXT NOT NULL DEFAULT '[]',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE revisions (
  id TEXT PRIMARY KEY,
  artifact_id TEXT NOT NULL REFERENCES artifacts(id),
  prompt TEXT NOT NULL,
  asset_path TEXT,
  status TEXT NOT NULL CHECK (status IN
    ('queued','generating','completed','failed','cancelled')),
  seed INTEGER,
  model TEXT,
  error TEXT,
  created_at INTEGER NOT NULL
);

CREATE TABLE layers (
  id TEXT PRIMARY KEY,
  beat_id TEXT NOT NULL REFERENCES artifacts(id),
  position INTEGER NOT NULL,
  artifact_ref TEXT NOT NULL REFERENCES artifacts(id),
  variant_ref TEXT REFERENCES artifacts(id),
  role TEXT NOT NULL CHECK (role IN ('backdrop','baked','dynamic')),
  anchor TEXT
);

CREATE TABLE lora_registry (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  base_model TEXT,
  multiplier REAL NOT NULL DEFAULT 1.0,
  artifact_id TEXT REFERENCES artifacts(id)
);

CREATE TABLE operation_log (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  artifact_id TEXT REFERENCES artifacts(id),
  op TEXT NOT NULL,
  args TEXT NOT NULL,
  origin TEXT NOT NULL CHECK (origin IN ('user','llm')),
  status TEXT NOT NULL CHECK (status IN ('proposed','applied','rejected','reverted')),
  created_at INTEGER NOT NULL
);

CREATE INDEX idx_revisions_artifact ON revisions(artifact_id);
CREATE INDEX idx_layers_beat ON layers(beat_id);
CREATE INDEX idx_operation_log_project ON operation_log(project_id);
";

// ------------------------------------------------------------ row plumbing

struct ArtifactRow {
    id: String,
    kind: String,
    name: String,
    description: String,
    variant_of_id: Option<String>,
    variant_axis: Option<String>,
    default_width: Option<i64>,
    default_height: Option<i64>,
    default_steps: Option<i64>,
    default_seed: Option<i64>,
    parent_id: Option<String>,
    composition_mode: Option<String>,
    active_revision_id: Option<String>,
    drafts_json: String,
}

struct RevisionRow {
    id: String,
    artifact_id: String,
    prompt: String,
    asset_path: Option<String>,
    status: String,
    seed: Option<i64>,
    model: Option<String>,
    error: Option<String>,
    origin: String,
}

struct LoraRow {
    id: String,
    name: String,
    path: String,
    base_model: Option<String>,
    multiplier: f64,
    artifact_id: Option<String>,
}

struct LayerRow {
    id: String,
    beat_id: String,
    position: i64,
    artifact_ref: String,
    variant_ref: Option<String>,
    role: String,
    anchor: Option<String>,
}

struct LogRow {
    id: String,
    artifact_id: Option<String>,
    args: String,
    origin: String,
    status: String,
}

/// Maps artifact id → index into `artifacts` (created in `ArtifactRegistry`
/// order, which the load query preserves).
fn index_by_id(artifacts: &[Artifact]) -> std::collections::HashMap<Uuid, usize> {
    artifacts
        .iter()
        .enumerate()
        .map(|(index, artifact)| (artifact.id, index))
        .collect()
}

fn single_project_id(conn: &Connection) -> Result<String, StoreError> {
    conn.query_row(
        "SELECT id FROM projects ORDER BY rowid LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .map_err(|source| StoreError::Load {
        path: PathBuf::new(),
        source,
    })
}

/// The serde tag name of an operation (single-sourced with the wire format).
fn op_name(operation: &Operation) -> &'static str {
    match operation {
        Operation::Create { .. } => "create",
        Operation::Variant { .. } => "variant",
        Operation::Regenerate { .. } => "regenerate",
        Operation::Resize { .. } => "resize",
        Operation::Set { .. } => "set",
        Operation::Compose { .. } => "compose",
        Operation::Draft { .. } => "draft",
        Operation::Modify { .. } => "modify",
        Operation::AttachLora { .. } => "attach-lora",
        Operation::LoraScale { .. } => "lora-scale",
        Operation::Describe { .. } => "describe",
        Operation::Revert { .. } => "revert",
        Operation::LoraRemove { .. } => "lora-remove",
    }
}

/// Encodes an enum as its serde wire string — the same value the domain
/// types serialize to, so the DB strings stay in sync with the format.
fn enum_to_db<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(value)) => value,
        _ => unreachable!("registry enums serialize to their wire string"),
    }
}

fn enum_from_db<T: DeserializeOwned>(
    path: &Path,
    column: &str,
    value: &str,
) -> Result<T, StoreError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(|_| {
        StoreError::Corrupt {
            path: path.to_path_buf(),
            column: column.to_owned(),
            value: value.to_owned(),
        }
    })
}

fn uuid_from_db(path: &Path, column: &str, value: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(value).map_err(|_| StoreError::Corrupt {
        path: path.to_path_buf(),
        column: column.to_owned(),
        value: value.to_owned(),
    })
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{
        ops::{self, ExecuteOptions, OpOrigin},
        pipeline::RunOptions,
        ArtifactKind, ArtifactRevision, GenerationDefaults, RevisionStatus, StoryDraft,
        VariantAxis,
    };

    fn test_dir() -> PathBuf {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn temp_db() -> (PathBuf, ProjectDb) {
        let path = test_dir().join(format!("svs-db-test-{}.db", Uuid::new_v4()));
        let db = ProjectDb::open(&path).expect("open should create the database");
        (path, db)
    }

    fn clean_up(path: &Path) {
        // WAL leaves -wal/-shm siblings behind; remove all three.
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
        let _ = std::fs::remove_file(source_path(path));
    }

    /// A fake generation backend: writes a dummy image file (so later ops
    /// using the revision as a reference pass the pipeline's disk check)
    /// and returns its path.
    struct VisualBackend;
    impl crate::registry::pipeline::GenerationBackend for VisualBackend {
        fn generate(
            &self,
            request: &agentkit_ai::native::sd::GenerateRequest,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<
                crate::registry::pipeline::GenerateResponse,
                crate::registry::pipeline::PipelineError,
            >,
        > {
            let request = request.clone();
            Box::pin(async move {
                let path = test_dir().join(format!("svs-persist-gen-{}.png", Uuid::new_v4()));
                std::fs::write(&path, b"png").expect("dummy image should write");
                Ok(crate::registry::pipeline::GenerateResponse {
                    status: "completed".to_owned(),
                    image_path: Some(path.display().to_string()),
                    seed: request.seed,
                    width: Some(request.width),
                    height: Some(request.height),
                    error: None,
                })
            })
        }
        fn llm_draft(
            &self,
            _request: &str,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<Option<String>, crate::registry::pipeline::PipelineError>,
        > {
            unreachable!()
        }
    }

    fn apply(registry: &mut ArtifactRegistry, operation: ops::Operation) {
        let backend = VisualBackend;
        // Each apply gets its own lora dir with a dummy adapter so the
        // LoRA ops resolve their file check.
        let lora_dir = test_dir().join(format!("svs-db-test-loras-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&lora_dir).expect("lora dir should create");
        std::fs::write(lora_dir.join("mia-v2.safetensors"), b"lora")
            .expect("dummy lora should write");
        let defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 25,
            cfg_scale: agentkit_ai::native::sd::DEFAULT_CFG_SCALE,
            sample_method: agentkit_ai::native::sd::DEFAULT_SAMPLE_METHOD.to_owned(),
            loras: crate::registry::LoraBindings {
                dir: lora_dir,
                ..Default::default()
            },
        };
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions::new(test_dir()),
            generation: Default::default(),
            defaults: &defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        tokio::runtime::Runtime::new()
            .expect("runtime should start")
            .block_on(ops::execute(registry, &operation, &options))
            .expect("operation should apply");
    }

    /// A registry exercising every §10 table: variants, a composed beat
    /// with layers, revisions, drafts, and the operation log.
    fn sample_registry() -> ArtifactRegistry {
        let mut registry = ArtifactRegistry::default();
        apply(
            &mut registry,
            ops::Operation::Create {
                kind: ArtifactKind::Story,
                description: "The Lighthouse".to_owned(),
                name: Some("story".to_owned()),
                size: None,
                steps: None,
                loras: Vec::new(),
            },
        );
        apply(
            &mut registry,
            ops::Operation::Create {
                kind: ArtifactKind::Character,
                description: "Mia, a lighthouse keeper".to_owned(),
                name: Some("mia".to_owned()),
                size: Some((512, 768)),
                steps: None,
                loras: Vec::new(),
            },
        );
        apply(
            &mut registry,
            ops::Operation::Create {
                kind: ArtifactKind::Environment,
                description: "The kitchen at dusk".to_owned(),
                name: Some("kitchen".to_owned()),
                size: None,
                steps: None,
                loras: Vec::new(),
            },
        );
        apply(
            &mut registry,
            ops::Operation::Create {
                kind: ArtifactKind::Scene,
                description: "Act one".to_owned(),
                name: Some("act1".to_owned()),
                size: None,
                steps: None,
                loras: Vec::new(),
            },
        );
        apply(
            &mut registry,
            ops::Operation::Variant {
                target: crate::registry::Ref::new("mia".to_owned()),
                description: "in rain gear".to_owned(),
                name: None,
                axis: Some(VariantAxis::Outfit),
                size: None,
                steps: None,
                loras: Vec::new(),
            },
        );
        apply(
            &mut registry,
            ops::Operation::Compose {
                scene: crate::registry::Ref::new("act1".to_owned()),
                description: "Mia lights the lantern".to_owned(),
                background: Some(crate::registry::Ref::new("kitchen".to_owned())),
                layers: vec![ops::ComposeLayer {
                    artifact: crate::registry::Ref::new("mia".to_owned()),
                    variant: Some(crate::registry::Ref::new("mia-outfit".to_owned())),
                }],
            },
        );
        apply(
            &mut registry,
            ops::Operation::AttachLora {
                target: crate::registry::Ref::new("mia".to_owned()),
                lora: "mia-v2.safetensors".to_owned(),
                multiplier: Some(0.8),
            },
        );
        registry
    }

    #[test]
    fn empty_registry_round_trips() {
        let (path, db) = temp_db();
        db.save_registry(&ArtifactRegistry::default())
            .expect("save should succeed");
        let stored = db.load().expect("load should succeed");
        assert_eq!(stored.registry, ArtifactRegistry::default());
        assert!(!stored.name.is_empty(), "project row is seeded with a name");
        clean_up(&path);
    }

    #[test]
    fn populated_registry_round_trips() {
        let (path, db) = temp_db();
        let registry = sample_registry();
        db.save_registry(&registry).expect("save should succeed");
        let stored = db.load().expect("load should succeed");
        assert_eq!(stored.registry, registry, "save/load must be lossless");
        clean_up(&path);
    }

    #[test]
    fn revisions_and_drafts_survive() {
        let (path, db) = temp_db();
        let mut registry = sample_registry();
        let mia = registry
            .resolve(&crate::registry::Ref::new("mia".to_owned()))
            .expect("mia resolves");
        let mia = registry.artifact_mut(mia).expect("artifact exists");
        mia.revisions.push(ArtifactRevision {
            id: Uuid::new_v4(),
            prompt: "make it warmer".to_owned(),
            asset_path: Some("assets/generated/mia.png".to_owned()),
            status: RevisionStatus::Completed,
            seed: Some(42),
            model: Some("qwen-image-2.1".to_owned()),
            error: None,
            origin: crate::registry::RevisionOrigin::Experiment,
        });
        mia.active_revision_id = mia.revisions.last().map(|revision| revision.id);
        let story = registry
            .resolve(&crate::registry::Ref::new("story".to_owned()))
            .expect("story resolves");
        registry
            .artifact_mut(story)
            .expect("artifact exists")
            .drafts
            .push(StoryDraft {
                id: Uuid::new_v4(),
                request: "write the opening".to_owned(),
                text: "The lamp is lit at dusk…".to_owned(),
                approved: true,
            });

        db.save_registry(&registry).expect("save should succeed");
        let stored = db.load().expect("load should succeed");
        assert_eq!(
            stored.registry, registry,
            "revisions/drafts must round-trip"
        );
        clean_up(&path);
    }

    #[test]
    fn project_id_is_stable_across_saves() {
        let (path, db) = temp_db();
        let first = db.load().expect("load should succeed");
        db.save_registry(&sample_registry())
            .expect("save should succeed");
        let second = db.load().expect("load should succeed");
        assert_eq!(first.id, second.id, "one project row per database");
        clean_up(&path);
    }

    #[test]
    fn reopening_migrates_idempotently() {
        let (path, db) = temp_db();
        db.save_registry(&sample_registry())
            .expect("save should succeed");
        drop(db);
        let reopened = ProjectDb::open(&path).expect("reopen should be idempotent");
        let stored = reopened.load().expect("load should succeed");
        assert_eq!(stored.registry.artifacts.len(), 6);
        clean_up(&path);
    }

    #[test]
    fn catalog_migration_preserves_the_populated_registry_graph_and_foreign_keys() {
        let (path, db) = temp_db();
        let registry = sample_registry();
        db.save_registry(&registry).unwrap();
        let original = db.load().unwrap();
        drop(db);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("ALTER TABLE projects DROP COLUMN registry_version; DROP TABLE source_write; DELETE FROM schema_meta WHERE version >= 11;").unwrap();
        drop(conn);
        let db = ProjectDb::open(&path).unwrap();
        let stored = db.load().unwrap();
        assert_eq!(stored.id, original.id);
        assert_eq!(stored.registry, original.registry);
        assert_eq!(stored.story, original.story);
        assert_eq!(stored.registry_version, 0);
        let conn = db.conn.lock();
        assert_eq!(
            conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(
            conn.execute("DELETE FROM artifacts", []).is_err(),
            "real revision/layer/variant foreign keys remain enforced"
        );
        drop(conn);
        db.save_registry_checked(&stored.registry, stored.registry_version)
            .unwrap();
        assert_eq!(db.load().unwrap().registry, stored.registry);
        clean_up(&path);
    }

    #[test]
    fn interrupted_authoring_publish_recovers_exact_source_with_its_catalog_and_never_replaces_unrelated_text(
    ) {
        let (path, db) = temp_db();
        let mut registry = ArtifactRegistry::default();
        let id = registry
            .create_artifact(
                crate::registry::ArtifactKind::Character,
                crate::registry::CreateArtifact {
                    name: "mara".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        db.save_registry(&registry).unwrap();
        let baseline = db.load().unwrap();
        let mut story = baseline.story.clone();
        let text = "// 🌊\r\nscene harbor\r\n  cast: captain_mara\r\n";
        story.set_source(text.into());
        registry.artifact_mut(id).unwrap().name = "captain_mara".into();
        // Simulate process exit after the catalog/intent commit and before the
        // source publish. This is the actual production transaction boundary.
        db.write_snapshot(
            &registry,
            Some(baseline.registry_version),
            Some(SourceEdit {
                document: &story,
                include: None,
            }),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(source_path(&path)).unwrap(),
            baseline.story.source()
        );
        let inspected = ProjectDb::inspect_existing(&path).unwrap();
        assert_eq!(inspected.story.source(), baseline.story.source());
        assert_eq!(
            inspected.registry.artifact(id).unwrap().name,
            "captain_mara"
        );
        assert!(db.pending_source(&db.conn.lock()).unwrap().is_some());
        assert!(matches!(
            ProjectDb::read_authoring_snapshot(&path),
            Err(StoreError::PendingSource)
        ));
        assert_eq!(
            std::fs::read_to_string(source_path(&path)).unwrap(),
            baseline.story.source()
        );
        std::fs::write(source_path(&path), "unrelated external draft").unwrap();
        assert!(matches!(
            db.load(),
            Err(StoreError::Story(StoryFileError::Conflict { .. }))
        ));
        assert_eq!(
            std::fs::read_to_string(source_path(&path)).unwrap(),
            "unrelated external draft"
        );
        assert!(db.pending_source(&db.conn.lock()).unwrap().is_some());
        std::fs::write(source_path(&path), baseline.story.source()).unwrap();
        // Simulate opening a v12 database that already has a pending entry save.
        db.conn.lock().execute_batch(
            "ALTER TABLE source_write DROP COLUMN source_file; DELETE FROM schema_meta WHERE version = 13;",
        ).unwrap();
        drop(db);
        let reopened = ProjectDb::open_existing(&path).unwrap();
        let stored = reopened.load().unwrap();
        assert_eq!(stored.story.source(), text);
        assert_eq!(stored.registry.artifact(id).unwrap().name, "captain_mara");
        assert!(reopened
            .pending_source(&reopened.conn.lock())
            .unwrap()
            .is_none());
        assert_eq!(reopened.load().unwrap(), stored);
        let mut next_source = stored.story.clone();
        next_source.set_source("// next accepted source\r\n".into());
        reopened
            .write_snapshot(
                &stored.registry,
                Some(stored.registry_version),
                Some(SourceEdit {
                    document: &next_source,
                    include: None,
                }),
            )
            .unwrap();
        std::fs::write(source_path(&path), "// external 🌊\r\n").unwrap();
        let preserved = reopened.preserve_source_and_recover().unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(&preserved).unwrap(),
            "// external 🌊\r\n"
        );
        assert_eq!(
            reopened.load().unwrap().story.source(),
            next_source.source()
        );
        assert!(reopened.preserve_source_and_recover().unwrap().is_none());
        std::fs::remove_file(preserved).unwrap();
        clean_up(&path);
    }

    #[test]
    fn interrupted_include_publish_recovers_only_the_intended_chapter() {
        let fixture = tempfile::tempdir_in(test_dir()).unwrap();
        let folder = crate::project::ProjectFolder::create(
            &fixture.path().join("story"),
            "Story",
            &ArtifactRegistry::default(),
            "include \"chapter.sf\"\n",
        )
        .unwrap();
        let db_path = folder.database();
        let db = ProjectDb::open_existing(&db_path).unwrap();
        let stored = db.load().unwrap();
        let entry = stored.story.path().unwrap();
        let path = entry.with_file_name("chapter.sf");
        let before = "scene original\n  end\n";
        std::fs::write(&path, before).unwrap();
        let mut chapter = StoryDocument::open(&path).unwrap();
        chapter.set_source("scene revised\n  cast: mara\n  end\n".into());
        let mut registry = stored.registry.clone();
        crate::catalog::reconcile_declarations(&mut registry, chapter.source(), false);

        std::fs::write(entry, "# External edit\ninclude \"chapter.sf\"\n").unwrap();
        assert!(matches!(
            db.save_included_authoring_checked(
                &registry,
                stored.registry_version,
                &stored.story,
                "chapter.sf",
                &mut chapter
            ),
            Err(StoreError::Story(StoryFileError::Conflict { .. }))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert!(db.pending_source(&db.conn.lock()).unwrap().is_none());
        std::fs::write(entry, stored.story.source()).unwrap();

        db.write_snapshot(
            &registry,
            Some(stored.registry_version),
            Some(SourceEdit {
                document: &chapter,
                include: Some((&stored.story, "chapter.sf")),
            }),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert!(matches!(
            ProjectDb::read_authoring_snapshot(&db_path),
            Err(StoreError::PendingSource)
        ));
        drop(db);
        let db = ProjectDb::open_existing(&db_path).unwrap();
        #[cfg(unix)]
        {
            let other = entry.with_file_name("other.sf");
            std::fs::rename(&path, &other).unwrap();
            std::os::unix::fs::symlink(&other, &path).unwrap();
            assert!(db.load().is_err());
            assert_eq!(std::fs::read_to_string(&other).unwrap(), before);
            std::fs::remove_file(&path).unwrap();
            std::fs::rename(&other, &path).unwrap();
        }
        std::fs::write(&path, "External chapter draft.").unwrap();
        assert!(matches!(
            db.load(),
            Err(StoreError::Story(StoryFileError::Conflict { .. }))
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "External chapter draft."
        );
        let preserved = db.preserve_source_and_recover().unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(preserved).unwrap(),
            "External chapter draft."
        );
        let recovered = db.load().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), chapter.source());
        assert_eq!(recovered.story, stored.story);
        assert_eq!(recovered.registry, registry);
        assert!(db.pending_source(&db.conn.lock()).unwrap().is_none());
    }

    #[test]
    fn failed_catalog_commit_does_not_publish_source_or_leave_a_source_intent() {
        let (path, db) = temp_db();
        let baseline = db.load().unwrap();
        let mut registry = baseline.registry.clone();
        let id = registry
            .create_artifact(
                crate::registry::ArtifactKind::Character,
                crate::registry::CreateArtifact {
                    name: "mara".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        registry.artifact_mut(id).unwrap().parent_id = Some(Uuid::new_v4());
        let mut story = baseline.story.clone();
        story.set_source("scene unpublished\n  cast: mara\n".into());
        assert!(db
            .save_authoring_checked(&registry, baseline.registry_version, &mut story)
            .is_err());
        assert!(story.is_dirty());
        assert_eq!(db.load().unwrap(), baseline);
        assert!(db.pending_source(&db.conn.lock()).unwrap().is_none());
        clean_up(&path);
    }

    #[test]
    fn schema_version_is_recorded() {
        let (_, db) = temp_db();
        let conn = db.conn.lock();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_meta", [], |row| row.get(0))
            .expect("schema_meta should exist");
        assert_eq!(version, MIGRATIONS.len() as i64);
    }

    #[test]
    fn rename_project_updates_the_row_name() {
        let (path, db) = temp_db();
        let original = db.load().expect("load should succeed");
        db.rename_project("The Lighthouse")
            .expect("rename should succeed");
        let stored = db.load().expect("load should succeed");
        assert_eq!(stored.name, "The Lighthouse");
        assert_eq!(stored.id, original.id, "row id stays stable");
        clean_up(&path);
    }

    #[test]
    fn v1_databases_migrate_to_v3_and_drop_the_legacy_column() {
        let path = test_dir().join(format!("svs-db-v1-{}.db", Uuid::new_v4()));
        {
            let conn = Connection::open(&path).expect("open should succeed");
            conn.execute_batch(SCHEMA_V1)
                .expect("v1 schema should apply");
            // A real v1 database records its version in schema_meta (the
            // runner creates the table, the migration records the row).
            conn.execute_batch(
                "CREATE TABLE schema_meta (version INTEGER NOT NULL, applied_at INTEGER NOT NULL);",
            )
            .expect("schema_meta should be created");
            conn.execute(
                "INSERT INTO schema_meta (version, applied_at) VALUES (1, 1)",
                [],
            )
            .expect("schema_meta row should insert");
            conn.execute(
                "INSERT INTO projects (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params![Uuid::new_v4().to_string(), "legacy v1", 1, 1],
            )
            .expect("project row should insert");
        }
        let db = ProjectDb::open(&path).expect("open should migrate to v3");
        let stored = db.load().expect("load should succeed");
        assert_eq!(stored.name, "legacy v1");
        assert!(stored.registry.artifacts.is_empty());
        {
            let conn = db.conn.lock();
            let columns: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'project_json'",
                    [],
                    |row| row.get(0),
                )
                .expect("pragma should read");
            assert_eq!(columns, 0, "project_json is dropped by v3");
        }
        clean_up(&path);
    }

    #[test]
    fn v2_databases_drop_the_legacy_storyboard_json_on_migration() {
        // A v2 database written by the pre-canvas GUI carries the storyboard
        // as JSON; opening it migrates to v3 and discards that column (the
        // canvas replaced the storyboard model — sanctioned one-time loss).
        let path = test_dir().join(format!("svs-db-v2-{}.db", Uuid::new_v4()));
        {
            let conn = Connection::open(&path).expect("open should succeed");
            conn.execute_batch(SCHEMA_V1)
                .expect("v1 schema should apply");
            conn.execute_batch(SCHEMA_V2)
                .expect("v2 schema should apply");
            conn.execute_batch(
                "CREATE TABLE schema_meta (version INTEGER NOT NULL, applied_at INTEGER NOT NULL);",
            )
            .expect("schema_meta should be created");
            conn.execute(
                "INSERT INTO schema_meta (version, applied_at) VALUES (2, 1)",
                [],
            )
            .expect("schema_meta row should insert");
            conn.execute(
                "INSERT INTO projects (id, name, project_json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    Uuid::new_v4().to_string(),
                    "legacy v2",
                    r#"{"id":"00000000-0000-0000-0000-000000000000","name":"legacy v2"}"#,
                    1,
                    1,
                ],
            )
            .expect("project row should insert");
        }
        let db = ProjectDb::open(&path).expect("open should migrate to v3");
        let stored = db.load().expect("load should succeed");
        assert_eq!(stored.name, "legacy v2");
        clean_up(&path);
    }

    #[test]
    fn corrupt_value_is_reported() {
        let (path, db) = temp_db();
        db.save_registry(&sample_registry())
            .expect("save should succeed");
        {
            let conn = db.conn.lock();
            // Enum columns are CHECK-guarded at the DB level, so the
            // corrupt-value path is exercised through `drafts_json`.
            conn.execute("UPDATE artifacts SET drafts_json = 'not json'", [])
                .expect("update should succeed");
        }
        let error = db.load().expect_err("load should fail on corrupt drafts");
        match error {
            StoreError::Corrupt { column, value, .. } => {
                assert_eq!(column, "artifacts.drafts_json");
                assert_eq!(value, "not json");
            }
            other => panic!("expected Corrupt, got {other}"),
        }
        clean_up(&path);
    }
}
