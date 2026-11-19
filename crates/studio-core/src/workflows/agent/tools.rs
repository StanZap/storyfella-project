//! The tools the assistant works with: look over the story, read a file, make
//! an exact edit, write or create a file, and move a scene. Every change goes
//! to the writer's draft at once, is held to the request's scope, and answers
//! with what the compiler now says about the whole story.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use agentkit_core::{ToolCall, ToolDefinition, ToolResult, ToolResultStatus, ToolRisk};
use agentkit_workflow::WorkflowToolSource;
use serde::Deserialize;
use serde_json::json;
use storyfella_compiler::SourceWorkspace;
use storyfella_language::{parse, NodeKind, Source, Span};
use tokio::sync::Mutex;

use super::{
    check::{self, Problem},
    moves::{self, Move},
    scope::{lines_of, Scope},
    StoryDraft,
};
use crate::{authoring::AssetCatalog, registry::ArtifactRegistry, workflows::Feedback};

/// The most lines one read returns.
const READ_LINES: usize = 400;
/// The most bytes one read returns.
const READ_BYTES: usize = 32 * 1024;

pub(super) type Sink = Arc<dyn Fn(Feedback) + Send + Sync>;

/// The story's tools for one request.
pub(super) struct StoryTools {
    draft: Arc<dyn StoryDraft>,
    registry: ArtifactRegistry,
    feedback: Sink,
    /// Held for a whole call, so calls made together run one after another
    /// against the draft rather than racing each other's writes.
    state: Mutex<State>,
}

struct State {
    scope: Scope,
    /// The scoped file's text as of the last call, to follow a passage
    /// through the writer's own edits.
    seen: Option<String>,
    /// The errors the story had when the request began.
    before: Vec<Problem>,
    /// The files this request has changed.
    changed: BTreeSet<String>,
}

/// A tool's refusal or failure, said to the model.
struct Refused(String);

impl<T: Into<String>> From<T> for Refused {
    fn from(message: T) -> Self {
        Self(message.into())
    }
}

impl StoryTools {
    pub(super) fn new(
        draft: Arc<dyn StoryDraft>,
        registry: ArtifactRegistry,
        scope: Scope,
        workspace: &SourceWorkspace,
        before: Vec<Problem>,
        feedback: Sink,
    ) -> Self {
        let seen = match &scope {
            Scope::Passage { file, .. } => workspace.files.get(file).cloned(),
            _ => None,
        };
        Self {
            draft,
            registry,
            feedback,
            state: Mutex::new(State {
                scope,
                seen,
                before,
                changed: BTreeSet::new(),
            }),
        }
    }

    /// The files this request has changed so far.
    pub(super) async fn changed(&self) -> BTreeSet<String> {
        self.state.lock().await.changed.clone()
    }

    /// The scope as the edits so far have moved it.
    pub(super) async fn scope(&self) -> Scope {
        self.state.lock().await.scope.clone()
    }

    fn status(&self, label: String) {
        (self.feedback)(Feedback::Status(label));
    }

    fn detail(&self, detail: String) {
        (self.feedback)(Feedback::Detail(detail));
    }

    /// The draft as it is now, with the passage moved to where the writer's
    /// edits since the last call put it.
    async fn current(&self, state: &mut State) -> Result<SourceWorkspace, Refused> {
        let workspace = self.draft.workspace().await.map_err(Refused)?;
        if let Scope::Passage { file, .. } = &state.scope {
            let file = file.clone();
            let now = workspace.files.get(&file).cloned();
            if let (Some(seen), Some(now)) = (&state.seen, &now) {
                state.scope.follow(&file, seen, now);
            }
            state.seen = now;
        }
        Ok(workspace)
    }

