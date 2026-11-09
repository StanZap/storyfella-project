//! Project explorer filesystem operations. Never follow links out of the project.
use std::{
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use crate::state::AppState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub folder: bool,
}

/// The explorer is rooted beside the entry source.
pub fn list(source: &Path) -> io::Result<Vec<Entry>> {
    fn visit(
        root: &Path,
        dir: &Path,
        entry_name: &std::ffi::OsStr,
        depth: usize,
        result: &mut Vec<Entry>,
    ) -> io::Result<()> {
        let mut items = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
        items.sort_by_key(|item| {
            (
                !item.file_type().map(|kind| kind.is_dir()).unwrap_or(false),
                item.file_name(),
            )
        });
        for item in items {
            let kind = item.file_type()?;
            let name = item.file_name().to_string_lossy().into_owned();
            let internal_file = kind.is_file()
                && (name == "storyfella.toml"
                    || name.ends_with(".db")
                    || name.ends_with(".db-wal")
                    || name.ends_with(".db-shm"));
            let is_entry = depth == 0 && item.file_name() == entry_name;
            if (!is_entry && (name.starts_with('.') || internal_file))
                || (depth == 0 && reserved_folder(root, &name))
                || (!kind.is_dir() && !kind.is_file())
            {
                continue;
            }
            let path = item.path();
            let relative = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_str()
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Project path is not UTF-8")
                })?
                .replace(std::path::MAIN_SEPARATOR, "/");
            result.push(Entry {
                path: relative,
                name,
                depth,
                folder: kind.is_dir(),
            });
            if result.len() > 4096 || depth >= 64 {
                return Err(io::Error::other(
                    "Project folder is too large or deeply nested",
                ));
            }
            if kind.is_dir() {
                visit(root, &path, entry_name, depth + 1, result)?;
            }
        }
        Ok(())
    }
    let (Some(parent), Some(entry_name)) = (source.parent(), source.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Story source has no directory",
        ));
    };
    let root = canonical_root(parent)?;
    let mut result = Vec::new();
    visit(&root, &root, entry_name, 0, &mut result)?;
    Ok(result)
}

fn canonical_root(root: &Path) -> io::Result<PathBuf> {
    if !fs::symlink_metadata(root)?.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Project source directory must be a folder and cannot be a symbolic link",
        ));
    }
    root.canonicalize()
}

fn reserved_folder(root: &Path, name: &str) -> bool {
    matches!(name, "artwork" | "atelier")
        && root
            .join(storyfella_studio_core::project::MANIFEST)
            .is_file()
}

pub fn destination(root: &Path, folder: &str, name: &str) -> io::Result<PathBuf> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose a valid name and a folder inside this project",
        )
    };
    let root = canonical_root(root)?;
    if cfg!(windows) && (folder.contains(':') || name.contains(':')) {
        return Err(invalid());
    }
    let folder_path = Path::new(folder);
    if !folder.is_empty()
        && !folder_path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(invalid());
    }
    let mut parts = Path::new(name).components();
    if !matches!(parts.next(), Some(Component::Normal(part)) if part == name)
        || parts.next().is_some()
        || name.starts_with('.')
        || (folder.is_empty() && reserved_folder(&root, name))
        || folder_path
            .iter()
            .next()
            .and_then(|part| part.to_str())
            .is_some_and(|part| reserved_folder(&root, part))
    {
        return Err(invalid());
    }
    let mut parent = root;
    for part in folder_path {
        parent.push(part);
        if !fs::symlink_metadata(&parent)?.file_type().is_dir() {
            return Err(invalid());
        }
    }
    Ok(parent.join(name))
}

pub fn create_folder(root: &Path, folder: &str, name: &str) -> io::Result<()> {
    fs::create_dir(destination(root, folder, name)?)
}

pub fn import_file(root: &Path, folder: &str, source: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(source)?.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Drop files rather than folders or links",
        ));
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "File name is not UTF-8"))?;
    let target = destination(root, folder, name)?;
    copy_file_new(source, &target)
}

