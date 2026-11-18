//! What the assistant column asks of the writing workflows: which passage —
//! or the whole project — a request is about, and running it off the UI
//! thread. Nothing here draws.
use std::{ops::Range, path::PathBuf};

use storyfella_language::{Source, Span};
use storyfella_studio_core::{
    planner::PlannerBackend,
    registry::ArtifactRegistry,
    workflows::{
        assistant::{Place, Suggestion},
        framework::SharedStreamingModel,
        novel::{self, NovelRequest, NovelResult},
        writing::{self, WritingAction, WritingRequest, WritingResult, WritingScope},
        Cancellation, Feedback, RunStore,
    },
};

use crate::{state::AppState, writing as files};

/// What a request is about: a passage of the open file, or every file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    Selection,
    #[default]
    Beat,
    Scene,
    File,
    Project,
}

impl Scope {
    pub const ALL: [Self; 5] = [
        Self::Selection,
        Self::Beat,
        Self::Scene,
        Self::File,
        Self::Project,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Selection => "Selection",
            Self::Beat => "Beat",
            Self::Scene => "Scene",
            Self::File => "File",
            Self::Project => "Project",
        }
    }
}

/// A request as sent: one passage of a file, or the whole project.
#[derive(Clone, Debug)]
pub enum Request {
    Passage {
        file: String,
        request: WritingRequest,
    },
    Project(NovelRequest),
}

impl Request {
    /// How the request's reach reads under it, such as "Beat · lines 4–9 of
    /// chapter.sf" or "Project · 3 files".
    pub fn describe(&self, scope: Scope) -> String {
        match self {
            Self::Passage { file, request } => {
                describe(scope, file, &request.source, request.range)
            }
            Self::Project(request) => match request.workspace.files.len() {
                1 => "Project · 1 file".into(),
                count => format!("Project · {count} files"),
            },
        }
    }

    /// Where the passage begins; none for the whole project.
    pub fn place(&self) -> Option<Place> {
        match self {
            Self::Passage { file, request } => Some(Place {
                file: file.clone(),
                offset: request.range.start,
            }),
            Self::Project(_) => None,
        }
    }
}

/// How a request was answered.
#[derive(Clone, Debug)]
pub enum Answered {
    Edit(Box<Suggestion>),
    Reply(String),
}

/// Where the story's workflow runs and conversation are kept.
pub fn store_path(state: &AppState) -> Option<PathBuf> {
    state
        .project_path
        .as_ref()
        .map(|project| project.with_extension("workflows.sqlite3"))
}

/// "Beat · lines 4–9 of chapter.sf", how a request's passage reads.
pub fn describe(scope: Scope, file: &str, source: &str, range: Span) -> String {
    let text = Source::new(source);
    let line = |offset: usize| text.position(offset).map_or(1, |at| at.line + 1);
    let first = line(range.start);
    // A range ending at a line break belongs to the line before it.
    let last = line(range.end.saturating_sub(1).max(range.start));
    if first == last {
        format!("{} · line {first} of {file}", scope.label())
    } else {
        format!("{} · lines {first}–{last} of {file}", scope.label())
    }
}

/// A request about what `scope` names: the passage of `file` around the
/// caret, or the whole project, following `instructions`.
pub fn capture(
    state: &AppState,
    file: &str,
    scope: Scope,
    selection: Range<usize>,
    instructions: &str,
    keep_facts: bool,
) -> Result<Request, String> {
    if scope == Scope::Project {
        return capture_project(state, instructions, keep_facts).map(Request::Project);
    }
    capture_passage(state, file, scope, selection, instructions, keep_facts).map(|request| {
        Request::Passage {
            file: file.to_owned(),
            request,
        }
    })
}

/// Every file of the story as it stands in the editor, drafts included.
pub fn capture_project(
    state: &AppState,
    instructions: &str,
    keep_facts: bool,
) -> Result<NovelRequest, String> {
    let project = state.project_path.clone().ok_or("Open a story first.")?;
    let workspace = state.workspace_snapshot()?;
    Ok(NovelRequest {
        project,
        workspace,
        action: WritingAction::Rewrite,
        instructions: instructions.trim().to_owned(),
        preserve_facts: keep_facts,
        catalog_digest: novel::catalog_digest(&state.registry)
            .map_err(|error| error.to_string())?,
    })
}

