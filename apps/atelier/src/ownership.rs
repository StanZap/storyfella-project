//! The shared drawing has one owner across desktop data directories and MCP.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use storyfella_studio_core::project::{ProjectFolder, MANIFEST};
use uuid::Uuid;

const LOCK: &str = ".storyfella-atelier.lock";
const RESERVATION: &str = ".storyfella-atelier-owner.json";

#[derive(Deserialize, Serialize)]
enum Reservation {
    Ready,
    Desktop { directory: PathBuf, epoch: Uuid },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct RecoveryIdentity {
    pub project: Uuid,
    epoch: Uuid,
}

#[derive(Clone, Debug)]
pub(crate) struct Owner {
    folder: ProjectFolder,
    lock: Arc<File>,
    desktop_epoch: Option<Uuid>,
}

/// Normalize aliases before identifying the one shared document. Other
/// .atelier files in the folder are independent copies.
pub(crate) fn target(path: &Path) -> Result<(PathBuf, Option<ProjectFolder>)> {
    let resolved = match path.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            match parent.canonicalize() {
                Ok(parent) => {
                    parent.join(path.file_name().context("Document filename is missing")?)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok((path.to_owned(), None))
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(error) => return Err(error.into()),
    };
    let root = drawing_root(path).or_else(|| {
        if resolved.join(MANIFEST).is_file()
            || resolved.file_name().is_some_and(|name| name == MANIFEST)
        {
            Some(resolved.as_path())
        } else {
            drawing_root(&resolved)
        }
    });
    if let Some(root) = root {
        let folder =
            ProjectFolder::locate_for_authoring(root)?.context("Shared project is missing")?;
        let path = folder.atelier_document()?;
        // The writable drawing and its parent must be at their designated locations.
        ensure!(
            path.parent()
                .context("Missing Atelier directory")?
                .canonicalize()?
                == folder.root().join("atelier")
                && fs::symlink_metadata(&path)
                    .map(|info| info.is_file())
                    .or_else(|error| {
                        if error.kind() == io::ErrorKind::NotFound {
                            Ok(true)
                        } else {
                            Err(error)
                        }
                    })?,
            "Shared Atelier drawing must use its original project location"
        );
        return Ok((path, Some(folder)));
    }
    Ok((resolved, None))
}

fn drawing_root(path: &Path) -> Option<&Path> {
    path.file_name()
        .filter(|name| *name == "main.atelier")
        .and_then(|_| path.parent())
        .filter(|parent| parent.file_name().is_some_and(|name| name == "atelier"))
        .and_then(Path::parent)
        .filter(|root| root.join(MANIFEST).is_file())
}

impl Owner {
    pub(crate) fn desktop(
        folder: ProjectFolder,
        data_dir: &Path,
        recovery: Option<RecoveryIdentity>,
    ) -> Result<Self> {
        let (mut owner, reservation) = Self::claim(folder)?;
        let directory = data_dir.canonicalize()?;
        if let Some(identity) = recovery {
            ensure!(
                identity.project == owner.folder.id()
                    && matches!(reservation, Some(Reservation::Desktop { directory: ref previous, epoch })
                        if *previous == directory && epoch == identity.epoch),
                "The shared drawing changed owners. Recovered edits must open as a separate copy"
            );
            owner.desktop_epoch = Some(identity.epoch);
        } else {
            ensure!(
                !matches!(reservation, Some(Reservation::Desktop { directory: ref previous, .. }) if *previous != directory),
                "Reopen the shared drawing in its original Atelier workspace, then Save and Close"
            );
            let epoch = Uuid::new_v4();
            owner.reserve(Reservation::Desktop { directory, epoch })?;
            owner.desktop_epoch = Some(epoch);
        }
        Ok(owner)
    }

    pub(crate) fn recovery_identity(&self) -> Option<RecoveryIdentity> {
        self.desktop_epoch.map(|epoch| RecoveryIdentity {
            project: self.folder.id(),
            epoch,
        })
    }

    pub(crate) fn headless(folder: ProjectFolder) -> Result<Self> {
        let (owner, reservation) = Self::claim(folder)?;
        match reservation {
            Some(Reservation::Ready) => {}
            Some(Reservation::Desktop { .. }) => bail!(
                "Open the drawing in its Atelier workspace and use Save and Close before connecting MCP"
            ),
            None => {
                ensure!(
                    !owner.folder.atelier_document()?.exists(),
                    "Use Save and Close in the updated Atelier before connecting to an existing drawing"
                );
                owner.reserve(Reservation::Ready)?;
            }
        }
        Ok(owner)
    }

    fn claim(folder: ProjectFolder) -> Result<(Self, Option<Reservation>)> {
        let path = folder.root().join(LOCK);
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = match options.clone().create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                ensure!(
                    fs::symlink_metadata(&path)?.is_file(),
                    "Invalid Atelier lock file"
                );
                options.open(&path)?
            }
            Err(error) => return Err(error.into()),
        };
        match lock.try_lock().map_err(io::Error::from) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                bail!("The shared drawing is open in Atelier or another MCP session")
            }
            Err(error) => return Err(error.into()),
        }
        let reservation_path = folder.root().join(RESERVATION);
        let reservation = match fs::symlink_metadata(&reservation_path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file() && metadata.len() <= 16 * 1024,
                    "Invalid Atelier recovery reservation"
                );
                Some(
                    serde_json::from_slice(&fs::read(&reservation_path)?)
                        .context("Invalid Atelier recovery reservation")?,
                )
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let owner = Self {
            folder,
            lock: Arc::new(lock),
            desktop_epoch: None,
        };
        owner.verify()?;
        Ok((owner, reservation))
    }

    pub(crate) fn verify(&self) -> Result<()> {
        let (_, current) = target(&self.folder.manifest())?;
        ensure!(
            current.is_some_and(
                |folder| folder.root() == self.folder.root() && folder.id() == self.folder.id()
            ) && same_file::Handle::from_file(self.lock.try_clone()?)?
                == same_file::Handle::from_path(self.folder.root().join(LOCK))?,
            "The shared drawing moved or was replaced. Reopen it to continue"
        );
        Ok(())
    }

    pub(crate) fn project_id(&self) -> uuid::Uuid {
        self.folder.id()
    }

    pub(crate) fn database(&self) -> PathBuf {
        self.folder.database()
    }

    pub(crate) fn release_desktop(&self) -> Result<()> {
        self.reserve(Reservation::Ready)
    }

    fn reserve(&self, reservation: Reservation) -> Result<()> {
        self.verify()?;
        crate::document::atomic_write(
            &self.folder.root().join(RESERVATION),
            &serde_json::to_vec(&reservation)?,
        )
    }
}

pub(crate) fn verify_write(path: &Path, owner: Option<&Owner>) -> Result<()> {
    let (_, folder) = target(path)?;
    if let Some(folder) = folder {
        let owner = owner.context("Open the shared drawing before saving to this location")?;
        ensure!(
            owner.folder.root() == folder.root() && owner.folder.id() == folder.id(),
            "Save As cannot replace another shared drawing"
        );
        owner.verify()?;
    }
    Ok(())
}
