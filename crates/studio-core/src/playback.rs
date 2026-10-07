//! Shared, GUI-free playback host: snapshot compilation and exact resource checks.
pub mod fixture;
use crate::{
    authoring::{artifact_kind, AssetCatalog},
    persistence::ProjectDb,
    registry::{ArtifactRegistry, RevisionStatus},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
use storyfella_compiler::{AssetBinding, StoryPlan};
use storyfella_language::{analyze, Source};
use storyfella_runtime::{Command, Event, Runtime, SavedState};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Visual {
    pub binding: AssetBinding,
    /// Decoded and normalized PNG retained for this fixed snapshot. Replacing
    /// the file or activating a newer revision cannot change an ongoing session.
    pub image_url: String,
}

#[derive(Clone, Debug)]
pub struct PlaybackSession {
    pub runtime: Runtime,
    pub visuals: Vec<Visual>,
    /// Only the most recent bounded batch, so cyclic playback does not grow
    /// an unbounded event log in a visual host.
    pub last_events: Vec<Event>,
}

impl PlaybackSession {
    /// Used by the Player's open dialog to request a resource project only
    /// when a valid ready compiled plan actually contains artwork bindings.
    pub fn plan_requires_project(path: &Path) -> Result<bool, String> {
        let plan = read_plan(path)?;
        storyfella_runtime::validate_plan(&plan)
            .map(|bindings| !bindings.is_empty())
            .map_err(|e| e.to_string())
    }
    pub fn compile(source: &str, registry: &ArtifactRegistry) -> Result<Self, String> {
        let catalog = AssetCatalog::from_registry(registry);
        let analysis = analyze(source, &catalog.assets);
        let compilation = storyfella_compiler::compile_analysis(&analysis, &catalog.bindings);
        let plan = compilation.plan.ok_or_else(|| {
            let source = Source::new(source);
            compilation
                .diagnostics
                .iter()
                .filter(|d| d.severity == storyfella_language::Severity::Error)
                .map(|d| {
                    let p = source.position(d.span.start).unwrap();
                    format!(
                        "{} at {}:{}: {}",
                        d.code,
                        p.line + 1,
                        p.character + 1,
                        d.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })?;
        Self::from_plan(plan, registry)
    }

    pub fn from_plan(plan: StoryPlan, registry: &ArtifactRegistry) -> Result<Self, String> {
        let bindings = storyfella_runtime::validate_plan(&plan).map_err(|e| e.to_string())?;
        let mut visuals = Vec::new();
        for binding in bindings {
            let resolve = || -> Result<Visual, String> {
                let id = Uuid::parse_str(&binding.id)
                    .map_err(|_| "asset identity must be a project UUID".to_string())?;
                let revision_id = Uuid::parse_str(&binding.revision)
                    .map_err(|_| "revision identity must be a project UUID".to_string())?;
                let artifact = registry
                    .artifact(id)
                    .ok_or("asset is absent from this project")?;
                if artifact.kind != artifact_kind(binding.kind) || artifact.name != binding.name {
                    return Err("project asset name/kind differs from the binding".into());
                }
                let revision = artifact
                    .revisions
                    .iter()
                    .find(|r| r.id == revision_id)
                    .ok_or("pinned revision is missing; no newer revision is substituted")?;
                if revision.status != RevisionStatus::Completed {
                    return Err("pinned revision is not completed".into());
                }
                let path = PathBuf::from(
                    revision
                        .asset_path
                        .as_ref()
                        .ok_or("pinned revision has no image path")?,
                );
                if !path.is_absolute() {
                    return Err("image paths must currently be absolute".into());
                }
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let image = image::load_from_memory(&bytes)
                    .map_err(|e| format!("cannot decode {}: {e}", path.display()))?;
                let mut png = Cursor::new(Vec::new());
                image
                    .write_to(&mut png, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
                Ok(Visual {
                    binding: binding.clone(),
                    image_url: format!(
                        "data:image/png;base64,{}",
                        STANDARD.encode(png.into_inner())
                    ),
                })
            };
            visuals.push(resolve().map_err(|e| {
                format!(
                    "Artwork '{}' revision {}: {e}",
                    binding.name, binding.revision
                )
            })?);
        }
        let (runtime, events) = Runtime::start(plan).map_err(|e| e.to_string())?;
        Ok(Self {
            runtime,
            visuals,
            last_events: events,
        })
    }

    /// Open a .sf/.db pair, or a compiled JSON plan with an optional resource DB.
    /// Refuse absent databases before ProjectDb can create one.
    pub fn open(input: &Path, project: Option<&Path>) -> Result<Self, String> {
        match input.extension().and_then(|e| e.to_str()) {
            Some("sf" | "db") => {
                if project.is_some() {
                    return Err("--project is only used with compiled JSON plans".into());
                }
                let database = input.with_extension("db");
                let stored = load_project(&database)?;
                Self::compile(stored.story.source(), &stored.registry)
            }
            Some("json") => {
                let plan = read_plan(input)?;
                let registry = match project {
                    Some(path) => load_project(path)?.registry,
                    None => ArtifactRegistry::new(),
                };
                Self::from_plan(plan, &registry)
            }
            _ => Err("Open a .sf/.db project pair or compiled .json plan".into()),
        }
    }
    pub fn dispatch(&mut self, command: Command) -> Result<(), String> {
        let events = self.runtime.dispatch(command).map_err(|e| e.to_string())?;
        self.last_events = events;
        Ok(())
    }
    pub fn restore(&mut self, saved: SavedState) -> Result<(), String> {
        let runtime =
            Runtime::restore(self.runtime.plan().clone(), saved).map_err(|e| e.to_string())?;
        self.runtime = runtime;
        self.last_events.clear();
        Ok(())
    }
    pub fn active_visuals(&self) -> Vec<&Visual> {
        self.runtime
            .state()
            .assets
            .iter()
            .filter_map(|binding| self.visuals.iter().find(|v| v.binding == *binding))
            .collect()
    }
}

fn load_project(path: &Path) -> Result<crate::persistence::StoredProject, String> {
    if !path.is_file() {
        return Err(format!(
            "Project database does not exist: {}",
            path.display()
        ));
    }
    ProjectDb::open(path)
        .and_then(|db| db.load())
        .map_err(|e| e.to_string())
}

fn read_plan(path: &Path) -> Result<StoryPlan, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid plan JSON: {e}"))
}
