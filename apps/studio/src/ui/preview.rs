use super::{AppScreen, WriterCaret};
use crate::state::AppState;
use dioxus::prelude::*;
use storyfella_playback_ui::{PlaybackModel, PlaybackPanel};
use storyfella_runtime::AssetPolicy;
use storyfella_studio_core::playback::PlaybackSession;

fn has_includes(source: &str) -> bool {
    source.lines().any(|line| line.starts_with("include "))
}

pub(super) fn workspace(state: &AppState) -> Result<storyfella_compiler::SourceWorkspace, String> {
    let path = state
        .story
        .path()
        .ok_or("Save this story before previewing included files")?;
    let entry = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Story filename is not UTF-8")?;
    let mut drafts = state
        .additional_sources
        .as_ref()
        .map_or_else(Default::default, |sources| sources.overrides());
    drafts.insert(entry.to_owned(), state.story.source().to_owned());
    storyfella_studio_core::story::load_source_workspace(path, &drafts)
}

fn workspace_position(
    state: &AppState,
    span: storyfella_language::Span,
    expected: &storyfella_compiler::StoryPlan,
) -> Result<(String, storyfella_language::Position), String> {
    let sources = workspace(state)?;
    let catalog = storyfella_studio_core::authoring::AssetCatalog::from_registry(&state.registry);
    let (compilation, expanded) = storyfella_compiler::compile_workspace_with_catalog(
        &sources,
        &catalog.assets,
        &catalog.bindings,
    )
    .map_err(|error| error.message)?;
    if compilation.plan.as_ref() != Some(expected) {
        return Err("The story changed. Restart Preview before opening its source.".into());
    }
    let (path, span) = expanded
        .original_span(span)
        .ok_or("This playback position has no source location")?;
    let position = storyfella_language::Source::new(&sources.files[path])
        .position(span.start)
        .ok_or("The source location is invalid")?;
    Ok((path.to_owned(), position))
}

pub fn start_preview(mut model: Signal<PlaybackModel>, app_state: Signal<AppState>) {
    let state = app_state.read();
    let result = if has_includes(state.story.source()) {
        workspace(&state).and_then(|sources| {
            PlaybackSession::compile_workspace(&sources, &state.registry, AssetPolicy::Preview)
                .map(|(session, _)| session)
        })
    } else {
        PlaybackSession::compile_with_policy(
            state.story.source(),
            &state.registry,
            AssetPolicy::Preview,
        )
    };
    drop(state);
    model.write().install(result);
}

#[component]
pub fn Preview(app_state: Signal<AppState>) -> Element {
    let model = use_context::<Signal<PlaybackModel>>();
    let mut screen = use_context::<Signal<AppScreen>>();
    let mut app_state = app_state;
    let mut error_report =
        use_context::<Signal<Option<storyfella_studio_core::errors::ReportedError>>>();
    let caret = use_context::<WriterCaret>();
    let multiple = has_includes(app_state.read().story.source());
    let changed = model.read().session.as_ref().is_some_and(|s| {
        let state = app_state.read();
        let catalog =
            storyfella_studio_core::authoring::AssetCatalog::from_registry(&state.registry);
        let plan = if has_includes(state.story.source()) {
            workspace(&state)
                .ok()
                .and_then(|sources| {
                    storyfella_compiler::compile_workspace_with_catalog(
                        &sources,
                        &catalog.assets,
                        &catalog.bindings,
                    )
                    .ok()
                })
                .and_then(|(result, _)| result.plan)
        } else {
            let analysis = storyfella_language::analyze(state.story.source(), &catalog.assets);
            storyfella_compiler::compile_analysis(&analysis, &catalog.bindings).plan
        };
        plan.as_ref() != Some(s.runtime.plan())
    });
    rsx! {
        div { class: "flex h-full min-h-0 flex-col",
        if multiple {
            p { class: "border-b border-white/10 px-4 py-2 text-xs text-zinc-400", role: "status",
                "Preview uses open drafts and saved files."
            }
        }
        div { class: "min-h-0 flex-1",
        PlaybackPanel { model, draft_changed: changed,
            on_apply: move |_| start_preview(model, app_state),
            on_source: if changed { None } else { Some(EventHandler::new(move |span: storyfella_language::Span| {
                    let destination = if multiple {
                        let state = app_state.read();
                        let playback = model.read();
                        playback.session.as_ref().ok_or_else(|| "Preview is not running".to_owned()).and_then(|session| workspace_position(&state, span, session.runtime.plan())).map(Some)
                    } else {
                        let state = app_state.read();
                        Ok(storyfella_language::Source::new(state.story.source()).position(span.start).map(|position| (String::new(), position)))
                    };
                    match destination {
                        Ok(Some((path, position))) => {
                            let entry = app_state.read().story.path().and_then(|path| path.file_name()).and_then(|name| name.to_str()).unwrap_or_default().to_owned();
                            if path.is_empty() || path == entry { app_state.write().active_source = None; }
                            else if let Err(error) = app_state.write().open_additional_source(&path) {
                                use storyfella_studio_core::errors::CodedError;
                                error_report.set(Some(error.report().log())); return;
                            }
                            caret.navigate(position);
                            screen.set(AppScreen::Writer);
                        }
                        Ok(None) => {}
                        Err(message) => error_report.set(Some(storyfella_studio_core::errors::ReportedError::plain(storyfella_studio_core::errors::ErrorCode::AppCommandInvalid, message))),
                    }
                })) }
        }
        }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyfella_studio_core::story::StoryDocument;

    #[test]
    fn workspace_preview_uses_the_unsaved_entry_draft() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("studio-preview-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let entry = directory.join("story.sf");
        std::fs::write(&entry, "scene saved\n  end\n").unwrap();
        std::fs::write(
            directory.join("chapter.sf"),
            "scene chapter\n  narrate: Hello\n  end\n",
        )
        .unwrap();
        let mut state = AppState::default();
        state.story = StoryDocument::open(&entry).unwrap();
        state.story.set_source("include \"chapter.sf\"\n".into());
        state.open_additional_source("chapter.sf").unwrap();
        state
            .set_additional_source(
                "chapter.sf",
                "scene chapter\n  narrate: Unsaved chapter\n  end\n".into(),
            )
            .unwrap();
        let sources = workspace(&state).unwrap();
        let (session, _) =
            PlaybackSession::compile_workspace(&sources, &state.registry, AssetPolicy::Preview)
                .unwrap();
        assert_eq!(session.runtime.plan().entry.0, "chapter");
        assert!(session.runtime.plan().source.contains("Unsaved chapter"));
        let (path, position) = workspace_position(
            &state,
            session.runtime.current().source,
            session.runtime.plan(),
        )
        .unwrap();
        assert_eq!(path, "chapter.sf");
        assert_eq!(position.line, 1);
        assert!(state.has_unsaved_changes);
        assert!(state.undo());
        assert_eq!(
            state
                .additional_sources
                .as_ref()
                .unwrap()
                .get("chapter.sf")
                .unwrap()
                .source(),
            "scene chapter\n  narrate: Hello\n  end\n"
        );
        assert!(state.redo());
        assert_eq!(
            std::fs::read_to_string(directory.join("chapter.sf")).unwrap(),
            "scene chapter\n  narrate: Hello\n  end\n"
        );
        assert_eq!(
            std::fs::read_to_string(entry).unwrap(),
            "scene saved\n  end\n"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