    async fn run(&self, name: &str, arguments: serde_json::Value) -> Result<String, Refused> {
        let mut state = self.state.lock().await;
        let workspace = self.current(&mut state).await?;
        match name {
            "outline" => {
                self.status("Looking over the story".into());
                Ok(format!(
                    "{}\nScope: {}",
                    outline(&workspace),
                    describe_scope(&state.scope, &workspace)
                ))
            }
            "read_file" => {
                let ReadFile {
                    path,
                    start_line,
                    end_line,
                } = arguments_of(arguments)?;
                let text = file(&workspace, &path)?;
                let (shown, first, last, total) = read(text, start_line, end_line);
                self.status(format!("Reading {path}"));
                self.detail(if first > last {
                    format!("{total} lines")
                } else {
                    format!("Lines {first}–{last} of {total}")
                });
                Ok(shown)
            }
            "edit_file" => {
                let EditFile {
                    path,
                    old_text,
                    new_text,
                } = arguments_of(arguments)?;
                self.status(format!("Editing {path}"));
                let text = file(&workspace, &path)?;
                if old_text.is_empty() {
                    return Err(
                        "old_text is empty. Quote the exact text to replace; to add \
                                text, quote the line it goes after and repeat it in new_text."
                            .into(),
                    );
                }
                let at = unique(text, &old_text, &path)?;
                let range = at..at + old_text.len();
                if !state.scope.allows(&path, range.clone()) {
                    return Err(outside(&state.scope, &workspace, &path, text, range));
                }
                let mut edited = text.to_owned();
                edited.replace_range(range, &new_text);
                let lines = lines_of(&edited, Span::new(at, at + new_text.len()));
                let report = self
                    .write(
                        &mut state,
                        workspace,
                        BTreeMap::from([(path.clone(), edited)]),
                    )
                    .await?;
                Ok(format!("Edited {path}{lines}. {report}"))
            }
            "write_file" => {
                let WriteFile { path, text } = arguments_of(arguments)?;
                let texts = match workspace.files.get(&path) {
                    Some(old) => {
                        self.status(format!("Rewriting {path}"));
                        if !state.scope.allows(&path, 0..old.len()) {
                            return Err(outside(
                                &state.scope,
                                &workspace,
                                &path,
                                old,
                                0..old.len(),
                            ));
                        }
                        BTreeMap::from([(path.clone(), text)])
                    }
                    None => {
                        self.status(format!("Creating {path}"));
                        if !state.scope.is_project() {
                            return Err(format!(
                                "Creating {path} is outside the scope. {}",
                                describe_scope(&state.scope, &workspace)
                            )
                            .into());
                        }
                        let mut texts = moves::apply(&workspace, std::slice::from_ref(&path), &[])?;
                        texts.insert(path.clone(), text);
                        texts
                    }
                };
                let created = !workspace.files.contains_key(&path);
                let report = self.write(&mut state, workspace, texts).await?;
                Ok(if created {
                    format!("Created {path} and included it from the entry file. {report}")
                } else {
                    format!("Wrote {path}. {report}")
                })
            }
            "move_scene" => {
                let MoveScene { scene, to } = arguments_of(arguments)?;
                self.status(format!("Moving scene {scene} to {to}"));
                if !state.scope.is_project() {
                    return Err(format!(
                        "Moving a scene is outside the scope. {}",
                        describe_scope(&state.scope, &workspace)
                    )
                    .into());
                }
                let new_files = if workspace.files.contains_key(&to) {
                    Vec::new()
                } else {
                    vec![to.clone()]
                };
                let texts = moves::apply(
                    &workspace,
                    &new_files,
                    &[Move {
                        scene: scene.clone(),
                        to: to.clone(),
                    }],
                )?;
                let report = self.write(&mut state, workspace, texts).await?;
                Ok(format!("Moved scene {scene} to {to}. {report}"))
            }
            other => Err(format!(
                "There is no tool named {other}. The tools are outline, read_file, edit_file, \
                 write_file and move_scene."
            )
            .into()),
        }
    }

