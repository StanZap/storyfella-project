use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde::Serialize;
use std::path::PathBuf;
use storyfella_compiler::{
    compile_analysis, Compilation, ExpandedWorkspace, Readiness, SourceWorkspace,
};
use storyfella_language::{Diagnostic, Position, Source};
use storyfella_studio_core::authoring::{AssetCatalog, CatalogEntry};

#[derive(Args)]
pub struct SourceArgs {
    /// UTF-8 .sf source file.
    path: PathBuf,
    /// JSON catalog array {id, name, kind, revision?}; omitted revision is pending artwork.
    #[arg(long)]
    assets: Option<PathBuf>,
    /// Fail unless source is valid, every asset is bound and directions are supported.
    #[arg(long)]
    require_ready: bool,
}

#[derive(Subcommand)]
pub enum StoryCommand {
    /// Execute a compiled JSON plan with a JSON array of runtime commands.
    Trace { plan: PathBuf, commands: PathBuf },
    /// Export a project's read-only asset context for external editors.
    Catalog { project: PathBuf },
    /// Print source validity, linking readiness and positioned diagnostics as JSON.
    Check(SourceArgs),
    /// Print the versioned story plan as JSON. Blocked requirements remain explicit.
    Compile(SourceArgs),
}

#[derive(Serialize)]
struct PositionedDiagnostic {
    #[serde(flatten)]
    diagnostic: Diagnostic,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    start: Position,
    end: Position,
}

#[derive(Serialize)]
struct CheckReport<'a> {
    valid: bool,
    ready: bool,
    readiness: Option<&'a Readiness>,
    diagnostics: Vec<PositionedDiagnostic>,
}

fn check_report<'a>(
    result: &'a Compilation,
    source: &Source,
    workspace: Option<(&SourceWorkspace, &ExpandedWorkspace)>,
) -> CheckReport<'a> {
    CheckReport {
        valid: result.plan.is_some() && !result.has_errors(),
        ready: result.ready(),
        readiness: result.plan.as_ref().map(|p| &p.readiness),
        diagnostics: result
            .diagnostics
            .iter()
            .map(|diagnostic| {
                let mut diagnostic = diagnostic.clone();
                if let Some((snapshot, expanded)) = workspace {
                    if let Some((path, span)) = expanded.original_span(diagnostic.span) {
                        let original = Source::new(&snapshot.files[path]);
                        diagnostic.span = span;
                        return PositionedDiagnostic {
                            start: original.position(span.start).unwrap(),
                            end: original.position(span.end).unwrap(),
                            file: Some(path.to_owned()),
                            diagnostic,
                        };
                    }
                }
                PositionedDiagnostic {
                    start: source.position(diagnostic.span.start).unwrap(),
                    end: source.position(diagnostic.span.end).unwrap(),
                    diagnostic,
                    file: None,
                }
            })
            .collect(),
    }
}

pub fn run(command: StoryCommand) -> Result<()> {
    let (args, emit_plan) = match command {
        StoryCommand::Trace { plan, commands } => {
            let plan = serde_json::from_slice(&std::fs::read(plan)?)?;
            let commands: Vec<storyfella_runtime::Command> =
                serde_json::from_slice(&std::fs::read(commands)?)?;
            let (mut runtime, mut events) = storyfella_runtime::Runtime::start(plan)?;
            for command in commands {
                events.extend(runtime.dispatch(command)?);
            }
            println!("{}", serde_json::to_string_pretty(&events)?);
            return Ok(());
        }
        StoryCommand::Check(args) => (args, false),
        StoryCommand::Compile(args) => (args, true),
        StoryCommand::Catalog { project } => {
            if !project.is_file() {
                bail!("project database does not exist: {}", project.display());
            }
            let stored = storyfella_studio_core::persistence::ProjectDb::open(project)?.load()?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &AssetCatalog::from_registry(&stored.registry).entries()
                )?
            );
            return Ok(());
        }
    };
    let text = std::fs::read_to_string(&args.path)
        .with_context(|| format!("could not read {}", args.path.display()))?;
    let entries: Vec<CatalogEntry> = match &args.assets {
        Some(path) => serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?,
        )
        .with_context(|| format!("invalid asset binding array in {}", path.display()))?,
        None => Vec::new(),
    };
    let catalog = AssetCatalog::from_entries(entries).map_err(anyhow::Error::msg)?;
    let multi_file = text.lines().any(|line| line.starts_with("include "));
    let workspace = if multi_file {
        let canonical_entry = args.path.canonicalize()?;
        let entry = canonical_entry
            .file_name()
            .and_then(|name| name.to_str())
            .context("story filename is not UTF-8")?;
        let drafts = std::collections::BTreeMap::from([(entry.to_owned(), text.clone())]);
        Some(
            storyfella_studio_core::story::load_source_workspace(&args.path, &drafts)
                .map_err(anyhow::Error::msg)?,
        )
    } else {
        None
    };
    let expanded = workspace
        .as_ref()
        .map(|workspace| {
            workspace.expand().map_err(|error| {
                anyhow::anyhow!("{}:{}: {}", error.path, error.span.start, error.message)
            })
        })
        .transpose()?;
    let source_text = expanded
        .as_ref()
        .map_or(text.as_str(), |expanded| expanded.source.as_str());
    let analysis = storyfella_language::analyze(source_text, &catalog.assets);
    let result = compile_analysis(&analysis, &catalog.bindings);
    if !emit_plan || result.has_errors() || (args.require_ready && !result.ready()) {
        println!(
            "{}",
            serde_json::to_string_pretty(&check_report(
                &result,
                &Source::new(source_text),
                workspace.as_ref().zip(expanded.as_ref())
            ))?
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
        let result = storyfella_compiler::compile(text, &[]);
        let source = Source::new(text);
        let report = check_report(&result, &source, None);
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
