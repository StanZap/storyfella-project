//! Ordered project sources and the mapping back to their authored locations.
use std::collections::{BTreeMap, BTreeSet};

use storyfella_language::{analyze, Asset, Span};

use crate::{compile_analysis, AssetBinding, Compilation};

/// A snapshot supplied by a host. Keys are project-relative paths.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceWorkspace {
    pub entry: String,
    pub files: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    missing_path: Option<String>,
}

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
    let compilation = compile_analysis(&analysis, bindings);
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
        let segment = self.segments.iter().find(|segment| {
            span.start >= segment.expanded.start
                && span.end <= segment.expanded.end
                && (span.start < segment.expanded.end
                    || segment.expanded.start == segment.expanded.end)
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
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.ends_with(".sf")
        && !path.contains('\\')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

impl SourceWorkspace {
    /// Load only sources reachable from the entry point using a host-owned
    /// loader. No filesystem policy is imposed on editors or other hosts.
    pub fn load_with<F>(entry: String, mut load: F) -> Result<Self, WorkspaceError>
    where
        F: FnMut(&str) -> Result<String, String>,
    {
        let mut workspace = Self {
            entry: entry.clone(),
            files: BTreeMap::new(),
        };
        loop {
            match workspace.expand() {
                Ok(_) => return Ok(workspace),
                Err(mut error) => {
                    let Some(missing) = error.missing_path.take() else {
                        return Err(error);
                    };
                    if workspace.files.len() >= 256 {
                        error.message = "Story contains more than 256 source files".into();
                        return Err(error);
                    }
                    let text = load(&missing).map_err(|message| {
                        error.message = message;
                        error
                    })?;
                    workspace.files.insert(missing, text);
                }
            }
        }
    }

    /// Expand top-level `include "relative.sf"` in place. Paths are relative to
    /// the including file, and a file may appear only once in a project graph.
    pub fn expand(&self) -> Result<ExpandedWorkspace, WorkspaceError> {
        let mut output = ExpandedWorkspace {
            source: String::new(),
            segments: Vec::new(),
        };
        let mut seen = BTreeSet::new();
        let mut active = BTreeSet::new();
        self.expand_file(&self.entry, &mut seen, &mut active, &mut output)?;
        Ok(output)
    }

    fn expand_file(
        &self,
        path: &str,
        seen: &mut BTreeSet<String>,
        active: &mut BTreeSet<String>,
        output: &mut ExpandedWorkspace,
    ) -> Result<(), WorkspaceError> {
        let fail = |span, message: String| WorkspaceError {
            path: path.to_owned(),
            span,
            message,
            missing_path: None,
        };
        if !valid_path(path) {
            return Err(fail(Span::new(0, 0), "Invalid project source path".into()));
        }
        if active.contains(path) {
            return Err(fail(
                Span::new(0, 0),
                format!("Include cycle involving {path}"),
            ));
        }
        if !seen.insert(path.to_owned()) {
            return Err(fail(
                Span::new(0, 0),
                format!("Source included more than once: {path}"),
            ));
        }
        if active.len() >= 256 {
            return Err(fail(
                Span::new(0, 0),
                "Include nesting exceeds 256 files".into(),
            ));
        }
        let text = self.files.get(path).ok_or_else(|| {
            let mut error = fail(Span::new(0, 0), format!("Source not found: {path}"));
            error.missing_path = Some(path.to_owned());
            error
        })?;
        active.insert(path.to_owned());
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let content = line.trim_end_matches(['\r', '\n']);
            if let Some(argument) = content.strip_prefix("include ") {
                let argument = argument.trim();
                let target = argument
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .filter(|s| valid_path(s))
                    .ok_or_else(|| {
                        fail(
                            Span::new(offset, offset + content.len()),
                            "Expected include followed by a quoted relative .sf path".into(),
                        )
                    })?;
                let child = match path.rsplit_once('/') {
                    Some((parent, _)) => format!("{parent}/{target}"),
                    None => target.to_owned(),
                };
                self.expand_file(&child, seen, active, output)
                    .map_err(|error| {
                        if error.path == child && error.span == Span::new(0, 0) {
                            let mut mapped =
                                fail(Span::new(offset, offset + content.len()), error.message);
                            mapped.missing_path = error.missing_path;
                            mapped
                        } else {
                            error
                        }
                    })?;
                if !output.source.is_empty() && !output.source.ends_with('\n') {
                    output.source.push('\n');
                }
            } else {
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
        active.remove(path);
        Ok(())
    }
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
}