    /// Puts `texts` in the draft, still based on `workspace`, and checks the
    /// story with them.
    async fn write(
        &self,
        state: &mut State,
        mut workspace: SourceWorkspace,
        texts: BTreeMap<String, String>,
    ) -> Result<String, Refused> {
        self.draft
            .write(workspace.clone(), texts.clone())
            .await
            .map_err(|error| format!("{error} Read the file again before editing it."))?;
        for (path, text) in texts {
            if let Some(old) = workspace.files.get(&path) {
                state.scope.follow(&path, old, &text);
            }
            if matches!(&state.scope, Scope::Passage { file, .. } if *file == path) {
                state.seen = Some(text.clone());
            }
            state.changed.insert(path.clone());
            workspace.files.insert(path, text);
        }
        let found = check::problems(&workspace, &AssetCatalog::from_registry(&self.registry));
        self.detail(check::summary(&found));
        Ok(check::report(&workspace, &found, &state.before))
    }
}

#[async_trait::async_trait]
impl WorkflowToolSource for StoryTools {
    async fn discover(&self) -> Result<Vec<ToolDefinition>, String> {
        Ok(definitions())
    }

    async fn call(&self, call: ToolCall) -> Result<ToolResult, String> {
        let (status, content) = match self.run(&call.name, call.arguments).await {
            Ok(content) => (ToolResultStatus::Success, content),
            Err(Refused(message)) => {
                self.detail(message.lines().next().unwrap_or_default().to_owned());
                (ToolResultStatus::Error, message)
            }
        };
        Ok(ToolResult {
            call_id: call.id,
            tool_name: call.name,
            status,
            content,
            metadata: None,
        })
    }

