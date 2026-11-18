use std::{fmt::Write as _, fs, path::Path, process::Stdio, time::Duration};

use agentkit_core::{ToolCall, ToolResultStatus};
use agentkit_mcp::{ExternalToolSource, StdioMcpClient};
use serde_json::{json, Value};
use storyfella_studio_core::{
    project::ProjectFolder,
    registry::{ArtifactKind, ArtifactRegistry, CreateArtifact},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

const INSPECT: &str = "storyfella_project_inspect";
const EXAMPLE: &str = "storyfella_story_example";
const READ: &str = "storyfella_story_read";
const EDIT: &str = "storyfella_story_edit";
const PLAYTEST: &str = "storyfella_story_playtest";

fn command(folder: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sf-cli"));
    command
        .args(["mcp", "--folder"])
        .arg(folder)
        .args(["--create-if-missing", "The Last Signal"])
        .kill_on_drop(true);
    command
}

fn fixture() -> tempfile::TempDir {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
    fs::create_dir_all(&scratch).unwrap();
    tempfile::tempdir_in(scratch.canonicalize().unwrap()).unwrap()
}

struct Client {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    id: u32,
}

impl Client {
    async fn start(folder: &Path) -> Self {
        let mut child = command(folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut client = Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            id: 0,
        };
        let init = client
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "storyfella-wire-test", "version": "1"}
                }),
            )
            .await;
        assert!(init["capabilities"]["tools"].is_object(), "{init}");
        client
            .send(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))
            .await;
        client
    }

    async fn send(&mut self, message: Value) {
        let mut bytes = serde_json::to_vec(&message).unwrap();
        bytes.push(b'\n');
        self.input.write_all(&bytes).await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(json!({"jsonrpc":"2.0", "id":self.id, "method":method, "params":params}))
            .await;
        let line = timeout(Duration::from_secs(10), self.output.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], self.id, "{response}");
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    async fn call(&mut self, name: &str, arguments: Value, error: bool) -> Value {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        assert_eq!(
            result["isError"].as_bool().unwrap_or_default(),
            error,
            "{result}"
        );
        assert!(result["structuredContent"].is_object(), "{result}");
        result["structuredContent"].clone()
    }

    async fn close(self) {
        drop(self.input);
        let mut child = self.child;
        assert!(timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success());
    }
}

