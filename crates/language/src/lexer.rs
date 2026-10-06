use crate::{Diagnostic, Source, Span};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenKind {
    Keyword,
    Word,
    String,
    Text,
    Colon,
    Comma,
    Whitespace,
    Comment,
    Newline,
    Unknown,
}

pub const KEYWORDS: &[&str] = &[
    "story",
    "scene",
    "beat",
    "choice",
    "option",
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
    "think",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub(crate) fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Lossless, line-aware tokens. Prose tails are opaque: punctuation and # are speech.
pub fn lex(source: &Source) -> (Vec<Token>, Vec<Diagnostic>) {
    let text = source.text();
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();
    for line in 0..source.line_count() {
        let span = source.line_span(line).unwrap();
        let raw = &text[span.start..span.end];
        let head = raw.trim_start().split(':').next().unwrap_or("").trim();
        let prose = !matches!(head, "cast" | "character" | "location" | "prop")
            && !head.starts_with("option ")
            && !head.starts_with("scene ")
            && !head.starts_with("beat ")
            && !head.starts_with("choice ");
        let mut i = span.start;
        while i < span.end {
            let start = i;
            let c = text[i..].chars().next().unwrap();
            let kind = if c.is_whitespace() {
                i += c.len_utf8();
                while i < span.end && text[i..].chars().next().unwrap().is_whitespace() {
                    i += text[i..].chars().next().unwrap().len_utf8();
                }
                TokenKind::Whitespace
            } else if c == '#' && text[span.start..i].trim().is_empty() {
                i = span.end;
                TokenKind::Comment
            } else if c == ':' {
                i += 1;
                tokens.push(Token {
                    kind: TokenKind::Colon,
                    span: Span::new(start, i),
                });
                if prose && i < span.end {
                    let rest = &text[i..span.end];
                    let spaces = rest.len() - rest.trim_start().len();
                    if spaces > 0 {
                        tokens.push(Token {
                            kind: TokenKind::Whitespace,
                            span: Span::new(i, i + spaces),
                        });
                        i += spaces;
                    }
                    if i < span.end {
                        tokens.push(Token {
                            kind: TokenKind::Text,
                            span: Span::new(i, span.end),
                        });
                        i = span.end;
                    }
                }
                continue;
            } else if c == ',' {
                i += 1;
                TokenKind::Comma
            } else if c == '"' {
                i += 1;
                let mut closed = false;
                while i < span.end {
                    let next = text[i..].chars().next().unwrap();
                    i += next.len_utf8();
                    if next == '\\' && i < span.end {
                        i += text[i..].chars().next().unwrap().len_utf8();
                    } else if next == '"' {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    diagnostics.push(Diagnostic::error(
                        "SF001",
                        "Unterminated quoted text",
                        Span::new(start, i),
                    ));
                }
                TokenKind::String
            } else if c.is_ascii_alphanumeric() || c == '_' {
                i += 1;
                while i < span.end
                    && text.as_bytes()[i].is_ascii()
                    && (text.as_bytes()[i].is_ascii_alphanumeric() || text.as_bytes()[i] == b'_')
                {
                    i += 1;
                }
                if KEYWORDS.contains(&&text[start..i]) {
                    TokenKind::Keyword
                } else {
                    TokenKind::Word
                }
            } else {
                i += c.len_utf8();
                TokenKind::Unknown
            };
            tokens.push(Token {
                kind,
                span: Span::new(start, i),
            });
        }
        // Keep CRLF byte-for-byte, including a final standalone CR.
        let end = source
            .line_span(line + 1)
            .map_or(text.len(), |next| next.start);
        if span.end < end {
            tokens.push(Token {
                kind: TokenKind::Newline,
                span: Span::new(span.end, end),
            });
        }
    }
    for (i, _) in text.match_indices('\0') {
        diagnostics.push(Diagnostic::error(
            "SF002",
            "NUL is not allowed in story source",
            Span::new(i, i + 1),
        ));
    }
    (tokens, diagnostics)
}
