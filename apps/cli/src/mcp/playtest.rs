use serde::Deserialize;
use serde_json::{json, Value};
use storyfella_compiler::{compile_workspace_with_catalog, ExpandedWorkspace, InstructionKind};
use storyfella_runtime::{AssetPolicy, Command, Event, Phase, Runtime};
use uuid::Uuid;

use super::{
    bounded_text, fail, source_unavailable, stale, version, AssetCatalog, Reply, StoryTools,
};

pub(super) const MAX_STEPS: usize = 32;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Playtest {
    session: Uuid,
    expected_version: String,
    #[serde(default)]
    choices: Vec<usize>,
}

impl StoryTools {
    pub(super) fn playtest(&self, args: Playtest) -> Reply {
        self.check_session(args.session)?;
        if args.choices.len() > MAX_STEPS {
            return Err(fail(
                "too_many_choices",
                "Supply at most 32 choice indexes.",
            ));
        }
        let (stored, workspace) = self.snapshot()?;
        let current_version = version(&workspace, stored.registry_version);
        if args.expected_version != current_version {
            return Err(stale(current_version));
        }
        let catalog = AssetCatalog::from_registry(&stored.registry);
        let (compilation, expanded) =
            compile_workspace_with_catalog(&workspace, &catalog.assets, &catalog.bindings)
                .map_err(|_| source_unavailable())?;
        let plan = compilation.plan.ok_or_else(|| {
            fail(
                "invalid_story",
                "Fix the story's compiler errors before playtesting.",
            )
        })?;
        let pending_assets = plan.readiness.pending_assets.len();
        let skipped_directions = plan.readiness.unsupported_directions.len();
        let (mut runtime, events) = Runtime::start_with_policy(plan, AssetPolicy::Preview)
            .map_err(|_| fail("playtest_failed", "The story plan could not start."))?;
        let mut trace = vec![frame(&runtime, &events, &expanded, &workspace.entry)];
        let mut choices = args.choices.iter();
        let mut choices_used = 0;
        let mut steps = 0;
        let stop = loop {
            let phase = runtime.state().phase;
            if phase == Phase::Complete {
                break "complete";
            }
            if phase == Phase::Budget {
                break "transition_budget";
            }
            if steps == MAX_STEPS {
                break "step_budget";
            }
            let command = if phase == Phase::Choice {
                let Some(&index) = choices.next() else {
                    break "choice_required";
                };
                choices_used += 1;
                Command::SelectChoice { index }
            } else {
                Command::Advance
            };
            let events = runtime.dispatch(command)
                .map_err(|_| fail("invalid_choice", "A choice index is out of range. Playtest without that index to inspect the options."))?;
            steps += 1;
            trace.push(frame(&runtime, &events, &expanded, &workspace.entry));
        };
        let flags = &runtime.state().flags;
        Ok(json!({
            "version": current_version,
            "stop": stop,
            "choices_used": choices_used,
            "pending_asset_count": pending_assets,
            "unsupported_direction_count": skipped_directions,
            "flags": flags.iter().take(64).collect::<std::collections::BTreeMap<_, _>>(),
            "flag_count": flags.len(),
            "trace": trace,
        }))
    }
}

fn frame(runtime: &Runtime, events: &[Event], expanded: &ExpandedWorkspace, entry: &str) -> Value {
    let state = runtime.state();
    let scene = &runtime.plan().scenes[state.scene].id;
    let instruction = runtime.current();
    let (file, span) = expanded.diagnostic_span(entry, instruction.source);
    let speaker = match &instruction.kind {
        InstructionKind::Speech { speaker, .. } => Some(speaker.name.value.as_str()),
        _ => None,
    };
    let content = events.iter().rev().find_map(|event| match event {
        Event::Text { text, kind, .. } if state.phase == Phase::Text => Some(json!({
            "kind": kind,
            "text": bounded_text(&text.value, 512),
            "speaker": speaker,
        })),
        Event::Choice { prompt, options, .. } if state.phase == Phase::Choice => Some(json!({
            "prompt": bounded_text(&prompt.value, 512),
            "options": options.iter().take(12).enumerate().map(|(index, option)| {
                json!({ "index": index, "label": bounded_text(&option.label.value, 128), "target": option.target })
            }).collect::<Vec<_>>(),
            "option_count": options.len(),
        })),
        _ => None,
    });
    json!({
        "scene": scene,
        "phase": state.phase,
        "file": file,
        "start": span.start,
        "end": span.end,
        "content": content,
    })
}