#[tokio::test]
async fn mcp_creates_and_playtests_a_branching_chapter_then_reopens_in_a_second_client() {
    let fixture = fixture();
    let folder = fixture.path().join("signal");
    let mut client = Client::start(&folder).await;
    let tools = client.request("tools/list", json!({})).await;
    assert!(tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == EXAMPLE));
    let example = client.call(EXAMPLE, json!({}), false).await;
    let chapter = example["source"].as_str().unwrap();
    let project = client.call(INSPECT, json!({}), false).await;
    let session = &project["session"];
    assert_eq!(project["name"], "The Last Signal");
    let path = folder.join("assets/story.sf");
    let first_version = &project["version"];

    let opened = ProjectFolder::locate_for_authoring(&folder)
        .unwrap()
        .unwrap();
    assert!(
        opened.claim_authoring().is_err(),
        "Studio's lease must be held by the child process"
    );
    let blocked = command(&folder).output().await.unwrap();
    assert!(!blocked.status.success());
    assert!(blocked.stdout.is_empty());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("open for editing"));

    let error = client
        .call(
            READ,
            json!({"session":session, "file":"../private.sf"}),
            true,
        )
        .await;
    assert_eq!(error["code"], "unknown_file");
    let missing_file = client.call(READ, json!({"session":session}), true).await;
    assert_eq!(missing_file["code"], "invalid_arguments");
    let invalid_line = "    option \"🌊\"";
    let invalid_source = format!("scene start\n  choice \"Continue?\"\n{invalid_line}\n");
    let invalid = client
        .call(
            EDIT,
            json!({"session":session,"file":project["entry"],"expected_version":first_version,"start":0,"end":0,
                "text":invalid_source}),
            false,
        )
        .await;
    assert_eq!(invalid["valid"], false);
    let syntax = invalid["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "SF100")
        .unwrap();
    assert_eq!(syntax["file"], "story.sf");
    assert_eq!(syntax["start"], invalid_source.find("option").unwrap());
    assert_eq!(syntax["end"], invalid_source.len() - 1);
    assert_eq!(syntax["start_position"], json!({"line":2,"character":4}));
    assert_eq!(syntax["end_position"], json!({"line":2,"character":15}));
    assert!(invalid["proposal"].is_null());
    assert_eq!(fs::read_to_string(&path).unwrap(), "");

    let mut edit = json!({"session":session,"file":project["entry"],"expected_version":first_version,"start":0,"end":0,"text":chapter});
    let proposed = client.call(EDIT, edit.clone(), false).await;
    assert_eq!(proposed["valid"], true);
    assert_eq!(proposed["saved"], false);
    assert_eq!(fs::read_to_string(&path).unwrap(), "");
    edit["proposal"] = proposed["proposal"].clone();
    let mut different = edit.clone();
    different["text"] = chapter
        .replace("Light the channel.", "Ring the bell.")
        .into();
    assert_eq!(
        client.call(EDIT, different, true).await["code"],
        "proposal_changed"
    );
    let saved = client.call(EDIT, edit.clone(), false).await;
    assert_eq!(saved["saved"], true);
    assert_eq!(fs::read_to_string(&path).unwrap(), chapter);
    assert_eq!(client.call(EDIT, edit, true).await["code"], "stale_version");
    let read = client
        .call(
            READ,
            json!({"session":session,"file":project["entry"]}),
            false,
        )
        .await;
    assert_eq!(read["source"], chapter);
    assert_eq!(read["version"], saved["version"]);
    let split_utf8 = chapter.find('🌊').unwrap() + 1;
    let error = client
        .call(
            READ,
            json!({"session":session,"file":project["entry"],"offset":split_utf8}),
            true,
        )
        .await;
    assert_eq!(error["code"], "invalid_offset");

    for (choices, stop, trusted, line) in [
        (json!([]), "choice_required", false, None),
        (json!([0]), "complete", true, Some("Light the channel.")),
        (
            json!([1]),
            "complete",
            false,
            Some("The channel stays dark."),
        ),
    ] {
        let result = client
            .call(
                PLAYTEST,
                json!({"session":session,"expected_version":saved["version"],"choices":choices}),
                false,
            )
            .await;
        assert_eq!(result["stop"], stop);
        assert_eq!(result["flags"]["trusted"], trusted);
        assert!(result["pending_asset_count"].as_u64().unwrap() > 0);
        assert_eq!(result["trace"][0]["content"]["speaker"], "mara");
        if let Some(line) = line {
            assert!(result["trace"]
                .as_array()
                .unwrap()
                .iter()
                .any(|frame| frame["content"]["text"] == line));
        } else {
            assert_eq!(
                result["trace"].as_array().unwrap().last().unwrap()["content"]["options"][1]
                    ["index"],
                1
            );
        }
    }

    let external = chapter.replace("Light the channel.", "An outside edit.");
    fs::write(&path, &external).unwrap();
    let error = client
        .call(
            PLAYTEST,
            json!({"session":session,"expected_version":saved["version"]}),
            true,
        )
        .await;
    assert_eq!(error["code"], "stale_version");
    client.close().await;
    let reopened = opened.load().unwrap();
    assert_eq!(reopened.story.source(), external);
    assert!(reopened
        .registry
        .artifacts
        .iter()
        .any(|asset| asset.name == "harbor"));
    let lease = opened.claim_authoring().unwrap();
    drop(lease);

    let other = timeout(
        Duration::from_secs(10),
        StdioMcpClient::connect_child(
            ExternalToolSource {
                source_id: "storyfella".into(),
                display_name: "Storyfella".into(),
            },
            command(&folder),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let tool = other
        .execute_tool_call(ToolCall {
            id: "inspect".into(),
            name: INSPECT.into(),
            arguments: json!({}),
        })
        .await
        .unwrap();
    assert_eq!(tool.status, ToolResultStatus::Success);
    let inspected: Value = serde_json::from_str(&tool.content).unwrap();
    assert_eq!(inspected["project_id"], project["project_id"]);
    assert_ne!(inspected["session"], *session);
    let stale_session = other
        .execute_tool_call(ToolCall {
            id: "read".into(),
            name: READ.into(),
            arguments: json!({"session":session,"file":inspected["entry"]}),
        })
        .await
        .unwrap();
    assert_eq!(stale_session.status, ToolResultStatus::Error);
    assert_eq!(
        serde_json::from_str::<Value>(&stale_session.content).unwrap()["code"],
        "session_changed"
    );
    timeout(Duration::from_secs(10), other.close())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), external);
}

