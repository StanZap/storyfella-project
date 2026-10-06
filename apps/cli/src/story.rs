use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde::Serialize;
use std::path::PathBuf;
use storyfella_compiler::{compile, AssetBinding, Compilation, Readiness};
use storyfella_language::{Diagnostic, Position, Source};

#[derive(Args)]
pub struct SourceArgs {
    /// UTF-8 .sf source file.
    path: PathBuf,
    /// JSON array of immutable {id, name, kind, revision} asset bindings.
    #[arg(long)]
    assets: Option<PathBuf>,
    /// Fail unless source is valid, every asset is bound and directions are supported.
    #[arg(long)]
    require_ready: bool,
}

#[derive(Subcommand)]
pub enum StoryCommand {
    /// Print source validity, linking readiness and positioned diagnostics as JSON.
    Check(SourceArgs),
    /// Print the versioned story plan as JSON. Blocked requirements remain explicit.
    Compile(SourceArgs),
}

#[derive(Serialize)]
struct PositionedDiagnostic<'a> {
    #[serde(flatten)]
    diagnostic: &'a Diagnostic,
    start: Position,
    end: Position,
}

#[derive(Serialize)]
struct CheckReport<'a> {
    valid: bool,
    ready: bool,
    readiness: Option<&'a Readiness>,
    diagnostics: Vec<PositionedDiagnostic<'a>>,
}

fn check_report<'a>(result: &'a Compilation, source: &Source) -> CheckReport<'a> {
    CheckReport {
        valid: result.plan.is_some() && !result.has_errors(),
        ready: result.ready(),
        readiness: result.plan.as_ref().map(|p| &p.readiness),
        diagnostics: result
            .diagnostics
            .iter()
            .map(|diagnostic| PositionedDiagnostic {
                diagnostic,
                start: source.position(diagnostic.span.start).unwrap(),
                end: source.position(diagnostic.span.end).unwrap(),
            })
            .collect(),
    }
}

pub fn run(command: StoryCommand) -> Result<()> {
    let (args, emit_plan) = match command {
        StoryCommand::Check(args) => (args, false),
        StoryCommand::Compile(args) => (args, true),
    };
    let text = std::fs::read_to_string(&args.path)
        .with_context(|| format!("could not read {}", args.path.display()))?;
    let bindings: Vec<AssetBinding> = match &args.assets {
        Some(path) => serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?,
        )
        .with_context(|| format!("invalid asset binding array in {}", path.display()))?,
        None => Vec::new(),
    };
    let result = compile(&text, &bindings);
    if !emit_plan || result.has_errors() || (args.require_ready && !result.ready()) {
        println!(
            "{}",
            serde_json::to_string_pretty(&check_report(&result, &Source::new(text)))?
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(result.plan.as_ref().unwrap())?
        );
    }
    if result.has_errors() {
        bail!("story compilation failed; see source diagnostics");
    }
    if args.require_ready && !result.ready() {
        bail!("story is blocked by pending assets or unsupported directions");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positioned_diagnostics_use_utf16_and_keep_original_spans() {
        let text =
            "scene start\n  choice \"🌊\"\n    option \"🌊\": unknown\n    option \"🌊\": start\n";
        let result = compile(text, &[]);
        let source = Source::new(text);
        let report = check_report(&result, &source);
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|d| d.diagnostic.code == "SF202")
            .unwrap();
        assert_eq!(
            diagnostic.start,
            Position {
                line: 2,
                character: 17
            }
        );
        assert_eq!(source.slice(diagnostic.diagnostic.span), Some("unknown"));
        assert!(!report.valid && !report.ready);
    }
}
