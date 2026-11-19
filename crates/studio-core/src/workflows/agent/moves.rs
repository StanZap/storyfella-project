//! Moving whole scenes between a story's files, and creating the files they
//! move to. The model only says where each scene goes; the text moves here
//! unchanged, so a split cannot be cut off or mistyped.
//!
//! Includes expand in place, so a new file's `include` takes the place of the
//! first scene moved into it: the story plays in the same order, from the
//! same first scene.
use std::collections::BTreeMap;

use serde::Deserialize;
use storyfella_compiler::SourceWorkspace;
use storyfella_language::{parse, valid_source_path, NodeKind};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct Move {
    pub(super) scene: String,
    pub(super) to: String,
}

/// The folder part of a project-relative path; empty at the project's root.
fn folder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// The text of `scene` in `text`: the comments directly above its `scene`
/// line, and everything up to the next top-level line except the blank lines
/// and comments leading into it.
fn block(text: &str, scene: &str) -> Option<std::ops::Range<usize>> {
    let parsed = parse(text);
    let node = parsed
        .story
        .scenes
        .iter()
        .find(|node| matches!(&node.kind, NodeKind::Scene { name } if name.value == scene))?;
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        lines.push((offset, line));
        offset += line.len();
    }
    let at = lines
        .iter()
        .rposition(|(offset, _)| *offset <= node.span.start)?;
    let top_level = |line: &str| !line.starts_with([' ', '\t']) && !line.trim().is_empty();
    let comment = |line: &str| line.starts_with('#');
    let mut first = at;
    while first > 0 && comment(lines[first - 1].1) {
        first -= 1;
    }
    let next = lines[at + 1..]
        .iter()
        .position(|(_, line)| top_level(line) && !comment(line))
        .map_or(lines.len(), |index| at + 1 + index);
    let mut last = next;
    while last > at + 1 && (lines[last - 1].1.trim().is_empty() || comment(lines[last - 1].1)) {
        last -= 1;
    }
    let end = lines.get(last).map_or(text.len(), |(offset, _)| *offset);
    Some(lines[first].0..end)
}

/// Why `name` cannot be moved, with what can be: a model that named a beat
/// or a guess learns the story's scene names.
fn not_a_scene(texts: &BTreeMap<String, String>, name: &str) -> String {
    let mut scenes = Vec::new();
    let mut beat_in = None;
    let beat = name.rsplit('.').next().unwrap_or(name);
    for text in texts.values() {
        for node in parse(text).story.scenes {
            let NodeKind::Scene { name: scene } = &node.kind else {
                continue;
            };
            if node
                .children
                .iter()
                .any(|child| matches!(&child.kind, NodeKind::Beat { name } if name.value == beat))
            {
                beat_in.get_or_insert_with(|| scene.value.clone());
            }
            scenes.push(scene.value.clone());
        }
    }
    let what = match beat_in {
        Some(scene) => format!("{beat} is a beat in scene {scene}, not a scene"),
        None => format!("{name} is not a scene of this story"),
    };
    match scenes.as_slice() {
        [] => format!("{what}. The story has no scenes to move; answer in prose instead."),
        [only] => format!(
            "{what}. Only whole scenes move, and the story has one scene, {only}: there is \
             nothing to split, so answer in prose instead."
        ),
        scenes => format!(
            "{what}. Only whole scenes move; the scenes are {}.",
            scenes.join(", ")
        ),
    }
}

/// `text` with `range` replaced by `with`, leaving at most one blank line
/// where the two sides meet.
fn splice(text: &str, range: std::ops::Range<usize>, with: &str) -> String {
    let before = &text[..range.start];
    let mut after = &text[range.end..];
    if with.is_empty() && (before.is_empty() || before.ends_with("\n\n")) {
        after = after.trim_start_matches('\n');
    }
    format!("{before}{with}{after}")
}