fn copy_file_new(source: &Path, target: &Path) -> io::Result<()> {
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    if let Err(error) = io::copy(&mut input, &mut output).and_then(|_| output.flush()) {
        drop(output);
        let _ = fs::remove_file(target);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn explorer_rejects_symbolic_link_directories() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("project-symlink-{}", uuid::Uuid::new_v4()));
        let outside = root.join("outside");
        let project = root.join("project");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&project).unwrap();
        std::os::unix::fs::symlink(&outside, project.join("assets")).unwrap();
        assert!(list(&project.join("assets/story.sf")).is_err());
        assert!(destination(&project, "assets", "test.sf").is_err());
        assert!(destination(&project.join("assets"), "", "test.sf").is_err());
        fs::create_dir(project.join("chapter")).unwrap();
        std::os::unix::fs::symlink(project.join("chapter"), project.join("alias")).unwrap();
        assert!(destination(&project, "alias", "test.sf").is_err());
        assert!(!outside.join("test.sf").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folders_and_imports_are_project_bound_and_never_overwrite() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("project-tree-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.png");
        fs::write(&source, b"image").unwrap();
        create_folder(&root, "", "chapters").unwrap();
        create_folder(&root, "chapters", "art").unwrap();
        create_folder(&root, "chapters", "empty").unwrap();
        assert!(list(&root.join("story.sf"))
            .unwrap()
            .iter()
            .any(|entry| entry.path == "chapters/empty" && entry.folder));
        import_file(&root, "chapters/art", &source).unwrap();
        assert!(import_file(&root, "chapters/art", &source).is_err());
        assert!(create_folder(&root, "../", "escape").is_err());
        assert!(create_folder(&root, "", "../escape").is_err());
        assert!(create_folder(&root, "", "bad-name/").is_err());
        assert!(list(&root.join("story.sf"))
            .unwrap()
            .iter()
            .any(|entry| entry.path == "chapters/art/source.png"));
        assert_eq!(
            fs::read(root.join("chapters/art/source.png")).unwrap(),
            b"image"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explorer_lists_only_assets_and_ignores_project_internals() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("project-internals-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::create_dir_all(root.join("artwork")).unwrap();
        fs::create_dir_all(root.join("atelier")).unwrap();
        for name in ["storyfella.toml", "story.db", "story.db-wal", "story.sf"] {
            fs::write(root.join(name), "").unwrap();
        }
        fs::write(root.join("assets/story.sf"), "").unwrap();
        let entries = list(&root.join("assets/story.sf")).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            ["story.sf"]
        );
        let source = root.join(".svs-project.sf");
        fs::write(&source, "").unwrap();
        fs::write(root.join(".hidden"), "").unwrap();
        let standalone = list(&source).unwrap();
        assert!(standalone
            .iter()
            .any(|entry| entry.path == ".svs-project.sf"));
        assert!(!standalone.iter().any(|entry| entry.path == ".hidden"));
        assert!(!standalone.iter().any(|entry| entry.path == "story.db"));
        fs::remove_dir_all(root).unwrap();
    }
}

/// Resolve an existing visible entry without allowing links outside the asset tree.
pub fn existing(root: &Path, relative: &str) -> io::Result<PathBuf> {
    let path = Path::new(relative);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid item"))?;
    let parent = path.parent().and_then(|p| p.to_str()).unwrap_or("");
    let target = destination(root, parent, name)?;
    let meta = fs::symlink_metadata(&target)?;
    if meta.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Links are not supported",
        ));
    }
    Ok(target)
}

pub fn duplicate(root: &Path, relative: &str) -> io::Result<()> {
    fn copy_tree(source: &Path, target: &Path) -> io::Result<()> {
        let kind = fs::symlink_metadata(source)?.file_type();
        if kind.is_file() {
            return copy_file_new(source, target);
        }
        if !kind.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Only regular files and folders can be duplicated",
            ));
        }
        fs::create_dir(target)?;
        let result = (|| {
            for item in fs::read_dir(source)? {
                let item = item?;
                copy_tree(&item.path(), &target.join(item.file_name()))?;
            }
            Ok(())
        })();
        if result.is_err() {
            fs::remove_dir_all(target)?;
        }
        result
    }
    let source = existing(root, relative)?;
    let parent = source
        .parent()
        .ok_or_else(|| io::Error::other("Missing parent"))?;
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let ext = source
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 1..10000 {
        let suffix = if n == 1 {
            " copy".to_owned()
        } else {
            format!(" copy {n}")
        };
        let target = parent.join(format!("{stem}{suffix}{ext}"));
        match target.symlink_metadata() {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return copy_tree(&source, &target)
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("Could not find an available name"))
}

