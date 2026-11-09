//! Manual, deterministic execution. Hosts resolve exact resources before start.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use storyfella_compiler::{
    AssetBinding, AssetUse, BeatId, ChoiceOption, Instruction, InstructionKind, SceneId, StoryPlan,
    VisualDirection, PLAN_VERSION,
};
use storyfella_language::{AssetKind, DirectionKind, Span, StageGroup, Text};

pub const TRANSITION_BUDGET: usize = 128;
pub const SAVE_VERSION: u32 = 1;
const MAX_RESTORE_COMMANDS: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetPolicy {
    RequireCompleted,
    AllowMissing,
    /// Studio's authoring preview may skip unsupported visual directions.
    Preview,
}

/// Scene-scoped visual effects, reconstructed by normal execution on restore.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presentation {
    pub shot: Option<VisualDirection>,
    pub mood: Option<VisualDirection>,
    pub style: Option<VisualDirection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedCharacter {
    pub instance: String,
    pub asset: AssetUse,
    pub group: StageGroup,
    pub scale: u16,
    pub depth: u16,
    pub position: Option<(u8, u8)>,
    /// Stable within a scene, so adding or hiding another instance does not move this one.
    pub slot: usize,
}

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
    validate_plan_with_policy(plan, AssetPolicy::RequireCompleted)
}

