use crate::{Diagnostic, Span, Text};

fn argument(line: &str) -> Option<&str> {
    let suffix = line.strip_prefix("include")?;
    (suffix.is_empty() || suffix.starts_with([' ', '\t'])).then_some(suffix)
}

/// Whether a standalone document needs project resolution, even if its
/// include directive is incomplete.
pub fn has_includes(source: &str) -> bool {
    source.lines().any(|line| argument(line).is_some())
}

/// Parse an unindented include line. The returned span covers the path inside
/// the quotes so editors can change it while preserving whitespace and endings.
pub fn parse_include(line: &str, offset: usize) -> Option<Result<Text, Diagnostic>> {
    let suffix = argument(line)?;
    let quoted = suffix.trim();
    let path = quoted
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .filter(|value| valid_source_path(value));
    Some(match path {
        Some(path) => {
            let start = offset + "include".len() + suffix.len() - suffix.trim_start().len() + 1;
            Ok(Text {
                value: path.to_owned(),
                span: Span::new(start, start + path.len()),
            })
        }
        None => Err(Diagnostic::error(
            "SF308",
            "Expected include followed by a quoted relative .sf path",
            Span::new(offset, offset + line.len()),
        )),
    })
}

pub fn valid_source_path(path: &str) -> bool {
    path.ends_with(".sf")
        && !path.contains(['\\', '"', '\0', '\n', '\r'])
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}