    /// The writer reviews the changes in the editor afterwards, so no call
    /// waits for approval.
    fn requires_approval(&self, _tool: &ToolDefinition) -> bool {
        false
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadFile {
    path: String,
    start_line: Option<usize>,
    end_line: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditFile {
    path: String,
    old_text: String,
    new_text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteFile {
    path: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveScene {
    scene: String,
    to: String,
}

fn arguments_of<T: serde::de::DeserializeOwned>(
    arguments: serde_json::Value,
) -> Result<T, Refused> {
    serde_json::from_value(arguments)
        .map_err(|error| Refused(format!("The arguments do not fit the tool: {error}")))
}

fn definitions() -> Vec<ToolDefinition> {
    let tool =
        |name: &str, description: &str, risk: ToolRisk, schema: serde_json::Value| ToolDefinition {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            risk,
            capabilities: Vec::new(),
        };
    let path = json!({"type": "string", "description": "A story file, exactly as outline lists it, such as chapter_two.sf"});
    vec![
        tool(
            "outline",
            "List the story's files with their scenes and beats and the lines each spans, and say what this request may change.",
            ToolRisk::ReadOnly,
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ),
        tool(
            "read_file",
            "Read a story file with line numbers. Give start_line and end_line to read part of a long file.",
            ToolRisk::ReadOnly,
            json!({"type": "object", "properties": {
                "path": path,
                "start_line": {"type": "integer", "minimum": 1},
                "end_line": {"type": "integer", "minimum": 1}
            }, "required": ["path"], "additionalProperties": false}),
        ),
        tool(
            "edit_file",
            "Replace one exact piece of a file. old_text must appear exactly once in the file, copied exactly, without line numbers; include a nearby line to make it unique. Returns what the compiler says about the whole story.",
            ToolRisk::Mutating,
            json!({"type": "object", "properties": {
                "path": path,
                "old_text": {"type": "string"},
                "new_text": {"type": "string"}
            }, "required": ["path", "old_text", "new_text"], "additionalProperties": false}),
        ),
        tool(
            "write_file",
            "Write a whole file. A new file is created next to the story's files and included from the entry file. Returns what the compiler says about the whole story.",
            ToolRisk::Mutating,
            json!({"type": "object", "properties": {
                "path": path,
                "text": {"type": "string"}
            }, "required": ["path", "text"], "additionalProperties": false}),
        ),
        tool(
            "move_scene",
            "Move a whole scene, unchanged, to another file, creating that file when it does not exist. Returns what the compiler says about the whole story.",
            ToolRisk::Mutating,
            json!({"type": "object", "properties": {
                "scene": {"type": "string", "description": "The scene's name"},
                "to": path
            }, "required": ["scene", "to"], "additionalProperties": false}),
        ),
    ]
}

/// The text of `path`, or why there is none.
fn file<'a>(workspace: &'a SourceWorkspace, path: &str) -> Result<&'a str, Refused> {
    workspace
        .files
        .get(path)
        .map(String::as_str)
        .ok_or_else(|| {
            let files: Vec<_> = workspace.files.keys().map(String::as_str).collect();
            Refused(format!(
                "{path} is not a file of this story. The files are {}.",
                files.join(", ")
            ))
        })
}

/// Where `needle` is in `text`, when it is there exactly once.
fn unique(text: &str, needle: &str, path: &str) -> Result<usize, Refused> {
    let mut found = text.match_indices(needle).map(|(at, _)| at);
    match (found.next(), found.next()) {
        (Some(at), None) => Ok(at),
        (None, _) => Err(format!(
            "old_text is not in {path}. Read the file again and copy the text exactly, without \
             line numbers."
        )
        .into()),
        (Some(_), Some(_)) => Err(format!(
            "old_text appears more than once in {path}. Include a nearby line to make it unique."
        )
        .into()),
    }
}

/// Why replacing `range` of `path` was refused.
fn outside(
    scope: &Scope,
    workspace: &SourceWorkspace,
    path: &str,
    text: &str,
    range: std::ops::Range<usize>,
) -> Refused {
    Refused(format!(
        "Refused: {path}{} is outside the scope. {} Say in your answer what else would need to \
         change.",
        lines_of(text, Span::new(range.start, range.end)),
        describe_scope(scope, workspace)
    ))
}

fn describe_scope(scope: &Scope, workspace: &SourceWorkspace) -> String {
    let source = match scope {
        Scope::Passage { file, .. } => workspace.files.get(file).map(String::as_str),
        _ => None,
    };
    scope.describe(source)
}

/// Lines `start..=end` of `text`, numbered, within the read limits; with
/// the first and last line shown and how many there are.
fn read(text: &str, start: Option<usize>, end: Option<usize>) -> (String, usize, usize, usize) {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return ("The file is empty.".into(), 1, 0, 0);
    }
    let first = start.unwrap_or(1).clamp(1, total);
    let mut last = end
        .unwrap_or(total)
        .clamp(first, total)
        .min(first + READ_LINES - 1);
    let width = total.to_string().len();
    let mut shown = String::new();
    for (index, line) in lines[first - 1..last].iter().enumerate() {
        let numbered = format!("{:>width$}| {line}\n", first + index);
        if shown.len() + numbered.len() > READ_BYTES {
            last = first + index - 1;
            break;
        }
        shown.push_str(&numbered);
    }
    if first > 1 || last < total {
        shown.push_str(&format!(
            "(lines {first}–{last} of {total}; read others with start_line and end_line)\n"
        ));
    }
    (shown, first, last, total)
}

/// Every file with its scenes and beats, and the lines each spans.
pub(super) fn outline(workspace: &SourceWorkspace) -> String {
    let mut outline = String::new();
    for (path, text) in &workspace.files {
        let lines = text.lines().count();
        let entry = if *path == workspace.entry {
            ", the entry file"
        } else {
            ""
        };
        outline.push_str(&format!("{path} ({lines} lines{entry})\n"));
        let source = Source::new(text.as_str());
        let line = |offset: usize| source.position(offset).map_or(1, |at| at.line + 1);
        for scene in parse(text).story.scenes {
            let NodeKind::Scene { name } = &scene.kind else {
                continue;
            };
            let beats: Vec<_> = scene
                .children
                .iter()
                .filter_map(|child| match &child.kind {
                    NodeKind::Beat { name } => Some(name.value.as_str()),
                    _ => None,
                })
                .collect();
            let end = line(scene.span.end.saturating_sub(1).max(scene.span.start));
            outline.push_str(&format!(
                "  scene {} (lines {}–{end})",
                name.value,
                line(scene.span.start)
            ));
            if !beats.is_empty() {
                outline.push_str(&format!(": beats {}", beats.join(", ")));
            }
            outline.push('\n');
        }
    }
    outline
}