fn is_source(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sf"))
}

fn guard_folder(path: &Path) -> io::Result<()> {
    let kind = fs::symlink_metadata(path)?.file_type();
    if kind.is_symlink() {
        return Err(io::Error::other("Links cannot be modified here"));
    }
    if kind.is_dir() {
        for entry in fs::read_dir(path)? {
            guard_folder(&entry?.path())?;
        }
    } else if is_source(path) {
        return Err(io::Error::other(
            "Rename or remove the story files in this folder individually first.",
        ));
    }
    Ok(())
}

fn guard_source(state: &AppState, root: &Path, relative: &str, target: &Path) -> io::Result<()> {
    let entry = state
        .story
        .path()
        .ok_or_else(|| io::Error::other("Open a story first"))?;
    if target == entry {
        return Err(io::Error::other(
            "The entry story file cannot be renamed or removed.",
        ));
    }
    if state.operation_in_progress()
        || state
            .additional_sources
            .as_ref()
            .and_then(|sources| sources.get(relative))
            .is_some_and(|doc| doc.is_dirty())
    {
        return Err(io::Error::other(
            "Save this file and finish any active operation first.",
        ));
    }

    let check = |file: &str, text: &str| {
        for line in text.lines() {
            let Some(include) = storyfella_language::parse_include(line, 0) else {
                continue;
            };
            let include = include.map_err(|error| {
                io::Error::other(format!(
                    "Fix the include in {file} first. {}",
                    error.message
                ))
            })?;
            let included = root.join(storyfella_compiler::include_path(file, &include.value));
            if included == target || included.canonicalize().is_ok_and(|path| path == target) {
                return Err(io::Error::other(format!(
                    "{file} includes this story file. Remove that include and save before trying again."
                )));
            }
        }
        Ok(())
    };

    let entry_name = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("Story filename is not UTF-8"))?;
    check(entry_name, state.story.source())?;
    if let Some(sources) = &state.additional_sources {
        for (file, text) in sources.overrides() {
            check(&file, &text)?;
        }
    }
    let mut directories = vec![(root.to_path_buf(), 0)];
    let mut scanned = 0;
    while let Some((directory, depth)) = directories.pop() {
        for item in fs::read_dir(directory)? {
            let item = item?;
            scanned += 1;
            if scanned > 4096 || depth >= 64 {
                return Err(io::Error::other(
                    "Too many project files to check source references safely.",
                ));
            }
            let kind = item.file_type()?;
            let path = item.path();
            if kind.is_dir() {
                if depth != 0
                    || !item
                        .file_name()
                        .to_str()
                        .is_some_and(|name| reserved_folder(root, name))
                {
                    directories.push((path, depth + 1));
                }
            } else if kind.is_file() && path != target && is_source(&path) {
                if item.metadata()?.len() > 4 * 1024 * 1024 {
                    return Err(io::Error::other(
                        "A story file is too large to check its includes safely.",
                    ));
                }
                let file = path
                    .strip_prefix(root)
                    .ok()
                    .and_then(Path::to_str)
                    .ok_or_else(|| io::Error::other("Story path is not UTF-8"))?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                check(&file, &fs::read_to_string(path)?)?;
            }
        }
    }
    Ok(())
}

pub fn rename(state: &mut AppState, root: &Path, relative: &str, name: &str) -> io::Result<()> {
    let source = existing(root, relative)?;
    let parent = Path::new(relative)
        .parent()
        .and_then(Path::to_str)
        .unwrap_or("");
    let target = destination(root, parent, name)?;
    if source == target {
        return Ok(());
    }
    let source_file = source.is_file() && is_source(&source);
    if source_file {
        if !storyfella_language::valid_source_path(name) {
            return Err(io::Error::other(
                "Use a valid story filename ending in .sf.",
            ));
        }
        guard_source(state, &canonical_root(root)?, relative, &source)?;
    } else {
        guard_folder(&source)?;
    }
    if source.is_dir() {
        if fs::symlink_metadata(&target).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "That name already exists",
            ));
        }
        fs::rename(source, target)?;
    } else {
        // Reserve the new name without replacing any file created concurrently.
        fs::hard_link(&source, &target)?;
        if let Err(error) = fs::remove_file(source) {
            fs::remove_file(target).map_err(|cleanup| {
                io::Error::other(format!(
                    "Could not finish the rename ({error}) or remove the extra link ({cleanup})."
                ))
            })?;
            return Err(error);
        }
    }
    if source_file {
        state.forget_source_file(relative);
    }
    Ok(())
}

