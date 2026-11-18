//! Per-client playback snapshots, independent of the interactive Preview.
use std::collections::VecDeque;

use storyfella_compiler::InstructionKind;
use storyfella_runtime::{AssetPolicy, Command as PlaybackCommand, Phase, SavedState};
use storyfella_studio_core::{
    desktop_bridge::{PreviewAction, PreviewCommand},
    playback::PlaybackSession,
};

use super::*;

const MAX_PINNED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Previews(VecDeque<Run>);

struct Run {
    id: Uuid,
    client: Uuid,
    owner: Uuid,
    identity: String,
    session: PlaybackSession,
    bookmark: Option<SavedState>,
}

impl Previews {
    pub(super) async fn dispatch(
        &mut self,
        state: &impl StateOwner,
        grant: &Grant,
        request: Request<PreviewCommand>,
        client: Uuid,
        epoch: Option<Uuid>,
    ) -> Reply {
        request.check_project(&grant.folder)?;
        let session = match &request.command {
            PreviewCommand::Start { session }
            | PreviewCommand::Step { session, .. }
            | PreviewCommand::Capture { session, .. } => *session,
        };
        if epoch.is_none() || session != client {
            return Err(Failure::new(
                "session_changed",
                "Inspect the Writer draft on this connection first.",
            ));
        }
        let folder = grant.folder.clone();
        let current = owned(state, move |app| {
            let current = owner(app, &folder, epoch)?;
            app.authoring_lease
                .as_ref()
                .expect("owner checked the lease")
                .verify()
                .map_err(|_| moved())?;
            Ok(current)
        })
        .await?;
        self.0.retain(|run| run.owner == current);
        match request.command {
            PreviewCommand::Start { .. } => {
                let draft = owned(state, |app| Ok(Draft::from(&*app))).await?;
                let session = tokio::task::spawn_blocking(move || compile(&draft))
                    .await
                    .map_err(|_| preview_error("Studio could not start Preview."))??;
                let folder = grant.folder.clone();
                owned(state, move |app| owner(app, &folder, Some(current))).await?;
                self.0.retain(|run| run.client != client);
                while self.0.len() == MAX_CONNECTIONS
                    || pinned_bytes(&session)
                        + self
                            .0
                            .iter()
                            .map(|run| pinned_bytes(&run.session))
                            .sum::<usize>()
                        > MAX_PINNED_BYTES
                {
                    self.0.pop_front();
                }
                let run = Run {
                    id: Uuid::new_v4(),
                    client,
                    owner: current,
                    identity: session.runtime.save().plan_identity,
                    session,
                    bookmark: None,
                };
                let reply = run.describe();
                self.0.push_back(run);
                Ok(reply)
            }
            PreviewCommand::Step {
                preview, action, ..
            } => {
                let run = self.run(client, preview)?;
                run.step(action)?;
                Ok(run.describe())
            }
            PreviewCommand::Capture {
                preview,
                width,
                height,
                ..
            } => {
                if !(360..=960).contains(&width) || !(360..=960).contains(&height) {
                    return Err(Failure::new(
                        "invalid_size",
                        "Use width and height from 360 to 960 points.",
                    ));
                }
                let session = self.run(client, preview)?.session.clone();
                let png = capture(session, width, height).await?;
                let folder = grant.folder.clone();
                owned(state, move |app| owner(app, &folder, Some(current))).await?;
                let mut reply = self.run(client, preview)?.describe();
                reply["viewport"] = json!([width, height]);
                reply["png"] = json!(png);
                Ok(reply)
            }
        }
    }

    fn run(&mut self, client: Uuid, preview: Uuid) -> Result<&mut Run, Failure> {
        self.0
            .iter_mut()
            .find(|run| run.client == client && run.id == preview)
            .ok_or_else(|| {
                Failure::new(
                    "preview_changed",
                    "Start an isolated Preview on this connection.",
                )
            })
    }
}

/// The stage as base64 PNG, as an isolated preview's reader would see it.
#[cfg(target_os = "macos")]
async fn capture(session: PlaybackSession, width: u32, height: u32) -> Result<String, Failure> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use storyfella_studio_core::{config::PreviewSettings, playback::stage::Viewer};
    let viewer = Viewer {
        previewing_draft: true,
        settings: PreviewSettings::default(),
    };
    tokio::task::spawn_blocking(move || crate::capture::stage_png(session, viewer, width, height))
        .await
        .map_err(|_| preview_error("Studio could not draw the stage."))?
        .map(|png| STANDARD.encode(png))
        .map_err(|error| preview_error(&format!("Studio could not draw the stage: {error:#}")))
}

#[cfg(not(target_os = "macos"))]
async fn capture(_: PlaybackSession, _: u32, _: u32) -> Result<String, Failure> {
    Err(Failure::new(
        "unsupported",
        "Preview capture is available in Studio on macOS.",
    ))
}

