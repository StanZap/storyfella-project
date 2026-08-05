//! SQLite persistence for the artifact registry + legacy storyboard
//! (`docs/ROADMAP.md` §10).
//!
//! The registry stays the in-memory source of truth for operations; the
//! database is a snapshot written after each applied operation. Each save
//! replaces the snapshot rows in one transaction (with
//! `PRAGMA defer_foreign_keys` so the circular `artifacts ↔ revisions`
//! references can be written in any order). `lora_registry` is schema-only
//! until the LoRA registry feature lands (§12 item 6).
//!
//! Deviations from the §10 sketch, all deliberate:
//! - Timestamps are unix epoch integers (`strftime('%s','now')`), not
//!   ISO-8601 text — no chrono dependency; `created_at`/`updated_at` are
//!   database metadata only (the in-memory model has no timestamps).
//! - `operation_log.id` is TEXT (the model uses `Uuid`), not
//!   AUTOINCREMENT; `op` holds the serde tag name and `args` the full
//!   serialized `Operation` JSON.
//! - `artifacts.composition_mode` (beat-only) and `artifacts.drafts_json`
//!   (the transitional home for pre-`.sf` story drafts) are not in the
//!   sketch; `revisions.error` and `revisions.seed`/`model` round out the
//!   model fields.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    models::Project,
    registry::{
        ops::{Operation, OperationLogEntry},
        Artifact, ArtifactRegistry, ArtifactRevision, BeatComposition, CompositionMode, Layer,
        StoredMask, StoryDraft,
    },
};

/// The `projects` row for a database, plus the full snapshot.
///
/// `id`/`name` mirror the `projects` row (stable per database file); the
/// legacy `Project` model (storyboard + timeline) is carried in
/// `project_json` for the GUI until the canvas replaces it.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredProject {
    pub id: Uuid,
    pub name: String,
    pub project: Project,
    pub registry: ArtifactRegistry,
}

/// SQLite-backed storage for one project (one registry per database).
///
/// Synchronous, single writer connection behind a mutex — the access shape
/// the desktop app needs (no async pool, no tokio involvement). WAL mode
/// so readers never block the writer and vice versa.
pub struct ProjectDb {
    conn: Mutex<Connection>,
    path: PathBuf,
}

