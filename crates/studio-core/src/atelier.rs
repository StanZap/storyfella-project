//! Local Studio → Atelier handoff. A per-data-directory OS lock identifies the
//! running receiver; atomic inbox files also support cold starts and retries.
use crate::{
    persistence::ProjectDb,
    project::ProjectFolder,
    registry::{ArtifactKind, ArtifactRegistry},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use uuid::Uuid;

const LIMIT: u64 = 262_144;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenRequest {
    pub version: u32,
    pub id: Uuid,
    pub project: PathBuf,
    pub project_id: Uuid,
    pub artifact_id: Uuid,
    pub context: String,
    pub description: String,
}

impl OpenRequest {
    pub fn prepare(database: &Path, source: &str, artifact_id: Uuid) -> Result<Self> {
        let folder = ProjectFolder::open(database.parent().context("Project folder is missing")?)?;
        ensure!(
            database.canonicalize()? == folder.database(),
            "Open this story as a shared project folder first"
        );
        let stored = ProjectDb::open_existing(folder.database())?.load()?;
        ensure!(
            stored.story.source() == source,
            "Save the current story before opening Atelier"
        );
        let context = story_context(&stored.registry, source, artifact_id)?;
        Ok(Self {
            version: 1,
            id: Uuid::new_v4(),
            project: folder.manifest(),
            project_id: folder.id(),
            artifact_id,
            context,
            description: stored
                .registry
                .artifact(artifact_id)
                .context("Item no longer exists")?
                .description
                .chars()
                .take(16_000)
                .collect(),
        })
    }

    pub fn validate(&self) -> Result<ProjectFolder> {
        ensure!(
            self.version == 1
                && self.context.len() <= LIMIT as usize / 2
                && self.description.len() <= 65_536,
            "Unsupported or oversized Atelier handoff"
        );
        let folder = ProjectFolder::open(&self.project)?;
        ensure!(
            folder.id() == self.project_id,
            "The handoff belongs to another project"
        );
        let stored = ProjectDb::open_existing(folder.database())?.load()?;
        let item = stored
            .registry
            .artifact(self.artifact_id)
            .context("The requested item no longer exists")?;
        ensure!(
            item.kind != ArtifactKind::Story,
            "Choose a character, place, prop, illustration or scene"
        );
        Ok(folder)
    }
}

fn story_context(registry: &ArtifactRegistry, source: &str, id: Uuid) -> Result<String> {
    let item = registry.artifact(id).context("Select an item first")?;
    ensure!(
        item.kind != ArtifactKind::Story,
        "Select a visual item or scene"
    );
    let catalog = crate::authoring::AssetCatalog::from_registry(registry);
    let analysis = storyfella_language::analyze(source, &catalog.assets);
    let id_text = id.to_string();
    let mut context = format!("{} ({})\n{}", item.name, item.kind, item.description);
    for scene in &analysis.document.story.scenes {
        let relevant = analysis.references.iter().any(|r| {
            r.asset_id.as_deref() == Some(id_text.as_str())
                && scene.span.start <= r.name.span.start
                && r.name.span.end <= scene.span.end
        }) || registry.story_scenes.iter().any(|s| {
            (s.artifact_id == id || s.beats.iter().any(|b| b.artifact_id == id))
                && matches!(&scene.kind, storyfella_language::NodeKind::Scene { name } if name.value == s.name)
        });
        if relevant {
            context.push_str("\n\n");
            context.push_str(&source[scene.span.start..scene.span.end]);
        }
    }
    Ok(context.chars().take(24_000).collect())
}

pub fn data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("STORYFELLA_ATELIER_DATA_DIR").filter(|p| !p.is_empty()) {
        return path.into();
    }
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(target_os = "macos")]
    let base = base.join("Library/Application Support");
    #[cfg(not(target_os = "macos"))]
    let base = base.join(".local/share");
    base.join("Atelier")
}

