use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Scene,
    Beat,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: Name,
    pub kind: SymbolKind,
    /// Beats are local to their scene; scenes are document-wide.
    pub scene: Option<String>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceKind {
    Scene,
    Asset(AssetKind),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub name: Name,
    pub kind: ReferenceKind,
    pub scene: String,
    pub beat: Option<String>,
    pub definition: Option<Span>,
    pub asset_id: Option<String>,
}

/// The host supplies stable asset identities. Analysis never loads Studio or a DB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub kind: AssetKind,
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub document: ParsedDocument,
    pub symbols: Vec<Symbol>,
    pub references: Vec<Reference>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn analyze(source: &str, assets: &[Asset]) -> Analysis {
    let document = parse(source);
    let mut symbols = Vec::new();
    let mut references = Vec::new();
    let mut diagnostics = document.diagnostics.clone();
    fn visit(
        nodes: &[Node],
        scene: &str,
        beat: Option<&str>,
        symbols: &mut Vec<Symbol>,
        references: &mut Vec<Reference>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for node in nodes {
            let mut scene = scene;
            let mut beat = beat;
            match &node.kind {
                NodeKind::Scene { name } | NodeKind::Beat { name } => {
                    let is_scene = matches!(node.kind, NodeKind::Scene { .. });
                    let symbol = Symbol {
                        name: name.clone(),
                        kind: if is_scene {
                            SymbolKind::Scene
                        } else {
                            SymbolKind::Beat
                        },
                        scene: if is_scene { None } else { Some(scene.into()) },
                        span: node.span,
                    };
                    if symbols.iter().any(|s| {
                        s.kind == symbol.kind
                            && s.scene == symbol.scene
                            && s.name.value == name.value
                    }) {
                        diagnostics.push(Diagnostic::error(
                            "SF200",
                            format!(
                                "Duplicate {} '{}'",
                                if is_scene { "scene" } else { "beat" },
                                name.value
                            ),
                            name.span,
                        ));
                    }
                    symbols.push(symbol);
                    if is_scene {
                        scene = &name.value;
                        beat = None;
                    } else {
                        beat = Some(&name.value);
                    }
                }
                NodeKind::Option { target, .. } | NodeKind::Goto { target } => {
                    references.push(Reference {
                        name: target.clone(),
                        kind: ReferenceKind::Scene,
                        scene: scene.into(),
                        beat: beat.map(str::to_owned),
                        definition: None,
                        asset_id: None,
                    })
                }
                NodeKind::Asset { kind, names } => {
                    for name in names {
                        references.push(Reference {
                            name: name.clone(),
                            kind: ReferenceKind::Asset(*kind),
                            scene: scene.into(),
                            beat: beat.map(str::to_owned),
                            definition: None,
                            asset_id: None,
                        });
                    }
                }
                NodeKind::Speech { speaker, .. } => references.push(Reference {
                    name: speaker.clone(),
                    kind: ReferenceKind::Asset(AssetKind::Character),
                    scene: scene.into(),
                    beat: beat.map(str::to_owned),
                    definition: None,
                    asset_id: None,
                }),
                NodeKind::Choice { .. } if node.children.len() < 2 => diagnostics.push(
                    Diagnostic::error("SF201", "A choice requires at least two options", node.span),
                ),
                _ => {}
            }
            visit(
                &node.children,
                scene,
                beat,
                symbols,
                references,
                diagnostics,
            );
        }
    }
    visit(
        &document.story.scenes,
        "",
        None,
        &mut symbols,
        &mut references,
        &mut diagnostics,
    );
    for reference in &mut references {
        match reference.kind {
            ReferenceKind::Scene => {
                let matches: Vec<_> = symbols
                    .iter()
                    .filter(|s| s.kind == SymbolKind::Scene && s.name.value == reference.name.value)
                    .collect();
                if matches.len() == 1 {
                    reference.definition = Some(matches[0].name.span);
                } else {
                    diagnostics.push(Diagnostic::error(
                        "SF202",
                        format!(
                            "{} scene '{}'",
                            if matches.is_empty() {
                                "Unknown"
                            } else {
                                "Ambiguous"
                            },
                            reference.name.value
                        ),
                        reference.name.span,
                    ));
                }
            }
            ReferenceKind::Asset(kind) => {
                let matches: Vec<_> = assets
                    .iter()
                    .filter(|a| a.name.eq_ignore_ascii_case(&reference.name.value))
                    .collect();
                match matches.as_slice() {
                    [] => diagnostics.push(Diagnostic {
                        code: "SF203".into(),
                        severity: Severity::Information,
                        message: format!("Pending {:?} asset '{}'", kind, reference.name.value),
                        span: reference.name.span,
                    }),
                    [asset] if asset.kind == kind => reference.asset_id = Some(asset.id.clone()),
                    [_] => diagnostics.push(Diagnostic::error(
                        "SF204",
                        format!(
                            "Asset '{}' has the wrong kind; expected {:?}",
                            reference.name.value, kind
                        ),
                        reference.name.span,
                    )),
                    _ => diagnostics.push(Diagnostic::error(
                        "SF205",
                        format!("Ambiguous asset name '{}'", reference.name.value),
                        reference.name.span,
                    )),
                }
            }
        }
    }
    diagnostics.sort_by_key(|d| (d.span.start, d.span.end));
    Analysis {
        document,
        symbols,
        references,
        diagnostics,
    }
}

