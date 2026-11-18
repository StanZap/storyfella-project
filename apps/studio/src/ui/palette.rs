//! What the command palette offers, and what each entry does.
use design_system::PaletteCommand;

use super::Studio;
use crate::host::Place;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Show(Place),
    NewStory,
    OpenStory,
    Save,
    Undo,
    Redo,
    Quit,
}

const ALL: [(&str, Command); 11] = [
    ("new-story", Command::NewStory),
    ("open-story", Command::OpenStory),
    ("save", Command::Save),
    ("undo", Command::Undo),
    ("redo", Command::Redo),
    ("show-home", Command::Show(Place::Home)),
    ("show-write", Command::Show(Place::Write)),
    ("show-play", Command::Show(Place::Play)),
    ("show-board", Command::Show(Place::Board)),
    ("settings", Command::Show(Place::Settings)),
    ("quit", Command::Quit),
];

impl Command {
    pub fn from_id(id: &str) -> Option<Self> {
        ALL.iter()
            .find(|(key, _)| *key == id)
            .map(|(_, command)| *command)
    }
}

/// The palette's entries, disabled where they cannot run right now.
pub fn commands(studio: &Studio) -> Vec<PaletteCommand> {
    let primary = if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl+"
    };
    let key = |keys: &str| format!("{primary}{keys}");
    let state = &studio.state;
    let open = state.project_path.is_some();
    let busy = state.operation_in_progress();
    vec![
        PaletteCommand::new("new-story", "New story…")
            .group("Story")
            .shortcut(key("N"))
            .keywords(["create", "project"]),
        PaletteCommand::new("open-story", "Open story…")
            .group("Story")
            .shortcut(key("O"))
            .keywords(["project", "file"]),
        PaletteCommand::new("save", "Save")
            .group("Story")
            .shortcut(key("S"))
            .disabled(busy || !state.has_unsaved_changes),
        PaletteCommand::new("undo", "Undo")
            .group("Edit")
            .shortcut(key("Z"))
            .disabled(!state.can_undo()),
        PaletteCommand::new("redo", "Redo")
            .group("Edit")
            .shortcut(key("⇧Z"))
            .disabled(!state.can_redo()),
        PaletteCommand::new("show-home", "Go to Home")
            .group("Navigate")
            .shortcut(key("⇧H"))
            .keywords(["projects", "recent"]),
        PaletteCommand::new("show-write", "Go to Write")
            .group("Navigate")
            .shortcut(key("1"))
            .keywords(["story", "editor"])
            .disabled(!open),
        PaletteCommand::new("show-play", "Go to Play")
            .group("Navigate")
            .shortcut(key("2"))
            .keywords(["preview", "playback"])
            .disabled(!open),
        PaletteCommand::new("show-board", "Go to Board")
            .group("Navigate")
            .shortcut(key("3"))
            .keywords(["assets", "artwork"])
            .disabled(!open),
        PaletteCommand::new("settings", "Settings…")
            .group("App")
            .shortcut(key(","))
            .keywords(["preferences", "models", "theme"]),
        PaletteCommand::new("quit", "Quit Storyfella Studio")
            .group("App")
            .shortcut(key("Q")),
    ]
}
