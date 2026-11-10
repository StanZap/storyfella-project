//! A closed-project MCP owner for linked artwork.
use std::{
    collections::HashMap,
    fs,
    io::{Cursor, IsTerminal},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
};

use agentkit_mcp::rmcp::{
    self,
    model::{
        CallToolRequestParams, CallToolResult, Content, ErrorData, Implementation, JsonObject,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
    ServerHandler, ServiceExt,
};
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use storyfella_studio_core::{
    persistence::{ProjectDb, StoredProject},
    project::{AuthoringLease, ProjectFolder},
    registry::RevisionStatus,
};
use uuid::Uuid;

use crate::{
    document::{self, Document, Reference},
    native_images::NativeImages,
    ownership::{self, Owner},
    providers::{self, Request, Settings},
    queue::{JobState, Queue, QueuedRequest},
    shared_save::{self, SaveStage},
    worker::{self, Event},
    workflow::{Destination, Review},
};

const PROJECT: &str = "atelier_project_inspect";
const INSPECT: &str = "atelier_asset_inspect";
const GENERATE: &str = "atelier_candidate_generate";
const POLL: &str = "atelier_job_poll";
const CANCEL: &str = "atelier_job_cancel";
const PREVIEW: &str = "atelier_candidate_preview";
const PUBLISH: &str = "atelier_asset_publish";
const RECOVER: &str = "atelier_job_recover";
const THUMBNAIL: &str = "atelier_job_thumbnail";

#[derive(Parser)]
#[command(name = "atelier mcp")]
pub(crate) struct Args {
    /// Absolute path to the shared project authorized for this client.
    #[arg(long)]
    folder: PathBuf,
    /// Use the provider settings from this Atelier workspace.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Allow requests to the configured image provider.
    #[arg(long)]
    allow_generate: bool,
    /// Maximum number of recorded submissions for this project, across restarts.
    #[arg(long, requires = "allow_generate")]
    max_submissions: Option<usize>,
    /// Allow checked publication of reviewed artwork.
    #[arg(long)]
    allow_publish: bool,
    /// Inspect the open drawing through this private Atelier channel.
    #[arg(long, value_name = "DIRECTORY", conflicts_with_all = ["allow_generate", "allow_publish", "data_dir"])]
    atelier_channel: Option<PathBuf>,
}

#[derive(Parser)]
#[command(name = "atelier approve")]
pub(crate) struct Approve {
    #[arg(long)]
    folder: PathBuf,
    /// Review ID displayed with the candidate preview.
    #[arg(long)]
    review: Uuid,
}

pub(crate) fn approve(args: Approve) -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal(),
        "Run this command from the reviewer’s local terminal"
    );
    record_approval(args)
}

fn record_approval(args: Approve) -> Result<()> {
    let folder = folder(&args.folder)?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(approval_path(&folder, args.review))?
        .sync_all()?;
    println!("Approved review {} for this project.", args.review);
    Ok(())
}

fn approval_path(folder: &ProjectFolder, review: Uuid) -> PathBuf {
    folder.root().join(format!(".atelier-approval-{review}"))
}

fn folder(path: &Path) -> Result<ProjectFolder> {
    ensure!(
        path.is_absolute() && path.is_dir(),
        "Choose an absolute shared project folder"
    );
    ownership::target(path)?
        .1
        .context("Choose a shared project folder")
}