#[tokio::test]
async fn mcp_rejects_changed_includes_and_a_replaced_writer_lock() {
    let fixture = fixture();
    let root = fixture.path().join("split");
    let entry = "include \"chapter.sf\"\n";
    ProjectFolder::create(&root, "Split", &ArtifactRegistry::default(), entry).unwrap();
    let chapter = root.join("assets/chapter.sf");
    fs::write(&chapter, "scene start\n  narrate: First.\n  end\n").unwrap();
    let mut client = Client::start(&root).await;
    let project = client.call(INSPECT, json!({}), false).await;
    let session = &project["session"];
    let included = client
        .call(READ, json!({"session":session,"file":"chapter.sf"}), false)
        .await;
    assert!(included["source"].as_str().unwrap().contains("First."));
    let mut edit = json!({"session":session,"file":project["entry"],"expected_version":project["version"],
        "start":0,"end":0,"text":"# Reviewed\n"});
    let preview = client.call(EDIT, edit.clone(), false).await;
    assert_eq!(preview["valid"], true, "{preview}");
    edit["proposal"] = preview["proposal"].clone();
    fs::write(
        &chapter,
        "scene start\n  narrate: Someone changed this file.\n  end\n",
    )
    .unwrap();
    assert_eq!(client.call(EDIT, edit, true).await["code"], "stale_version");
    assert_eq!(
        fs::read_to_string(root.join("assets/story.sf")).unwrap(),
        entry
    );

    let broken = "scene start\r\n  choice \"Where?\"\r\n    option \"🌊\": unknown\r\n    option \"Back\": start\r\n";
    fs::write(&chapter, broken).unwrap();
    let current = client.call(INSPECT, json!({}), false).await;
    let preview = client
        .call(
            EDIT,
            json!({"session":session,"file":current["entry"],"expected_version":current["version"],"start":0,"end":0,
                "text":"# Reviewed\n"}),
            false,
        )
        .await;
    assert_eq!(preview["valid"], false);
    let target = preview["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "SF202")
        .unwrap();
    let start = broken.find("unknown").unwrap();
    assert_eq!(target["file"], "chapter.sf");
    assert_eq!(target["start"], start);
    assert_eq!(target["end"], start + "unknown".len());
    assert_eq!(target["start_position"], json!({"line":2,"character":17}));
    assert_eq!(target["end_position"], json!({"line":2,"character":24}));

    let lock = root.join(".storyfella-studio.lock");
    let old_lock = root.join(".old-lock");
    fs::rename(&lock, &old_lock).unwrap();
    fs::copy(&old_lock, &lock).unwrap();
    let error = client.call(INSPECT, json!({}), true).await;
    assert_eq!(error["code"], "project_moved");
    client.close().await;
    assert_eq!(
        fs::read_to_string(root.join("assets/story.sf")).unwrap(),
        entry
    );
}

#[tokio::test]
async fn mcp_edits_one_chapter_in_a_large_project_and_playtests_cross_file_choices() {
    const CHAPTERS: usize = 96;
    let fixture = fixture();
    let root = fixture.path().join("anthology");
    let mut entry = String::from(
        "scene opening\n  choice \"Which chapter?\"\n    option \"First\": chapter_0\n    option \"Last\": chapter_95\n",
    );
    for index in 0..CHAPTERS {
        writeln!(entry, "include \"chapters/chapter_{index}.sf\"").unwrap();
    }
    let mut registry = ArtifactRegistry::default();
    let illustrations: Vec<_> = (0..64)
        .map(|index| {
            registry
                .create_artifact(
                    ArtifactKind::Illustration,
                    CreateArtifact {
                        name: format!("illustration_{index}"),
                        ..Default::default()
                    },
                )
                .unwrap()
        })
        .collect();
    let folder = ProjectFolder::create(&root, "Anthology", &registry, &entry).unwrap();
    fs::create_dir(root.join("assets/chapters")).unwrap();
    for index in 0..CHAPTERS {
        fs::write(
            root.join(format!("assets/chapters/chapter_{index}.sf")),
            format!("scene chapter_{index}\n  location: room_{index}\n  cast: speaker_{index}\n  speaker_{index}: Chapter {index}.\n  end\n"),
        )
        .unwrap();
    }
    let mut client = Client::start(&root).await;
    let project = client.call(INSPECT, json!({}), false).await;
    let session = &project["session"];
    assert_eq!(project["files"].as_array().unwrap().len(), CHAPTERS + 1);
    let file = "chapters/chapter_95.sf";
    let read = client
        .call(READ, json!({"session":session,"file":file}), false)
        .await;
    let target = root.join("assets").join(file);
    let new_source = "scene chapter_95\n  location: room_95\n  cast: archivist\n  archivist: The archive opens.\n  end\n";
    let mut edit = json!({"session":session,"expected_version":project["version"],"file":file,
        "start":0,"end":read["bytes"],"text":new_source});
    let mut unknown = edit.clone();
    unknown["file"] = json!("../outside.sf");
    assert_eq!(
        client.call(EDIT, unknown, true).await["code"],
        "unknown_file"
    );
    let preview = client.call(EDIT, edit.clone(), false).await;
    assert_eq!(preview["valid"], true, "{preview}");
    assert_eq!(preview["saved"], false);
    assert_eq!(fs::read_to_string(&target).unwrap(), read["source"]);
    edit["proposal"] = preview["proposal"].clone();
    let saved = client.call(EDIT, edit.clone(), false).await;
    assert_eq!(saved["saved"], true, "{saved}");
    assert_eq!(fs::read_to_string(&target).unwrap(), new_source);
    assert_eq!(
        fs::read_to_string(root.join("assets/story.sf")).unwrap(),
        entry
    );
    assert_eq!(client.call(EDIT, edit, true).await["code"], "stale_version");

    for (choice, file, line) in [
        (0, "chapters/chapter_0.sf", "Chapter 0."),
        (1, file, "The archive opens."),
    ] {
        let result = client
            .call(
                PLAYTEST,
                json!({
                    "session":session,"expected_version":saved["version"],"choices":[choice]
                }),
                false,
            )
            .await;
        assert_eq!(result["stop"], "complete", "{result}");
        assert!(
            result["trace"]
                .as_array()
                .unwrap()
                .iter()
                .any(|frame| { frame["file"] == file && frame["content"]["text"] == line }),
            "{result}"
        );
    }
    client.close().await;
    let reopened = folder.load().unwrap();
    assert_eq!(reopened.registry.story_scenes.len(), CHAPTERS + 1);
    assert_eq!(
        reopened
            .registry
            .artifacts
            .iter()
            .filter(|asset| asset.kind == ArtifactKind::Character)
            .count(),
        CHAPTERS
    );
    assert!(reopened
        .registry
        .artifacts
        .iter()
        .any(|asset| asset.name == "archivist"));
    for id in illustrations {
        assert_eq!(reopened.registry.artifact(id), registry.artifact(id));
    }
}