/// Every file `moves` change or create, with the new files' includes in
/// place. A new file nothing moves into is created empty and included at the
/// end of the entry, for the model to write.
pub(super) fn apply(
    workspace: &SourceWorkspace,
    new_files: &[String],
    moves: &[Move],
) -> Result<BTreeMap<String, String>, String> {
    let mut texts = workspace.files.clone();
    for path in new_files {
        if !valid_source_path(path) {
            return Err(format!(
                "{path} is not a usable file name: use letters, digits, - or _ and end in .sf"
            ));
        }
        if texts.contains_key(path) {
            return Err(format!("{path} is already a file of this story"));
        }
        if !workspace
            .files
            .keys()
            .any(|file| folder(file) == folder(path))
        {
            return Err(format!(
                "{path} must be in the same folder as one of the story's files"
            ));
        }
        texts.insert(path.clone(), String::new());
    }
    // Where each new file is included: the file its first scene came from.
    let mut included: BTreeMap<&str, String> = BTreeMap::new();
    for Move { scene, to } in moves {
        if !texts.contains_key(to) {
            return Err(format!(
                "{to} is not a file of this story; list it in new_files to create it"
            ));
        }
        let (from, range) = texts
            .iter()
            .find_map(|(path, text)| block(text, scene).map(|range| (path.clone(), range)))
            .ok_or_else(|| not_a_scene(&texts, scene))?;
        if from == *to {
            return Err(format!("{scene} is already in {to}"));
        }
        let mut moved = texts[&from][range.clone()].to_owned();
        if !moved.ends_with('\n') {
            moved.push('\n');
        }
        let is_new = new_files.contains(to);
        let include = match included.get(to.as_str()) {
            Some(source) if *source != from => {
                return Err(format!(
                    "Move scenes into {to} from one file only; {scene} is in {from}"
                ));
            }
            Some(_) => String::new(),
            None if is_new => {
                let relative = match folder(&from) {
                    "" => to.as_str(),
                    dir => to
                        .strip_prefix(&format!("{dir}/"))
                        .ok_or_else(|| format!("Put {to} in the same folder as {from}"))?,
                };
                included.insert(to.as_str(), from.clone());
                format!("include \"{relative}\"\n")
            }
            None => String::new(),
        };
        let source = splice(&texts[&from], range, &include);
        texts.insert(from, source);
        let target = texts.get_mut(to).expect("checked above");
        if !target.is_empty() && !target.ends_with('\n') {
            target.push('\n');
        }
        if !target.trim().is_empty() {
            target.push('\n');
        }
        target.push_str(&moved);
    }
    for path in new_files {
        if !included.contains_key(path.as_str()) {
            let entry = texts
                .get_mut(&workspace.entry)
                .expect("the entry is a file");
            if !entry.is_empty() && !entry.ends_with('\n') {
                entry.push('\n');
            }
            entry.push_str(&format!("include \"{path}\"\n"));
        }
    }
    texts.retain(|path, text| workspace.files.get(path) != Some(text));
    Ok(texts)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORY: &str = "story \"Two\"\n\nscene bell\n  narrate: Ring.\n  goto well\n\n# The well.\nscene well\n  narrate: Deep.\n  goto lantern\n\nscene lantern\n  narrate: Light.\n  end\n";

    fn workspace() -> SourceWorkspace {
        SourceWorkspace {
            entry: "story.sf".into(),
            files: BTreeMap::from([("story.sf".into(), STORY.into())]),
        }
    }

    fn moving(pairs: &[(&str, &str)]) -> Vec<Move> {
        pairs
            .iter()
            .map(|(scene, to)| Move {
                scene: (*scene).into(),
                to: (*to).into(),
            })
            .collect()
    }

    #[test]
    fn a_split_moves_scenes_unchanged_and_includes_them_where_they_were() {
        let files = apply(
            &workspace(),
            &["bell.sf".into(), "lantern.sf".into()],
            &moving(&[
                ("bell", "bell.sf"),
                ("well", "bell.sf"),
                ("lantern", "lantern.sf"),
            ]),
        )
        .unwrap();
        assert_eq!(
            files["story.sf"],
            "story \"Two\"\n\ninclude \"bell.sf\"\n\ninclude \"lantern.sf\"\n"
        );
        assert_eq!(
            files["bell.sf"],
            "scene bell\n  narrate: Ring.\n  goto well\n\n# The well.\nscene well\n  narrate: Deep.\n  goto lantern\n",
            "a comment above a scene moves with it"
        );
        assert_eq!(
            files["lantern.sf"],
            "scene lantern\n  narrate: Light.\n  end\n"
        );
        // Expanded, the story says what it said, in the same order.
        let mut split = workspace();
        split.files.extend(files);
        let expanded = split.expand().unwrap().source;
        for scene in ["scene bell", "scene well", "scene lantern"] {
            assert!(expanded.contains(scene), "{expanded}");
        }
        assert!(expanded.find("scene bell") < expanded.find("scene lantern"));
    }

    #[test]
    fn a_new_file_nothing_moves_into_is_included_at_the_end() {
        let files = apply(&workspace(), &["epilogue.sf".into()], &[]).unwrap();
        assert_eq!(files["epilogue.sf"], "");
        assert!(files["story.sf"].ends_with("  end\ninclude \"epilogue.sf\"\n"));
    }

    #[test]
    fn a_beat_named_as_a_scene_is_answered_with_the_scenes_there_are() {
        let workspace = SourceWorkspace {
            entry: "story.sf".into(),
            files: BTreeMap::from([(
                "story.sf".into(),
                "scene harbor\n  beat arrival\n    narrate: Dawn.\n    end\n".into(),
            )]),
        };
        for name in ["arrival", "harbor.arrival"] {
            let reason = apply(
                &workspace,
                &["arrival.sf".into()],
                &[Move {
                    scene: name.into(),
                    to: "arrival.sf".into(),
                }],
            )
            .unwrap_err();
            assert_eq!(
                reason,
                "arrival is a beat in scene harbor, not a scene. Only whole scenes move, and \
                 the story has one scene, harbor: there is nothing to split, so answer in \
                 prose instead."
            );
        }
    }

    #[test]
    fn a_plan_that_cannot_be_carried_out_says_why() {
        let reason = |new: &[&str], moves: &[(&str, &str)]| {
            let new: Vec<String> = new.iter().map(|path| (*path).into()).collect();
            apply(&workspace(), &new, &moving(moves)).unwrap_err()
        };
        assert_eq!(
            reason(&[], &[("ghost", "story.sf")]),
            "ghost is not a scene of this story. Only whole scenes move; the scenes are \
             bell, well, lantern."
        );
        assert!(reason(&[], &[("bell", "other.sf")]).contains("new_files"));
        assert!(reason(&["story.sf"], &[]).contains("already a file"));
        assert!(reason(&["../out.sf"], &[]).contains("not a usable"));
        assert!(reason(&["deep/out.sf"], &[]).contains("same folder"));
        assert!(reason(&[], &[("bell", "story.sf")]).contains("already in"));
    }
}
