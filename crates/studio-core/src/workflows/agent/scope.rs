//! What the assistant may change: the whole project, one file, or one passage
//! of a file. The scope is enforced by the tools, not asked of the model, and
//! a passage follows the edits made inside it — and the writer's own edits
//! around it — for as long as the run lasts.
use std::ops::Range;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use storyfella_language::{parse, Node, NodeKind, Span};

/// What a request may change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Project,
    File {
        file: String,
    },
    /// Part of one file: a selection, a beat or a scene.
    Passage {
        file: String,
        span: Span,
        /// How the passage reads to the model and the writer: "beat arrival".
        label: String,
    },
}

/// Which passage around the caret a request is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Passage {
    Selection,
    Beat,
    Scene,
}

impl Scope {
    /// The passage of `source` that `kind` names: the selection itself, or the
    /// beat or scene around its start, in whole lines.
    pub fn passage(
        kind: Passage,
        file: &str,
        source: &str,
        selection: Range<usize>,
    ) -> Result<Self> {
        let scenes = parse(source).story.scenes;
        let (mut span, label) = match kind {
            Passage::Selection => {
                ensure!(
                    selection.start < selection.end,
                    "Select the passage to work on first."
                );
                (
                    Span::new(selection.start, selection.end),
                    "the selection".to_owned(),
                )
            }
            Passage::Beat => enclosing(&scenes, selection.start, true)
                .map(|(span, name)| (span, format!("beat {name}")))
                .context("Place the cursor inside a beat.")?,
            Passage::Scene => enclosing(&scenes, selection.start, false)
                .map(|(span, name)| (span, format!("scene {name}")))
                .context("Place the cursor inside a scene.")?,
        };
        if kind != Passage::Selection {
            span = whole_lines(source, span);
        }
        ensure!(
            source.get(span.start..span.end).is_some(),
            "The passage is not part of {file}."
        );
        Ok(Self::Passage {
            file: file.to_owned(),
            span,
            label,
        })
    }

    /// Whether replacing `range` of `file` stays inside the scope.
    pub(super) fn allows(&self, file: &str, range: Range<usize>) -> bool {
        match self {
            Self::Project => true,
            Self::File { file: scoped } => scoped == file,
            Self::Passage {
                file: scoped, span, ..
            } => scoped == file && span.start <= range.start && range.end <= span.end,
        }
    }

    /// Whether the scope reaches beyond one file: creating files and moving
    /// scenes need it.
    pub(super) fn is_project(&self) -> bool {
        matches!(self, Self::Project)
    }

    /// The scope as the model reads it, with the passage's lines in `source`.
    pub(super) fn describe(&self, source: Option<&str>) -> String {
        match self {
            Self::Project => "Project: you may change any file and create new ones.".into(),
            Self::File { file } => {
                format!("File: you may change {file} only, and create no other file.")
            }
            Self::Passage { file, span, label } => {
                let lines = source
                    .map(|source| lines_of(source, *span))
                    .unwrap_or_default();
                format!(
                    "Passage: you may change {label} in {file}{lines} only. An edit outside it \
                     is refused."
                )
            }
        }
    }

    /// Follows the passage through an edit that replaced `old` with `new`.
    pub(super) fn follow(&mut self, edited: &str, old: &str, new: &str) {
        if let Self::Passage { file, span, .. } = self {
            if file == edited && old != new {
                *span = remap(old, new, *span);
            }
        }
    }
}

/// The innermost beat (or scene) node containing `offset`, with its name.
fn enclosing(nodes: &[Node], offset: usize, beat: bool) -> Option<(Span, String)> {
    for node in nodes {
        if node.span.contains(offset) || node.span.end == offset {
            if let Some(found) = enclosing(&node.children, offset, beat) {
                return Some(found);
            }
            match (&node.kind, beat) {
                (NodeKind::Beat { name }, true) | (NodeKind::Scene { name }, false) => {
                    return Some((node.span, name.value.clone()))
                }
                _ => {}
            }
        }
    }
    None
}

