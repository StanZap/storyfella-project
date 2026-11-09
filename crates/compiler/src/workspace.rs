//! Ordered project sources and the mapping back to their authored locations.
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::{Deserialize, Serialize};
use storyfella_language::{
    analyze, normalize_diagnostics, parse, parse_include, valid_source_path, Asset, Diagnostic,
    Position, Severity, Source, Span,
};

use crate::{compile_analysis, AssetBinding, Compilation};

/// A snapshot supplied by a host. Keys are project-relative paths.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceWorkspace {
    pub entry: String,
    pub files: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSegment {
    pub path: String,
    pub expanded: Span,
    pub original: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedWorkspace {
    pub source: String,
    pub segments: Vec<SourceSegment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceError {
    pub path: String,
    pub span: Span,
    pub message: String,
    pub start: Position,
    pub end: Position,
}

impl WorkspaceError {
    pub fn entry(path: String, message: String) -> Self {
        Self::at(&path, Span::new(0, 0), "", message)
    }

    fn at(path: &str, span: Span, text: &str, message: String) -> Self {
        let source = Source::new(text);
        Self {
            path: path.to_owned(),
            span,
            message,
            start: source.position(span.start).unwrap_or_default(),
            end: source.position(span.end).unwrap_or_default(),
        }
    }

    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            code: "SF308".into(),
            severity: Severity::Error,
            message: self.message.clone(),
            span: self.span,
        }
    }
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.path,
            self.start.line + 1,
            self.start.character + 1,
            self.message
        )
    }
}

impl std::error::Error for WorkspaceError {}

/// Compile one immutable project snapshot with the same compiler as a single
/// source. The mapping lets hosts report diagnostics in the authored files.
pub fn compile_workspace(
    workspace: &SourceWorkspace,
    bindings: &[AssetBinding],
) -> Result<(Compilation, ExpandedWorkspace), WorkspaceError> {
    let assets: Vec<_> = bindings
        .iter()
        .map(|binding| Asset {
            id: binding.id.clone(),
            name: binding.name.clone(),
            kind: binding.kind,
        })
        .collect();
    compile_workspace_with_catalog(workspace, &assets, bindings)
}

/// Compile with the full catalog, including assets that have no artwork yet.
pub fn compile_workspace_with_catalog(
    workspace: &SourceWorkspace,
    assets: &[Asset],
    bindings: &[AssetBinding],
) -> Result<(Compilation, ExpandedWorkspace), WorkspaceError> {
    let expanded = workspace.expand()?;
    let analysis = analyze(&expanded.source, assets);
    let mut compilation = compile_analysis(&analysis, bindings);
    // A chapter cannot continue the indentation scope of its including file.
    // Check each file's own structure before accepting the combined plan.
    for (path, text) in &workspace.files {
        for mut diagnostic in parse(text).diagnostics {
            if let Some(start) = expanded.expanded_offset(path, diagnostic.span.start) {
                diagnostic.span =
                    Span::new(start, start + diagnostic.span.end - diagnostic.span.start);
                compilation.diagnostics.push(diagnostic);
            }
        }
    }
    normalize_diagnostics(&mut compilation.diagnostics);
    if compilation.has_errors() {
        compilation.plan = None;
    }
    if let Some(plan) = &mut compilation.plan {
        plan.source_map = Some(crate::PlanSourceMap {
            workspace: workspace.clone(),
            segments: expanded.segments.clone(),
        });
    }
    Ok((compilation, expanded))
}

impl ExpandedWorkspace {
    /// Translate a byte offset from an authored file into the expanded snapshot.
    pub fn expanded_offset(&self, path: &str, offset: usize) -> Option<usize> {
        self.segments
            .iter()
            .find(|segment| {
                segment.path == path
                    && offset >= segment.original.start
                    && offset < segment.original.end
            })
            .map(|segment| segment.expanded.start + offset - segment.original.start)
    }
    /// Translate an expanded span only when it lies within one original segment.
    pub fn original_span(&self, span: Span) -> Option<(&str, Span)> {
        original_span(&self.segments, span)
    }

    /// Diagnostics can cover a whole scene across a file boundary. Anchor
    /// those to the authored portion of their first segment.
    pub fn diagnostic_span<'a>(&'a self, entry: &'a str, span: Span) -> (&'a str, Span) {
        self.original_span(span)
            .or_else(|| {
                let first = self
                    .segments
                    .iter()
                    .find(|segment| segment.expanded.contains(span.start))?;
                self.original_span(Span::new(span.start, span.end.min(first.expanded.end)))
            })
            .unwrap_or((entry, Span::default()))
    }
}

