//! Shared, GUI-free playback host: snapshot compilation and exact resource checks.
pub mod fixture;
mod model;
pub mod stage;

use crate::{
    authoring::{artifact_kind, AssetCatalog},
    persistence::ProjectDb,
    registry::{ArtifactRegistry, RevisionStatus},
};
pub use model::PlaybackModel;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};
use storyfella_compiler::{AssetBinding, Origin, StoryPlan};
use storyfella_language::{analyze, Source, Span};
use storyfella_runtime::{AssetPolicy, Command, Event, Phase, Runtime, SavedState};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Visual {
    pub binding: AssetBinding,
    /// The image, normalized to PNG and retained for this fixed snapshot.
    /// Replacing the file or activating a newer revision cannot change an
    /// ongoing session.
    pub png: Arc<[u8]>,
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
        Self::plan_requires_project_with_policy(path, AssetPolicy::RequireCompleted)
    }
    pub fn plan_requires_project_with_policy(
        path: &Path,
        asset_policy: AssetPolicy,
    ) -> Result<bool, String> {
        let plan = read_plan(path)?;
        storyfella_runtime::validate_plan_with_policy(&plan, asset_policy)
            .map(|bindings| asset_policy == AssetPolicy::RequireCompleted && !bindings.is_empty())
            .map_err(|e| e.to_string())
    }
    pub fn compile(source: &str, registry: &ArtifactRegistry) -> Result<Self, String> {
        Self::compile_with_policy(source, registry, AssetPolicy::RequireCompleted)
    }
    /// Build one playback snapshot from all referenced story files.
    pub fn compile_workspace(
        workspace: &storyfella_compiler::SourceWorkspace,
        registry: &ArtifactRegistry,
        asset_policy: AssetPolicy,
    ) -> Result<Self, String> {
        let catalog = AssetCatalog::from_registry(registry);
        let (compilation, expanded) = storyfella_compiler::compile_workspace_with_catalog(
            workspace,
            &catalog.assets,
            &catalog.bindings,
        )
        .map_err(|error| format!("{}: {}", error.path, error.message))?;
        let plan = compilation.plan.ok_or_else(|| {
            compilation
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == storyfella_language::Severity::Error)
                .map(|diagnostic| {
                    let (path, span) = expanded.diagnostic_span(&workspace.entry, diagnostic.span);
                    let source = Source::new(&workspace.files[path]);
                    if let Some(position) = source.position(span.start) {
                        return format!(
                            "{} at {}:{}:{}: {}",
                            diagnostic.code,
                            path,
                            position.line + 1,
                            position.character + 1,
                            diagnostic.message
                        );
                    }
                    format!("{}: {}", diagnostic.code, diagnostic.message)
                })
                .collect::<Vec<_>>()
                .join("\n")
        })?;
        Self::from_plan_with_policy(plan, registry, asset_policy)
    }
    pub fn compile_with_policy(
        source: &str,
        registry: &ArtifactRegistry,
        asset_policy: AssetPolicy,
    ) -> Result<Self, String> {
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
        Self::from_plan_with_policy(plan, registry, asset_policy)
    }

    pub fn from_plan(plan: StoryPlan, registry: &ArtifactRegistry) -> Result<Self, String> {
        Self::from_plan_with_policy(plan, registry, AssetPolicy::RequireCompleted)
    }

    pub fn from_plan_with_policy(
        plan: StoryPlan,
        registry: &ArtifactRegistry,
        asset_policy: AssetPolicy,
    ) -> Result<Self, String> {
        let bindings = storyfella_runtime::validate_plan_with_policy(&plan, asset_policy)
            .map_err(|e| e.to_string())?;
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
                    png: png.into_inner().into(),
                })
            };
            match resolve() {
                Ok(visual) => visuals.push(visual),
                Err(_error) if asset_policy != AssetPolicy::RequireCompleted => {}
                Err(error) => {
                    return Err(format!(
                        "Artwork '{}' revision {}: {error}",
                        binding.name, binding.revision
                    ));
                }
            }
        }
        let (runtime, events) =
            Runtime::start_with_policy(plan, asset_policy).map_err(|e| e.to_string())?;
        Ok(Self {
            runtime,
            visuals,
            last_events: events,
        })
    }

    /// Open a shared project folder or manifest, a .sf/.db pair, or a compiled JSON plan.
    /// Refuse absent databases before ProjectDb can create one.
    pub fn open(input: &Path, project: Option<&Path>) -> Result<Self, String> {
        Self::open_with_policy(input, project, AssetPolicy::RequireCompleted)
    }

    pub fn open_with_policy(
        input: &Path,
        project: Option<&Path>,
        asset_policy: AssetPolicy,
    ) -> Result<Self, String> {
        if input.is_dir()
            || input
                .file_name()
                .is_some_and(|name| name == crate::project::MANIFEST)
        {
            if project.is_some() {
                return Err("--project is only used with compiled JSON plans".into());
            }
            let folder =
                crate::project::ProjectFolder::open(input).map_err(|error| error.to_string())?;
            return Self::open_with_policy(&folder.database(), None, asset_policy);
        }
        match input.extension().and_then(|e| e.to_str()) {
            Some("sf" | "db") => {
                if project.is_some() {
                    return Err("--project is only used with compiled JSON plans".into());
                }
                let database = crate::project::database_for_source(input)
                    .map_err(|error| error.to_string())?
                    .unwrap_or_else(|| input.with_extension("db"));
                let stored = load_project(&database)?;
                if storyfella_language::has_includes(stored.story.source()) {
                    let path = stored.story.path().ok_or("Story source has no path")?;
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or("Story filename is not UTF-8")?;
                    let drafts = std::collections::BTreeMap::from([(
                        name.to_owned(),
                        stored.story.source().to_owned(),
                    )]);
                    let workspace = crate::story::load_source_workspace(path, &drafts)
                        .map_err(|error| error.to_string())?;
                    Self::compile_workspace(&workspace, &stored.registry, asset_policy)
                } else {
                    Self::compile_with_policy(stored.story.source(), &stored.registry, asset_policy)
                }
            }
            Some("json") => {
                let plan = read_plan(input)?;
                let registry = match project {
                    Some(path) => load_project(path)?.registry,
                    None => ArtifactRegistry::new(),
                };
                Self::from_plan_with_policy(plan, &registry, asset_policy)
            }
            _ => Err("Open a Storyfella folder, storyfella.toml, .sf/.db project pair or compiled .json plan".into()),
        }
    }
    pub fn dispatch(&mut self, command: Command) -> Result<(), String> {
        let events = self.runtime.dispatch(command).map_err(|e| e.to_string())?;
        self.last_events = events;
        Ok(())
    }
    /// Where the line on stage was written: its file, relative to the
    /// entry's folder, and its span there. A single-file story is all
    /// `entry`. `None` once the story has finished.
    pub fn on_stage<'a>(&'a self, entry: &'a str) -> Option<(&'a str, Span)> {
        if self.runtime.state().phase == Phase::Complete {
            return None;
        }
        self.written(self.runtime.current().source, entry)
    }

    /// Plays from the start of the scene written around `offset` in `file`
    /// up to the line there, with every flag as the story begins. Stops
    /// early where the scene asks for a choice or leaves. `Ok(false)` when
    /// no scene is written there.
    pub fn play_to(&mut self, file: &str, offset: usize, entry: &str) -> Result<bool, String> {
        let plan = self.runtime.plan();
        let before = |span: Span| {
            self.written(span, entry)
                .filter(|(written, span)| *written == file && span.start <= offset)
                .map(|(_, span)| span.start)
        };
        let Some((scene, _)) = plan
            .scenes
            .iter()
            .enumerate()
            .filter_map(|(index, scene)| before(scene.source).map(|start| (index, start)))
            .max_by_key(|(_, start)| *start)
        else {
            return Ok(false);
        };
        let instructions = &plan.scenes[scene].instructions;
        let target = instructions
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction.origin == Origin::Authored)
            .filter_map(|(index, instruction)| {
                before(instruction.source).map(|start| (index, start))
            })
            .max_by_key(|(_, start)| *start)
            .map_or(0, |(index, _)| index);
        let (id, steps) = (plan.scenes[scene].id.clone(), instructions.len());
        self.dispatch(Command::Restart)?;
        self.dispatch(Command::Jump { scene: id })?;
        for _ in 0..steps {
            let state = self.runtime.state();
            if state.scene != scene || state.instruction >= target || state.phase != Phase::Text {
                break;
            }
            self.dispatch(Command::Advance)?;
        }
        Ok(true)
    }

    fn written<'a>(&'a self, span: Span, entry: &'a str) -> Option<(&'a str, Span)> {
        match &self.runtime.plan().source_map {
            Some(map) => map.original_span(span),
            None => Some((entry, span)),
        }
    }

    pub fn restore(&mut self, saved: SavedState) -> Result<(), String> {
        let runtime = Runtime::restore_with_policy(
            self.runtime.plan().clone(),
            saved,
            self.runtime.asset_policy(),
        )
        .map_err(|e| e.to_string())?;
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

    pub fn staged_visual(&self, actor: &storyfella_runtime::StagedCharacter) -> Option<&Visual> {
        let binding = actor.asset.binding.as_ref()?;
        self.visuals
            .iter()
            .find(|visual| &visual.binding == binding)
    }

    /// Active authored uses whose pinned image could not be resolved.
    pub fn missing_uses(&self) -> Vec<&storyfella_compiler::AssetUse> {
        self.runtime
            .active_uses()
            .iter()
            .filter(|asset| {
                asset.binding.as_ref().is_none_or(|binding| {
                    !self.visuals.iter().any(|visual| visual.binding == *binding)
                })
            })
            .collect()
    }
}

fn load_project(path: &Path) -> Result<crate::persistence::StoredProject, String> {
    if path.is_dir()
        || path
            .file_name()
            .is_some_and(|name| name == crate::project::MANIFEST)
    {
        let folder =
            crate::project::ProjectFolder::open(path).map_err(|error| error.to_string())?;
        return load_project(&folder.database());
    }
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