/// `span` widened to the start of its first line and past its last line break.
fn whole_lines(source: &str, mut span: Span) -> Span {
    span.start = source[..span.start].rfind('\n').map_or(0, |at| at + 1);
    if source[span.end..].starts_with("\r\n") {
        span.end += 2;
    } else if source[span.end..].starts_with('\n') {
        span.end += 1;
    }
    span
}

/// " (lines 4–9)", where `span` lies in `source`.
pub(super) fn lines_of(source: &str, span: Span) -> String {
    let line = |offset: usize| source[..offset.min(source.len())].matches('\n').count() + 1;
    let first = line(span.start);
    // A span ending at a line break belongs to the line before it.
    let last = line(span.end.saturating_sub(1).max(span.start));
    if first == last {
        format!(" (line {first})")
    } else {
        format!(" (lines {first}–{last})")
    }
}

/// Where `span` of `old` lies in `new`. Text changed before the span shifts
/// it, text changed after it leaves it alone, and a change overlapping it
/// becomes part of it.
fn remap(old: &str, new: &str, span: Span) -> Span {
    let prefix = old
        .char_indices()
        .zip(new.chars())
        .find(|((_, a), b)| a != b)
        .map_or(old.len().min(new.len()), |((at, _), _)| at);
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    let shift = |offset: usize| offset + new_end - old_end;
    if old_end <= span.start {
        Span::new(shift(span.start), shift(span.end))
    } else if span.end <= prefix {
        span
    } else {
        Span::new(span.start.min(prefix), shift(span.end.max(old_end)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORY: &str = "scene harbor\n  beat arrival\n    narrate: Fog.\n  beat departure\n    narrate: Bells.\n    end\n";

    fn passage(scope: &Scope) -> &str {
        let Scope::Passage { span, .. } = scope else {
            panic!("a passage")
        };
        &STORY[span.start..span.end]
    }

    #[test]
    fn a_beat_or_scene_is_found_around_the_caret_in_whole_lines() {
        let caret = STORY.find("Fog").unwrap();
        let beat = Scope::passage(Passage::Beat, "story.sf", STORY, caret..caret).unwrap();
        assert_eq!(passage(&beat), "  beat arrival\n    narrate: Fog.\n");
        assert!(matches!(&beat, Scope::Passage { label, .. } if label == "beat arrival"));
        let scene = Scope::passage(Passage::Scene, "story.sf", STORY, caret..caret).unwrap();
        assert_eq!(passage(&scene), STORY);
        assert!(Scope::passage(Passage::Selection, "story.sf", STORY, caret..caret).is_err());
    }

    #[test]
    fn a_passage_allows_only_edits_inside_it_and_in_its_file() {
        let caret = STORY.find("Fog").unwrap();
        let beat = Scope::passage(Passage::Beat, "story.sf", STORY, caret..caret).unwrap();
        assert!(beat.allows("story.sf", caret..caret + 3));
        assert!(!beat.allows("story.sf", STORY.find("Bells").unwrap()..STORY.len()));
        assert!(!beat.allows("other.sf", caret..caret + 3));
        let file = Scope::File {
            file: "story.sf".into(),
        };
        assert!(file.allows("story.sf", 0..STORY.len()));
        assert!(!file.allows("other.sf", 0..1));
    }

    #[test]
    fn a_passage_follows_edits_inside_before_and_across_it() {
        let span = Span::new(10, 20);
        let old = "0123456789abcdefghij0123456789";
        // Inside: the passage grows with the edit.
        let grown = old.replacen("cde", "c-d-e", 1);
        assert_eq!(remap(old, &grown, span), Span::new(10, 22));
        // Before: it shifts.
        let before = format!("xx{old}");
        assert_eq!(remap(old, &before, span), Span::new(12, 22));
        // After: it stays.
        let after = format!("{old}yy");
        assert_eq!(remap(old, &after, span), span);
        // Across its start: the change joins it.
        let across = old.replacen("789abc", "Z", 1);
        assert_eq!(remap(old, &across, span), Span::new(7, 15));
    }

    #[test]
    fn remapping_respects_character_boundaries() {
        let old = "é scene ü";
        let new = "è scene ü";
        let span = Span::new(old.find('ü').unwrap(), old.len());
        assert_eq!(remap(old, new, span), span);
    }
}
