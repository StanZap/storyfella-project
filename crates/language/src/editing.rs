use crate::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub replace: Span,
    pub new_text: String,
}

pub fn apply_edits(source: &str, edits: &[TextEdit]) -> Result<String, String> {
    let mut edits: Vec<_> = edits.iter().collect();
    edits.sort_by_key(|edit| (edit.replace.start, edit.replace.end));
    let mut end = 0;
    for edit in &edits {
        if edit.replace.start < end || source.get(edit.replace.start..edit.replace.end).is_none() {
            return Err("Edit ranges overlap or split a UTF-8 character".into());
        }
        end = edit.replace.end;
    }
    let mut result = source.to_owned();
    for edit in edits.into_iter().rev() {
        result.replace_range(edit.replace.start..edit.replace.end, &edit.new_text);
    }
    Ok(result)
}

/// Textareas normalize CRLF. Retain original endings outside the user's edit
/// and use the document's existing ending for newly inserted line breaks.
pub fn textarea_input(original: &str, input: &str) -> String {
    let normalized = original.replace("\r\n", "\n");
    let prefix: usize = normalized
        .chars()
        .zip(input.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = normalized[prefix..]
        .chars()
        .rev()
        .zip(input[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let offset = |target: usize| {
        let mut n = 0;
        let mut byte = 0;
        while n < target {
            if original[byte..].starts_with("\r\n") {
                byte += 2;
                n += 1;
            } else {
                let size = original[byte..].chars().next().unwrap().len_utf8();
                byte += size;
                n += size;
            }
        }
        byte
    };
    let start = offset(prefix);
    let end = offset(normalized.len() - suffix);
    let replacement = if original.contains("\r\n") {
        input[prefix..input.len() - suffix].replace('\n', "\r\n")
    } else {
        input[prefix..input.len() - suffix].into()
    };
    format!("{}{}{}", &original[..start], replacement, &original[end..])
}

/// Rename only resolved scene/beat declarations and their resolved targets.
/// Asset identity lives in the host catalog and needs host persistence first.
pub fn rename(analysis: &Analysis, offset: usize, name: &str) -> Result<Vec<TextEdit>, String> {
    if !crate::lexer::identifier(name) {
        return Err("Use a letter/underscore followed by letters, digits or underscores".into());
    }
    if analysis
        .references
        .iter()
        .any(|r| r.name.span.contains(offset) && matches!(r.kind, ReferenceKind::Asset(_)))
    {
        return Err("Asset rename requires a host catalog persistence contract".into());
    }
    let definition = analysis
        .definition_at(offset)
        .ok_or("Select a resolved scene or beat name")?;
    let symbol = analysis
        .symbols
        .iter()
        .find(|s| s.name.span == definition)
        .ok_or("No declaration for this target")?;
    if analysis.symbols.iter().any(|s| {
        s.name.span != definition
            && s.kind == symbol.kind
            && s.scene == symbol.scene
            && (s.name.value == name || s.name.value == symbol.name.value)
    }) {
        return Err("Rename would collide with another declaration".into());
    }
    Ok(analysis
        .references_at(offset, true)
        .into_iter()
        .map(|replace| TextEdit {
            replace,
            new_text: name.into(),
        })
        .collect())
}

/// Canonicalize only whitespace gaps between structural tokens. Never change
/// indentation, line endings, comments, quoted text or an opaque prose tail.
pub fn format(analysis: &Analysis) -> Result<Vec<TextEdit>, String> {
    if analysis
        .document
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        return Err("Formatting requires syntax-valid source; this draft is preserved".into());
    }
    let mut edits = Vec::new();
    for line in 0..analysis.document.source.line_count() {
        let span = analysis.document.source.line_span(line).unwrap();
        let tokens: Vec<_> = analysis
            .document
            .tokens
            .iter()
            .filter(|t| {
                t.span.start >= span.start
                    && t.span.end <= span.end
                    && !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)
            })
            .collect();
        if tokens.first().is_some_and(|t| t.kind == TokenKind::Comment) {
            continue;
        }
        for pair in tokens.windows(2) {
            let gap = Span::new(pair[0].span.end, pair[1].span.start);
            let expected = if matches!(pair[1].kind, TokenKind::Colon | TokenKind::Comma) {
                ""
            } else {
                " "
            };
            if analysis.document.source.slice(gap) != Some(expected) {
                edits.push(TextEdit {
                    replace: gap,
                    new_text: expected.into(),
                });
            }
        }
    }
    Ok(edits)
}

/// Legend shared with the LSP transport and usable by other editors.
pub const SEMANTIC_TOKEN_TYPES: &[&str] = &["keyword", "string", "comment", "variable", "function"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticToken {
    pub span: Span,
    pub kind: u32,
}

pub fn semantic_tokens(analysis: &Analysis) -> Vec<SemanticToken> {
    analysis
        .document
        .tokens
        .iter()
        .filter_map(|token| {
            let kind = match token.kind {
                TokenKind::Keyword => 0,
                TokenKind::String | TokenKind::Text => 1,
                TokenKind::Comment => 2,
                TokenKind::Word
                    if analysis.symbols.iter().any(|s| s.name.span == token.span)
                        || analysis.references.iter().any(|r| {
                            r.name.span == token.span && r.kind == ReferenceKind::Scene
                        }) =>
                {
                    4
                }
                TokenKind::Word
                    if analysis
                        .references
                        .iter()
                        .any(|r| r.name.span == token.span) =>
                {
                    3
                }
                _ => return None,
            };
            Some(SemanticToken {
                span: token.span,
                kind,
            })
        })
        .collect()
}
