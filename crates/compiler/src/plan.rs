use serde::{Deserialize, Serialize};
use storyfella_language::{
    AssetKind, Condition, DirectionKind, Flag, Name, Span, StageGroup, Text,
};

use crate::workspace::{original_span, SourceSegment, SourceWorkspace};

pub const PLAN_VERSION: u32 = 1;

/// Supported visual effects. Unknown authored text remains an explicit
/// readiness requirement rather than being silently interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualDirection {
    Wide,
    Close,
    Neutral,
    Tense,
    Normal,
    Monochrome,
}

pub fn visual_direction(kind: DirectionKind, text: &str) -> Option<VisualDirection> {
    match (kind, text.trim().to_ascii_lowercase().as_str()) {
        (DirectionKind::Shot, "wide") => Some(VisualDirection::Wide),
        (DirectionKind::Shot, "close") => Some(VisualDirection::Close),
        (DirectionKind::Mood, "neutral") => Some(VisualDirection::Neutral),
        (DirectionKind::Mood, "tense") => Some(VisualDirection::Tense),
        (DirectionKind::Style, "normal") => Some(VisualDirection::Normal),
        (DirectionKind::Style, "monochrome") => Some(VisualDirection::Monochrome),
        _ => None,
    }
}

/// Authored scene names give stable identities across reordering and text edits.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SceneId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeatId {
    pub scene: SceneId,
    pub name: String,
}

/// The host supplies an immutable revision identity. The compiler never resolves files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetBinding {
    pub id: String,
    pub name: String,
    pub kind: AssetKind,
    pub revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetUse {
    pub name: Name,
    pub kind: AssetKind,
    pub binding: Option<AssetBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    Authored,
    SceneBoundary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instruction {
    pub source: Span,
    pub origin: Origin,
    pub beat: Option<BeatId>,
    pub kind: InstructionKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub source: Span,
    pub label: Text,
    pub target: SceneId,
    pub target_source: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchTarget {
    pub scene: SceneId,
    pub source: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstructionKind {
    BeginBeat {
        id: BeatId,
    },
    Assets {
        kind: AssetKind,
        assets: Vec<AssetUse>,
    },
    Stage {
        instance: Name,
        asset: AssetUse,
        group: StageGroup,
        scale: u16,
        depth: u16,
        position: Option<(u8, u8)>,
    },
    Swap {
        instance: Name,
        asset: AssetUse,
    },
    Hide {
        instance: Name,
    },
    Direction {
        kind: DirectionKind,
        text: Text,
    },
    Speech {
        speaker: AssetUse,
        thought: bool,
        text: Text,
    },
    Narration {
        text: Text,
    },
    Choice {
        prompt: Text,
        options: Vec<ChoiceOption>,
    },
    Jump {
        target: SceneId,
        target_source: Span,
    },
    SetFlag {
        flag: Name,
        value: bool,
    },
    Conditional {
        condition: Condition,
        when_true: BranchTarget,
        when_false: BranchTarget,
    },
    End,
}

impl InstructionKind {
    pub(crate) fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Choice { .. } | Self::Jump { .. } | Self::Conditional { .. } | Self::End
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenePlan {
    pub id: SceneId,
    pub source: Span,
    pub instructions: Vec<Instruction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAsset {
    pub kind: AssetKind,
    pub name: Name,
}

/// Retain unrecognized direction requests and block strict playback until
/// their semantics are implemented.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedDirection {
    pub kind: DirectionKind,
    pub text: Text,
    pub source: Span,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readiness {
    pub pending_assets: Vec<PendingAsset>,
    pub unsupported_directions: Vec<UnsupportedDirection>,
}
impl Readiness {
    /// Linking and supported-instruction readiness only; this does not launch a runtime.
    pub fn ready(&self) -> bool {
        self.pending_assets.is_empty() && self.unsupported_directions.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSourceMap {
    pub workspace: SourceWorkspace,
    pub segments: Vec<SourceSegment>,
}

impl PlanSourceMap {
    pub fn original_span(&self, span: Span) -> Option<(&str, Span)> {
        original_span(&self.segments, span)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryPlan {
    pub version: u32,
    pub source: String,
    /// Multi-file plans embed exact authored sources and their expansion map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_map: Option<PlanSourceMap>,
    pub title: Option<Text>,
    pub entry: SceneId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<Flag>,
    pub scenes: Vec<ScenePlan>,
    pub readiness: Readiness,
}