pub(crate) fn run(args: Args) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        if let Some(channel) = args.atelier_channel {
            #[cfg(unix)]
            return crate::live_mcp::run_client(folder(&args.folder)?, &channel).await;
            #[cfg(not(unix))]
            bail!(
                "Atelier's private MCP channel requires macOS or Linux: {}",
                channel.display()
            );
        }
        let tools = Tools::open(args)?;
        Server(Arc::new(Mutex::new(tools)))
            .serve(rmcp::transport::stdio())
            .await?
            .waiting()
            .await?;
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    #[serde(default)]
    offset: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    asset: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Generate {
    session: Uuid,
    asset: Uuid,
    expected_version: String,
    prompt: String,
    #[serde(default)]
    revise: bool,
    reference_revision: Option<Uuid>,
    size: Option<[u32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Poll {
    session: Uuid,
    job: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recover {
    session: Uuid,
    job: Uuid,
    expected_version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Checked {
    session: Uuid,
    asset: Uuid,
    expected_version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    session: Uuid,
    review: Uuid,
    #[serde(default)]
    agent_approval: bool,
}

struct Snapshot {
    doc: Document,
    version: String,
    name: String,
    revision: Option<Uuid>,
}

impl Snapshot {
    fn check(&self, version: &str) -> Result<()> {
        ensure!(
            self.version == version,
            "The drawing changed. Inspect it again"
        );
        Ok(())
    }

    fn review_version(&self) -> Result<String> {
        document::version(&(
            self.doc.current_canvas(),
            self.doc
                .artwork_bases
                .get(&self.doc.canvas_id)
                .copied()
                .flatten(),
        ))
    }
}

struct ReviewProposal {
    id: Uuid,
    version: String,
    approved: bool,
}

#[derive(Serialize, Deserialize)]
struct CapturedJob {
    project: Uuid,
    asset: Uuid,
    artboard: u32,
    version: String,
    size: [u32; 2],
    reference_revision: Option<Uuid>,
    provider: String,
    model: String,
}

struct Tools {
    folder: ProjectFolder,
    owner: Owner,
    authoring: AuthoringLease,
    path: PathBuf,
    jobs_dir: PathBuf,
    session: Uuid,
    allow_generate: bool,
    max_submissions: Option<usize>,
    allow_publish: bool,
    settings: Settings,
    data_dir: PathBuf,
    native: Arc<NativeImages>,
    queue: Queue,
    sender: mpsc::Sender<Event>,
    events: mpsc::Receiver<Event>,
    provider: fn(Request) -> Result<Reference>,
    active_job: Option<ActiveJob>,
    reviews: HashMap<Uuid, ReviewProposal>,
}

struct ActiveJob {
    queued: u64,
    id: Uuid,
    cancel: Arc<AtomicBool>,
}

impl Tools {
    fn open(args: Args) -> Result<Self> {
        let folder = folder(&args.folder)?;
        // This first slice works with both editors closed. Only Studio's
        // authoring open can recover a pending source save.
        let authoring = folder.claim_authoring()?;
        let owner = Owner::headless(folder.clone())?;
        let stored = ProjectDb::read_authoring_snapshot(&folder.database())?;
        ensure!(
            stored.id == folder.id(),
            "Manifest and database project identities differ"
        );
        let path = folder.atelier_document()?;
        let jobs_dir = folder.root().join("atelier/mcp-jobs");
        let data_dir = args
            .data_dir
            .unwrap_or_else(storyfella_studio_core::atelier::data_dir);
        let settings = match fs::read(data_dir.join("settings.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Invalid Atelier settings")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(error) => return Err(error.into()),
        };
        let (sender, events) = mpsc::channel();
        Ok(Self {
            folder,
            owner,
            authoring,
            path,
            settings,
            sender,
            events,
            data_dir,
            jobs_dir,
            session: Uuid::new_v4(),
            allow_generate: args.allow_generate,
            max_submissions: args.max_submissions,
            allow_publish: args.allow_publish,
            native: Arc::default(),
            queue: Queue::default(),
            provider: providers::generate,
            active_job: None,
            reviews: HashMap::new(),
        })
    }

    fn check_session(&self, session: Uuid) -> Result<()> {
        ensure!(
            session == self.session,
            "This session changed. Inspect the project again"
        );
        Ok(())
    }

    fn catalog(&self) -> Result<StoredProject> {
        self.verify_owner()?;
        let stored = ProjectDb::read_authoring_snapshot(&self.folder.database()).map_err(|_| {
            anyhow::anyhow!("Project catalog is unavailable. Resolve any pending save in Studio")
        })?;
        ensure!(
            stored.id == self.folder.id(),
            "Project identity changed. Restart this MCP session"
        );
        Ok(stored)
    }

    fn verify_owner(&self) -> Result<()> {
        self.owner
            .verify()
            .and_then(|()| self.authoring.verify())
            .map_err(|_| anyhow::anyhow!("Project ownership changed. Restart this MCP session"))
    }

    fn snapshot(&self, asset: Uuid) -> Result<Snapshot> {
        let stored = self.catalog()?;
        self.asset_snapshot(asset, &stored)
    }

    fn asset_snapshot(&self, asset: Uuid, stored: &StoredProject) -> Result<Snapshot> {
        let item = stored
            .registry
            .artifact(asset)
            .filter(|item| item.kind.is_visual())
            .context("Choose an existing visual asset UUID from this project")?;
        let (mut doc, version) = if self.path.exists() {
            document::load_versioned(&self.path)
                .map_err(|_| anyhow::anyhow!("Could not read the saved Atelier drawing"))?
        } else {
            let doc = Document {
                title: stored.name.clone(),
                ..Default::default()
            };
            let version = document::version(&doc)?;
            (doc, version)
        };
        if let Some(artboard) = linked_canvas(&doc, asset) {
            doc.select_canvas(artboard)?;
        }
        Ok(Snapshot {
            doc,
            version,
            name: item.name.chars().take(256).collect(),
            revision: item.active_revision_id,
        })
    }

    fn project(&self, args: Project) -> Result<Value> {
        let stored = self.catalog()?;
        let jobs = self.submissions()?;
        let mut assets = stored
            .registry
            .artifacts
            .iter()
            .filter(|item| item.kind.is_visual());
        let page: Vec<_> = assets.by_ref().skip(args.offset).take(48).map(|item| json!({
            "asset": item.id, "kind": item.kind, "name": item.name.chars().take(256).collect::<String>(),
            "variant_of": item.variant_of, "published_revision": item.active_revision_id,
        })).collect();
        Ok(json!({
            "schema_version": 2,
            "project": self.folder.id(), "session": self.session,
            "assets": page, "next_offset": assets.next().map(|_| args.offset + page.len()),
            "can_generate": self.allow_generate, "can_publish": self.allow_publish,
            "provider": self.settings.provider.name(),
            "model": self.settings.model, "edit_model": self.settings.edit_model,
            "max_submissions": self.max_submissions,
            "submissions": jobs.len(),
            "recent_jobs": jobs.iter().take(16).map(|(_, id)| id).collect::<Vec<_>>(),
        }))
    }

    fn inspect(&self, args: Asset) -> Result<Value> {
        let snap = self.snapshot(args.asset)?;
        let artboard = linked_canvas(&snap.doc, args.asset);
        Ok(json!({
            "asset": args.asset, "name": snap.name, "version": snap.version,
            "published_revision": snap.revision, "artboard": artboard,
            "has_candidate": artboard.is_some() && candidate(&snap.doc)?.is_some(),
        }))
    }

    fn generate(&mut self, args: Generate) -> Result<Value> {
        self.check_session(args.session)?;
        ensure!(
            self.allow_generate,
            "Provider submission was not granted at launch"
        );
        ensure!(
            self.queue.outstanding() == 0,
            "Poll the current job before starting another"
        );
        self.check_save_retry()?;
        ensure!(
            !args.prompt.trim().is_empty() && args.prompt.len() <= 8192,
            "Use a prompt of 1 to 8192 UTF-8 bytes"
        );
        ensure!(
            !args.revise || args.reference_revision.is_none(),
            "Choose either a pending-candidate revision or a published base image"
        );
        let stored = self.catalog()?;
        let mut snap = self.asset_snapshot(args.asset, &stored)?;
        snap.check(&args.expected_version)?;
        let artboard = linked_canvas(&snap.doc, args.asset);
        ensure!(snap.revision.is_none(), "Choose a pending asset");
        let pending = if artboard.is_some() {
            check_candidate_destination(&snap.doc)?;
            candidate(&snap.doc)?
        } else {
            None
        };
        ensure!(
            pending.is_some() == args.revise,
            "Use revise=true to edit the pending candidate, or start without a candidate"
        );
        let source = match args.reference_revision {
            Some(revision) => {
                let base = stored
                    .registry
                    .artifact(args.asset)
                    .and_then(|asset| asset.variant_of)
                    .context("Choose a variant of the published reference asset")?;
                let path = stored
                    .registry
                    .active_revision(base)
                    .filter(|active| {
                        active.id == revision && active.status == RevisionStatus::Completed
                    })
                    .and_then(|active| active.asset_path.as_ref())
                    .context("Inspect the base asset and use its current published revision")?;
                ensure!(
                    fs::metadata(path)?.len() <= 20 * 1024 * 1024,
                    "Reference exceeds 20 MB"
                );
                Some(Arc::new(document::reference_from_bytes(
                    "Published base artwork".into(),
                    &fs::read(path)?,
                    revision.to_string(),
                )?))
            }
            None => pending.map(|(_, reference)| reference),
        };
        let mut settings = self.settings.clone();
        if artboard.is_some() {
            let size = [snap.doc.width, snap.doc.height];
            ensure!(
                args.size.is_none_or(|requested| requested == size),
                "Use the existing artboard size"
            );
            [settings.width, settings.height] = size;
        } else if let Some(size) = args.size {
            [settings.width, settings.height] = size;
        }
        let editing = source.is_some();
        providers::validate_capabilities(&settings, editing, 0, false)
            .map_err(|_| anyhow::anyhow!("The configured Atelier provider cannot run this request. Check its model and size in Atelier Settings"))?;

        self.check_jobs_dir()?;
        fs::create_dir_all(&self.jobs_dir)?;
        if let Some(max) = self.max_submissions {
            let submitted = self.submissions()?.len();
            ensure!(
                submitted < max,
                "This project reached its submission limit ({max})"
            );
        }
        if artboard.is_none() {
            snap.doc
                .ensure_story_artboard(args.asset, String::new(), String::new())?;
            snap.doc.width = settings.width;
            snap.doc.height = settings.height;
            snap.doc
                .artwork_bases
                .insert(snap.doc.canvas_id, snap.revision);
            save_editable(&self.path, &snap.doc)?;
            snap.version = document::version(&snap.doc)?;
        }
        let captured = CapturedJob {
            project: self.folder.id(),
            asset: args.asset,
            artboard: snap.doc.canvas_id,
            version: snap.version,
            size: [settings.width, settings.height],
            reference_revision: args.reference_revision,
            provider: settings.provider.name().into(),
            model: if editing {
                settings.edit_model.clone()
            } else {
                settings.model.clone()
            },
        };
        let id = Uuid::new_v4();
        document::atomic_write(&self.job_path(id), &serde_json::to_vec(&captured)?)?;
        let request = QueuedRequest {
            settings,
            prompt: args.prompt,
            source,
            references: Vec::new(),
            mask: None,
            editing,
            destination: Destination::Candidate {
                artboard: captured.artboard,
                rect: (0, 0, captured.size[0], captured.size[1]),
            },
            blend: None,
        };
        self.queue.clear_finished(|_| false);
        let queued = self.queue.enqueue(request.clone(), 0);
        self.queue.get_mut(queued).expect("Queued job exists").state = JobState::Running {
            progress: String::new(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_job = Some(ActiveJob {
            queued,
            id,
            cancel: cancel.clone(),
        });
        worker::generate_using(
            request.into_provider_request(self.native.clone(), cancel),
            queued,
            self.sender.clone(),
            self.data_dir.clone(),
            self.provider,
            Some(self.result_path(id)),
        );
        Ok(json!({ "job": id, "asset": args.asset, "status": "running" }))
    }

    fn check_jobs_dir(&self) -> Result<()> {
        ensure!(
            self.path.parent().is_some_and(|parent| parent
                .canonicalize()
                .is_ok_and(|resolved| resolved == self.folder.root().join("atelier"))),
            "The Atelier directory moved. Restart this MCP session"
        );
        match fs::symlink_metadata(&self.jobs_dir) {
            Ok(metadata) => ensure!(metadata.is_dir(), "The project job directory must be local"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn job_path(&self, job: Uuid) -> PathBuf {
        self.jobs_dir.join(format!("{job}.job.json"))
    }

    fn result_path(&self, job: Uuid) -> PathBuf {
        self.jobs_dir.join(format!("{job}.result.json"))
    }

    fn cancellation_path(&self, job: Uuid) -> PathBuf {
        self.jobs_dir.join(format!("{job}.cancel"))
    }

    fn cancellation_requested(&self, job: Uuid) -> Result<bool> {
        match fs::symlink_metadata(self.cancellation_path(job)) {
            Ok(metadata) => {
                ensure!(metadata.is_file(), "Invalid job cancellation receipt");
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn cancel(&mut self, args: Poll) -> Result<Value> {
        self.check_session(args.session)?;
        ensure!(
            self.allow_generate,
            "Provider job controls were not granted at launch"
        );
        self.verify_owner()?;
        self.job(args.job)?;
        if !self.cancellation_requested(args.job)? {
            self.refresh_queue();
            let active = self.active_job.as_ref()
                .filter(|active| active.id == args.job)
                .filter(|active| self.queue.get(active.queued).is_some_and(|job| matches!(job.state, JobState::Running { .. })))
                .context("Only a running job in this session can be cancelled. Poll this job for its current status")?;
            // Record attachment suppression before signalling the worker.
            document::atomic_write(&self.cancellation_path(args.job), b"")?;
            self.queue.abort(active.queued);
            active.cancel.store(true, Ordering::Relaxed);
        }
        self.poll(args)
    }

    fn submissions(&self) -> Result<Vec<(std::time::SystemTime, Uuid)>> {
        self.check_jobs_dir()?;
        let entries = match fs::read_dir(&self.jobs_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut jobs = Vec::new();
        for entry in entries {
            let entry = entry?;
            if let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".job.json"))
                .and_then(|stem| Uuid::parse_str(stem).ok())
            {
                jobs.push((entry.metadata()?.modified()?, id));
            }
        }
        jobs.sort_unstable_by(|a, b| b.cmp(a));
        Ok(jobs)
    }

    fn job(&self, id: Uuid) -> Result<CapturedJob> {
        self.check_jobs_dir()?;
        let path = self.job_path(id);
        ensure!(
            fs::symlink_metadata(&path).is_ok_and(|entry| entry.is_file() && entry.len() <= 4096),
            "Unknown job in this project"
        );
        let job: CapturedJob = serde_json::from_slice(&fs::read(path)?)?;
        ensure!(
            job.project == self.folder.id(),
            "The job belongs to another project"
        );
        Ok(job)
    }

    fn result(&self, id: Uuid) -> Result<Option<Reference>> {
        let path = self.result_path(id);
        match fs::symlink_metadata(&path) {
            Ok(entry) => ensure!(
                entry.is_file() && entry.len() <= 32 * 1024 * 1024,
                "The retained result is invalid"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
    }

    fn refresh_queue(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::GenerationProgress(id, progress) => {
                    if let Some(job) = self
                        .queue
                        .get_mut(id)
                        .filter(|job| matches!(job.state, JobState::Running { .. }))
                    {
                        job.state = JobState::Running { progress };
                    }
                }
                Event::Generated(id, result) => {
                    let job = self.queue.get_mut(id).expect("Running job exists");
                    job.state = if matches!(job.state, JobState::Aborting) {
                        JobState::Cancelled
                    } else {
                        match result {
                            Ok(_) => JobState::Done,
                            Err(error) => {
                                eprintln!("Atelier MCP generation failed: {error:#}");
                                JobState::Failed(id)
                            }
                        }
                    };
                }
                _ => {}
            }
        }
    }

    fn poll(&mut self, args: Poll) -> Result<Value> {
        self.check_session(args.session)?;
        self.verify_owner()?;
        let job = self.job(args.job)?;
        let cancellation_requested = self.cancellation_requested(args.job)?;
        self.refresh_queue();
        let queued = self
            .active_job
            .as_ref()
            .filter(|active| active.id == args.job)
            .and_then(|active| self.queue.get(active.queued));
        if let Some(queued) = queued {
            if let JobState::Running { progress } = &queued.state {
                return Ok(json!({
                    "job": args.job, "asset": job.asset, "status": "running",
                    "progress": progress.chars().take(160).collect::<String>(),
                }));
            }
        }
        if cancellation_requested
            && queued.is_some_and(|queued| matches!(queued.state, JobState::Aborting))
        {
            return Ok(json!({
                "job": args.job, "asset": job.asset, "status": "cancelling",
                "attachment_suppressed": true, "remote_cancellation": "unconfirmed",
            }));
        }
        let Some(reference) = self.result(args.job)? else {
            if cancellation_requested {
                return Ok(json!({
                    "job": args.job, "asset": job.asset, "status": "cancelled",
                    "attachment_suppressed": true, "remote_cancellation": "unconfirmed",
                }));
            }
            let failed = queued.is_some_and(|queued| matches!(queued.state, JobState::Failed(_)));
            return Ok(json!({
                "job": args.job, "asset": job.asset,
                "status": if failed { "failed" } else { "unknown" },
                "message": "No completed result was saved. Check the provider before submitting again.",
            }));
        };
        let snap = self.snapshot(job.asset)?;
        let mut version = snap.version.clone();
        let attached = if contains_result(&snap, &job, &reference)? {
            true
        } else if !cancellation_requested
            && snap.version == job.version
            && self.queue.outstanding() == 0
            && self.check_save_retry().is_ok()
        {
            match self.attach(snap, &job, reference) {
                Ok(saved) => {
                    version = saved;
                    true
                }
                Err(error) => {
                    eprintln!("Atelier MCP attachment failed: {error:#}");
                    false
                }
            }
        } else {
            false
        };
        Ok(json!({
            "job": args.job, "asset": job.asset, "version": version, "result_retained": true,
            "attachment_suppressed": cancellation_requested && !attached,
            "status": if attached { "done" } else { "retained" },
        }))
    }

    fn recover(&mut self, args: Recover) -> Result<Value> {
        self.check_session(args.session)?;
        self.verify_owner()?;
        self.refresh_queue();
        ensure!(
            self.queue.outstanding() == 0,
            "Wait for the running job first"
        );
        self.check_save_retry()?;
        let job = self.job(args.job)?;
        let snap = self.snapshot(job.asset)?;
        snap.check(&args.expected_version)?;
        let reference = self
            .result(args.job)?
            .context("This job has no completed result")?;
        let version = if contains_result(&snap, &job, &reference)? {
            snap.version
        } else {
            self.attach(snap, &job, reference)?
        };
        Ok(json!({
            "job": args.job, "asset": job.asset, "version": version,
            "status": "done", "result_retained": true,
        }))
    }

    fn attach(
        &mut self,
        mut snap: Snapshot,
        job: &CapturedJob,
        reference: Reference,
    ) -> Result<String> {
        ensure!(
            snap.revision.is_none()
                && linked_canvas(&snap.doc, job.asset) == Some(job.artboard)
                && [snap.doc.width, snap.doc.height] == job.size,
            "The original pending artboard is no longer available at its captured size"
        );
        check_candidate_destination(&snap.doc)?;
        if let Some((index, _)) = candidate(&snap.doc)? {
            snap.doc.active_layer = index;
            snap.doc.review_candidate_layer(Review::Discard);
        }
        snap.doc.attach_result(
            Arc::new(reference),
            Destination::Candidate {
                artboard: job.artboard,
                rect: (0, 0, job.size[0], job.size[1]),
            },
        )?;
        save_editable(&self.path, &snap.doc)?;
        self.clear_review(job.asset);
        document::version(&snap.doc)
    }

    fn thumbnail(&self, args: Poll) -> Result<CallToolResult> {
        self.check_session(args.session)?;
        self.verify_owner()?;
        let job = self.job(args.job)?;
        let reference = self
            .result(args.job)?
            .context("This job has no completed result")?;
        let image = document::thumbnail(document::decode_reference(&reference)?, 512).to_rgba8();
        image_result(
            json!({
                "job": args.job, "asset": job.asset, "width": image.width(), "height": image.height(),
                "provider": job.provider, "model": job.model, "reference_revision": job.reference_revision,
            }),
            image,
        )
    }

    fn check_save_retry(&self) -> Result<()> {
        ensure!(
            self.reviews.values().all(|review| !review.approved),
            "Finish retrying the approved save first"
        );
        Ok(())
    }

    fn clear_review(&mut self, asset: Uuid) {
        if let Some(review) = self.reviews.remove(&asset) {
            let _ = fs::remove_file(approval_path(&self.folder, review.id));
        }
    }

    fn preview(&mut self, args: Checked) -> Result<CallToolResult> {
        self.check_session(args.session)?;
        let snap = self.snapshot(args.asset)?;
        snap.check(&args.expected_version)?;
        ensure!(
            linked_canvas(&snap.doc, args.asset).is_some(),
            "Generate a candidate first"
        );
        let pending = candidate(&snap.doc)?.is_some();
        ensure!(
            pending
                || snap
                    .doc
                    .layers
                    .iter()
                    .any(|layer| layer.image.is_some() && snap.doc.layer_is_visible(layer)),
            "Generate a candidate first"
        );
        let image = document::raster(&snap.doc, 512, true)
            .map_err(|_| anyhow::anyhow!("Could not render the artboard preview"))?;
        let baseline = snap
            .doc
            .artwork_bases
            .get(&snap.doc.canvas_id)
            .copied()
            .flatten();
        let review = if self.allow_publish
            && (pending || snap.revision.is_none() || baseline != snap.revision)
        {
            let version = snap.review_version()?;
            if self
                .reviews
                .get(&args.asset)
                .is_none_or(|review| review.version != version)
            {
                self.check_save_retry()?;
                self.clear_review(args.asset);
                self.reviews.insert(
                    args.asset,
                    ReviewProposal {
                        id: Uuid::new_v4(),
                        version,
                        approved: false,
                    },
                );
            }
            self.reviews.get(&args.asset).map(|review| review.id)
        } else {
            None
        };
        image_result(
            json!({
                "asset": args.asset, "artboard": snap.doc.canvas_id,
                "version": snap.version, "review": review, "candidate": pending,
                "expected_artwork_revision": baseline,
                "current_artwork_revision": snap.revision,
                "width": image.width(), "height": image.height(),
            }),
            image,
        )
    }

    fn publish(&mut self, args: Publish) -> Result<CallToolResult> {
        self.check_session(args.session)?;
        ensure!(self.allow_publish, "Publication was not granted at launch");
        ensure!(
            self.queue.outstanding() == 0,
            "Wait for the queued revision and review it before publishing"
        );
        let asset = self
            .reviews
            .iter()
            .find_map(|(asset, review)| (review.id == args.review).then_some(*asset))
            .context("Preview the current candidate to get its review ID")?;
        let mut snap = self.snapshot(asset)?;
        let proposal = self.reviews.get_mut(&asset).expect("Review was checked");
        ensure!(
            linked_canvas(&snap.doc, asset) == Some(snap.doc.canvas_id)
                && snap.review_version()? == proposal.version,
            "The reviewed artboard changed. Preview it again"
        );
        if !proposal.approved && !args.agent_approval {
            let path = approval_path(&self.folder, proposal.id);
            ensure!(fs::symlink_metadata(&path).is_ok_and(|entry| entry.is_file()),
                "Human approval is required. Ask the reviewer to run atelier approve with this review ID");
            fs::remove_file(path)
                .map_err(|_| anyhow::anyhow!("Could not consume the local review approval"))?;
        }
        proposal.approved = true;
        if let Some((index, _)) = candidate(&snap.doc)? {
            snap.doc.active_layer = index;
            snap.doc.review_candidate_layer(Review::Accept);
        }
        let accepted_version = snap.review_version()?;
        match shared_save::save_raster(&self.path, &snap.doc, snap.doc.canvas_id) {
            Ok(publications) => {
                self.reviews.remove(&asset);
                let publication = publications
                    .first()
                    .context("The approved drawing contained no publishable artwork")?;
                Ok(CallToolResult::structured(json!({
                    "published": true, "asset": publication.asset_id, "artboard": publication.canvas_id,
                    "revision": publication.revision_id,
                })))
            }
            Err(error) => {
                let stage = error.downcast_ref::<SaveStage>();
                let committed = shared_save::committed_publications(&error)
                    .first()
                    .map(|item| item.revision_id);
                let (code, message) = match stage {
                    Some(SaveStage::Artwork) => ("artwork", "Editable artwork saved. Check the current catalog revision and retry the approved Save if the baseline still matches"),
                    Some(SaveStage::RevisionLinks(_)) => ("revision_links", "Artwork published. Retry this approved Save to finish recording the revision link"),
                    _ => ("editable_document", "Could not save the editable drawing. Retry the approved Save"),
                };
                if matches!(
                    stage,
                    Some(SaveStage::Artwork | SaveStage::RevisionLinks(_))
                ) {
                    proposal.version = accepted_version;
                }
                eprintln!("Atelier MCP save failed: {error:#}");
                Ok(CallToolResult::structured_error(json!({
                    "stage": code, "message": message, "published_revision": committed,
                })))
            }
        }
    }

    fn call(&mut self, name: &str, args: Value) -> Result<CallToolResult> {
        fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T> {
            serde_json::from_value(args)
                .map_err(|_| anyhow::anyhow!("Use the tool's declared argument schema"))
        }
        let result = match name {
            PROJECT => self.project(parse(args)?)?,
            INSPECT => self.inspect(parse(args)?)?,
            GENERATE => self.generate(parse(args)?)?,
            POLL => self.poll(parse(args)?)?,
            CANCEL => self.cancel(parse(args)?)?,
            RECOVER => self.recover(parse(args)?)?,
            THUMBNAIL => return self.thumbnail(parse(args)?),
            PREVIEW => return self.preview(parse(args)?),
            PUBLISH => return self.publish(parse(args)?),
            _ => bail!("Unknown Atelier tool"),
        };
        Ok(CallToolResult::structured(result))
    }
}

fn linked_canvas(doc: &Document, asset: Uuid) -> Option<u32> {
    doc.asset_links
        .iter()
        .find(|link| link.asset_id == asset)
        .map(|link| link.canvas_id)
}

fn check_candidate_destination(doc: &Document) -> Result<()> {
    ensure!(
        doc.layers
            .iter()
            .all(|layer| layer.candidate || (layer.image.is_none() && layer.strokes.is_empty())),
        "This workflow needs an empty artboard or one pending candidate"
    );
    Ok(())
}

fn contains_result(snap: &Snapshot, job: &CapturedJob, reference: &Reference) -> Result<bool> {
    Ok(linked_canvas(&snap.doc, job.asset) == Some(job.artboard)
        && candidate(&snap.doc)?
            .is_some_and(|(_, image)| crate::library::id(&image) == crate::library::id(reference)))
}

fn image_result(metadata: Value, image: image::RgbaImage) -> Result<CallToolResult> {
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    let mut response = CallToolResult::structured(metadata);
    response.content.push(Content::image(
        STANDARD.encode(png.into_inner()),
        "image/png",
    ));
    Ok(response)
}

fn candidate(doc: &Document) -> Result<Option<(usize, Arc<Reference>)>> {
    let mut layers = doc
        .layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| layer.candidate);
    let first = layers.next();
    ensure!(
        layers.next().is_none(),
        "Use one candidate on this artboard"
    );
    first
        .map(|(index, layer)| {
            let image = layer
                .image
                .as_ref()
                .filter(|_| doc.layer_is_visible(layer))
                .context("The candidate must be a visible image layer")?;
            Ok((index, image.reference.clone()))
        })
        .transpose()
}

fn save_editable(path: &Path, doc: &Document) -> Result<()> {
    document::save(path, doc).map_err(|error| {
        eprintln!("Atelier MCP editable save failed: {error:#}");
        anyhow::anyhow!("Could not save the editable drawing")
    })
}

#[derive(Clone)]
struct Server(Arc<Mutex<Tools>>);

impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("storyfella-atelier", env!("CARGO_PKG_VERSION")))
            .with_instructions("Inspect the project and target an asset UUID. Poll jobs for status and request PNG previews only when needed. Visually inspect the candidate before deciding. Only if the user explicitly delegates artwork approval in chat, explicitly approve the exact review using agent_approval=true when publishing. Otherwise show the image and review ID to the human for local approval.")
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let tools = self.0.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut tools = tools
                .lock()
                .map_err(|_| anyhow::anyhow!("Atelier worker stopped. Restart this MCP session"))?;
            tools.call(
                &request.name,
                Value::Object(request.arguments.unwrap_or_default()),
            )
        })
        .await
        .map_err(|_| ErrorData::internal_error("Atelier worker stopped", None))?;
        Ok(result.unwrap_or_else(|error| {
            CallToolResult::structured_error(json!({ "message": error.to_string() }))
        }))
    }
}

fn tools() -> Vec<Tool> {
    fn tool(
        name: &'static str,
        description: &'static str,
        properties: Value,
        required: &[&str],
    ) -> Tool {
        let schema: JsonObject = serde_json::from_value(json!({
            "type": "object", "properties": properties, "required": required, "additionalProperties": false,
        })).expect("Atelier tool schema is an object");
        Tool::new(name, description, schema)
    }
    let uuid = json!({"type": "string", "format": "uuid"});
    let version = json!({"type": "string"});
    vec![
        tool(PROJECT, "List visual assets and their published revisions in this project. Use next_offset to continue.", json!({"offset": {"type": "integer", "minimum": 0}}), &[]),
        tool(INSPECT, "Inspect one asset and the current saved drawing version.", json!({"asset": uuid}), &["asset"]),
        tool(GENERATE, "Start a job for a pending asset. For a variant, pass its base asset's published reference_revision. Use revise=true to edit a pending candidate. Size is [width, height] in pixels for a new artboard.", json!({"session": uuid, "asset": uuid, "expected_version": version, "prompt": {"type": "string", "maxLength": 8192}, "revise": {"type": "boolean"}, "reference_revision": uuid, "size": {"type": "array", "items": {"type": "integer", "minimum": 1}, "minItems": 2, "maxItems": 2}}), &["session", "asset", "expected_version", "prompt"]),
        tool(POLL, "Read compact job status and attach a completed result if its captured drawing still matches. This works after reconnecting.", json!({"session": uuid, "job": uuid}), &["session", "job"]),
        tool(CANCEL, "Request cancellation of a running job and suppress automatic attachment. A late completed result remains recoverable. Remote cancellation is unconfirmed.", json!({"session": uuid, "job": uuid}), &["session", "job"]),
        tool(RECOVER, "Reattach a retained result to its original pending artboard using the drawing version you just inspected. Replaces any pending candidate there. Does not call the provider.", json!({"session": uuid, "job": uuid, "expected_version": version}), &["session", "job", "expected_version"]),
        tool(THUMBNAIL, "View a retained job's bounded PNG thumbnail without attaching or creating a review.", json!({"session": uuid, "job": uuid}), &["session", "job"]),
        tool(PREVIEW, "View a bounded artboard PNG and get the ID for human review of this exact candidate.", json!({"session": uuid, "asset": uuid, "expected_version": version}), &["session", "asset", "expected_version"]),
        tool(PUBLISH, "Publish only the exact reviewed asset. Set agent_approval=true after visually approving the preview only when the user explicitly delegates artwork approval in chat. Otherwise use local human approval. Retry the same review if a save fails.", json!({"session": uuid, "review": uuid, "agent_approval": {"type": "boolean"}}), &["session", "review"]),
    ]
}

#[cfg(test)]
mod tests;
