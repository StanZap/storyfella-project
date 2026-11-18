//! Whether a story compiles, counted the way the status bar and Home show it,
//! and each problem placed in the file that holds it.
use std::collections::BTreeMap;

use storyfella_compiler::{compile_workspace_with_catalog, SourceWorkspace, WorkspaceError};
use storyfella_language::{Diagnostic, Severity};
use storyfella_studio_core::{authoring::AssetCatalog, registry::ArtifactRegistry};

/// A compiler diagnostic in the file it belongs to; its span is a byte range
/// of that file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiagnostic {
    pub file: String,
    pub diagnostic: Diagnostic,
}

/// One compile of a story's files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub problems: Problems,
    pub diagnostics: Vec<FileDiagnostic>,
    /// Each file as it read when compiled, so a diagnostic is only placed in
    /// text that has not changed since.
    pub files: BTreeMap<String, String>,
}

impl Check {
    /// Compiles every file of `workspace` against the project's catalog.
    pub fn of(
        workspace: &SourceWorkspace,
        registry: &ArtifactRegistry,
    ) -> Result<Self, WorkspaceError> {
        let catalog = AssetCatalog::from_registry(registry);
        let (compilation, expanded) =
            compile_workspace_with_catalog(workspace, &catalog.assets, &catalog.bindings)?;
        let count = |severity| {
            compilation
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == severity)
                .count()
        };
        let problems = Problems {
            errors: count(Severity::Error),
            warnings: count(Severity::Warning),
        };
        let diagnostics = compilation
            .diagnostics
            .into_iter()
            .map(|mut diagnostic| {
                let (file, span) = expanded.diagnostic_span(&workspace.entry, diagnostic.span);
                diagnostic.span = span;
                FileDiagnostic {
                    file: file.to_owned(),
                    diagnostic,
                }
            })
            .collect();
        Ok(Self {
            problems,
            diagnostics,
            files: workspace.files.clone(),
        })
    }
}

/// A story's compiler diagnostics by severity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Problems {
    pub errors: usize,
    pub warnings: usize,
}

impl Problems {
    /// Compiles every file of `workspace` against the project's catalog.
    pub fn of(workspace: &SourceWorkspace, registry: &ArtifactRegistry) -> Result<Self, String> {
        Check::of(workspace, registry)
            .map(|check| check.problems)
            .map_err(|error| error.to_string())
    }

    /// "Compiled", or the counts that stop it reading so.
    pub fn summary(self) -> String {
        let plural = |count: usize, noun: &str| {
            format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
        };
        match (self.errors, self.warnings) {
            (0, 0) => "Compiled".into(),
            (0, warnings) => format!("Compiled · {}", plural(warnings, "warning")),
            (errors, 0) => plural(errors, "error"),
            (errors, warnings) => {
                format!(
                    "{} · {}",
                    plural(errors, "error"),
                    plural(warnings, "warning")
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(source: &str) -> SourceWorkspace {
        let directory = tempfile::tempdir().unwrap();
        let entry = directory.path().join("story.sf");
        std::fs::write(&entry, source).unwrap();
        storyfella_studio_core::story::load_source_workspace(&entry, &Default::default()).unwrap()
    }

    #[test]
    fn a_clean_story_compiles_and_a_broken_one_counts_its_errors() {
        let registry = ArtifactRegistry::default();
        let clean = Problems::of(
            &workspace("scene harbor\n  narrate: Dusk.\n  end\n"),
            &registry,
        );
        assert_eq!(clean.unwrap().summary(), "Compiled");
        let broken = Problems::of(&workspace("scene harbor\n  goto nowhere\n"), &registry).unwrap();
        assert!(broken.errors > 0, "{broken:?}");
        assert!(broken.summary().contains("error"));
    }
}
