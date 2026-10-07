use serde::{Deserialize, Serialize};

/// Half-open UTF-8 byte range in the original document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    pub fn contains(self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }
}

/// Zero-based line and UTF-16 code-unit column, matching LSP's default encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Debug)]
pub struct Source {
    text: String,
    starts: Vec<usize>,
}
impl Source {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self { text, starts }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }
    pub fn line_span(&self, line: usize) -> Option<Span> {
        let start = *self.starts.get(line)?;
        let end = self.starts.get(line + 1).map_or(self.text.len(), |n| n - 1);
        let end = if self.text[start..end].ends_with('\r') {
            end - 1
        } else {
            end
        };
        Some(Span::new(start, end))
    }
    pub fn position(&self, offset: usize) -> Option<Position> {
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return None;
        }
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        let end = offset.min(self.line_span(line)?.end);
        Some(Position {
            line: line as u32,
            character: self.text[self.starts[line]..end].encode_utf16().count() as u32,
        })
    }
    /// Columns beyond the end of a line clamp there; split surrogate pairs fail.
    pub fn offset(&self, position: Position) -> Option<usize> {
        let span = self.line_span(position.line as usize)?;
        let mut column = 0;
        for (i, c) in self.text[span.start..span.end].char_indices() {
            if column == position.character {
                return Some(span.start + i);
            }
            column += c.len_utf16() as u32;
            if column > position.character {
                return None;
            }
        }
        Some(span.end)
    }
    pub fn slice(&self, span: Span) -> Option<&str> {
        self.text.get(span.start..span.end)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Information,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub span: Span,
}
impl Diagnostic {
    pub(crate) fn error(code: &str, message: impl Into<String>, span: Span) -> Self {
        Self {
            code: code.into(),
            severity: Severity::Error,
            message: message.into(),
            span,
        }
    }
}

pub fn normalize_diagnostics(diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.sort_by(|a, b| {
        (a.span.start, a.span.end, &a.code, &a.message).cmp(&(
            b.span.start,
            b.span.end,
            &b.code,
            &b.message,
        ))
    });
    diagnostics.dedup();
}