pub(crate) fn original_span(segments: &[SourceSegment], span: Span) -> Option<(&str, Span)> {
    let segment = segments
        .iter()
        .find(|segment| {
            span.start >= segment.expanded.start
                && span.end <= segment.expanded.end
                && (span.start < segment.expanded.end
                    || segment.expanded.start == segment.expanded.end)
        })
        .or_else(|| {
            (span.start == span.end)
                .then(|| {
                    segments
                        .iter()
                        .rev()
                        .find(|segment| segment.expanded.end == span.start)
                })
                .flatten()
        })?;
    let offset = segment.original.start;
    Some((
        &segment.path,
        Span::new(
            offset + span.start - segment.expanded.start,
            offset + span.end - segment.expanded.start,
        ),
    ))
}

impl SourceWorkspace {
    /// Load only sources reachable from the entry point using a host-owned
    /// loader. No filesystem policy is imposed on editors or other hosts.
    pub fn load_with<F>(entry: String, mut load: F) -> Result<Self, WorkspaceError>
    where
        F: FnMut(&str) -> Result<String, String>,
    {
        let mut files = BTreeMap::new();
        Expander::new(
            |path: &str| {
                let text = load(path)?;
                files.insert(path.to_owned(), text.clone());
                Ok(text)
            },
            None,
        )
        .visit(&entry)?;
        Ok(Self { entry, files })
    }

    /// Expand top-level `include "relative.sf"` in place. Paths are relative to
    /// the including file, and a file may appear only once in a project graph.
    pub fn expand(&self) -> Result<ExpandedWorkspace, WorkspaceError> {
        let output = ExpandedWorkspace {
            source: String::new(),
            segments: Vec::new(),
        };
        let mut expander = Expander::new(
            |path: &str| {
                self.files
                    .get(path)
                    .ok_or_else(|| format!("Source not found: {path}"))
            },
            Some(output),
        );
        expander.visit(&self.entry)?;
        if self.files.len() != expander.seen.len() {
            return Err(WorkspaceError::entry(
                self.entry.clone(),
                "Snapshot contains sources that are outside the entry's include graph".into(),
            ));
        }
        Ok(expander.output.expect("expansion keeps its output"))
    }
}

struct Expander<F> {
    load: F,
    seen: BTreeSet<String>,
    active: BTreeSet<String>,
    output: Option<ExpandedWorkspace>,
}

impl<F, T> Expander<F>
where
    F: FnMut(&str) -> Result<T, String>,
    T: AsRef<str>,
{
    fn new(load: F, output: Option<ExpandedWorkspace>) -> Self {
        Self {
            load,
            output,
            seen: BTreeSet::new(),
            active: BTreeSet::new(),
        }
    }

    fn visit(&mut self, path: &str) -> Result<(), WorkspaceError> {
        let fail = |message| WorkspaceError::entry(path.to_owned(), message);
        if !valid_source_path(path) {
            return Err(fail("Invalid project source path".into()));
        }
        if self.active.contains(path) {
            return Err(fail(format!("Include cycle involving {path}")));
        }
        if self.seen.contains(path) {
            return Err(fail(format!("Source included more than once: {path}")));
        }
        if self.seen.len() >= 256 {
            return Err(fail("Story contains more than 256 source files".into()));
        }
        let text = (self.load)(path).map_err(fail)?;
        let text = text.as_ref();
        let fail = |span, message| WorkspaceError::at(path, span, text, message);
        self.seen.insert(path.to_owned());
        self.active.insert(path.to_owned());
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let content = line.trim_end_matches(['\r', '\n']);
            if let Some(include) = parse_include(content, offset) {
                let target = include
                    .map_err(|error| fail(error.span, error.message))?
                    .value;
                let child = include_path(path, &target);
                self.visit(&child).map_err(|error| {
                    if error.path == child && error.span == Span::new(0, 0) {
                        fail(Span::new(offset, offset + content.len()), error.message)
                    } else {
                        error
                    }
                })?;
                if let Some(output) = &mut self.output {
                    if !output.source.is_empty() && !output.source.ends_with('\n') {
                        output.source.push('\n');
                    }
                }
            } else if let Some(output) = &mut self.output {
                let start = output.source.len();
                output.source.push_str(line);
                if let Some(previous) = output.segments.last_mut().filter(|segment| {
                    segment.path == path
                        && segment.expanded.end == start
                        && segment.original.end == offset
                }) {
                    previous.expanded.end = output.source.len();
                    previous.original.end = offset + line.len();
                } else {
                    output.segments.push(SourceSegment {
                        path: path.to_owned(),
                        expanded: Span::new(start, output.source.len()),
                        original: Span::new(offset, offset + line.len()),
                    });
                }
            }
            offset += line.len();
        }
        self.active.remove(path);
        Ok(())
    }
}

