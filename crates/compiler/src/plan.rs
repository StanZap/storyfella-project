use serde::{Deserialize, Serialize};
use storyfella_language::{AssetKind, DirectionKind, Name, Span, Text};

pub const PLAN_VERSION: u32 = 1;

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
pub enum InstructionKind {
    BeginBeat {
        id: BeatId,
    },
    Assets {
        kind: AssetKind,
        assets: Vec<AssetUse>,
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
    End,
}

impl InstructionKind {
    pub(crate) fn terminal(&self) -> bool {
        matches!(self, Self::Choice { .. } | Self::Jump { .. } | Self::End)
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

/// Shot/style/mood execution is deferred in language-plan milestone 5.
/// Retain the exact requests and block readiness until their semantics are implemented.
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
pub struct StoryPlan {
    pub version: u32,
    pub source: String,
    pub title: Option<Text>,
    pub entry: SceneId,
    pub scenes: Vec<ScenePlan>,
    pub readiness: Readiness,
}