fn compile(draft: &Draft) -> Result<PlaybackSession, Failure> {
    let entry = draft.entry.path().ok_or_else(moved)?;
    let name = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(moved)?;
    let sources = load_bounded_source_workspace(entry, &draft.overrides(name), MAX_STORY_BYTES)
        .map_err(|_| workspace_unavailable())?;
    let session =
        PlaybackSession::compile_workspace(&sources, &draft.registry, AssetPolicy::Preview)
            .map_err(|error| preview_error(&error.chars().take(4096).collect::<String>()))?;
    if pinned_bytes(&session) > MAX_PINNED_BYTES {
        return Err(preview_error(
            "This draft has too much artwork to pin in an isolated Preview.",
        ));
    }
    Ok(session)
}

fn pinned_bytes(session: &PlaybackSession) -> usize {
    session.visuals.iter().map(|visual| visual.png.len()).sum()
}

impl Run {
    fn step(&mut self, action: PreviewAction) -> Result<(), Failure> {
        match action {
            PreviewAction::Inspect => Ok(()),
            PreviewAction::Advance => self.session.dispatch(PlaybackCommand::Advance),
            PreviewAction::Choose { index } => self
                .session
                .dispatch(PlaybackCommand::SelectChoice { index }),
            PreviewAction::Checkpoint => {
                self.bookmark = Some(self.session.runtime.save());
                Ok(())
            }
            PreviewAction::Restore => match &self.bookmark {
                Some(bookmark) => self.session.restore(bookmark.clone()),
                None => Err("Create a checkpoint before restoring.".into()),
            },
        }
        .map_err(|error| preview_error(&error))
    }

    fn describe(&self) -> serde_json::Value {
        let runtime = &self.session.runtime;
        let state = runtime.state();
        let instruction = runtime.current();
        let position = runtime.plan().source_map.as_ref().and_then(|map| {
            let (file, span) = map.original_span(instruction.source)?;
            Some(json!({"file":file,"start":span.start,"end":span.end}))
        });
        let passage = match &instruction.kind {
            InstructionKind::Speech { text, .. } | InstructionKind::Narration { text }
                if state.phase == Phase::Text =>
            {
                json!({"text":text.value.chars().take(4096).collect::<String>()})
            }
            InstructionKind::Choice { prompt, options } if state.phase == Phase::Choice => {
                let choices: Vec<_> = options.iter().enumerate().take(64).map(|(index, option)| {
                    json!({"index":index,"label":option.label.value.chars().take(256).collect::<String>()})
                }).collect();
                json!({"prompt":prompt.value.chars().take(4096).collect::<String>(), "choices":choices, "choice_count":options.len()})
            }
            _ => serde_json::Value::Null,
        };
        let staged: Vec<_> = runtime.staged().iter().map(|actor| json!({
            "instance": actor.instance, "asset": actor.asset.name.value,
            "revision": self.session.staged_visual(actor).map(|visual| &visual.binding.revision),
        })).collect();
        json!({
            "preview": self.id, "plan_identity": self.identity,
            "scene":runtime.plan().scenes[state.scene].id, "phase":state.phase,
            "source":position, "passage":passage, "flags":state.flags, "staged":staged,
            "active_assets":state.assets, "checkpoint":self.bookmark.is_some(),
        })
    }
}

fn preview_error(message: &str) -> Failure {
    Failure::new("preview_error", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_preview_restores_its_choice_without_advancing_another_session() {
        let source = r#"flag sent false
scene start
  choice "Send now?"
    option "Send": rescue
    option "Wait": vigil
scene rescue
  set sent true
  narrate: Rescue.
  end
scene vigil
  narrate: Vigil.
  end
"#;
        let session = PlaybackSession::compile_with_policy(
            source,
            &ArtifactRegistry::default(),
            AssetPolicy::Preview,
        )
        .unwrap();
        let other = session.clone();
        let mut run = Run {
            id: Uuid::new_v4(),
            client: Uuid::new_v4(),
            owner: Uuid::new_v4(),
            identity: session.runtime.save().plan_identity,
            session,
            bookmark: None,
        };
        run.step(PreviewAction::Checkpoint).unwrap();
        run.step(PreviewAction::Choose { index: 0 }).unwrap();
        assert_eq!(run.describe()["scene"], "rescue");
        assert_eq!(run.describe()["flags"]["sent"], true);
        run.step(PreviewAction::Restore).unwrap();
        run.step(PreviewAction::Choose { index: 1 }).unwrap();
        assert_eq!(run.describe()["scene"], "vigil");
        assert_eq!(run.describe()["flags"]["sent"], false);
        assert_eq!(other.runtime.state().phase, Phase::Choice);
    }
}
