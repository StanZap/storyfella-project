use super::*;
use std::{cell::RefCell, fs, path::Path, rc::Rc, time::Duration};
use tokio::time::timeout;

use serde_json::Value;
use storyfella_studio_core::desktop_bridge::ProjectIdentity;

const ENTRY: &str = "include \"opening.sf\"\n";
const OPENING: &str = "scene start\n  narrate: First draft.\n  end\n";
const LOCAL: &str = "scene start\n  narrate: Local edit.\n  end\n";

fn edit(session: Uuid, version: &Value, text: &str, start: usize, end: usize) -> WriterCommand {
    WriterCommand::Edit {
        session,
        file: "story.sf".into(),
        expected_version: version.as_str().unwrap().into(),
        start,
        end,
        text: text.into(),
        apply: true,
    }
}

fn session(reply: &Value) -> Uuid {
    reply["session"].as_str().unwrap().parse().unwrap()
}

async fn send(
    state: &Rc<RefCell<AppState>>,
    grant: &Grant,
    project: &ProjectFolder,
    client: Uuid,
    epoch: &mut Option<Uuid>,
    command: WriterCommand,
) -> std::result::Result<Value, String> {
    let request = Request {
        schema: SCHEMA,
        project: ProjectIdentity::from(project),
        command,
    };
    let (response, _pending) = oneshot::channel();
    let (next, reply) = dispatch_call(state, grant, request, client, *epoch, &response).await;
    *epoch = next;
    reply.map_err(|failure| failure.code)
}