pub fn validate_plan_with_policy(
    plan: &StoryPlan,
    asset_policy: AssetPolicy,
) -> Result<Vec<AssetBinding>, Error> {
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
            InstructionKind::Stage { asset, .. } | InstructionKind::Swap { asset, .. } => {
                std::slice::from_ref(asset)
            }
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
    let recompiled = if let Some(map) = &plan.source_map {
        storyfella_compiler::compile_workspace(&map.workspace, &bindings)
            .map_err(|error| Error::InvalidPlan(error.to_string()))?
            .0
    } else {
        storyfella_compiler::compile(&plan.source, &bindings)
    };
    if recompiled.plan.as_ref() != Some(plan) {
        return Err(Error::InvalidPlan(
            "instructions, identities, ranges or requirements differ from the compiled source"
                .into(),
        ));
    }
    if (asset_policy != AssetPolicy::Preview && !plan.readiness.unsupported_directions.is_empty())
        || (asset_policy == AssetPolicy::RequireCompleted
            && !plan.readiness.pending_assets.is_empty())
    {
        let pending = plan
            .readiness
            .pending_assets
            .iter()
            .map(|a| a.name.value.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let directions = plan
            .readiness
            .unsupported_directions
            .iter()
            .map(|d| format!("{:?}: {}", d.kind, d.text.value))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(Error::NotReady(format!(
            "pending artwork [{}]; unsupported directions [{}]. Resolve these before playback",
            pending, directions
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
    FlagSet {
        name: String,
        value: bool,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flags: BTreeMap<String, bool>,
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
    asset_policy: AssetPolicy,
    // Rebuilt by replay on restore; absent artwork still has an authored name.
    active_uses: Vec<AssetUse>,
    presentation: Presentation,
    staged: Vec<StagedCharacter>,
    staging_used: bool,
}

fn initial_flags(plan: &StoryPlan) -> BTreeMap<String, bool> {
    plan.flags
        .iter()
        .map(|flag| (flag.name.value.clone(), flag.initial))
        .collect()
}

impl Runtime {
    pub fn start(plan: StoryPlan) -> Result<(Self, Vec<Event>), Error> {
        Self::start_with_policy(plan, AssetPolicy::RequireCompleted)
    }

    pub fn start_with_policy(
        plan: StoryPlan,
        asset_policy: AssetPolicy,
    ) -> Result<(Self, Vec<Event>), Error> {
        validate_plan_with_policy(&plan, asset_policy)?;
        let identity = Sha256::digest(serde_json::to_vec(&plan).expect("serializable plan"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let flags = initial_flags(&plan);
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
                flags,
            },
            commands: Vec::new(),
            asset_policy,
            active_uses: Vec::new(),
            presentation: Presentation::default(),
            staged: Vec::new(),
            staging_used: false,
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
    pub fn active_uses(&self) -> &[AssetUse] {
        &self.active_uses
    }
    pub fn presentation(&self) -> Presentation {
        self.presentation
    }
    pub fn staged(&self) -> &[StagedCharacter] {
        &self.staged
    }
    pub fn has_staging(&self) -> bool {
        self.staging_used
    }
    pub fn asset_policy(&self) -> AssetPolicy {
        self.asset_policy
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
        Self::restore_with_policy(plan, saved, AssetPolicy::RequireCompleted)
    }
    pub fn restore_with_policy(
        plan: StoryPlan,
        saved: SavedState,
        asset_policy: AssetPolicy,
    ) -> Result<Self, Error> {
        let (mut runtime, _) = Self::start_with_policy(plan, asset_policy)?;
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
                self.state.flags = initial_flags(&self.plan);
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
        self.active_uses.clear();
        self.presentation = Presentation::default();
        self.staged.clear();
        self.staging_used = false;
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
                    self.active_uses.retain(|asset| asset.kind != kind);
                    self.active_uses.extend(assets.iter().cloned());
                    let bindings: Vec<_> = assets.into_iter().filter_map(|a| a.binding).collect();
                    self.state.assets.retain(|a| a.kind != kind);
                    self.state.assets.extend(bindings.clone());
                    events.push(Event::Assets {
                        kind,
                        bindings,
                        source: instruction.source,
                    });
                }
                InstructionKind::Stage {
                    instance,
                    asset,
                    group,
                    scale,
                    depth,
                    position,
                } => {
                    self.staging_used = true;
                    let previous = self
                        .staged
                        .iter()
                        .find(|item| item.instance == instance.value);
                    let slot = previous
                        .filter(|item| item.group == group)
                        .map(|item| item.slot)
                        .unwrap_or_else(|| {
                            (0..)
                                .find(|slot| {
                                    !self
                                        .staged
                                        .iter()
                                        .any(|item| item.group == group && item.slot == *slot)
                                })
                                .expect("unbounded slots")
                        });
                    let staged = StagedCharacter {
                        instance: instance.value,
                        asset,
                        group,
                        scale,
                        depth,
                        position,
                        slot,
                    };
                    if let Some(current) = self
                        .staged
                        .iter_mut()
                        .find(|item| item.instance == staged.instance)
                    {
                        *current = staged;
                    } else {
                        self.staged.push(staged);
                    }
                }
                InstructionKind::Swap { instance, asset } => {
                    let current = self
                        .staged
                        .iter_mut()
                        .find(|item| item.instance == instance.value)
                        .expect("validated stage instance");
                    current.asset = asset;
                }
                InstructionKind::Hide { instance } => {
                    self.staged.retain(|item| item.instance != instance.value)
                }
                InstructionKind::Speech {
                    speaker,
                    thought,
                    text,
                } => {
                    if !self.active_uses.iter().any(|asset| {
                        asset.kind == speaker.kind
                            && asset.name.value.eq_ignore_ascii_case(&speaker.name.value)
                    }) {
                        self.active_uses.push(speaker.clone());
                    }
                    if let Some(binding) = speaker.binding.as_ref() {
                        if !self.state.assets.iter().any(|a| a.id == binding.id) {
                            self.state.assets.push(binding.clone());
                        }
                    }
                    self.state.phase = Phase::Text;
                    events.push(Event::Text {
                        kind: if thought {
                            TextKind::Thought
                        } else {
                            TextKind::Dialogue
                        },
                        speaker: speaker.binding,
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
                InstructionKind::SetFlag { flag, value } => {
                    let current = self
                        .state
                        .flags
                        .get_mut(&flag.value)
                        .expect("validated flag");
                    *current = value;
                    events.push(Event::FlagSet {
                        name: flag.value,
                        value,
                        source: instruction.source,
                    });
                }
                InstructionKind::Conditional {
                    condition,
                    when_true,
                    when_false,
                } => {
                    let target = if self.state.flags[&condition.flag.value] != condition.negated {
                        when_true
                    } else {
                        when_false
                    };
                    let scene = self.scene_index(&target.scene).expect("validated target");
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
                InstructionKind::Direction { kind, text } => {
                    if let Some(effect) = storyfella_compiler::visual_direction(kind, &text.value) {
                        match kind {
                            DirectionKind::Shot => self.presentation.shot = Some(effect),
                            DirectionKind::Mood => self.presentation.mood = Some(effect),
                            DirectionKind::Style => self.presentation.style = Some(effect),
                        }
                    }
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