pub fn remove(state: &mut AppState, root: &Path, relative: &str) -> io::Result<()> {
    let path = existing(root, relative)?;
    if path.is_file() && is_source(&path) {
        guard_source(state, &canonical_root(root)?, relative, &path)?;
        fs::remove_file(path)?;
        state.forget_source_file(relative);
    } else {
        guard_folder(&path)?;
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn copy_item(path: &Path) -> io::Result<()> {
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg("on run argv\nset the clipboard to POSIX file (item 1 of argv)\nend run")
        .arg(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("Could not copy item to clipboard"))
    }
}
#[cfg(not(target_os = "macos"))]
pub fn copy_item(_path: &Path) -> io::Result<()> {
    Err(io::Error::other(
        "Copying files is not available on this platform",
    ))
}

#[cfg(test)]
mod explorer_action_tests {
    use super::*;
    #[test]
    fn duplicate_preserves_contents_and_never_overwrites() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("studio-explorer-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("assets/folder")).unwrap();
        fs::write(root.join("assets/folder/note.txt"), "hello").unwrap();
        let assets = root.join("assets");
        duplicate(&assets, "folder").unwrap();
        duplicate(&assets, "folder").unwrap();
        assert_eq!(
            fs::read_to_string(root.join("assets/folder copy/note.txt")).unwrap(),
            "hello"
        );
        assert!(root.join("assets/folder copy 2/note.txt").exists());
        assert!(existing(&assets, "../outside").is_err());
        fs::write(root.join("assets/folder/story.sf"), "").unwrap();
        assert!(guard_folder(&root.join("assets/folder")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("note.txt", assets.join("folder/link")).unwrap();
            assert!(duplicate(&assets, "folder").is_err());
            assert!(!assets.join("folder copy 3").exists());
            assert!(assets.join("folder/note.txt").is_file());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_changes_protect_entry_drafts_disk_includes_and_existing_files() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("studio-source-rename-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let entry = root.join("story.sf");
        let chapter = "scene chapter\n  end\n";
        fs::write(&entry, "scene start\n  end\n").unwrap();
        fs::write(root.join("chapter.sf"), chapter).unwrap();
        let mut state = AppState::default();
        state.story = storyfella_studio_core::story::StoryDocument::open(&entry).unwrap();
        assert!(remove(&mut state, &root, "story.sf").is_err());
        state.set_story_source("include \"chapter.sf\"\n".into());
        assert!(rename(&mut state, &root, "chapter.sf", "next.sf")
            .unwrap_err()
            .to_string()
            .contains("story.sf includes"));
        state.set_story_source("scene start\n  end\n".into());
        fs::write(root.join("elsewhere.sf"), "include \"chapter.sf\"\n").unwrap();
        assert!(remove(&mut state, &root, "chapter.sf")
            .unwrap_err()
            .to_string()
            .contains("elsewhere.sf includes"));
        fs::write(root.join("elsewhere.sf"), "scene elsewhere\n  end\n").unwrap();
        state.open_additional_source("chapter.sf").unwrap();
        state
            .set_additional_source("chapter.sf", "unsaved text".into())
            .unwrap();
        assert!(remove(&mut state, &root, "chapter.sf").is_err());
        state
            .set_additional_source("chapter.sf", chapter.into())
            .unwrap();
        assert!(rename(&mut state, &root, "chapter.sf", "elsewhere.sf").is_err());
        assert_eq!(
            fs::read_to_string(root.join("chapter.sf")).unwrap(),
            chapter
        );
        rename(&mut state, &root, "chapter.sf", "next.sf").unwrap();
        assert!(!root.join("chapter.sf").exists());
        assert_eq!(fs::read_to_string(root.join("next.sf")).unwrap(), chapter);
        assert!(state.active_source.is_none());
        assert!(!state.can_undo());
        remove(&mut state, &root, "next.sf").unwrap();
        assert!(!root.join("next.sf").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