impl Analysis {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
    pub fn definition_at(&self, offset: usize) -> Option<Span> {
        if let Some(reference) = self
            .references
            .iter()
            .find(|r| r.name.span.contains(offset))
        {
            return reference.definition;
        }
        self.symbols
            .iter()
            .find(|s| s.name.span.contains(offset))
            .map(|s| s.name.span)
    }
    pub fn references_at(&self, offset: usize, include_declaration: bool) -> Vec<Span> {
        let Some(definition) = self.definition_at(offset) else {
            return Vec::new();
        };
        let mut spans: Vec<_> = self
            .references
            .iter()
            .filter(|r| r.definition == Some(definition))
            .map(|r| r.name.span)
            .collect();
        if include_declaration {
            spans.push(definition);
        }
        spans.sort_by_key(|s| s.start);
        spans
    }
    pub fn scope_at(&self, offset: usize) -> (Option<&str>, Option<&str>) {
        let scene = self
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Scene && s.span.contains(offset));
        let beat = self
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Beat && s.span.contains(offset));
        (
            scene.map(|s| s.name.value.as_str()),
            beat.map(|s| s.name.value.as_str()),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Scene,
    Asset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub replace: Span,
    pub kind: CompletionKind,
}

/// Contextual completion uses shared symbols/catalog and returns source-byte edits.
pub fn complete(analysis: &Analysis, offset: usize, assets: &[Asset]) -> Vec<Completion> {
    let source = &analysis.document.source;
    let Some(position) = source.position(offset) else {
        return Vec::new();
    };
    let line = source.line_span(position.line as usize).unwrap();
    if offset > line.end {
        return Vec::new();
    }
    let prefix = &source.text()[line.start..offset];
    let trimmed = prefix.trim_start();
    if trimmed.starts_with('#') {
        return Vec::new();
    }
    if analysis.document.tokens.iter().any(|token| {
        matches!(token.kind, TokenKind::String | TokenKind::Text)
            && token.span.start < offset
            && offset <= token.span.end
    }) {
        return Vec::new();
    }
    let start = prefix
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
        .last()
        .map_or(offset, |(i, _)| line.start + i);
    let fragment = &source.text()[start..offset];
    let before = source.text()[line.start..start].trim();
    let end = offset
        + source.text()[offset..line.end]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .map(char::len_utf8)
            .sum::<usize>();
    let replace = Span::new(start, end);
    let mut result = Vec::new();
    let option_target = before.starts_with("option ")
        && analysis.document.tokens.iter().any(|token| {
            token.kind == TokenKind::Colon
                && token.span.start >= line.start
                && token.span.end <= start
        });
    if before == "goto" || option_target {
        for symbol in &analysis.symbols {
            if symbol.kind == SymbolKind::Scene && symbol.name.value.starts_with(fragment) {
                result.push(Completion {
                    label: symbol.name.value.clone(),
                    replace,
                    kind: CompletionKind::Scene,
                });
            }
        }
    } else {
        let asset_kind = before.split_once(':').and_then(|(slot, _)| match slot {
            "cast" | "character" => Some(AssetKind::Character),
            "location" => Some(AssetKind::Location),
            "prop" => Some(AssetKind::Prop),
            _ => None,
        });
        if let Some(kind) = asset_kind {
            for asset in assets {
                if asset.kind == kind
                    && crate::lexer::identifier(&asset.name)
                    && asset
                        .name
                        .to_ascii_lowercase()
                        .starts_with(&fragment.to_ascii_lowercase())
                {
                    result.push(Completion {
                        label: asset.name.clone(),
                        replace,
                        kind: CompletionKind::Asset,
                    });
                }
            }
        } else if before.is_empty() {
            let indentation = prefix.len() - prefix.trim_start().len();
            fn container(
                nodes: &[Node],
                depth: usize,
                offset: usize,
                eof: usize,
            ) -> Option<&NodeKind> {
                let node = nodes
                    .iter()
                    .find(|n| n.span.contains(offset) || (offset == eof && n.span.end == eof))?;
                if depth == 0 {
                    Some(&node.kind)
                } else {
                    container(&node.children, depth - 1, offset, eof)
                }
            }
            let parent = if indentation >= 2 && indentation.is_multiple_of(2) {
                container(
                    &analysis.document.story.scenes,
                    indentation / 2 - 1,
                    offset,
                    source.text().len(),
                )
            } else {
                None
            };
            let statements: &[&str] = &[
                "choice",
                "goto",
                "end",
                "cast",
                "character",
                "location",
                "prop",
                "style",
                "shot",
                "mood",
                "narrate",
            ];
            let keywords: &[&str] = if indentation == 0 {
                &["story", "scene"]
            } else {
                match parent {
                    Some(NodeKind::Choice { .. }) => &["option"],
                    Some(NodeKind::Scene { .. } | NodeKind::Beat { .. }) => statements,
                    _ => &[],
                }
            };
            if matches!(parent, Some(NodeKind::Scene { .. })) && "beat".starts_with(fragment) {
                result.push(Completion {
                    label: "beat".into(),
                    replace,
                    kind: CompletionKind::Keyword,
                });
            }
            for keyword in keywords {
                if keyword.starts_with(fragment) {
                    result.push(Completion {
                        label: (*keyword).into(),
                        replace,
                        kind: CompletionKind::Keyword,
                    });
                }
            }
            if matches!(parent, Some(NodeKind::Scene { .. } | NodeKind::Beat { .. })) {
                for asset in assets {
                    if asset.kind == AssetKind::Character
                        && crate::lexer::identifier(&asset.name)
                        && asset
                            .name
                            .to_ascii_lowercase()
                            .starts_with(&fragment.to_ascii_lowercase())
                    {
                        result.push(Completion {
                            label: asset.name.clone(),
                            replace,
                            kind: CompletionKind::Asset,
                        });
                    }
                }
            }
        }
    }
    result.sort_by(|a, b| a.label.cmp(&b.label));
    result.dedup_by(|a, b| a.label == b.label);
    result
}