#[derive(Debug, Error)]
pub enum StoreError {
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

impl ProjectDb {
    /// Opens (creating if necessary) the project database at `path`,
    /// migrates it to the latest schema, and seeds the single `projects`
    /// row on first creation.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open(&path).map_err(|source| StoreError::Open {
            path: path.clone(),
            source,
        })?;
        let db = Self {
            conn: Mutex::new(conn),
            path,
        };
        db.configure()?;
        db.migrate()?;
        db.seed_project_row()?;
        Ok(db)
    }

    /// Replaces the stored snapshot with `registry` in one transaction,
    /// leaving the `projects` row (and the legacy `project_json`) untouched.
    pub fn save_registry(&self, registry: &ArtifactRegistry) -> Result<(), StoreError> {
        self.write_snapshot(None, registry)
    }

    /// Replaces the stored snapshot with `project` + `registry` in one
    /// transaction (the GUI save path; also mirrors the project name on the
    /// `projects` row).
    pub fn save_project(
        &self,
        project: &Project,
        registry: &ArtifactRegistry,
    ) -> Result<(), StoreError> {
        self.write_snapshot(Some(project), registry)
    }

    fn write_snapshot(
        &self,
        project: Option<&Project>,
        registry: &ArtifactRegistry,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock();
        let project_id = single_project_id(&conn)?;
        let tx = conn.transaction().map_err(|source| StoreError::Save {
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
        for table in ["masks", "layers", "revisions", "artifacts", "operation_log"] {
            tx.execute(&format!("DELETE FROM {table}"), [])
                .map_err(|source| StoreError::Save {
                    path: self.path.clone(),
                    source,
                })?;
        }
        let now = unix_now();
        match project {
            Some(project) => {
                let project_json =
                    serde_json::to_string(project).map_err(|source| StoreError::Serialize {
                        path: self.path.clone(),
                        source,
                    })?;
                tx.execute(
                    "UPDATE projects SET name = ?1, project_json = ?2, updated_at = ?3",
                    params![project.name, project_json, now],
                )
                .map_err(|source| StoreError::Save {
                    path: self.path.clone(),
                    source,
                })?;
            }
            None => {
                tx.execute("UPDATE projects SET updated_at = ?1", [now])
                    .map_err(|source| StoreError::Save {
                        path: self.path.clone(),
                        source,
                    })?;
            }
        }

        for artifact in &registry.artifacts {
            let drafts_json = serde_json::to_string(&artifact.drafts).map_err(|source| {
                StoreError::Serialize {
                    path: self.path.clone(),
                    source,
                }
            })?;
            tx.execute(
                "INSERT INTO artifacts (id, project_id, kind, name, description, \
                 variant_of_id, variant_axis, parent_id, composition_mode, \
                 active_revision_id, drafts_json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    artifact.id.to_string(),
                    project_id,
                    enum_to_db(artifact.kind),
                    artifact.name,
                    artifact.description,
                    artifact.variant_of.map(|id| id.to_string()),
                    artifact.variant_axis.map(enum_to_db),
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
                     seed, model, error, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        revision.id.to_string(),
                        artifact.id.to_string(),
                        revision.prompt,
                        revision.asset_path,
                        enum_to_db(revision.status),
                        revision.seed.map(|seed| seed as i64),
                        revision.model,
                        revision.error,
                        now,
                    ],
                )
                .map_err(|source| StoreError::Save {
                    path: self.path.clone(),
                    source,
                })?;

                for mask in &revision.masks {
                    tx.execute(
                        "INSERT INTO masks (id, revision_id, asset_path, source, prompt, score) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            mask.id.to_string(),
                            revision.id.to_string(),
                            mask.asset_path,
                            enum_to_db(mask.source),
                            mask.prompt,
                            mask.score,
                        ],
                    )
                    .map_err(|source| StoreError::Save {
                        path: self.path.clone(),
                        source,
                    })?;
                }
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

        tx.commit().map_err(|source| StoreError::Save {
            path: self.path.clone(),
            source,
        })
    }

    /// Loads the project row, the legacy `Project` model, and the full
    /// registry snapshot.
    pub fn load(&self) -> Result<StoredProject, StoreError> {
        let conn = self.conn.lock();
        let (row_id, row_name, project_json) = conn
            .query_row(
                "SELECT id, name, project_json FROM projects ORDER BY rowid LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
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
        // CLI-created databases carry no legacy project; synthesize a fresh
        // one from the row so the GUI always has a valid `Project`.
        let project = if project_json.trim().is_empty() {
            Project {
                id: project_id,
                name: row_name.clone(),
                timeline: crate::timeline::Timeline::default(),
                storyboard: Vec::new(),
            }
        } else {
            serde_json::from_str(&project_json).map_err(|_| StoreError::Corrupt {
                path: self.path.clone(),
                column: "projects.project_json".to_owned(),
                value: project_json.clone(),
            })?
        };

        let mut registry = ArtifactRegistry::default();

        // Artifacts, in creation order. Revisions/masks/layers are attached
        // below once every row is in hand.
        let mut artifacts: Vec<Artifact> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, name, description, variant_of_id, variant_axis, \
                     parent_id, composition_mode, active_revision_id, drafts_json \
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
                        parent_id: row.get(6)?,
                        composition_mode: row.get(7)?,
                        active_revision_id: row.get(8)?,
                        drafts_json: row.get(9)?,
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

        // Revisions, then masks grouped per revision.
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, artifact_id, prompt, asset_path, status, seed, model, error \
                     FROM revisions ORDER BY rowid",
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

            let masks: std::collections::HashMap<String, Vec<MaskRow>> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, revision_id, asset_path, source, prompt, score \
                         FROM masks ORDER BY rowid",
                    )
                    .map_err(|source| StoreError::Load {
                        path: self.path.clone(),
                        source,
                    })?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(MaskRow {
                            id: row.get(0)?,
                            revision_id: row.get(1)?,
                            asset_path: row.get(2)?,
                            source: row.get(3)?,
                            prompt: row.get(4)?,
                            score: row.get(5)?,
                        })
                    })
                    .map_err(|source| StoreError::Load {
                        path: self.path.clone(),
                        source,
                    })?;
                let mut masks = std::collections::HashMap::<String, Vec<MaskRow>>::new();
                for row in rows {
                    let row = row.map_err(|source| StoreError::Load {
                        path: self.path.clone(),
                        source,
                    })?;
                    masks.entry(row.revision_id.clone()).or_default().push(row);
                }
                masks
            };

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
                    masks: masks
                        .get(&revision.id)
                        .map(|rows| {
                            rows.iter()
                                .map(|mask| {
                                    Ok::<_, StoreError>(StoredMask {
                                        id: uuid_from_db(&self.path, "masks.id", &mask.id)?,
                                        asset_path: mask.asset_path.clone(),
                                        source: enum_from_db(
                                            &self.path,
                                            "masks.source",
                                            &mask.source,
                                        )?,
                                        prompt: mask.prompt.clone(),
                                        score: mask.score,
                                    })
                                })
                                .collect::<Result<Vec<_>, _>>()
                        })
                        .transpose()?
                        .unwrap_or_default(),
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

        registry.artifacts = artifacts;
        Ok(StoredProject {
            id: project_id,
            name: row_name,
            project,
            registry,
        })
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
            let tx = conn.transaction().map_err(|source| StoreError::Migrate {
                path: self.path.clone(),
                source,
            })?;
            tx.execute_batch(migration)
                .map_err(|source| StoreError::Migrate {
                    path: self.path.clone(),
                    source,
                })?;
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
const MIGRATIONS: &[&str] = &[SCHEMA_V1, SCHEMA_V2];

