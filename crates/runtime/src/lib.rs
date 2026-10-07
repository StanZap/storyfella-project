//! Manual, deterministic execution. Hosts resolve exact resources before start.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use storyfella_compiler::{
    AssetBinding, BeatId, ChoiceOption, Instruction, InstructionKind, SceneId, StoryPlan,
    PLAN_VERSION,
};
use storyfella_language::{AssetKind, Span, Text};

pub const TRANSITION_BUDGET: usize = 128;
pub const SAVE_VERSION: u32 = 1;
const MAX_RESTORE_COMMANDS: usize = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid story plan: {0}")]
    InvalidPlan(String),
    #[error("Story is not ready: {0}")]
    NotReady(String),
    #[error("Cannot perform command: {0}")]
    InvalidCommand(&'static str),
    #[error("Invalid saved state: {0}")]
    InvalidState(String),
}

/// Recompile the retained source using only its pinned bindings. Equality
/// validates identities, flow, ranges, requirements and every instruction using
/// the single compiler implementation, including arbitrary imported JSON.
pub fn validate_plan(plan: &StoryPlan) -> Result<Vec<AssetBinding>, Error> {
    if plan.version != PLAN_VERSION {
        return Err(Error::InvalidPlan(format!(
            "unsupported version {}",
            plan.version
        )));
    }
    let mut bindings = BTreeMap::<String, AssetBinding>::new();
    for instruction in plan.scenes.iter().flat_map(|s| &s.instructions) {
        let uses = match &instruction.kind {
            InstructionKind::Assets { assets, .. } => assets.as_slice(),
            InstructionKind::Speech { speaker, .. } => std::slice::from_ref(speaker),
            _ => &[],
        };
        for binding in uses.iter().filter_map(|a| a.binding.as_ref()) {
            if let Some(previous) = bindings.insert(binding.id.clone(), binding.clone()) {
                if previous != *binding {
                    return Err(Error::InvalidPlan(format!(
                        "conflicting bindings for {}",
                        binding.id
                    )));
                }
            }
        }
    }
    let bindings: Vec<_> = bindings.into_values().collect();
    if storyfella_compiler::compile(&plan.source, &bindings)
        .plan
        .as_ref()
        != Some(plan)
    {
        return Err(Error::InvalidPlan(
            "instructions, identities, ranges or requirements differ from the compiled source"
                .into(),
        ));
    }
    if !plan.readiness.ready() {
        return Err(Error::NotReady(format!(
            "{} pending asset uses, {} unsupported directions; resolve them before playback",
            plan.readiness.pending_assets.len(),
            plan.readiness.unsupported_directions.len()
        )));
    }
    Ok(bindings)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Advance,
    SelectChoice { index: usize },
    Jump { scene: SceneId },
    End,
    Pause,
    Resume,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextKind {
    Dialogue,
    Thought,
    Narration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    SceneEntered {
        scene: SceneId,
        source: Span,
    },
    BeatEntered {
        beat: BeatId,
        source: Span,
    },
    Assets {
        kind: AssetKind,
        bindings: Vec<AssetBinding>,
        source: Span,
    },
    Text {
        kind: TextKind,
        speaker: Option<AssetBinding>,
        text: Text,
        source: Span,
    },
    Choice {
        prompt: Text,
        options: Vec<ChoiceOption>,
        source: Span,
    },
    Completed {
        source: Span,
    },
    TransitionBudgetExceeded {
        source: Span,
    },
    Paused,
    Resumed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Text,
    Choice,
    Budget,
    Complete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub scene: usize,
    pub instruction: usize,
    pub beat: Option<BeatId>,
    pub assets: Vec<AssetBinding>,
    pub phase: Phase,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedState {
    pub version: u32,
    pub plan_identity: String,
    pub commands: Vec<Command>,
    pub checkpoint: State,
}

#[derive(Clone, Debug)]
pub struct Runtime {
    plan: StoryPlan,
    identity: String,
    state: State,
    commands: Vec<Command>,
}

impl Runtime {
    pub fn start(plan: StoryPlan) -> Result<(Self, Vec<Event>), Error> {
        validate_plan(&plan)?;
        let identity = Sha256::digest(serde_json::to_vec(&plan).expect("serializable plan"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let mut runtime = Self {
            plan,
            identity,
            state: State {
                scene: 0,
                instruction: 0,
                beat: None,
                assets: Vec::new(),
                phase: Phase::Budget,
                paused: false,
            },
            commands: Vec::new(),
        };
        let mut events = Vec::new();
        runtime.enter(0, &mut events);
        runtime.pump(&mut events);
        Ok((runtime, events))
    }

    pub fn plan(&self) -> &StoryPlan {
        &self.plan
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn current(&self) -> &Instruction {
        &self.plan.scenes[self.state.scene].instructions[self.state.instruction]
    }
    pub fn save(&self) -> SavedState {
        SavedState {
            version: SAVE_VERSION,
            plan_identity: self.identity.clone(),
            commands: self.commands.clone(),
            checkpoint: self.state.clone(),
        }
    }
    /// Replay bounded transitions and compare the checkpoint instead of trusting
    /// deserialized indices, active assets or a purported waiting position.
    pub fn restore(plan: StoryPlan, saved: SavedState) -> Result<Self, Error> {
        let (mut runtime, _) = Self::start(plan)?;
        if saved.version != SAVE_VERSION || saved.plan_identity != runtime.identity {
            return Err(Error::InvalidState(
                "version or exact plan identity differs; restart this story".into(),
            ));
        }
        if saved.commands.len() > MAX_RESTORE_COMMANDS {
            return Err(Error::InvalidState(
                "command history exceeds the restore limit".into(),
            ));
        }
        for command in saved.commands {
            runtime
                .dispatch(command)
                .map_err(|e| Error::InvalidState(e.to_string()))?;
        }
        if runtime.state != saved.checkpoint {
            return Err(Error::InvalidState(
                "checkpoint does not match the verified execution history".into(),
            ));
        }
        Ok(runtime)
    }

    pub fn dispatch(&mut self, command: Command) -> Result<Vec<Event>, Error> {
        if self.commands.len() >= MAX_RESTORE_COMMANDS && command != Command::Restart {
            return Err(Error::InvalidCommand(
                "session history is full; restart to begin a new session",
            ));
        }
        let mut events = Vec::new();
        match &command {
            Command::Pause => {
                self.state.paused = true;
                events.push(Event::Paused);
            }
            Command::Resume => {
                self.state.paused = false;
                events.push(Event::Resumed);
            }
            Command::End => {
                self.state.phase = Phase::Complete;
                events.push(Event::Completed {
                    source: self.current().source,
                });
            }
            Command::Restart => {
                self.state.paused = false;
                self.enter(0, &mut events);
                self.pump(&mut events);
            }
            _ if self.state.paused => {
                return Err(Error::InvalidCommand(
                    "resume before advancing, choosing or jumping",
                ))
            }
            Command::Advance => {
                match self.state.phase {
                    Phase::Text => self.state.instruction += 1,
                    Phase::Budget => {}
                    Phase::Choice => return Err(Error::InvalidCommand("select a choice option")),
                    Phase::Complete => {
                        return Err(Error::InvalidCommand("story completed; restart or jump"))
                    }
                }
                self.pump(&mut events);
            }
            Command::SelectChoice { index } => {
                let InstructionKind::Choice { options, .. } = &self.current().kind else {
                    return Err(Error::InvalidCommand("no choice is presented"));
                };
                if self.state.phase != Phase::Choice {
                    return Err(Error::InvalidCommand("no choice is presented"));
                }
                let target = options
                    .get(*index)
                    .ok_or(Error::InvalidCommand("choice index is out of range"))?
                    .target
                    .clone();
                let scene = self.scene_index(&target)?;
                self.enter(scene, &mut events);
                self.pump(&mut events);
            }
            Command::Jump { scene } => {
                let scene = self.scene_index(scene)?;
                self.enter(scene, &mut events);
                self.pump(&mut events);
            }
        }
        if matches!(command, Command::Restart) {
            self.commands.clear();
        } else {
            self.commands.push(command);
        }
        Ok(events)
    }

    fn scene_index(&self, id: &SceneId) -> Result<usize, Error> {
        self.plan
            .scenes
            .iter()
            .position(|s| s.id == *id)
            .ok_or(Error::InvalidCommand("scene does not exist"))
    }
    fn enter(&mut self, scene: usize, events: &mut Vec<Event>) {
        self.state.scene = scene;
        self.state.instruction = 0;
        self.state.beat = None;
        self.state.assets.clear();
        self.state.phase = Phase::Budget;
        events.push(Event::SceneEntered {
            scene: self.plan.scenes[scene].id.clone(),
            source: self.plan.scenes[scene].source,
        });
    }
    fn pump(&mut self, events: &mut Vec<Event>) {
        for _ in 0..TRANSITION_BUDGET {
            let instruction = self.current().clone();
            match instruction.kind {
                InstructionKind::BeginBeat { id } => {
                    self.state.beat = Some(id.clone());
                    events.push(Event::BeatEntered {
                        beat: id,
                        source: instruction.source,
                    });
                }
                InstructionKind::Assets { kind, assets } => {
                    let bindings: Vec<_> = assets
                        .into_iter()
                        .map(|a| a.binding.expect("validated ready plan"))
                        .collect();
                    self.state.assets.retain(|a| a.kind != kind);
                    self.state.assets.extend(bindings.clone());
                    events.push(Event::Assets {
                        kind,
                        bindings,
                        source: instruction.source,
                    });
                }
                InstructionKind::Speech {
                    speaker,
                    thought,
                    text,
                } => {
                    let binding = speaker.binding.expect("validated ready plan");
                    if !self.state.assets.iter().any(|a| a.id == binding.id) {
                        self.state.assets.push(binding.clone());
                    }
                    self.state.phase = Phase::Text;
                    events.push(Event::Text {
                        kind: if thought {
                            TextKind::Thought
                        } else {
                            TextKind::Dialogue
                        },
                        speaker: Some(binding),
                        text,
                        source: instruction.source,
                    });
                    return;
                }
                InstructionKind::Narration { text } => {
                    self.state.phase = Phase::Text;
                    events.push(Event::Text {
                        kind: TextKind::Narration,
                        speaker: None,
                        text,
                        source: instruction.source,
                    });
                    return;
                }
                InstructionKind::Choice { prompt, options } => {
                    self.state.phase = Phase::Choice;
                    events.push(Event::Choice {
                        prompt,
                        options,
                        source: instruction.source,
                    });
                    return;
                }
                InstructionKind::Jump { target, .. } => {
                    let scene = self.scene_index(&target).expect("validated target");
                    self.enter(scene, events);
                    continue;
                }
                InstructionKind::End => {
                    self.state.phase = Phase::Complete;
                    events.push(Event::Completed {
                        source: instruction.source,
                    });
                    return;
                }
                InstructionKind::Direction { .. } => {
                    unreachable!("unsupported directions block validated plans")
                }
            }
            self.state.instruction += 1;
        }
        self.state.phase = Phase::Budget;
        events.push(Event::TransitionBudgetExceeded {
            source: self.current().source,
        });
    }
}
