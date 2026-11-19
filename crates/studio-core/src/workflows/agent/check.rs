//! What the compiler says about the story as a whole, written for the model
//! that has just changed it: where each error is, what is wrong, and the
//! names it could have meant.
use storyfella_compiler::{compile_workspace_with_catalog, SourceWorkspace};
use storyfella_language::{parse, NodeKind, Severity, Source};

use crate::authoring::AssetCatalog;

/// The most errors one check lists; the count says how many there are.
const LISTED: usize = 12;

/// One error, in the file it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Problem {
    pub(super) path: String,
    /// The file, code and message, which an edit elsewhere in the file does
    /// not change, unlike the line.
    cause: String,
    /// `path:line:column: code: message`, then the line itself.
    pub(super) full: String,
}

/// The errors `workspace` compiles with.
pub(super) fn problems(workspace: &SourceWorkspace, catalog: &AssetCatalog) -> Vec<Problem> {
    let (compilation, expanded) =
        match compile_workspace_with_catalog(workspace, &catalog.assets, &catalog.bindings) {
            Ok(compiled) => compiled,
            Err(error) => {
                return vec![Problem {
                    path: error.path.clone(),
                    cause: format!("{}: {}", error.path, error.message),
                    full: error.to_string(),
                }]
            }
        };
    compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| {
            let (path, span) = expanded.diagnostic_span(&workspace.entry, diagnostic.span);
            let text = Source::new(workspace.files.get(path).map_or("", String::as_str));
            let position = text.position(span.start).unwrap_or_default();
            let line = text
                .line_span(position.line as usize)
                .and_then(|span| text.slice(span))
                .unwrap_or("");
            Problem {
                path: path.to_owned(),
                cause: format!("{path}: {}: {}", diagnostic.code, diagnostic.message),
                full: format!(
                    "{path}:{}:{}: {}: {}\n> {}",
                    position.line + 1,
                    position.character + 1,
                    diagnostic.code,
                    diagnostic.message,
                    line.trim_end()
                ),
            }
        })
        .collect()
}

/// How many of `found` were already in `before`, matching each one once.
fn carried_over(before: &[Problem], found: &[Problem]) -> usize {
    let mut unmatched: Vec<&str> = before
        .iter()
        .map(|problem| problem.cause.as_str())
        .collect();
    found
        .iter()
        .filter(
            |problem| match unmatched.iter().position(|cause| *cause == problem.cause) {
                Some(index) => {
                    unmatched.swap_remove(index);
                    true
                }
                None => false,
            },
        )
        .count()
}

/// The check as the model reads it after an edit. `before` is what the story
/// had when the request began, so the model can tell its own errors from the
/// writer's.
pub(super) fn report(workspace: &SourceWorkspace, found: &[Problem], before: &[Problem]) -> String {
    if found.is_empty() {
        return "The story compiles.".into();
    }
    let mut report = match found.len() {
        1 => "The story has 1 error:".to_owned(),
        count => format!("The story has {count} errors:"),
    };
    for problem in found.iter().take(LISTED) {
        report.push('\n');
        report.push_str(&problem.full);
    }
    if found.len() > LISTED {
        report.push_str(&format!("\n…and {} more.", found.len() - LISTED));
    }
    match carried_over(before, found) {
        0 => {}
        old if old == found.len() => {
            report.push_str("\nAll of them were there before this request.")
        }
        old => report.push_str(&format!("\n{old} of them were there before this request.")),
    }
    let scenes = scene_names(workspace);
    if !scenes.is_empty() {
        report.push_str(&format!("\nScenes in the story: {}.", scenes.join(", ")));
    }
    report
}

/// A short line for the writer: what the last check found.
pub(super) fn summary(found: &[Problem]) -> String {
    match found.len() {
        0 => "The story compiles".into(),
        1 => "1 error so far".into(),
        count => format!("{count} errors so far"),
    }
}

/// Every scene in the story, in file order, so a mistyped target can be
/// matched against what exists.
pub(super) fn scene_names(workspace: &SourceWorkspace) -> Vec<String> {
    workspace
        .files
        .values()
        .flat_map(|text| parse(text).story.scenes)
        .filter_map(|node| match node.kind {
            NodeKind::Scene { name } => Some(name.value),
            _ => None,
        })
        .collect()
}
