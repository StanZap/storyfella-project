//! What the assistant is told: how to work, the language, and the request
//! with everything around it the model would otherwise spend steps finding.
use std::fmt::Write as _;

use storyfella_compiler::SourceWorkspace;
use storyfella_language::{parse, Node, NodeKind, Source};

use super::{check::Problem, scope::lines_of, tools::outline, AgentRequest, Scope};
use crate::registry::ArtifactRegistry;

pub(super) const SYSTEM: &str = r#"You are the writing assistant in Storyfella Studio. You work on a branching story written in the Storyfella DSL, across one or more files. The writer sees each file change as you edit it and reviews your changes afterwards, so work directly in the files.

How to work:
- Work in small steps. Read what you need with read_file, change it with edit_file, create files with write_file, and move whole scenes with move_scene.
- Every change answers with what the compiler says about the whole story. When your change caused an error, fix it next. The story may be incomplete for a step or two — a goto to a scene you are about to write — but do not finish with an error you caused.
- For a story across several files: create the files and stub every scene first (`scene NAME` with a line saying `end`), then wire the gotos and choices, then write each scene.
- Stay inside the scope you are given. An edit outside it is refused; if the request needs one, say so in your answer.
- Keep the story's facts — names, events, relationships — unless keeping facts is off.
- Errors that were in the story before the request are the writer's. Fix them only when asked, or when they are in your way.
- If the request is a question or needs no change, answer it without editing.
- If you are offered no tools, you have used all your steps: say what you did and what is left.
- Finish with one or two plain sentences saying what you changed. Never paste the story into your answer.

Storyfella DSL:
- Exactly two spaces per indentation level, no tabs. Each statement is one line.
- Scenes are `scene NAME` at column zero; beats are `  beat NAME` inside a scene; statements inside a beat have four spaces.
- Narration is `narrate: TEXT`; dialogue is `NAME: TEXT`; thoughts are `NAME think: TEXT`.
- Quoted text escapes double quotes as \" and backslashes as \\; use \n for a line break inside it.
- `goto NAME` continues at a scene; `end` ends a path.
- Choices are `choice "text"` followed by indented `option "text": SCENE_NAME` lines.
- Direction keys are shot, mood and style. Asset keys are cast (comma-separated names), character, location and prop; use only the assets listed.
- Names match [A-Za-z_][A-Za-z0-9_]*. Never invent declaration syntax.
- `include "file.sf"` lines in the entry file bring the other files in. Keep them; write_file adds one for each file it creates."#;

/// The first user turn: the request, its scope, where the writer is, the
/// story's shape, the assets it can name, and the errors it already has.
pub(super) fn request(
    request: &AgentRequest,
    workspace: &SourceWorkspace,
    registry: &ArtifactRegistry,
    before: &[Problem],
) -> String {
    let mut text = format!("Request: {}\n", request.instructions.trim());
    let scoped = match &request.scope {
        Scope::Passage { file, .. } => workspace.files.get(file).map(String::as_str),
        _ => None,
    };
    let _ = writeln!(text, "Scope: {}", request.scope.describe(scoped));
    let _ = writeln!(
        text,
        "Keep facts: {}",
        if request.preserve_facts { "yes" } else { "no" }
    );
    if let Some(editor) = &request.editor {
        if let Some(source) = workspace.files.get(&editor.file) {
            text.push_str(&where_the_writer_is(&editor.file, source, editor));
        }
    }
    let _ = write!(text, "\nStory files:\n{}", outline(workspace));
    if !registry.artifacts.is_empty() {
        text.push_str("\nAssets:\n");
        for asset in &registry.artifacts {
            let description = asset.description.lines().next().unwrap_or_default();
            let _ = writeln!(text, "- {}: {description}", asset.name);
        }
    }
    if !before.is_empty() {
        let _ = writeln!(text, "\nErrors the story has before this request:");
        for problem in before {
            let _ = writeln!(text, "{}", problem.full);
        }
    }
    if let Some(previous) = &request.continuing {
        let _ = write!(
            text,
            "\nThis continues a request that ran out of steps. Its last answer was:\n{}\n\
             Carry on from the files as they are now.\n",
            previous.trim()
        );
    }
    text
}

/// "The writer is in story.sf at line 12, in beat arrival of scene harbor.",
/// with the selection when there is one.
fn where_the_writer_is(file: &str, source: &str, editor: &super::EditorContext) -> String {
    let caret = editor.caret.min(source.len());
    let line = Source::new(source)
        .position(caret)
        .map_or(1, |at| at.line + 1);
    let mut text = format!("Editor: the writer is in {file} at line {line}");
    let scenes = parse(source).story.scenes;
    if let Some((scene, NodeKind::Scene { name: scene_name })) =
        around(&scenes, caret).map(|scene| (scene, &scene.kind))
    {
        match around(&scene.children, caret).map(|beat| &beat.kind) {
            Some(NodeKind::Beat { name }) => {
                let _ = write!(
                    text,
                    ", in beat {} of scene {}",
                    name.value, scene_name.value
                );
            }
            _ => {
                let _ = write!(text, ", in scene {}", scene_name.value);
            }
        }
    }
    text.push('.');
    if let Some(selection) = editor
        .selection
        .filter(|span| span.start < span.end && source.get(span.start..span.end).is_some())
    {
        let _ = write!(
            text,
            " Selected{}:\n{}",
            lines_of(source, selection),
            &source[selection.start..selection.end]
        );
    }
    text.push('\n');
    text
}

/// The node in `nodes` that contains `offset`.
fn around(nodes: &[Node], offset: usize) -> Option<&Node> {
    nodes
        .iter()
        .find(|node| node.span.contains(offset) || node.span.end == offset)
}
