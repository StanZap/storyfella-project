use crate::{
    AssetBinding, AssetUse, BeatId, ChoiceOption, Instruction, InstructionKind, Origin,
    PendingAsset, Readiness, SceneId, ScenePlan, StoryPlan, UnsupportedDirection, PLAN_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use storyfella_language::{
    analyze, normalize_diagnostics, Analysis, Asset, AssetKind, Diagnostic, Name, Node, NodeKind,
    Severity, Source, Span,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compilation {
    pub plan: Option<StoryPlan>,
    pub diagnostics: Vec<Diagnostic>,
}
impl Compilation {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
    pub fn ready(&self) -> bool {
        self.plan.as_ref().is_some_and(|p| p.readiness.ready()) && !self.has_errors()
    }
}

fn error(code: &str, message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
        severity: Severity::Error,
        span,
    }
}

struct Lowerer<'a> {
    source: &'a Source,
    bindings: &'a [AssetBinding],
    diagnostics: Vec<Diagnostic>,
    readiness: Readiness,
}
impl Lowerer<'_> {
    fn header(&self, span: Span) -> Span {
        let line = self.source.position(span.start).unwrap().line;
        Span::new(
            span.start,
            self.source.line_span(line as usize).unwrap().end,
        )
    }
    fn asset(&mut self, name: &Name, kind: AssetKind) -> AssetUse {
        // Shared analysis has already rejected wrong-kind and collision errors.
        let binding = self
            .bindings
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(&name.value) && b.kind == kind)
            .cloned();
        if binding.is_none() {
            if !self
                .diagnostics
                .iter()
                .any(|d| d.code == "SF203" && d.span == name.span)
            {
                self.diagnostics.push(Diagnostic {
                    code: "SF306".into(),
                    severity: Severity::Information,
                    message: format!("Pending artwork revision for '{}'", name.value),
                    span: name.span,
                });
            }
            self.readiness.pending_assets.push(PendingAsset {
                kind,
                name: name.clone(),
            });
        }
        AssetUse {
            name: name.clone(),
            kind,
            binding,
        }
    }
    fn nodes(
        &mut self,
        nodes: &[Node],
        scene: &SceneId,
        beat: Option<&BeatId>,
        instructions: &mut Vec<Instruction>,
    ) {
        for node in nodes {
            if instructions.last().is_some_and(|i| i.kind.terminal()) {
                self.diagnostics.push(error(
                    "SF301",
                    "Unreachable statement after choice, goto or end; these terminate the scene",
                    self.header(node.span),
                ));
                continue;
            }
            let kind = match &node.kind {
                NodeKind::Beat { name } => {
                    let id = BeatId {
                        scene: scene.clone(),
                        name: name.value.clone(),
                    };
                    instructions.push(Instruction {
                        source: self.header(node.span),
                        origin: Origin::Authored,
                        beat: Some(id.clone()),
                        kind: InstructionKind::BeginBeat { id: id.clone() },
                    });
                    self.nodes(&node.children, scene, Some(&id), instructions);
                    continue;
                }
                NodeKind::Asset { kind, names } => InstructionKind::Assets {
                    kind: *kind,
                    assets: names.iter().map(|name| self.asset(name, *kind)).collect(),
                },
                NodeKind::Direction { kind, text } => {
                    self.readiness
                        .unsupported_directions
                        .push(UnsupportedDirection {
                            kind: *kind,
                            text: text.clone(),
                            source: node.span,
                        });
                    self.diagnostics.push(Diagnostic {
                        code: "SF302".into(),
                        severity: Severity::Information,
                        message: format!(
                            "{:?} direction execution is not implemented; request retained in the plan",
                            kind
                        ),
                        span: node.span,
                    });
                    InstructionKind::Direction {
                        kind: *kind,
                        text: text.clone(),
                    }
                }
                NodeKind::Speech {
                    speaker,
                    thought,
                    text,
                } => InstructionKind::Speech {
                    speaker: self.asset(speaker, AssetKind::Character),
                    thought: *thought,
                    text: text.clone(),
                },
                NodeKind::Narration { text } => InstructionKind::Narration { text: text.clone() },
                NodeKind::Goto { target } => InstructionKind::Jump {
                    target: SceneId(target.value.clone()),
                    target_source: target.span,
                },
                NodeKind::End => InstructionKind::End,
                NodeKind::Choice { prompt } => {
                    let mut options = Vec::new();
                    let mut labels = BTreeSet::new();
                    for option in &node.children {
                        let NodeKind::Option { label, target } = &option.kind else {
                            unreachable!("parser validates choice children");
                        };
                        if label.value.trim().is_empty() || !labels.insert(label.value.clone()) {
                            self.diagnostics.push(error(
                                "SF303",
                                "Choice option labels must be nonempty and distinct",
                                label.span,
                            ));
                        }
                        options.push(ChoiceOption {
                            source: option.span,
                            label: label.clone(),
                            target: SceneId(target.value.clone()),
                            target_source: target.span,
                        });
                    }
                    if prompt.value.trim().is_empty() {
                        self.diagnostics.push(error(
                            "SF303",
                            "Choice prompt must be nonempty",
                            prompt.span,
                        ));
                    }
                    InstructionKind::Choice {
                        prompt: prompt.clone(),
                        options,
                    }
                }
                NodeKind::Scene { .. } | NodeKind::Option { .. } => {
                    unreachable!("parser validates scene contents")
                }
            };
            instructions.push(Instruction {
                source: node.span,
                origin: Origin::Authored,
                beat: beat.cloned(),
                kind,
            });
        }
    }
}