/// A rewrite of the passage of `file` that `scope` names around the caret.
pub fn capture_passage(
    state: &AppState,
    file: &str,
    scope: Scope,
    selection: Range<usize>,
    instructions: &str,
    keep_facts: bool,
) -> Result<WritingRequest, String> {
    let project = state.project_path.as_deref().ok_or("Open a story first.")?;
    let folder = state
        .story
        .path()
        .and_then(|entry| entry.parent())
        .ok_or("Save the story first.")?;
    let source = files::source(state, file).ok_or("Open a story file first.")?;
    if source.trim().is_empty() {
        return Err(format!(
            "{file} is empty. The assistant edits passages that exist, so write a first line, \
             then ask."
        ));
    }
    let scope = match scope {
        Scope::Selection if selection.is_empty() => {
            return Err("Select the passage to work on first.".into())
        }
        Scope::Selection => WritingScope::Selection(Span::new(selection.start, selection.end)),
        Scope::Beat => WritingScope::Beat(selection.start),
        Scope::Scene => WritingScope::Scene(selection.start),
        Scope::File | Scope::Project => WritingScope::File,
    };
    let mut request =
        WritingRequest::capture(project, &folder.join(file), source, scope, &state.registry)
            .map_err(|error| error.to_string())?;
    request.action = WritingAction::Rewrite;
    request.instructions = instructions.trim().to_owned();
    request.preserve_facts = keep_facts;
    Ok(request)
}

/// What writes the suggestion: the provider chosen in Settings, or a model
/// handed in directly, as tests do.
#[derive(Clone)]
pub enum Engine {
    Provider(PlannerBackend),
    Model(SharedStreamingModel),
}

/// Runs `request` to a suggestion or a reply on its own runtime; meant for a
/// worker thread. `Ok(None)` means it was cancelled.
pub fn suggest(
    request: Request,
    engine: Engine,
    registry: ArtifactRegistry,
    store: PathBuf,
    cancel: Cancellation,
    feedback: impl Fn(Feedback) + Send + Sync + 'static,
) -> anyhow::Result<Option<Answered>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let store = RunStore::open(&store)?;
        Ok(match request {
            Request::Passage { file, request } => {
                let result = match engine {
                    Engine::Provider(backend) => {
                        writing::run_with_backend(
                            request, &backend, &registry, &store, cancel, feedback,
                        )
                        .await?
                    }
                    Engine::Model(model) => {
                        writing::run_with_model(request, model, &registry, &store, cancel, feedback)
                            .await?
                    }
                };
                result.map(|result| match result {
                    WritingResult::Proposal(proposal) => {
                        Answered::Edit(Box::new(Suggestion::Passage { file, proposal }))
                    }
                    WritingResult::Reply { text, .. } => Answered::Reply(text),
                })
            }
            Request::Project(request) => {
                let result = match engine {
                    Engine::Provider(backend) => {
                        novel::run_with_backend(
                            request, &backend, &registry, &store, cancel, feedback,
                        )
                        .await?
                    }
                    Engine::Model(model) => {
                        novel::run_with_model(request, model, &registry, &store, cancel, feedback)
                            .await?
                    }
                };
                result.map(|result| match result {
                    NovelResult::Proposal(proposal) => {
                        Answered::Edit(Box::new(Suggestion::Project(proposal)))
                    }
                    NovelResult::Reply { text, .. } => Answered::Reply(text),
                })
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passage_reads_as_its_scope_and_lines() {
        let source = "scene harbor\n  narrate: Dusk.\n  end\n";
        assert_eq!(
            describe(Scope::File, "story.sf", source, Span::new(0, source.len())),
            "File · lines 1–3 of story.sf"
        );
        assert_eq!(
            describe(Scope::Selection, "story.sf", source, Span::new(15, 20)),
            "Selection · line 2 of story.sf"
        );
    }
}