pub struct Mailbox {
    root: PathBuf,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandoffReply {
    Opened,
    Failed(String),
}
impl Mailbox {
    pub fn new(data: &Path) -> Self {
        Self {
            root: data.join("handoff"),
        }
    }
    fn setup(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    pub fn claim(&self) -> Result<Option<File>> {
        self.setup()?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("instance.lock"))?;
        match file.try_lock().map_err(std::io::Error::from) {
            Ok(()) => Ok(Some(file)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    pub fn send(&self, request: &OpenRequest) -> Result<()> {
        self.setup()?;
        let bytes = serde_json::to_vec(request)?;
        ensure!(
            bytes.len() as u64 <= LIMIT,
            "The story context is too large"
        );
        let temporary = self.root.join(format!("{}.tmp", request.id));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        fs::rename(
            temporary,
            self.root.join(format!("{time:032}-{}.request", request.id)),
        )?;
        Ok(())
    }
    pub fn take(&self) -> Result<Option<OpenRequest>> {
        if !self.root.exists() {
            return Ok(None);
        }
        let mut paths = fs::read_dir(&self.root)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "request"))
            .collect::<Vec<_>>();
        paths.sort();
        let Some(path) = paths.first() else {
            return Ok(None);
        };
        let result = (|| {
            ensure!(
                fs::symlink_metadata(path)?.is_file(),
                "Invalid handoff file"
            );
            let mut bytes = Vec::new();
            File::open(path)?.take(LIMIT + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() as u64 <= LIMIT, "Oversized handoff file");
            Ok(serde_json::from_slice(&bytes)?)
        })();
        fs::remove_file(path)?;
        result.map(Some)
    }
    pub fn complete(&self, id: Uuid, result: Result<()>) -> Result<()> {
        let message = match result {
            Ok(()) => HandoffReply::Opened,
            Err(error) => HandoffReply::Failed(format!("{error:#}")),
        };
        let temporary = self.root.join(format!("{id}.reply-tmp"));
        fs::write(&temporary, serde_json::to_vec(&message)?)?;
        fs::rename(temporary, self.root.join(format!("{id}.reply")))?;
        Ok(())
    }
    pub fn reply(&self, id: Uuid) -> Result<Option<HandoffReply>> {
        let path = self.root.join(format!("{id}.reply"));
        if !path.exists() {
            return Ok(None);
        }
        let message = serde_json::from_slice(&fs::read(&path)?)?;
        fs::remove_file(path)?;
        Ok(Some(message))
    }
    pub fn cancel(&self, id: Uuid) {
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(&format!("-{id}.request"))
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
}

pub fn send_and_launch(request: &OpenRequest) -> Result<()> {
    request.validate()?;
    let mailbox = Mailbox::new(&data_dir());
    let guard = mailbox.claim()?;
    mailbox.send(request)?;
    if guard.is_none() {
        return Ok(());
    }
    // Release before spawning: the child must be able to claim its lease even
    // when the OS launch call waits for the application to initialize.
    drop(guard);
    let result = launch();
    if result.is_err() {
        mailbox.cancel(request.id);
    }
    result
}

fn launch() -> Result<()> {
    if let Some(path) = std::env::var_os("STORYFELLA_ATELIER").filter(|p| !p.is_empty()) {
        return launch_path(Path::new(&path));
    }
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "macos")]
    {
        let sibling = exe
            .ancestors()
            .find(|path| path.extension().is_some_and(|ext| ext == "app"))
            .and_then(Path::parent)
            .map(|path| path.join("Atelier.app"));
        for path in sibling.into_iter().chain([
            crate::paths::root().join("dist/atelier/Atelier.app"),
            PathBuf::from("/Applications/Atelier.app"),
        ]) {
            if path.is_dir() {
                return launch_path(&path);
            }
        }
        let status = Command::new("open")
            .args(["-b", "com.storyfella.atelier"])
            .status()?;
        ensure!(status.success(), "Atelier could not be found. Build Atelier or set STORYFELLA_ATELIER to its application path");
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let sibling = exe
            .parent()
            .context("Executable directory is missing")?
            .join(if cfg!(windows) {
                "atelier.exe"
            } else {
                "atelier"
            });
        if sibling.is_file() {
            return launch_path(&sibling);
        }
        launch_path(Path::new("atelier"))
    }
}

fn launch_path(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    if path.extension().is_some_and(|ext| ext == "app") {
        let status = Command::new("open").arg("-a").arg(path).status()?;
        ensure!(
            status.success(),
            "Could not open Atelier at {}",
            path.display()
        );
        return Ok(());
    }
    Command::new(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Could not start Atelier at {}", path.display()))?;
    Ok(())
}
