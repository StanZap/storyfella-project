use crate::Span;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Name {
    pub value: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub value: String,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Character,
    Location,
    Prop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectionKind {
    Style,
    Shot,
    Mood,
}

/// Children are ordered exactly as authored. Comments/trivia live in the token stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub span: Span,
    pub kind: NodeKind,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Scene {
        name: Name,
    },
    Beat {
        name: Name,
    },
    Choice {
        prompt: Text,
    },
    Option {
        label: Text,
        target: Name,
    },
    Asset {
        kind: AssetKind,
        names: Vec<Name>,
    },
    Direction {
        kind: DirectionKind,
        text: Text,
    },
    Speech {
        speaker: Name,
        thought: bool,
        text: Text,
    },
    Narration {
        text: Text,
    },
    Goto {
        target: Name,
    },
    End,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Story {
    pub title: Option<Text>,
    pub scenes: Vec<Node>,
}
