use std::{fs, path::PathBuf};

use rusqlite::{params, Connection};
use storyfella_studio_core::{
    persistence::ProjectDb,
    registry::{ArtifactKind, ArtifactRegistry, CreateArtifact},
    story::{source_path, StoryDocument, StoryFileError},
};
use uuid::Uuid;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("story-files-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn source(&self, text: &str) -> PathBuf {
        let path = self.0.join("story.sf");
        fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn edits_save_exact_text_and_external_changes_block_overwrite() {
    let fixture = Fixture::new();
    let path = fixture.source("// 🌊\r\nscene harbor\r\n  narrate: café\r\n");
    let mut story = StoryDocument::open(&path).unwrap();
    assert!(!story.is_dirty());
    let edited = "// unfinished draft 🌊\r\nscene harbor\r\n   malformed indentation\r\n";
    story.set_source(edited.into());
    story.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), edited);
    assert!(!story.is_dirty());
    story.set_source("local edits".into());
    fs::write(&path, "external edits").unwrap();
    assert!(matches!(story.save(), Err(StoryFileError::Conflict { .. })));
    assert_eq!(story.source(), "local edits");
    assert!(story.is_dirty());
    assert_eq!(fs::read_to_string(&path).unwrap(), "external edits");
    story.reload().unwrap();
    assert_eq!(story.source(), "external edits");
    assert!(!story.is_dirty());
    assert_eq!(
        fs::read_dir(&fixture.0).unwrap().count(),
        1,
        "no staging files remain"
    );
}

#[test]
fn clean_save_refreshes_external_edits_and_failures_preserve_local_text() {
    let fixture = Fixture::new();
    let path = fixture.source("original");
    let mut story = StoryDocument::open(&path).unwrap();
    fs::write(&path, "from editor").unwrap();
    story.save().unwrap();
    assert_eq!(story.source(), "from editor");
    story.set_source("unsaved".into());
    fs::remove_file(&path).unwrap();
    assert!(matches!(story.save(), Err(StoryFileError::Read { .. })));
    assert!(story.reload().is_err());
    assert_eq!(story.source(), "unsaved");
    assert!(story.is_dirty());
    assert!(
        !path.exists(),
        "saving does not recreate an externally deleted file"
    );
    let mut draft = StoryDocument::default();
    draft.set_source("unsaved".into());
    assert!(matches!(draft.save(), Err(StoryFileError::Unbound)));
}

#[test]
fn invalid_utf8_is_reported_without_replacing_the_editor_text() {
    let fixture = Fixture::new();
    let path = fixture.source("original");
    let mut story = StoryDocument::open(&path).unwrap();
    fs::write(&path, [0xff]).unwrap();
    assert!(matches!(story.reload(), Err(StoryFileError::Read { .. })));
    assert_eq!(story.source(), "original");
}

#[test]
fn new_project_reads_existing_source_and_keeps_artifact_ids_across_moves() {
    let fixture = Fixture::new();
    let path = fixture.source("scene a\n  mara: Hello\n");
    let project_path = path.with_extension("db");
    let db = ProjectDb::open(&project_path).unwrap();
    let mut registry = ArtifactRegistry::default();
    let id = registry
        .create_artifact(
            ArtifactKind::Character,
            CreateArtifact {
                name: "mara".into(),
                ..Default::default()
            },
        )
        .unwrap();
    db.save_registry(&registry).unwrap();
    let stored = db.load().unwrap();
    assert_eq!(stored.story.source(), "scene a\n  mara: Hello\n");
    assert_eq!(stored.registry.artifacts[0].id, id);
    drop(db);
    let moved = fixture.0.join("moved");
    fs::create_dir(&moved).unwrap();
    fs::rename(&project_path, moved.join("story.db")).unwrap();
    fs::rename(&path, moved.join("story.sf")).unwrap();
    let moved_db = ProjectDb::open(moved.join("story.db")).unwrap();
    let reopened = moved_db.load().unwrap();
    assert_eq!(reopened.story.source(), stored.story.source());
    assert_eq!(reopened.registry, stored.registry);
    assert_eq!(
        reopened.story.path(),
        Some(moved.join("story.sf").canonicalize().unwrap().as_path())
    );
}

#[test]
fn missing_source_in_current_schema_is_an_error_not_an_empty_draft() {
    let fixture = Fixture::new();
    let project_path = fixture.0.join("story.db");
    let db = ProjectDb::open(&project_path).unwrap();
    fs::remove_file(source_path(&project_path)).unwrap();
    assert!(db.load().is_err());
    assert!(!source_path(&project_path).exists());
}

// Reconstruct the v8 Writer column from the current schema. Artifact tables
// are unchanged by v9; this avoids maintaining a second registry schema.
fn v8_project(path: &std::path::Path, source: &str) {
    let current = ProjectDb::open(path).unwrap();
    drop(current);
    fs::remove_file(source_path(path)).unwrap();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("ALTER TABLE projects ADD COLUMN story_source TEXT NOT NULL DEFAULT ''; DELETE FROM schema_meta WHERE version = 9;").unwrap();
    conn.execute("UPDATE projects SET story_source = ?1", params![source])
        .unwrap();
}

#[test]
fn saving_a_copy_preserves_conflicted_edits_and_never_overwrites_a_file() {
    let fixture = Fixture::new();
    let path = fixture.source("original");
    let mut story = StoryDocument::open(&path).unwrap();
    story.set_source("local draft 🌊\r\n".into());
    fs::write(&path, "external source").unwrap();
    let copy = fixture.0.join("copy.sf");
    story.save_copy(&copy).unwrap();
    assert_eq!(fs::read_to_string(&copy).unwrap(), story.source());
    assert_eq!(fs::read_to_string(&path).unwrap(), "external source");
    assert!(story.is_dirty());
    assert!(matches!(story.save(), Err(StoryFileError::Conflict { .. })));
    assert!(story.save_copy(&copy).is_err());
    assert!(matches!(
        story.save_copy(&fixture.0.join("missing/copy.sf")),
        Err(StoryFileError::Write { .. })
    ));
    assert_eq!(story.source(), "local draft 🌊\r\n");
}

#[test]
fn v8_draft_is_exported_before_the_column_is_removed() {
    let fixture = Fixture::new();
    let path = fixture.0.join("old.db");
    let text = "// 🌊\r\nscene draft\r\n   incomplete source\r\n";
    v8_project(&path, text);
    let db = ProjectDb::open(&path).unwrap();
    assert_eq!(db.load().unwrap().story.source(), text);
    assert_eq!(fs::read_to_string(source_path(&path)).unwrap(), text);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'story_source'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    assert_eq!(
        ProjectDb::open(&path)
            .unwrap()
            .load()
            .unwrap()
            .story
            .source(),
        text
    );
}

#[test]
fn conflicting_migration_keeps_both_copies_and_can_retry_after_reconciliation() {
    let fixture = Fixture::new();
    let path = fixture.0.join("old.db");
    v8_project(&path, "database draft");
    fs::write(source_path(&path), "external source").unwrap();
    assert!(ProjectDb::open(&path).is_err());
    assert_eq!(
        fs::read_to_string(source_path(&path)).unwrap(),
        "external source"
    );
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT story_source FROM projects", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "database draft"
    );
    assert_eq!(
        conn.query_row("SELECT MAX(version) FROM schema_meta", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        8
    );
    drop(conn);
    fs::write(source_path(&path), "database draft").unwrap();
    assert_eq!(
        ProjectDb::open(&path)
            .unwrap()
            .load()
            .unwrap()
            .story
            .source(),
        "database draft"
    );
}
