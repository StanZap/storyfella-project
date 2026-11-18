//! What the stage shows at the current moment, decided once for every host.
//!
//! Hosts draw a [`Frame`]; none of them re-derive which artwork is the
//! backdrop, where an actor stands, or which placeholders appear. Positions
//! are percentages of the stage, so a host maps them onto its own size.
use super::{PlaybackSession, Visual};
use crate::config::PreviewSettings;
use storyfella_compiler::{AssetUse, InstructionKind, VisualDirection};
use storyfella_language::{AssetKind, StageGroup};
use storyfella_runtime::{Phase, Presentation};

/// Who is looking at the stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Viewer {
    /// Studio previews a draft; the Player reads a finished story.
    pub previewing_draft: bool,
    pub settings: PreviewSettings,
}

impl Viewer {
    /// Missing-artwork notes are an authoring aid, so only a draft preview
    /// with diagnostics on reports them.
    fn reports_missing(&self) -> bool {
        self.previewing_draft && self.settings.show_diagnostics
    }
}

/// Visual treatments the current shot, mood and style ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// The backdrop is drawn 12% larger, as for a close shot.
    pub close_shot: bool,
    pub tense: bool,
    /// Artwork is drawn in greyscale.
    pub monochrome: bool,
}

impl From<Presentation> for Effects {
    fn from(presentation: Presentation) -> Self {
        Self {
            close_shot: presentation.shot == Some(VisualDirection::Close),
            tense: presentation.mood == Some(VisualDirection::Tense),
            monochrome: presentation.style == Some(VisualDirection::Monochrome),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Backdrop<'a> {
    pub visual: &'a Visual,
    /// Fit inside the stage instead of covering it. Illustrations always fit.
    pub contained: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Actor<'a> {
    pub instance: &'a str,
    pub name: &'a str,
    /// Horizontal centre, as a percentage of the stage width.
    pub left: f32,
    /// Bottom edge, as a percentage of the actors' band height.
    pub bottom: f32,
    /// Height, as a percentage of the actors' band height.
    pub height: f32,
    pub depth: u16,
    /// `None` draws a labelled placeholder.
    pub visual: Option<&'a Visual>,
}

/// Approximate stand-in for an active character or prop with no artwork.
#[derive(Clone, Copy, Debug)]
pub struct Placeholder<'a> {
    pub kind: AssetKind,
    pub name: &'a str,
    /// Horizontal centre, as a percentage of the stage width.
    pub left: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SceneLabel<'a> {
    pub scene: &'a str,
    pub beat: Option<&'a str>,
    pub missing_artwork: bool,
    pub ignored_directions: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum Portrait<'a> {
    Image(&'a Visual),
    Missing,
    None,
}

#[derive(Clone, Copy, Debug)]
pub enum Passage<'a> {
    Speech {
        speaker: &'a str,
        thought: bool,
        text: &'a str,
        portrait: Portrait<'a>,
    },
    Narration(&'a str),
    Choice {
        prompt: &'a str,
        options: &'a [storyfella_compiler::ChoiceOption],
    },
    /// Automatic transitions reached their limit.
    Budget,
    Complete,
    /// An instruction with nothing to read, such as staging.
    Silent,
}

#[derive(Clone, Debug)]
pub struct Frame<'a> {
    pub effects: Effects,
    pub backdrop: Option<Backdrop<'a>>,
    /// Shown behind the backdrop when the scene's location or illustration
    /// has no artwork.
    pub missing_backdrop: Option<&'a AssetUse>,
    /// Neither artwork nor a known missing location to name.
    pub backdrop_unavailable: bool,
    pub actors: Vec<Actor<'a>>,
    pub placeholders: Vec<Placeholder<'a>>,
    pub label: SceneLabel<'a>,
    pub passage: Passage<'a>,
}

impl<'a> Frame<'a> {
    pub fn new(session: &'a PlaybackSession, viewer: Viewer) -> Self {
        let runtime = &session.runtime;
        let state = runtime.state();
        let settings = viewer.settings;
        let visuals = session.active_visuals();
        let missing = if viewer.reports_missing() {
            session.missing_uses()
        } else {
            Vec::new()
        };
        let staged = runtime.staged();
        let missing_backdrop = missing
            .iter()
            .find(|asset| asset.kind == AssetKind::Illustration)
            .or_else(|| {
                missing
                    .iter()
                    .find(|asset| asset.kind == AssetKind::Location)
            })
            .copied();
        let scene_backdrop = visuals
            .iter()
            .find(|v| v.binding.kind == AssetKind::Illustration)
            .or_else(|| {
                visuals
                    .iter()
                    .find(|v| v.binding.kind == AssetKind::Location)
            })
            .copied();
        // With no location, keep the available artwork intact rather than
        // treating a portrait or prop as a landscape.
        let backdrop = scene_backdrop.or_else(|| {
            if runtime.has_staging() {
                None
            } else {
                visuals.first().copied()
            }
        });
        let actors = staged
            .iter()
            .filter_map(|actor| {
                let visual = session.staged_visual(actor);
                if visual.is_none() && !settings.show_diagnostics {
                    return None;
                }
                Some(Actor {
                    instance: &actor.instance,
                    name: &actor.asset.name.value,
                    left: actor
                        .position
                        .map(|(x, _)| f32::from(x))
                        .unwrap_or_else(|| actor_position(actor.group, actor.slot)),
                    bottom: actor.position.map(|(_, y)| f32::from(y)).unwrap_or(0.),
                    height: f32::from(actor.scale) * 0.72,
                    depth: actor.depth,
                    visual,
                })
            })
            .collect();
        let placeholders = missing
            .iter()
            .filter(|asset| {
                !matches!(asset.kind, AssetKind::Location | AssetKind::Illustration)
                    && !staged.iter().any(|actor| {
                        actor.asset.kind == asset.kind
                            && actor
                                .asset
                                .name
                                .value
                                .eq_ignore_ascii_case(&asset.name.value)
                    })
            })
            .enumerate()
            .map(|(index, asset)| Placeholder {
                kind: asset.kind,
                name: &asset.name.value,
                left: placeholder_position(index),
            })
            .collect();
        let current = runtime.current();
        let passage = match state.phase {
            Phase::Complete => Passage::Complete,
            Phase::Budget => Passage::Budget,
            _ => match &current.kind {
                InstructionKind::Speech {
                    speaker,
                    thought,
                    text,
                } => {
                    let portrait = speaker
                        .binding
                        .as_ref()
                        .and_then(|binding| visuals.iter().find(|v| &v.binding == binding))
                        .copied()
                        .filter(|v| backdrop.is_none_or(|b| b.binding != v.binding));
                    Passage::Speech {
                        speaker: &speaker.name.value,
                        thought: *thought,
                        text: &text.value,
                        portrait: match portrait {
                            Some(visual) => Portrait::Image(visual),
                            None if settings.show_diagnostics => Portrait::Missing,
                            None => Portrait::None,
                        },
                    }
                }
                InstructionKind::Narration { text } => Passage::Narration(&text.value),
                InstructionKind::Choice { prompt, options } => Passage::Choice {
                    prompt: &prompt.value,
                    options,
                },
                _ => Passage::Silent,
            },
        };
        Self {
            effects: runtime.presentation().into(),
            backdrop: backdrop.map(|visual| Backdrop {
                visual,
                contained: visual.binding.kind != AssetKind::Location || settings.contain_artwork,
            }),
            missing_backdrop,
            backdrop_unavailable: backdrop.is_none()
                && missing_backdrop.is_none()
                && settings.show_diagnostics,
            actors,
            placeholders,
            label: SceneLabel {
                scene: &runtime.plan().scenes[state.scene].id.0,
                beat: state.beat.as_ref().map(|beat| beat.name.as_str()),
                missing_artwork: !missing.is_empty(),
                ignored_directions: viewer.reports_missing()
                    && !runtime.plan().readiness.unsupported_directions.is_empty(),
            },
            passage,
        }
    }
}

/// Instances of one group fan out from its anchor, alternating left and right.
pub fn actor_position(group: StageGroup, slot: usize) -> f32 {
    let anchor = match group {
        StageGroup::Left => 20.0,
        StageGroup::Center => 50.0,
        StageGroup::Right => 80.0,
    };
    let offset = if slot == 0 {
        0.0
    } else if slot % 2 == 1 {
        -(slot.div_ceil(2) as f32) * 8.0
    } else {
        (slot / 2) as f32 * 8.0
    };
    anchor + offset
}

/// Placeholders alternate from the left and right edges inwards.
fn placeholder_position(index: usize) -> f32 {
    if index % 2 == 0 {
        (22 + (index / 2) * 9) as f32
    } else {
        78_usize.saturating_sub((index / 2) * 9) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ArtifactRegistry;
    use storyfella_runtime::AssetPolicy;

    fn session(source: &str) -> PlaybackSession {
        PlaybackSession::compile_with_policy(source, &ArtifactRegistry::new(), AssetPolicy::Preview)
            .unwrap()
    }

    fn previewing() -> Viewer {
        Viewer {
            previewing_draft: true,
            settings: PreviewSettings::default(),
        }
    }

    #[test]
    fn staged_groups_space_instances_around_their_anchor() {
        assert_eq!(actor_position(StageGroup::Left, 0), 20.0);
        assert_eq!(actor_position(StageGroup::Center, 1), 42.0);
        assert_eq!(actor_position(StageGroup::Center, 2), 58.0);
        assert_eq!(actor_position(StageGroup::Right, 0), 80.0);
    }

    #[test]
    fn effects_follow_the_runtime_presentation_and_reset() {
        let active = Effects::from(Presentation {
            shot: Some(VisualDirection::Close),
            mood: Some(VisualDirection::Tense),
            style: Some(VisualDirection::Monochrome),
        });
        assert!(active.close_shot && active.tense && active.monochrome);
        let reset = Effects::from(Presentation {
            shot: Some(VisualDirection::Wide),
            mood: Some(VisualDirection::Neutral),
            style: Some(VisualDirection::Normal),
        });
        assert_eq!(reset, Effects::default());
    }

    #[test]
    fn staged_characters_without_artwork_stand_at_authored_positions() {
        let session = session(
            "scene harbor\n  stage: lead, mara, left, 125, 4\n  stage: friend, tam, right, 100, 2, 73, 12\n  narrate: Hello\n",
        );
        let frame = Frame::new(&session, previewing());
        let actors: Vec<_> = frame
            .actors
            .iter()
            .map(|a| {
                (
                    a.name,
                    a.left,
                    a.bottom,
                    a.height,
                    a.depth,
                    a.visual.is_none(),
                )
            })
            .collect();
        assert_eq!(
            actors,
            [
                ("mara", 20., 0., 90., 4, true),
                ("tam", 73., 12., 72., 2, true)
            ]
        );
        assert!(matches!(frame.passage, Passage::Narration("Hello")));
    }

    #[test]
    fn a_draft_preview_names_missing_active_artwork() {
        let session = session(
            "scene harbor\n  location: dock\n  cast: mara\n  prop: lantern\n  shot: wide\n  mara: Hello\n  end\n",
        );
        let frame = Frame::new(&session, previewing());
        assert_eq!(frame.missing_backdrop.unwrap().name.value, "dock");
        let names: Vec<_> = frame
            .placeholders
            .iter()
            .map(|p| (p.name, p.left))
            .collect();
        assert_eq!(names, [("mara", 22.), ("lantern", 78.)]);
        assert!(frame.label.missing_artwork);
        assert!(matches!(
            frame.passage,
            Passage::Speech {
                speaker: "mara",
                portrait: Portrait::Missing,
                ..
            }
        ));
    }

    #[test]
    fn the_player_does_not_report_missing_artwork() {
        let session =
            session("scene harbor\n  location: dock\n  cast: mara\n  mara: Hello\n  end\n");
        let frame = Frame::new(
            &session,
            Viewer {
                previewing_draft: false,
                ..previewing()
            },
        );
        assert!(frame.missing_backdrop.is_none());
        assert!(frame.placeholders.is_empty());
        assert!(!frame.label.missing_artwork);
        assert!(frame.backdrop_unavailable);
    }
}
