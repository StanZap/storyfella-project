//! A closed-project MCP owner for a single linked illustration.
use std::{
    fs,
    io::{Cursor, IsTerminal},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, mpsc, Arc, Mutex},
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
use serde::Deserialize;
use serde_json::{json, Value};
use storyfella_studio_core::{
    persistence::ProjectDb,
    project::{AuthoringLease, ProjectFolder},
    registry::ArtifactKind,
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

const INSPECT: &str = "atelier_illustration_inspect";
const GENERATE: &str = "atelier_candidate_generate";
const POLL: &str = "atelier_job_poll";
const PREVIEW: &str = "atelier_candidate_preview";
const PUBLISH: &str = "atelier_illustration_publish";

#[derive(Parser)]
#[command(name = "atelier mcp")]
pub(crate) struct Args {
    /// Absolute path to one closed shared project folder.
    #[arg(long)]
    folder: PathBuf,
    /// Registry UUID of the existing illustration.
    #[arg(long)]
    illustration: Uuid,
    /// Use the provider settings from this Atelier workspace.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Allow requests to the configured image provider.
    #[arg(long)]
    allow_generate: bool,
    /// Allow checked publication after separate local approval.
    #[arg(long)]
    allow_publish: bool,
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
    let tools = Tools::open(args)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
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
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Generate {
    session: Uuid,
    expected_version: String,
    prompt: String,
    #[serde(default)]
    revise: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Poll {
    session: Uuid,
    job: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Checked {
    session: Uuid,
    expected_version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    session: Uuid,
    review: Uuid,
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
}

struct ReviewProposal {
    id: Uuid,
    version: String,
    approved: bool,
}

type Dispatch = fn(Request, u64, mpsc::Sender<Event>, PathBuf);

struct Tools {
    folder: ProjectFolder,
    owner: Owner,
    authoring: AuthoringLease,
    path: PathBuf,
    illustration: Uuid,
    session: Uuid,
    allow_generate: bool,
    allow_publish: bool,
    settings: Settings,
    data_dir: PathBuf,
    native: Arc<NativeImages>,
    queue: Queue,
    sender: mpsc::Sender<Event>,
    events: mpsc::Receiver<Event>,
    dispatch: Dispatch,
    job_version: String,
    job_artwork_revision: Option<Uuid>,
    job_error: Option<&'static str>,
    review: Option<ReviewProposal>,
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
        ensure!(
            stored
                .registry
                .artifact(args.illustration)
                .is_some_and(|item| item.kind == ArtifactKind::Illustration),
            "Choose an existing illustration UUID from this project"
        );
        let path = folder.atelier_document()?;
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
            illustration: args.illustration,
            session: Uuid::new_v4(),
            allow_generate: args.allow_generate,
            allow_publish: args.allow_publish,
            native: Arc::default(),
            queue: Queue::default(),
            dispatch: worker::generate,
            job_version: String::new(),
            job_artwork_revision: None,
            job_error: None,
            review: None,
        })
    }

    fn check_session(&self, session: Uuid) -> Result<()> {
        ensure!(
            session == self.session,
            "This session changed. Inspect the illustration again"
        );
        Ok(())
    }

    fn snapshot(&self) -> Result<Snapshot> {
        self.owner
            .verify()
            .and_then(|()| self.authoring.verify())
            .map_err(|_| anyhow::anyhow!("Project ownership changed. Restart this MCP session"))?;
        let stored = ProjectDb::read_authoring_snapshot(&self.folder.database()).map_err(|_| {
            anyhow::anyhow!("Project catalog is unavailable. Resolve any pending save in Studio")
        })?;
        ensure!(
            stored.id == self.folder.id(),
            "Project identity changed. Restart this MCP session"
        );
        let item = stored
            .registry
            .artifact(self.illustration)
            .filter(|item| item.kind == ArtifactKind::Illustration)
            .context("The selected illustration no longer exists")?;
        let (doc, version) = if self.path.exists() {
            document::load_versioned(&self.path)
                .map_err(|_| anyhow::anyhow!("Could not read the saved Atelier drawing"))?
        } else {
            let doc = Document {
                title: stored.name,
                ..Default::default()
            };
            let version = document::version(&doc)?;
            (doc, version)
        };
        ensure!(
            doc.asset_links.is_empty()
                || (doc.asset_links.len() == 1
                    && doc.asset_links[0].asset_id == self.illustration
                    && doc.asset_links[0].canvas_id == doc.canvas_id),
            "Use a disposable drawing with just this illustration on the current artboard"
        );
        Ok(Snapshot {
            doc,
            version,
            name: item.name.chars().take(256).collect(),
            revision: item.active_revision_id,
        })
    }

    fn inspect(&self) -> Result<Value> {
        let snap = self.snapshot()?;
        Ok(json!({
            "schema_version": 1,
            "project": self.folder.id(), "session": self.session,
            "illustration": self.illustration, "name": snap.name, "version": snap.version,
            "published_revision": snap.revision,
            "artboard": (!snap.doc.asset_links.is_empty()).then_some(snap.doc.canvas_id),
            "has_candidate": snap.doc.layers.iter().any(|layer| layer.candidate),
            "can_generate": self.allow_generate, "can_publish": self.allow_publish,
            "provider": self.settings.provider.name(),
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
        ensure!(
            self.review.as_ref().is_none_or(|review| !review.approved),
            "Finish retrying the approved save first"
        );
        ensure!(
            !args.prompt.trim().is_empty() && args.prompt.len() <= 8192,
            "Use a prompt of 1 to 8192 UTF-8 bytes"
        );
        let mut snap = self.snapshot()?;
        snap.check(&args.expected_version)?;
        ensure!(
            snap.doc.layers.iter().all(|layer| layer.candidate || (layer.image.is_none() && layer.strokes.is_empty())),
            "This workflow generates into an empty artboard or revises its pending candidate"
        );
        let candidate = candidate(&snap.doc)?;
        ensure!(
            candidate.is_some() == args.revise,
            "Use revise=true to edit the pending candidate, or start without a candidate"
        );
        let mut settings = self.settings.clone();
        if !snap.doc.asset_links.is_empty() {
            settings.width = snap.doc.width;
            settings.height = snap.doc.height;
        }
        providers::validate_capabilities(&settings, args.revise, 0, false)
            .map_err(|_| anyhow::anyhow!("The configured Atelier provider cannot run this request. Check its model, size and readiness in Atelier Settings"))?;
        ensure!(
            snap.revision.is_none(),
            "This illustration already has artwork. Choose a pending illustration"
        );
        let source = candidate.map(|(_, reference)| reference);
        if snap.doc.asset_links.is_empty() {
            snap.doc
                .ensure_story_artboard(self.illustration, String::new(), String::new())?;
            snap.doc.width = settings.width;
            snap.doc.height = settings.height;
            snap.doc
                .artwork_bases
                .insert(snap.doc.canvas_id, snap.revision);
            save_editable(&self.path, &snap.doc)?;
            snap.version = document::version(&snap.doc)?;
        }
        let request = QueuedRequest {
            settings,
            prompt: args.prompt,
            source,
            references: Vec::new(),
            mask: None,
            editing: args.revise,
            destination: Destination::Candidate {
                artboard: snap.doc.canvas_id,
                rect: (0, 0, snap.doc.width, snap.doc.height),
            },
            blend: None,
        };
        self.queue.clear_finished(|_| false);
        self.job_error = None;
        self.job_version = snap.version;
        self.job_artwork_revision = snap.revision;
        let job = self.queue.enqueue(request.clone(), 0);
        self.queue.get_mut(job).expect("Queued job exists").state = JobState::Running {
            progress: String::new(),
        };
        (self.dispatch)(
            request.into_provider_request(self.native.clone(), Arc::new(AtomicBool::new(false))),
            job,
            self.sender.clone(),
            self.data_dir.clone(),
        );
        Ok(json!({ "job": job, "status": "running" }))
    }

    fn poll(&mut self, args: Poll) -> Result<Value> {
        self.check_session(args.session)?;
        ensure!(
            self.queue.get(args.job).is_some(),
            "Unknown job in this session"
        );
        while let Ok(event) = self.events.try_recv() {
            if let Event::Generated(id, result) = event {
                self.finish(id, result.map(|(reference, _)| Arc::new(reference)));
            }
        }
        let job = self
            .queue
            .get(args.job)
            .context("Unknown job in this session")?;
        Ok(json!({
            "job": job.id, "status": job.state.label().to_lowercase(),
            "message": self.job_error,
            "result_retained": job.result.is_some(),
            "version": self.job_version,
        }))
    }

    fn finish(&mut self, id: u64, result: Result<Arc<Reference>>) {
        let job = self.queue.get_mut(id).expect("Running job exists");
        let reference = match result {
            Ok(reference) => reference,
            Err(error) => {
                eprintln!("Atelier MCP generation failed: {error:#}");
                job.state = JobState::Failed(id);
                self.job_error =
                    Some("Generation failed. Check Atelier's configured provider and server log");
                return;
            }
        };
        job.result = Some(reference.clone());
        let destination = job.request.destination;
        let saved = (|| {
            let mut snap = self.snapshot()?;
            snap.check(&self.job_version)?;
            ensure!(
                snap.revision == self.job_artwork_revision,
                "The published artwork changed during generation"
            );
            if let Some((index, _)) = candidate(&snap.doc)? {
                snap.doc.active_layer = index;
                snap.doc.review_candidate_layer(Review::Discard);
            }
            snap.doc.attach_result(reference, destination)?;
            save_editable(&self.path, &snap.doc)?;
            document::version(&snap.doc)
        })();
        match saved {
            Ok(version) => {
                self.clear_review();
                let job = self.queue.get_mut(id).expect("Running job exists");
                self.job_version = version;
                job.state = JobState::Done;
            }
            Err(error) => {
                eprintln!("Atelier MCP attachment failed: {error:#}");
                self.queue.get_mut(id).expect("Running job exists").state = JobState::Failed(id);
                self.job_error = Some("The candidate could not attach. The generated original remains in Atelier storage");
            }
        }
    }

    fn clear_review(&mut self) {
        if let Some(review) = self.review.take() {
            let _ = fs::remove_file(approval_path(&self.folder, review.id));
        }
    }

    fn preview(&mut self, args: Checked) -> Result<CallToolResult> {
        self.check_session(args.session)?;
        let snap = self.snapshot()?;
        snap.check(&args.expected_version)?;
        ensure!(
            !snap.doc.asset_links.is_empty(),
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
            .map_err(|_| anyhow::anyhow!("Could not render the illustration preview"))?;
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png)?;
        let review = if self.allow_publish {
            if self
                .review
                .as_ref()
                .is_none_or(|review| review.version != snap.version)
            {
                self.clear_review();
                self.review = Some(ReviewProposal {
                    id: Uuid::new_v4(),
                    version: snap.version.clone(),
                    approved: false,
                });
            }
            self.review.as_ref().map(|review| review.id)
        } else {
            None
        };
        let mut response = CallToolResult::structured(json!({
            "illustration": self.illustration, "artboard": snap.doc.canvas_id,
            "version": snap.version, "review": review, "candidate": pending,
            "expected_artwork_revision": snap.doc.artwork_bases.get(&snap.doc.canvas_id).copied().flatten(),
            "current_artwork_revision": snap.revision,
            "width": image.width(), "height": image.height(),
            "approval": "Show the image and review ID to the human. Ask them to approve it from a separate terminal using atelier approve.",
        }));
        response.content.push(Content::image(
            STANDARD.encode(png.into_inner()),
            "image/png",
        ));
        Ok(response)
    }

    fn publish(&mut self, args: Publish) -> Result<CallToolResult> {
        self.check_session(args.session)?;
        ensure!(self.allow_publish, "Publication was not granted at launch");
        ensure!(
            self.queue.outstanding() == 0,
            "Wait for the queued revision and review it before publishing"
        );
        let mut snap = self.snapshot()?;
        let proposal = self
            .review
            .as_mut()
            .filter(|review| review.id == args.review)
            .context("Preview the current candidate to get its review ID")?;
        snap.check(&proposal.version)?;
        if !proposal.approved {
            let path = approval_path(&self.folder, proposal.id);
            ensure!(fs::symlink_metadata(&path).is_ok_and(|entry| entry.is_file()),
                "Human approval is required. Ask the reviewer to run atelier approve with this review ID");
            fs::remove_file(path)
                .map_err(|_| anyhow::anyhow!("Could not consume the local review approval"))?;
            proposal.approved = true;
        }
        if let Some((index, _)) = candidate(&snap.doc)? {
            snap.doc.active_layer = index;
            snap.doc.review_candidate_layer(Review::Accept);
        }
        let accepted_version = document::version(&snap.doc)?;
        match shared_save::save_raster(&self.path, &snap.doc) {
            Ok(publications) => {
                self.review = None;
                let publication = publications
                    .first()
                    .context("The approved drawing contained no publishable artwork")?;
                Ok(CallToolResult::structured(json!({
                    "published": true, "illustration": publication.asset_id, "artboard": publication.canvas_id,
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
            INSPECT => {
                let _: Empty = parse(args)?;
                self.inspect()?
            }
            GENERATE => self.generate(parse(args)?)?,
            POLL => self.poll(parse(args)?)?,
            PREVIEW => return self.preview(parse(args)?),
            PUBLISH => return self.publish(parse(args)?),
            _ => bail!("Unknown Atelier tool"),
        };
        Ok(CallToolResult::structured(result))
    }
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
            .with_instructions("Generate or revise a candidate, poll until done, then preview it visually. The human must approve the specific review from a separate local terminal before publication.")
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
        tool(INSPECT, "Inspect the selected illustration and saved drawing version.", json!({}), &[]),
        tool(GENERATE, "Start an image job for this artboard. Use revise=true to edit its current pending candidate as the input image.", json!({"session": uuid, "expected_version": version, "prompt": {"type": "string", "maxLength": 8192}, "revise": {"type": "boolean"}}), &["session", "expected_version", "prompt"]),
        tool(POLL, "Poll a queued image job and attach its completed result as an editable candidate.", json!({"session": uuid, "job": {"type": "integer", "minimum": 1}}), &["session", "job"]),
        tool(PREVIEW, "Inspect a bounded PNG preview and get the ID for human review of this exact drawing.", json!({"session": uuid, "expected_version": version}), &["session", "expected_version"]),
        tool(PUBLISH, "Publish the exact reviewed drawing after separate local human approval. Retry the same review if saving revision links fails.", json!({"session": uuid, "review": uuid}), &["session", "review"]),
    ]
}

#[cfg(test)]
mod tests;
