//! Authoring intents shared by asset navigation and the palette.
use dioxus::prelude::*;
use storyfella_studio_core::registry::ArtifactKind;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArtAction {
    Create(ArtifactKind),
    Variant(Uuid),
}

#[derive(Clone, Copy)]
pub struct ActionRequest(pub Signal<Option<ArtAction>>);
impl ActionRequest {
    pub fn send(mut self, action: ArtAction) {
        self.0.set(Some(action));
    }
}
