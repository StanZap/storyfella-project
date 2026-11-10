//! Private local framing for a stdio bridge to a desktop project owner.
use std::{
    fs::{self, DirBuilder},
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt},
        net::UnixStream as BlockingStream,
    },
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{mpsc, oneshot},
    task::JoinSet,
    time::timeout,
};
use uuid::Uuid;

use crate::project::ProjectFolder;

const SOCKET: &str = "writer.sock";
const MAX_FRAME: usize = 512 * 1024;
pub const MAX_CONNECTIONS: usize = 8;
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
pub const SCHEMA: u32 = 1;
pub const EDIT_BYTES: usize = 64 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectIdentity {
    id: Uuid,
    root_digest: [u8; 32],
}

impl From<&ProjectFolder> for ProjectIdentity {
    fn from(folder: &ProjectFolder) -> Self {
        Self {
            id: folder.id(),
            root_digest: Sha256::digest(folder.root().as_os_str().as_encoded_bytes()).into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request<C = Command> {
    pub schema: u32,
    pub project: ProjectIdentity,
    pub command: C,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Inspect {},
    Read {
        session: Uuid,
        #[serde(default)]
        offset: usize,
    },
    Edit {
        session: Uuid,
        expected_version: String,
        start: usize,
        end: usize,
        text: String,
        #[serde(default)]
        apply: bool,
    },
}

#[derive(Serialize, Deserialize)]
pub struct Failure {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
}

impl Failure {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            current_version: None,
        }
    }
}

pub type Reply = std::result::Result<Value, Failure>;

pub struct Call<C> {
    pub request: Request<C>,
    pub session: Uuid,
    pub epoch: Option<Uuid>,
    pub response: oneshot::Sender<(Option<Uuid>, Reply)>,
}

pub async fn serve<C: DeserializeOwned + Send + 'static>(
    directory: &Path,
    sender: mpsc::Sender<Call<C>>,
) -> Result<()> {
    let channel = Channel::create(directory)?;
    let listener = UnixListener::bind(channel.socket())?;
    let mut clients = JoinSet::new();
    loop {
        tokio::select! {
            client = listener.accept(), if clients.len() < MAX_CONNECTIONS => {
                let (stream, _) = client?;
                if channel.accepts(&stream) {
                    clients.spawn(connection(stream, sender.clone()));
                }
            }
            _ = clients.join_next(), if !clients.is_empty() => {}
            _ = sender.closed() => return Ok(()),
        }
    }
}

async fn connection<C: DeserializeOwned>(
    mut stream: UnixStream,
    sender: mpsc::Sender<Call<C>>,
) -> Result<()> {
    let session = Uuid::new_v4();
    let mut epoch = None;
    let mut header = [0; 4];
    while stream.read(&mut header[..1]).await? != 0 {
        let request = timeout(FRAME_TIMEOUT, async {
            stream.read_exact(&mut header[1..]).await?;
            let mut bytes = vec![0; frame_size(u32::from_be_bytes(header))?];
            stream.read_exact(&mut bytes).await?;
            Ok::<Request<C>, anyhow::Error>(serde_json::from_slice(&bytes)?)
        })
        .await??;
        let (response, reply) = oneshot::channel();
        // A connection permits one in-flight request. Closing it while the
        // owner prepares a command drops the reply and revokes pending work.
        let (next_epoch, result) = tokio::select! {
            biased;
            closed = stream.read(&mut header[..1]) => {
                closed?;
                return Ok(());
            }
            result = async {
                sender.send(Call { request, session, epoch, response }).await
                    .map_err(|_| anyhow::anyhow!("Desktop owner closed the channel"))?;
                Ok::<_, anyhow::Error>(reply.await?)
            } => result?,
        };
        epoch = next_epoch;
        timeout(FRAME_TIMEOUT, stream.write_all(&encode(&result)?)).await??;
    }
    Ok(())
}

/// Created by the desktop owner for this grant. Never reuse an existing directory.
struct Channel {
    directory: PathBuf,
    uid: u32,
}

impl Channel {
    fn create(path: &Path) -> Result<Self> {
        ensure!(
            path.is_absolute(),
            "Choose an absolute MCP channel directory"
        );
        DirBuilder::new()
            .mode(0o700)
            .create(path)
            .context("Create a new private MCP channel directory")?;
        let channel = Self {
            directory: path.to_owned(),
            uid: private_directory(path)?.uid(),
        };
        Ok(channel)
    }

    fn socket(&self) -> PathBuf {
        self.directory.join(SOCKET)
    }

    fn accepts(&self, peer: &UnixStream) -> bool {
        peer.peer_cred().is_ok_and(|cred| cred.uid() == self.uid)
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.socket());
        let _ = fs::remove_dir(&self.directory);
    }
}

fn private_directory(path: &Path) -> Result<fs::Metadata> {
    ensure!(
        path.is_absolute(),
        "Choose an absolute MCP channel directory"
    );
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && metadata.mode() & 0o777 == 0o700,
        "The MCP channel must be a private directory created by the desktop app"
    );
    Ok(metadata)
}

fn connect(path: &Path) -> Result<BlockingStream> {
    let directory = private_directory(path)?;
    let socket = path.join(SOCKET);
    let metadata = fs::symlink_metadata(&socket)?;
    ensure!(
        metadata.file_type().is_socket() && metadata.uid() == directory.uid(),
        "The desktop MCP socket is invalid"
    );
    Ok(BlockingStream::connect(socket)?)
}

pub struct Client {
    project: ProjectIdentity,
    stream: Mutex<Option<BlockingStream>>,
}

impl Client {
    pub fn connect(folder: &ProjectFolder, channel: &Path) -> Result<Self> {
        let stream = connect(channel)?;
        let timeout = Some(Duration::from_secs(30));
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
        Ok(Self {
            project: ProjectIdentity::from(folder),
            stream: Mutex::new(Some(stream)),
        })
    }

    pub fn call(&self, command: impl Serialize) -> Reply {
        let request = Request {
            schema: SCHEMA,
            project: self.project.clone(),
            command,
        };
        let disconnected = || {
            Failure::new(
            "owner_disconnected",
            "Reopen the project in the desktop app, reconnect this MCP client and inspect again before retrying.",
        )
        };
        // A rejected oversized request leaves the connection usable.
        let frame = encode(&request)
            .map_err(|_| Failure::new("invalid_arguments", "The desktop request is too large."))?;
        let mut stream = self.stream.lock().map_err(|_| disconnected())?;
        match exchange(stream.as_mut().ok_or_else(disconnected)?, &frame) {
            Ok(reply) => reply,
            Err(_) => {
                *stream = None;
                Err(disconnected())
            }
        }
    }
}

fn encode(message: &impl Serialize) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(message)?;
    let length = frame_size(u32::try_from(body.len())?)?;
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend((length as u32).to_be_bytes());
    frame.extend(body);
    Ok(frame)
}

fn frame_size(length: u32) -> Result<usize> {
    ensure!(
        length > 0 && length as usize <= MAX_FRAME,
        "Invalid desktop MCP frame size"
    );
    Ok(length as usize)
}

fn exchange<T: DeserializeOwned>(stream: &mut (impl Read + Write), frame: &[u8]) -> Result<T> {
    stream.write_all(frame)?;
    let mut size = [0; 4];
    stream.read_exact(&mut size)?;
    let mut bytes = vec![0; frame_size(u32::from_be_bytes(size))?];
    stream.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