/// Schema version 2 — the GUI carries the legacy `Project` model
/// (storyboard + timeline) as JSON on the `projects` row until the canvas
/// replaces it (roadmap §12 item 4).
const SCHEMA_V2: &str = "ALTER TABLE projects ADD COLUMN project_json TEXT NOT NULL DEFAULT '';";

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

CREATE TABLE masks (
  id TEXT PRIMARY KEY,
  revision_id TEXT NOT NULL REFERENCES revisions(id),
  asset_path TEXT NOT NULL,
  source TEXT NOT NULL CHECK (source IN ('auto','painted')),
  prompt TEXT,
  score REAL
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
CREATE INDEX idx_masks_revision ON masks(revision_id);
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
}

struct MaskRow {
    id: String,
    revision_id: String,
    asset_path: String,
    source: String,
    prompt: Option<String>,
    score: Option<f64>,
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
        Operation::Compose { .. } => "compose",
        Operation::Draft { .. } => "draft",
        Operation::Modify { .. } => "modify",
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
    use crate::{
        models::RevisionStatus,
        registry::{
            ops::{self, ExecuteOptions, OpOrigin},
            pipeline::RunOptions,
            ArtifactKind, ArtifactRevision, MaskSource, StoredMask, StoryDraft, VariantAxis,
        },
    };

    fn temp_db() -> (PathBuf, ProjectDb) {
        let path = std::env::temp_dir().join(format!("svs-db-test-{}.db", Uuid::new_v4()));
        let db = ProjectDb::open(&path).expect("open should create the database");
        (path, db)
    }

    fn clean_up(path: &Path) {
        // WAL leaves -wal/-shm siblings behind; remove all three.
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    fn apply(registry: &mut ArtifactRegistry, operation: ops::Operation) {
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions::new(std::env::temp_dir()),
            generation: Default::default(),
            manual_text: None,
            origin: OpOrigin::User,
        };
        tokio::runtime::Runtime::new()
            .expect("runtime should start")
            .block_on(ops::execute(registry, &operation, &options))
            .expect("operation should apply");
    }