async fn probe(root: PathBuf) {
    let state = Rc::new(RefCell::new(AppState::default()));
    {
        {
            let first = root.join("first");
            let folder =
                ProjectFolder::create(&first, "First", &ArtifactRegistry::default(), ENTRY)
                    .unwrap();
            let chapter = first.join("assets/opening.sf");
            fs::write(&chapter, OPENING).unwrap();
            let second = ProjectFolder::create(
                &root.join("second"),
                "Second",
                &ArtifactRegistry::default(),
                "scene other\n  end\n",
            )
            .unwrap();
            let mut grant = Grant {
                folder: folder.clone(),
                directory: root.join("channel"),
                edit: true,
            };
            crate::commands::open_project(&first, &mut state.borrow_mut()).unwrap();

            let a = Uuid::new_v4();
            let b = Uuid::new_v4();
            let mut owner_a = None;
            let mut owner_b = None;
            let initial = send(
                &state,
                &grant,
                &folder,
                a,
                &mut owner_a,
                WriterCommand::Inspect {},
            )
            .await
            .unwrap();
            assert!(!initial["dirty"].as_bool().unwrap());
            assert_eq!(session(&initial), a);
            let wrong_project = send(
                &state,
                &grant,
                &second,
                b,
                &mut owner_b,
                WriterCommand::Inspect {},
            )
            .await
            .unwrap_err();
            assert_eq!(wrong_project, "project_mismatch");
            let other = send(
                &state,
                &grant,
                &folder,
                b,
                &mut owner_b,
                WriterCommand::Inspect {},
            )
            .await
            .unwrap();
            assert_ne!(session(&other), a);
            let cross_client = send(
                &state,
                &grant,
                &folder,
                b,
                &mut owner_b,
                WriterCommand::Read {
                    session: a,
                    file: "story.sf".into(),
                    offset: 0,
                },
            )
            .await
            .unwrap_err();
            assert_eq!(cross_client, "session_changed");

            state
                .borrow_mut()
                .open_additional_source("opening.sf")
                .unwrap();
            state
                .borrow_mut()
                .set_additional_source("opening.sf", LOCAL.into())
                .unwrap();
            state.borrow_mut().set_story_source(format!("{ENTRY}\n"));
            let stale = send(
                &state,
                &grant,
                &folder,
                a,
                &mut owner_a,
                edit(a, &initial["version"], "", 0, 1),
            )
            .await
            .unwrap_err();
            assert_eq!(stale, "stale_version");
            let draft = send(
                &state,
                &grant,
                &folder,
                b,
                &mut owner_b,
                WriterCommand::Read {
                    session: b,
                    file: "story.sf".into(),
                    offset: 0,
                },
            )
            .await
            .unwrap();
            assert_eq!(draft["source"], format!("{ENTRY}\n"));
            assert_eq!(draft["saved_digest"], initial["saved_digest"]);
            assert_ne!(draft["draft_digest"], initial["draft_digest"]);
            let append = "scene ending\n  end\n";
            let at = ENTRY.len() + 1;
            grant.edit = false;
            let forbidden = send(
                &state,
                &grant,
                &folder,
                b,
                &mut owner_b,
                edit(b, &draft["version"], append, at, at),
            )
            .await
            .unwrap_err();
            assert_eq!(forbidden, "scope_denied");
            grant.edit = true;
            let result = send(
                &state,
                &grant,
                &folder,
                b,
                &mut owner_b,
                edit(b, &draft["version"], append, at, at),
            )
            .await
            .unwrap();
            assert_eq!(result["applied"], true);
            assert!(state.borrow().story.source().ends_with(append));
            assert_eq!(
                state
                    .borrow()
                    .additional_sources
                    .as_ref()
                    .unwrap()
                    .get("opening.sf")
                    .unwrap()
                    .source(),
                LOCAL
            );
            assert_eq!(fs::read_to_string(&chapter).unwrap(), OPENING);
            assert_eq!(
                fs::read_to_string(first.join("assets/story.sf")).unwrap(),
                ENTRY
            );
            assert!(state.borrow_mut().undo());
            assert_eq!(state.borrow().story.source(), format!("{ENTRY}\n"));

            let chapter_read = send(
                &state,
                &grant,
                &folder,
                b,
                &mut owner_b,
                WriterCommand::Read {
                    session: b,
                    file: "opening.sf".into(),
                    offset: 0,
                },
            )
            .await
            .unwrap();
            assert_eq!(chapter_read["source"], LOCAL);
            let start = LOCAL.find("Local edit.").unwrap();
            let chapter_edit = WriterCommand::Edit {
                session: b,
                file: "opening.sf".into(),
                expected_version: chapter_read["version"].as_str().unwrap().into(),
                start,
                end: start + "Local edit.".len(),
                text: "MCP chapter.".into(),
                apply: true,
            };
            assert_eq!(
                send(&state, &grant, &folder, b, &mut owner_b, chapter_edit)
                    .await
                    .unwrap()["applied"],
                true
            );
            assert!(state
                .borrow()
                .additional_sources
                .as_ref()
                .unwrap()
                .get("opening.sf")
                .unwrap()
                .source()
                .contains("MCP chapter."));
            assert_eq!(fs::read_to_string(&chapter).unwrap(), OPENING);
            assert!(state.borrow_mut().undo());

            crate::commands::open_project(&second.database(), &mut state.borrow_mut()).unwrap();
            let closed = send(
                &state,
                &grant,
                &folder,
                b,
                &mut owner_b,
                WriterCommand::Inspect {},
            )
            .await
            .unwrap_err();
            assert_eq!(closed, "owner_changed");
            crate::commands::open_project(&first, &mut state.borrow_mut()).unwrap();
            let old = send(
                &state,
                &grant,
                &folder,
                a,
                &mut owner_a,
                WriterCommand::Inspect {},
            )
            .await
            .unwrap_err();
            assert_eq!(old, "owner_changed");
            let reconnected = send(
                &state,
                &grant,
                &folder,
                Uuid::new_v4(),
                &mut None,
                WriterCommand::Inspect {},
            )
            .await
            .unwrap();
            assert_ne!(session(&reconnected), a);
            assert_eq!(reconnected["dirty"], false);
            assert_eq!(fs::read_to_string(chapter).unwrap(), LOCAL);
        }
    }
}

#[tokio::test]
async fn checked_writer_edit_preserves_local_work_and_expires_with_its_owner() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
    fs::create_dir_all(&scratch).unwrap();
    let root = tempfile::tempdir_in(scratch.canonicalize().unwrap()).unwrap();
    timeout(Duration::from_secs(15), probe(root.path().to_owned()))
        .await
        .unwrap();
}