/// First declared scene is entry. Nonterminal scenes fall through in declaration order.
/// Explicit choice/goto/end stops the scene. The last implicit boundary ends the story.
/// Missing assets/direction actuators retain a plan with blocked readiness; source errors do not.
pub fn compile(source: &str, bindings: &[AssetBinding]) -> Compilation {
    let catalog: Vec<_> = bindings
        .iter()
        .map(|b| Asset {
            id: b.id.clone(),
            name: b.name.clone(),
            kind: b.kind,
        })
        .collect();
    let analysis = analyze(source, &catalog);
    compile_analysis(&analysis, bindings)
}

/// Reuse the host's shared analysis, including catalog entries without artwork.
pub fn compile_analysis(analysis: &Analysis, bindings: &[AssetBinding]) -> Compilation {
    let mut diagnostics = analysis.diagnostics.clone();
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for binding in bindings {
        if binding.name.trim().is_empty()
            || binding.id.trim().is_empty()
            || binding.revision.trim().is_empty()
            || !ids.insert(&binding.id)
            || !names.insert(binding.name.to_ascii_lowercase())
        {
            let span = analysis
                .references
                .iter()
                .find(|r| r.name.value.eq_ignore_ascii_case(&binding.name))
                .map_or(Span::default(), |r| r.name.span);
            diagnostics.push(error(
                "SF304",
                "Asset bindings require nonempty immutable IDs/revisions and unique names/IDs",
                span,
            ));
        }
    }
    if analysis.document.story.scenes.is_empty() {
        diagnostics.push(error(
            "SF300",
            "A compiled story requires at least one scene",
            Span::default(),
        ));
    }
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        normalize_diagnostics(&mut diagnostics);
        return Compilation {
            plan: None,
            diagnostics,
        };
    }
    let mut lowerer = Lowerer {
        source: &analysis.document.source,
        bindings,
        diagnostics,
        readiness: Readiness::default(),
    };
    let nodes = &analysis.document.story.scenes;
    let scene_ids: Vec<_> = nodes
        .iter()
        .map(|node| match &node.kind {
            NodeKind::Scene { name } => SceneId(name.value.clone()),
            _ => unreachable!("parser validates root scenes"),
        })
        .collect();
    let mut scenes = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let id = &scene_ids[index];
        let mut instructions = Vec::new();
        lowerer.nodes(&node.children, id, None, &mut instructions);
        if !instructions.last().is_some_and(|i| i.kind.terminal()) {
            let kind = scene_ids
                .get(index + 1)
                .map_or(InstructionKind::End, |next| InstructionKind::Jump {
                    target: next.clone(),
                    target_source: Span::new(node.span.end, node.span.end),
                });
            instructions.push(Instruction {
                source: Span::new(node.span.end, node.span.end),
                origin: Origin::SceneBoundary,
                beat: None,
                kind,
            });
        }
        scenes.push(ScenePlan {
            id: id.clone(),
            source: node.span,
            instructions,
        });
    }
    if lowerer
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        normalize_diagnostics(&mut lowerer.diagnostics);
        return Compilation {
            plan: None,
            diagnostics: lowerer.diagnostics,
        };
    }
    let by_id: BTreeMap<_, _> = scenes.iter().map(|scene| (&scene.id, scene)).collect();
    let mut visited = BTreeSet::new();
    let mut queue = vec![&scene_ids[0]];
    while let Some(id) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        let scene = by_id[id];
        match &scene.instructions.last().unwrap().kind {
            InstructionKind::Jump { target, .. } => queue.push(target),
            InstructionKind::Choice { options, .. } => {
                queue.extend(options.iter().map(|option| &option.target))
            }
            InstructionKind::End => {}
            _ => unreachable!("lowering always emits a terminal instruction"),
        }
    }
    for scene in &scenes {
        if !visited.contains(&scene.id) {
            lowerer.diagnostics.push(Diagnostic {
                code: "SF305".into(),
                severity: Severity::Warning,
                message: format!("Scene '{}' is unreachable from the story entry", scene.id.0),
                span: lowerer.header(scene.source),
            });
        }
    }
    normalize_diagnostics(&mut lowerer.diagnostics);
    Compilation {
        plan: Some(StoryPlan {
            version: PLAN_VERSION,
            source: analysis.document.source.text().into(),
            title: analysis.document.story.title.clone(),
            entry: scene_ids[0].clone(),
            scenes,
            readiness: lowerer.readiness,
        }),
        diagnostics: lowerer.diagnostics,
    }
}
