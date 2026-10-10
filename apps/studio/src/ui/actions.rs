//! UI intent shared by navigation, the palette, and contextual controls.
use dioxus::prelude::*;
use storyfella_studio_core::registry::ArtifactKind;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArtAction {
    Create(ArtifactKind),
    Variant(Uuid),
    Regenerate(Uuid),
    Modify(Uuid),
    AttachAdapter(Uuid),
}
impl ArtAction {
    pub fn target(self) -> Option<Uuid> {
        match self {
            Self::Create(_) => None,
            Self::Variant(id)
            | Self::Regenerate(id)
            | Self::Modify(id)
            | Self::AttachAdapter(id) => Some(id),
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Create(_) => "New artifact",
            Self::Variant(_) => "New variant",
            Self::Regenerate(_) => "Regenerate image",
            Self::Modify(_) => "Edit image",
            Self::AttachAdapter(_) => "Attach adapter",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StudioAction {
    Art(ArtAction),
    ImportArtwork,
    FocusCommand,
}

#[derive(Clone, Copy)]
pub struct ActionRequest(pub Signal<Option<StudioAction>>);
impl ActionRequest {
    pub fn send(mut self, action: StudioAction) {
        self.0.set(Some(action));
    }
}
