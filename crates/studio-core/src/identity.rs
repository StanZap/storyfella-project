//! One-time data migration, not a fallback runtime identity. Publish complete
//! copies under Studio's canonical namespace and retain the originals.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};
use directories::ProjectDirs;
use rusqlite::{Connection, OpenFlags};

pub(crate) fn migrate() -> Result<()> {
    let Some(old) = ProjectDirs::from("dev", "Smart Visual Sequencer", "Smart Visual Sequencer")
    else {
        return Ok(());
    };
    let Some(new) = crate::paths::project_dirs() else {
        return Ok(());
    };
    let mut mappings = Vec::new();
    for (old, new) in [
        (old.config_dir(), new.config_dir()),
        (old.data_dir(), new.data_dir()),
        (old.data_local_dir(), new.data_local_dir()),
        (old.cache_dir(), new.cache_dir()),
    ] {
        if !mappings.iter().any(|(source, _)| source == old) {
            mappings.push((old.to_owned(), new.to_owned()));
        }
    }
    migrate_roots(&mappings)
}

fn remap(path: &str, mappings: &[(PathBuf, PathBuf)]) -> String {
    let path_ref = Path::new(path);
    for (old, new) in mappings {
        if let Ok(relative) = path_ref.strip_prefix(old) {
            return new.join(relative).to_string_lossy().into_owned();
        }
    }
    path.to_owned()
}

