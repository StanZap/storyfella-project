//! The assistant against a scripted model and an in-memory draft. The model
//! answers differently on every call, as a real one does; the draft refuses a
//! write whose base moved on, as Studio's does.
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use agentkit_ai::{ChatModel, ChatRequest, ChatResponse, ChatStreamEvent, StreamingChatModel};
use agentkit_core::{Capability, MessageRole, ProviderErrorKind, ProviderId, RuntimeError};
use serde_json::json;
use storyfella_compiler::SourceWorkspace;

use super::*;
use crate::registry::ArtifactRegistry;

const STORY: &str = "scene harbor\n  beat arrival\n    narrate: Fog rolls in.\n  beat departure\n    narrate: The bell rings.\n    end\n";

/// One scripted answer: tool calls, or a final text.
enum Turn {
    Calls(Vec<(&'static str, serde_json::Value)>),
    Text(&'static str),
}

/// Answers from a script and keeps every request it was sent.
struct Scripted {
    script: Mutex<VecDeque<Turn>>,
    seen: Mutex<Vec<ChatRequest>>,
    calls: Mutex<usize>,
}

impl Scripted {
    fn new(script: Vec<Turn>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(script.into()),
            seen: Mutex::new(Vec::new()),
            calls: Mutex::new(0),
        })
    }

    /// What each tool call was answered with, in order.
    fn tool_results(&self) -> Vec<String> {
        let seen = self.seen.lock().unwrap();
        seen.last()
            .map(|request| {
                request
                    .messages
                    .iter()
                    .filter(|message| message.role == MessageRole::Tool)
                    .map(|message| message.content.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl ChatModel for Scripted {
    fn provider_id(&self) -> ProviderId {
        ProviderId("scripted".into())
    }
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Chat, Capability::ToolCalling]
    }
    async fn chat(&self, _: ChatRequest) -> Result<ChatResponse, RuntimeError> {
        unreachable!("the assistant streams")
    }
}

#[async_trait::async_trait]
impl StreamingChatModel for Scripted {
    async fn chat_stream(
        &self,
        request: ChatRequest,
        on_event: &mut agentkit_ai::ChatStreamSink<'_>,
    ) -> Result<(), RuntimeError> {
        self.seen.lock().unwrap().push(request);
        let turn = self.script.lock().unwrap().pop_front().ok_or_else(|| {
            RuntimeError::provider(
                ProviderErrorKind::Provider,
                "asked more often than scripted",
            )
        })?;
        let call = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        match turn {
            Turn::Calls(calls) => {
                for (index, (name, arguments)) in calls.into_iter().enumerate() {
                    on_event(ChatStreamEvent::ToolCallDelta {
                        index,
                        id: Some(format!("call_{call}_{index}")),
                        name: Some(name.into()),
                        arguments: Some(arguments.to_string()),
                    })?;
                }
            }
            Turn::Text(text) => on_event(ChatStreamEvent::TextDelta {
                content: text.into(),
            })?,
        }
        on_event(ChatStreamEvent::Done)
    }
}

/// A draft held in memory, refusing a write whose base moved on.
struct Memory(Mutex<SourceWorkspace>);

impl Memory {
    fn new(files: &[(&str, &str)]) -> Arc<Self> {
        Arc::new(Self(Mutex::new(SourceWorkspace {
            entry: files[0].0.into(),
            files: files
                .iter()
                .map(|(path, text)| (path.to_string(), text.to_string()))
                .collect(),
        })))
    }

    fn file(&self, path: &str) -> Option<String> {
        self.0.lock().unwrap().files.get(path).cloned()
    }

    fn workspace(&self) -> SourceWorkspace {
        self.0.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl StoryDraft for Memory {
    async fn workspace(&self) -> Result<SourceWorkspace, String> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn write(
        &self,
        base: SourceWorkspace,
        texts: BTreeMap<String, String>,
    ) -> Result<(), String> {
        let mut current = self.0.lock().unwrap();
        for path in texts.keys() {
            if current.files.get(path) != base.files.get(path) {
                return Err(format!("{path} changed."));
            }
        }
        current.files.extend(texts);
        Ok(())
    }
}

fn request(instructions: &str, scope: Scope) -> AgentRequest {
    AgentRequest {
        instructions: instructions.into(),
        preserve_facts: true,
        scope,
        editor: None,
        continuing: None,
    }
}

async fn run(
    request: AgentRequest,
    model: Arc<Scripted>,
    draft: Arc<Memory>,
    settings: AssistantSettings,
) -> AgentOutcome {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    run_with_model(
        request,
        settings,
        model,
        Story {
            draft,
            registry: ArtifactRegistry::new(),
        },
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await
    .unwrap()
    .expect("not cancelled")
}

fn errors(draft: &Memory) -> usize {
    check::problems(
        &draft.workspace(),
        &AssetCatalog::from_registry(&ArtifactRegistry::new()),
    )
    .len()
}

#[tokio::test]
async fn the_assistant_edits_sees_its_error_and_fixes_it_across_files() {
    let draft = Memory::new(&[("story.sf", STORY)]);
    assert_eq!(errors(&draft), 0, "the fixture compiles");
    let model = Scripted::new(vec![
        Turn::Calls(vec![(
            "write_file",
            json!({"path": "lighthouse.sf", "text": "scene lighthouse\n  beat climb\n    narrate: Steps wind upward.\n    goto keeper\n"}),
        )]),
        Turn::Calls(vec![(
            "edit_file",
            json!({"path": "story.sf", "old_text": "    narrate: The bell rings.\n    end\n", "new_text": "    narrate: The bell rings.\n    goto lighthouse\n"}),
        )]),
        Turn::Calls(vec![(
            "edit_file",
            json!({"path": "lighthouse.sf", "old_text": "    goto keeper\n", "new_text": "    end\n"}),
        )]),
        Turn::Text("I added a lighthouse scene in lighthouse.sf and sent the harbor there."),
    ]);
    let outcome = run(
        request("Add a lighthouse", Scope::Project),
        model.clone(),
        draft.clone(),
        AssistantSettings::default(),
    )
    .await;

    let results = model.tool_results();
    assert!(results[0].starts_with("Created lighthouse.sf and included it"));
    assert!(
        results[0].contains("keeper") && results[0].contains("error"),
        "the first check reports the dangling goto: {}",
        results[0]
    );
    assert!(
        results[2].ends_with("The story compiles."),
        "{}",
        results[2]
    );
    assert_eq!(errors(&draft), 0);
    assert!(draft
        .file("story.sf")
        .unwrap()
        .contains("include \"lighthouse.sf\""));
    assert_eq!(
        outcome
            .changed
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["lighthouse.sf", "story.sf"]
    );
    assert!(!outcome.out_of_steps);
    assert!(outcome.reply.starts_with("I added a lighthouse"));
}

#[tokio::test]
async fn a_passage_request_cannot_change_anything_outside_its_passage() {
    let draft = Memory::new(&[("story.sf", STORY)]);
    let caret = STORY.find("Fog").unwrap();
    let scope = Scope::passage(Passage::Beat, "story.sf", STORY, caret..caret).unwrap();
    let model = Scripted::new(vec![
        Turn::Calls(vec![(
            "edit_file",
            json!({"path": "story.sf", "old_text": "The bell rings.", "new_text": "A gull cries."}),
        )]),
        Turn::Calls(vec![(
            "write_file",
            json!({"path": "extra.sf", "text": "scene extra\n  end\n"}),
        )]),
        Turn::Calls(vec![(
            "edit_file",
            json!({"path": "story.sf", "old_text": "Fog rolls in.", "new_text": "Fog rolls in, thick and grey.\n    narrate: Nobody waits."}),
        )]),
        // The passage grew with the edit, so its new last line is still in it.
        Turn::Calls(vec![(
            "edit_file",
            json!({"path": "story.sf", "old_text": "Nobody waits.", "new_text": "Nobody is waiting."}),
        )]),
        Turn::Text("I thickened the fog in the arrival beat."),
    ]);
    let outcome = run(
        request("Make the arrival bleaker", scope),
        model.clone(),
        draft.clone(),
        AssistantSettings::default(),
    )
    .await;

    let results = model.tool_results();
    assert!(results[0].starts_with("Refused: story.sf (line 5) is outside the scope"));
    assert!(results[1].starts_with("Creating extra.sf is outside the scope"));
    assert!(results[2].starts_with("Edited story.sf"), "{}", results[2]);
    assert!(results[3].starts_with("Edited story.sf"), "{}", results[3]);
    let story = draft.file("story.sf").unwrap();
    assert!(story.contains("The bell rings."));
    assert!(story.contains("Nobody is waiting."));
    assert!(draft.file("extra.sf").is_none());
    assert_eq!(outcome.changed.len(), 1);
}

#[tokio::test]
async fn renaming_a_scene_needs_project_scope_and_a_draft_that_can_rename() {
    for (scope, refusal) in [
        (
            Scope::File {
                file: "story.sf".into(),
            },
            "Renaming a scene is outside the scope",
        ),
        (Scope::Project, "This story cannot rename scenes"),
    ] {
        let draft = Memory::new(&[("story.sf", STORY)]);
        let model = Scripted::new(vec![
            Turn::Calls(vec![(
                "rename_scene",
                json!({"scene": "harbor", "to": "quay"}),
            )]),
            Turn::Text("I could not rename it."),
        ]);
        let outcome = run(
            request("Call the harbor the quay", scope),
            model.clone(),
            draft.clone(),
            AssistantSettings::default(),
        )
        .await;
        let results = model.tool_results();
        assert!(results[0].contains(refusal), "{}", results[0]);
        assert_eq!(draft.file("story.sf").unwrap(), STORY);
        assert!(outcome.changed.is_empty());
    }
}

#[tokio::test]
async fn a_request_out_of_steps_answers_without_tools_and_says_so() {
    let draft = Memory::new(&[("story.sf", STORY)]);
    let model = Scripted::new(vec![
        Turn::Calls(vec![("outline", json!({}))]),
        Turn::Calls(vec![("read_file", json!({"path": "story.sf"}))]),
        Turn::Calls(vec![(
            "read_file",
            json!({"path": "story.sf", "start_line": 2, "end_line": 3}),
        )]),
        Turn::Text("I read the story; rewriting the departure is left."),
    ]);
    let outcome = run(
        request("Rewrite everything", Scope::Project),
        model.clone(),
        draft,
        AssistantSettings {
            max_steps: 4,
            ..AssistantSettings::default()
        },
    )
    .await;

    let seen = model.seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    assert!(seen[..3].iter().all(|request| !request.tools.is_empty()));
    assert!(
        seen[3].tools.is_empty(),
        "the last pass is offered no tools"
    );
    assert!(outcome.out_of_steps);
    assert!(outcome.changed.is_empty());
    assert!(outcome.reply.contains("left"));
    drop(seen);
    let results = model.tool_results();
    assert!(results[1].contains("1| scene harbor"));
    assert!(
        results[2].starts_with("2|   beat arrival"),
        "{}",
        results[2]
    );
}

#[tokio::test]
async fn a_request_that_answers_with_a_step_to_spare_is_not_out_of_steps() {
    let draft = Memory::new(&[("story.sf", STORY)]);
    let model = Scripted::new(vec![
        Turn::Calls(vec![("outline", json!({}))]),
        Turn::Calls(vec![
            ("read_file", json!({"path": "story.sf"})),
            (
                "read_file",
                json!({"path": "story.sf", "start_line": 4, "end_line": 6}),
            ),
        ]),
        Turn::Text("<think>Both beats are short.</think>\nThe harbor has two beats."),
    ]);
    let outcome = run(
        request("How long is the harbor?", Scope::Project),
        model.clone(),
        draft,
        AssistantSettings {
            max_steps: 4,
            ..AssistantSettings::default()
        },
    )
    .await;

    assert_eq!(model.seen.lock().unwrap().len(), 3);
    assert!(!outcome.out_of_steps, "three of four steps were used");
    assert_eq!(outcome.reply, "The harbor has two beats.");
}

#[tokio::test]
async fn the_request_tells_the_model_where_the_writer_is_and_what_the_story_holds() {
    let draft = Memory::new(&[("story.sf", STORY)]);
    let model = Scripted::new(vec![Turn::Text("The arrival beat opens on fog.")]);
    let selection = STORY.find("Fog").unwrap();
    let mut asked = request("What happens here?", Scope::Project);
    asked.editor = Some(EditorContext {
        file: "story.sf".into(),
        caret: selection,
        selection: Some(Span::new(selection, selection + 3)),
    });
    let outcome = run(asked, model.clone(), draft, AssistantSettings::default()).await;
    let seen = model.seen.lock().unwrap();
    let task = &seen[0]
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .unwrap()
        .content;
    assert!(task.contains("in story.sf at line 3, in beat arrival of scene harbor"));
    assert!(task.contains("Selected (line 3):\nFog"));
    assert!(task.contains("scene harbor (lines 1–6): beats arrival, departure"));
    assert_eq!(outcome.reply, "The arrival beat opens on fog.");
    assert!(outcome.changed.is_empty());
}

#[tokio::test]
async fn a_cancelled_request_ends_without_an_outcome() {
    let draft = Memory::new(&[("story.sf", STORY)]);
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let outcome = run_with_model(
        request("Anything", Scope::Project),
        AssistantSettings::default(),
        Scripted::new(vec![Turn::Text("unused")]),
        Story {
            draft,
            registry: ArtifactRegistry::new(),
        },
        &store,
        Arc::new(AtomicBool::new(true)),
        |_| {},
    )
    .await
    .unwrap();
    assert!(outcome.is_none());
}