/// Project-relative identity of an include whose argument has been parsed.
pub fn include_path(including: &str, target: &str) -> String {
    including.rsplit_once('/').map_or_else(
        || target.to_owned(),
        |(parent, _)| format!("{parent}/{target}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_preserves_order_and_maps_unicode_locations() {
        let workspace = SourceWorkspace {
            entry: "story.sf".into(),
            files: BTreeMap::from([
                (
                    "story.sf".into(),
                    "story \"Test\"\ninclude \"chapter.sf\"\nscene end\n  end\n".into(),
                ),
                (
                    "chapter.sf".into(),
                    "scene start\n  narrate: 🌊\n  goto end\n".into(),
                ),
            ]),
        };
        let expanded = workspace.expand().unwrap();
        assert!(
            expanded.source.find("scene start").unwrap()
                < expanded.source.find("scene end").unwrap()
        );
        let start = expanded.source.find('🌊').unwrap();
        let (path, span) = expanded.original_span(Span::new(start, start + 4)).unwrap();
        assert_eq!(path, "chapter.sf");
        assert_eq!(&workspace.files[path][span.start..span.end], "🌊");
    }

    #[test]
    fn rejects_missing_cycles_duplicates_and_escaping_paths() {
        for (entry, files) in [
            ("story.sf", vec![("story.sf", "include \"missing.sf\"\n")]),
            ("story.sf", vec![("story.sf", "include \"story.sf\"\n")]),
            (
                "story.sf",
                vec![
                    ("story.sf", "include \"a.sf\"\ninclude \"a.sf\"\n"),
                    ("a.sf", ""),
                ],
            ),
            ("story.sf", vec![("story.sf", "include \"../secret.sf\"\n")]),
        ] {
            let workspace = SourceWorkspace {
                entry: entry.into(),
                files: files
                    .into_iter()
                    .map(|(a, b)| (a.into(), b.into()))
                    .collect(),
            };
            assert!(workspace.expand().is_err());
        }
    }

    #[test]
    fn compilation_resolves_branches_across_files_and_maps_diagnostics() {
        let mut workspace = SourceWorkspace {
            entry: "story.sf".into(),
            files: BTreeMap::from([
                (
                    "story.sf".into(),
                    "include \"first.sf\"\ninclude \"second.sf\"\n".into(),
                ),
                ("first.sf".into(), "scene start\n  goto ending\n".into()),
                ("second.sf".into(), "scene ending\n  end\n".into()),
            ]),
        };
        assert!(compile_workspace(&workspace, &[]).unwrap().0.plan.is_some());
        workspace
            .files
            .insert("first.sf".into(), "scene start\n  goto missing\n".into());
        let (compilation, expanded) = compile_workspace(&workspace, &[]).unwrap();
        let diagnostic = compilation
            .diagnostics
            .iter()
            .find(|d| d.code == "SF202")
            .unwrap();
        let (path, span) = expanded.original_span(diagnostic.span).unwrap();
        assert_eq!(path, "first.sf");
        assert_eq!(&workspace.files[path][span.start..span.end], "missing");
    }

    #[test]
    fn included_files_cannot_inherit_the_callers_indentation() {
        let workspace = SourceWorkspace {
            entry: "story.sf".into(),
            files: BTreeMap::from([
                (
                    "story.sf".into(),
                    "scene start\n  narrate: Start\ninclude \"chapter.sf\"\n".into(),
                ),
                ("chapter.sf".into(), "  end\n".into()),
            ]),
        };
        let (result, expanded) = compile_workspace(&workspace, &[]).unwrap();
        assert!(result.plan.is_none());
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|d| d.code == "SF102")
            .unwrap();
        let (path, _) = expanded.original_span(diagnostic.span).unwrap();
        assert_eq!(path, "chapter.sf");
    }
}