fn migrate_roots(mappings: &[(PathBuf, PathBuf)]) -> Result<()> {
    for (old, new) in mappings {
        if !old.exists() || new.exists() {
            continue;
        }
        let parent = new
            .parent()
            .context("Application directory has no parent")?;
        fs::create_dir_all(parent)?;
        let staged = parent.join(format!(".studio-migration-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&staged)?;
        let result = copy_tree(old, &staged, mappings).and_then(|_| {
            // Another process may have completed migration while this copy ran.
            if !new.exists() {
                fs::rename(&staged, new)?;
            }
            Ok(())
        });
        if staged.exists() {
            fs::remove_dir_all(&staged)?;
        }
        result.with_context(|| {
            format!(
                "Copy {} to {}; original data was retained",
                old.display(),
                new.display()
            )
        })?;
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path, mappings: &[(PathBuf, PathBuf)]) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        ensure!(
            !kind.is_symlink(),
            "Resolve the symbolic link {} before migrating application data",
            from.display()
        );
        if kind.is_dir() {
            fs::create_dir(&to)?;
            copy_tree(&from, &to, mappings)?;
        } else if kind.is_file() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(database_name) = ["-wal", "-shm", "-journal"]
                .iter()
                .find_map(|suffix| name.strip_suffix(suffix))
            {
                let database = source.join(database_name);
                if database.is_file() && is_database(&database)? {
                    continue;
                }
            }
            if is_database(&from)? {
                // SQLite's backup API includes committed WAL pages consistently;
                // copying the database and sidecars as independent files cannot.
                let db = Connection::open_with_flags(&from, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                db.backup(rusqlite::MAIN_DB, &to, None)?;
                remap_database(&to, mappings)?;
            } else {
                fs::copy(&from, &to)?;
                if name == "recent-projects.json" {
                    // Preserve corrupt optional indexes for the existing UI
                    // recovery flow rather than blocking valid project data.
                    if let Ok(paths) = serde_json::from_slice::<Vec<String>>(&fs::read(&to)?) {
                        let paths: Vec<_> =
                            paths.iter().map(|path| remap(path, mappings)).collect();
                        fs::write(&to, serde_json::to_vec_pretty(&paths)?)?;
                    }
                } else if name == crate::paths::CONFIG_FILE {
                    let original = fs::read_to_string(&to)?;
                    let mut config: toml::Value = toml::from_str(&original)?;
                    if remap_config(&mut config, mappings) {
                        fs::write(&to, toml::to_string_pretty(&config)?)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn remap_config(value: &mut toml::Value, mappings: &[(PathBuf, PathBuf)]) -> bool {
    match value {
        toml::Value::String(text) => {
            let mapped = remap(text, mappings);
            if *text == mapped {
                false
            } else {
                *text = mapped;
                true
            }
        }
        toml::Value::Table(table) => table.iter_mut().fold(false, |changed, (_, item)| {
            remap_config(item, mappings) | changed
        }),
        toml::Value::Array(array) => array.iter_mut().fold(false, |changed, item| {
            remap_config(item, mappings) | changed
        }),
        _ => false,
    }
}

fn is_database(path: &Path) -> Result<bool> {
    let mut header = Vec::new();
    fs::File::open(path)?.take(16).read_to_end(&mut header)?;
    Ok(header == b"SQLite format 3\0")
}

fn remap_session(value: &mut serde_json::Value, mappings: &[(PathBuf, PathBuf)]) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, item) in object {
                if matches!(key.as_str(), "project" | "source_path") {
                    if let Some(path) = item.as_str() {
                        *item = remap(path, mappings).into();
                    }
                } else {
                    remap_session(item, mappings);
                }
            }
        }
        serde_json::Value::Array(array) => {
            for item in array {
                remap_session(item, mappings);
            }
        }
        _ => {}
    }
}

fn remap_database(path: &Path, mappings: &[(PathBuf, PathBuf)]) -> Result<()> {
    let mut db = Connection::open(path)?;
    let tx = db.transaction()?;
    // Only live resource references move; request/log provenance stays exact.
    for (table, column) in [("revisions", "asset_path"), ("lora_registry", "path")] {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get(0),
        )?;
        if !exists {
            continue;
        }
        let values = tx
            .prepare(&format!(
                "SELECT id, {column} FROM {table} WHERE {column} IS NOT NULL"
            ))?
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, before) in values {
            let after = remap(&before, mappings);
            if before != after {
                tx.execute(
                    &format!("UPDATE {table} SET {column} = ?1 WHERE id = ?2"),
                    [after, id],
                )?;
            }
        }
    }
    let sessions: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'writing_sessions')", [], |row| row.get(0))?;
    if sessions {
        let values = tx
            .prepare("SELECT source_path, snapshot FROM writing_sessions")?
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (before, snapshot) in values {
            let mut snapshot: serde_json::Value = serde_json::from_str(&snapshot)?;
            remap_session(&mut snapshot, mappings);
            tx.execute(
                "UPDATE writing_sessions SET source_path=?1, snapshot=?2 WHERE source_path=?3",
                [remap(&before, mappings), snapshot.to_string(), before],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite3_writing_sessions_keep_drafts_and_relocate_only_their_owner_paths() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old");
        let new = root.path().join("new");
        fs::create_dir(&old).unwrap();
        let source = old.join("story.sf").to_string_lossy().into_owned();
        let project = old.join("story.db").to_string_lossy().into_owned();
        let draft = format!("// Keep literal path {source}\r\n");
        let snapshot = serde_json::json!({"project":project,"source_path":source,"draft":draft,"request":{"project":project,"source_path":source,"source":draft}});
        let db = Connection::open(old.join("workflows.sqlite3")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE writing_sessions(source_path TEXT PRIMARY KEY, snapshot TEXT NOT NULL);").unwrap();
        db.execute(
            "INSERT INTO writing_sessions VALUES (?1, ?2)",
            [&source, &snapshot.to_string()],
        )
        .unwrap();
        migrate_roots(&[(old, new.clone())]).unwrap();
        let copied = Connection::open(new.join("workflows.sqlite3")).unwrap();
        let (key, json): (String, String) = copied
            .query_row(
                "SELECT source_path,snapshot FROM writing_sessions",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(key, new.join("story.sf").to_string_lossy());
        assert_eq!(json["source_path"], key);
        assert_eq!(
            json["request"]["project"],
            new.join("story.db").to_string_lossy().as_ref()
        );
        assert_eq!(json["draft"], draft);
        assert_eq!(json["request"]["source"], draft);
    }

    #[test]
    fn failed_migration_publishes_nothing_and_can_retry_without_losing_originals() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old");
        let new = root.path().join("new");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("studio.toml"), "[invalid").unwrap();
        let mappings = [(old.clone(), new.clone())];
        assert!(migrate_roots(&mappings).is_err());
        assert!(!new.exists());
        assert_eq!(
            fs::read_to_string(old.join("studio.toml")).unwrap(),
            "[invalid"
        );
        fs::write(old.join("studio.toml"), "# settings\n").unwrap();
        fs::write(old.join("recent-projects.json"), "corrupt index").unwrap();
        migrate_roots(&mappings).unwrap();
        assert_eq!(
            fs::read_to_string(new.join("studio.toml")).unwrap(),
            "# settings\n"
        );
        assert_eq!(
            fs::read_to_string(new.join("recent-projects.json")).unwrap(),
            "corrupt index"
        );
    }
    #[test]
    fn migration_preserves_wal_artwork_ids_settings_and_recents_without_replacing_existing_data() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old");
        let new = root.path().join("new");
        fs::create_dir_all(old.join("projects")).unwrap();
        fs::create_dir_all(old.join("assets")).unwrap();
        fs::write(old.join("assets/image.png"), b"exact image bytes").unwrap();
        fs::write(old.join("assets/drawing-wal"), b"ordinary user file").unwrap();
        let db_path = old.join("projects/story.db");
        let db = Connection::open(&db_path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE revisions(id TEXT, asset_path TEXT); CREATE TABLE lora_registry(id TEXT, path TEXT);").unwrap();
        db.execute(
            "INSERT INTO revisions VALUES ('stable-id', ?1)",
            [old.join("assets/image.png").to_str().unwrap()],
        )
        .unwrap();
        fs::write(old.join("projects/story.sf"), "// 🌊\r\nscene harbor\r\n").unwrap();
        fs::write(
            old.join("projects/recent-projects.json"),
            serde_json::to_vec(&[db_path]).unwrap(),
        )
        .unwrap();
        fs::write(
            old.join("studio.toml"),
            format!(
                "[paths]\nproject_dir = {:?}\n",
                old.join("projects").to_str().unwrap()
            ),
        )
        .unwrap();
        migrate_roots(&[(old.clone(), new.clone())]).unwrap();
        let migrated = Connection::open(new.join("projects/story.db")).unwrap();
        let (id, asset): (String, String) = migrated
            .query_row("SELECT id, asset_path FROM revisions", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(id, "stable-id");
        assert_eq!(fs::read(asset).unwrap(), b"exact image bytes");
        assert_eq!(
            fs::read(new.join("assets/drawing-wal")).unwrap(),
            b"ordinary user file"
        );
        assert_eq!(
            fs::read(new.join("projects/story.sf")).unwrap(),
            fs::read(old.join("projects/story.sf")).unwrap()
        );
        let recent: Vec<PathBuf> =
            serde_json::from_slice(&fs::read(new.join("projects/recent-projects.json")).unwrap())
                .unwrap();
        assert_eq!(recent, [new.join("projects/story.db")]);
        let config: toml::Value =
            toml::from_str(&fs::read_to_string(new.join("studio.toml")).unwrap()).unwrap();
        assert_eq!(
            config["paths"]["project_dir"].as_str(),
            new.join("projects").to_str()
        );
        fs::write(new.join("studio.toml"), "# current settings\n").unwrap();
        migrate_roots(&[(old.clone(), new.clone())]).unwrap();
        assert_eq!(
            fs::read_to_string(new.join("studio.toml")).unwrap(),
            "# current settings\n"
        );
        assert!(old.join("projects/story.db").is_file());
        assert_eq!(
            remap("/external/project.db", &[(old, new)]),
            "/external/project.db"
        );
    }
}