    /// A registry exercising every §10 table: variants, a composed beat
    /// with layers, revisions with masks, drafts, and the operation log.
    fn sample_registry() -> ArtifactRegistry {
        let mut registry = ArtifactRegistry::default();
        apply(
            &mut registry,
            ops::Operation::Create {
                kind: ArtifactKind::Story,
                description: "The Lighthouse".to_owned(),
                name: Some("story".to_owned()),
            },
        );
        apply(
            &mut registry,
            ops::Operation::Create {
                kind: ArtifactKind::Character,
                description: "Mia, a lighthouse keeper".to_owned(),
                name: Some("mia".to_owned()),
            },
        );
        apply(
            &mut registry,
            ops::Operation::Create {
                kind: ArtifactKind::Environment,
                description: "The kitchen at dusk".to_owned(),
                name: Some("kitchen".to_owned()),
            },
        );
        apply(
            &mut registry,
            ops::Operation::Create {
                kind: ArtifactKind::Scene,
                description: "Act one".to_owned(),
                name: Some("act1".to_owned()),
            },
        );
        apply(
            &mut registry,
            ops::Operation::Variant {
                target: crate::registry::Ref::new("mia".to_owned()),
                description: "in rain gear".to_owned(),
                axis: Some(VariantAxis::Outfit),
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
    fn revisions_masks_and_drafts_survive() {
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
            model: Some("krea-2-q4".to_owned()),
            error: None,
            masks: vec![StoredMask {
                id: Uuid::new_v4(),
                asset_path: "assets/generated/mia-mask.png".to_owned(),
                source: MaskSource::Auto,
                prompt: Some("her hair".to_owned()),
                score: Some(0.92),
            }],
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
            "revisions/masks/drafts must round-trip"
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
    fn schema_version_is_recorded() {
        let (_, db) = temp_db();
        let conn = db.conn.lock();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_meta", [], |row| row.get(0))
            .expect("schema_meta should exist");
        assert_eq!(version, MIGRATIONS.len() as i64);
    }

    #[test]
    fn legacy_project_round_trips_through_save_project() {
        let (path, db) = temp_db();
        let mut project = Project {
            id: Uuid::new_v4(),
            name: "The Lighthouse".to_owned(),
            timeline: crate::timeline::Timeline::default(),
            storyboard: vec![crate::models::StoryboardFrame {
                id: Uuid::new_v4(),
                prompt: "A quiet lighthouse above a silver sea at dusk".to_owned(),
                asset_path: Some("assets/generated/frame.png".to_owned()),
                revisions: Vec::new(),
                active_revision_id: None,
            }],
        };
        let registry = sample_registry();
        db.save_project(&project, &registry)
            .expect("save should succeed");
        let stored = db.load().expect("load should succeed");
        assert_eq!(stored.project, project);
        assert_eq!(stored.registry, registry);
        assert_eq!(stored.name, project.name, "row name mirrors the project");

        project.name = "Renamed".to_owned();
        db.save_project(&project, &registry)
            .expect("save should succeed");
        let stored = db.load().expect("load should succeed");
        assert_eq!(stored.project.name, "Renamed");
        assert_eq!(
            stored.id,
            db.load().expect("load").id,
            "row id stays stable"
        );
        clean_up(&path);
    }

    #[test]
    fn v1_databases_migrate_to_v2_with_a_synthesized_project() {
        let path = std::env::temp_dir().join(format!("svs-db-v1-{}.db", Uuid::new_v4()));
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
        let db = ProjectDb::open(&path).expect("open should migrate to v2");
        let stored = db.load().expect("load should succeed");
        assert_eq!(
            stored.project.name, "legacy v1",
            "project synthesized from the row"
        );
        assert!(stored.project.storyboard.is_empty());

        // project_json now exists and save_project works on the migrated DB.
        let project = Project {
            id: Uuid::new_v4(),
            name: "Migrated".to_owned(),
            timeline: crate::timeline::Timeline::default(),
            storyboard: Vec::new(),
        };
        db.save_project(&project, &ArtifactRegistry::default())
            .expect("save_project should succeed on v2");
        let stored = db.load().expect("load should succeed");
        assert_eq!(stored.project.name, "Migrated");
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
