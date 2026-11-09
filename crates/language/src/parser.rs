use crate::lexer::identifier;
use crate::*;

#[derive(Clone, Debug)]
pub struct ParsedDocument {
    pub source: Source,
    pub tokens: Vec<Token>,
    pub story: Story,
    pub diagnostics: Vec<Diagnostic>,
}

struct Line<'a> {
    source: &'a Source,
    tokens: Vec<&'a Token>,
    span: Span,
}
impl Line<'_> {
    fn raw(&self, i: usize) -> &str {
        self.tokens
            .get(i)
            .and_then(|t| self.source.slice(t.span))
            .unwrap_or("")
    }
    fn name(&self, i: usize) -> Result<Name, Diagnostic> {
        let value = self.raw(i);
        if !identifier(value) {
            return Err(self.error(
                "Expected a name (letter or underscore, then letters, digits or underscores)",
            ));
        }
        Ok(Name {
            value: value.into(),
            span: self.tokens[i].span,
        })
    }
    fn error(&self, message: &str) -> Diagnostic {
        Diagnostic::error("SF100", message, self.span)
    }
    fn count(&self, n: usize) -> Result<(), Diagnostic> {
        if self.tokens.len() == n {
            Ok(())
        } else {
            Err(self.error("Unexpected or missing tokens"))
        }
    }
    fn boolean(&self, index: usize) -> Result<bool, Diagnostic> {
        match self.raw(index) {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(self.error("Expected true or false")),
        }
    }
    fn text(&self, i: usize, quoted: bool) -> Result<Text, Diagnostic> {
        let Some(token) = self.tokens.get(i) else {
            return Err(self.error("Expected text"));
        };
        let raw = self.source.text()[token.span.start..self.span.end].trim_end();
        let span = Span::new(token.span.start, token.span.start + raw.len());
        if raw.starts_with('"') {
            if raw.len() < 2 || !raw.ends_with('"') {
                return Err(self.error("Unterminated quoted text"));
            }
            let mut value = String::new();
            let mut chars = raw[1..raw.len() - 1].chars();
            while let Some(c) = chars.next() {
                if c == '"' {
                    return Err(self.error("Unexpected text after a quoted string"));
                }
                if c != '\\' {
                    value.push(c);
                    continue;
                }
                match chars.next() {
                    Some('"') => value.push('"'),
                    Some('\\') => value.push('\\'),
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    _ => return Err(self.error("Supported escapes are quote, backslash, n and t")),
                }
            }
            Ok(Text { value, span })
        } else if quoted {
            Err(self.error("Expected quoted text"))
        } else {
            Ok(Text {
                value: raw.into(),
                span,
            })
        }
    }
    fn node(&self) -> Result<NodeKind, Diagnostic> {
        let head = self.raw(0);
        let speech = self.raw(1) == ":" || (self.raw(1) == "think" && self.raw(2) == ":");
        match head {
            "scene" => {
                self.count(2)?;
                Ok(NodeKind::Scene {
                    name: self.name(1)?,
                })
            }
            "beat" => {
                self.count(2)?;
                Ok(NodeKind::Beat {
                    name: self.name(1)?,
                })
            }
            "choice" => {
                self.count(2)?;
                Ok(NodeKind::Choice {
                    prompt: self.text(1, true)?,
                })
            }
            "option" => {
                self.count(4)?;
                if self.raw(2) != ":" {
                    return Err(self.error("Expected option \"label\": scene_name"));
                }
                // Read the label only, not the target following it.
                let label_token = self.tokens[1];
                let label_line = Line {
                    source: self.source,
                    tokens: vec![label_token],
                    span: label_token.span,
                };
                Ok(NodeKind::Option {
                    label: label_line.text(0, true)?,
                    target: self.name(3)?,
                })
            }
            "goto" => {
                self.count(2)?;
                Ok(NodeKind::Goto {
                    target: self.name(1)?,
                })
            }
            "set" if !speech => {
                self.count(3)?;
                Ok(NodeKind::SetFlag {
                    flag: self.name(1)?,
                    value: self.boolean(2)?,
                })
            }
            "if" if !speech => {
                let negated = self.tokens.len() == 7 && self.raw(1) == "not";
                let flag = if negated { 2 } else { 1 };
                if self.tokens.len() != flag + 5
                    || self.raw(flag + 1) != "goto"
                    || self.raw(flag + 3) != "else"
                {
                    return Err(self.error("Expected if [not] flag goto scene else scene"));
                }
                Ok(NodeKind::Conditional {
                    condition: Condition {
                        flag: self.name(flag)?,
                        negated,
                    },
                    when_true: self.name(flag + 2)?,
                    when_false: self.name(flag + 4)?,
                })
            }
            "end" => {
                self.count(1)?;
                Ok(NodeKind::End)
            }
            "stage" => {
                if !matches!(self.tokens.len(), 11 | 15) {
                    return Err(self.error(
                        "Expected stage: instance, character, group, scale, depth[, x, y]",
                    ));
                }
                if self.raw(1) != ":"
                    || [3, 5, 7, 9].iter().any(|index| self.raw(*index) != ",")
                    || (self.tokens.len() == 15 && (self.raw(11) != "," || self.raw(13) != ","))
                {
                    return Err(self.error(
                        "Expected stage: instance, character, left|center|right, scale, depth",
                    ));
                }
                let group = match self.raw(6) {
                    "left" => StageGroup::Left,
                    "center" => StageGroup::Center,
                    "right" => StageGroup::Right,
                    _ => return Err(self.error("Stage group must be left, center, or right")),
                };
                let scale = self
                    .raw(8)
                    .parse::<u16>()
                    .ok()
                    .filter(|value| (25..=300).contains(value))
                    .ok_or_else(|| {
                        self.error("Stage scale must be an integer from 25 to 300 percent")
                    })?;
                let depth = self
                    .raw(10)
                    .parse::<u16>()
                    .ok()
                    .filter(|value| *value <= 999)
                    .ok_or_else(|| self.error("Stage depth must be an integer from 0 to 999"))?;
                let position = if self.tokens.len() == 15 {
                    let coordinate = |index| {
                        self.raw(index)
                            .parse::<u8>()
                            .ok()
                            .filter(|value| *value <= 100)
                            .ok_or_else(|| {
                                self.error("Stage x and y must be integers from 0 to 100 percent")
                            })
                    };
                    Some((coordinate(12)?, coordinate(14)?))
                } else {
                    None
                };
                Ok(NodeKind::Stage {
                    instance: self.name(2)?,
                    asset: self.name(4)?,
                    group,
                    scale,
                    depth,
                    position,
                })
            }
            "swap" => {
                self.count(5)?;
                if self.raw(1) != ":" || self.raw(3) != "," {
                    return Err(self.error("Expected swap: instance, character"));
                }
                Ok(NodeKind::Swap {
                    instance: self.name(2)?,
                    asset: self.name(4)?,
                })
            }
            "hide" => {
                self.count(3)?;
                if self.raw(1) != ":" {
                    return Err(self.error("Expected hide: instance"));
                }
                Ok(NodeKind::Hide {
                    instance: self.name(2)?,
                })
            }
            "cast" | "character" | "location" | "prop" => {
                if self.raw(1) != ":" {
                    return Err(self.error("Expected ':' after asset slot"));
                }
                let mut names = Vec::new();
                for i in (2..self.tokens.len()).step_by(2) {
                    names.push(self.name(i)?);
                    if i + 1 < self.tokens.len() && self.raw(i + 1) != "," {
                        return Err(self.error("Expected ',' between asset names"));
                    }
                }
                if names.is_empty() || self.tokens.len() % 2 != 1 {
                    return Err(self.error("Expected an asset name after ':' or ','"));
                }
                if head != "cast" && names.len() != 1 {
                    return Err(self.error("This asset slot takes one name; cast takes a list"));
                }
                let kind = match head {
                    "location" => AssetKind::Location,
                    "prop" => AssetKind::Prop,
                    _ => AssetKind::Character,
                };
                Ok(NodeKind::Asset { kind, names })
            }
            "style" | "shot" | "mood" | "narrate" => {
                if self.raw(1) != ":" {
                    return Err(self.error("Expected ':' before text"));
                }
                let text = self.text(2, false)?;
                if head == "narrate" {
                    return Ok(NodeKind::Narration { text });
                }
                let kind = match head {
                    "style" => DirectionKind::Style,
                    "shot" => DirectionKind::Shot,
                    _ => DirectionKind::Mood,
                };
                Ok(NodeKind::Direction { kind, text })
            }
            _ => {
                let speaker = self.name(0)?;
                let thought = self.raw(1) == "think";
                if self.raw(if thought { 2 } else { 1 }) != ":" {
                    return Err(self.error("Expected a statement or speaker: text"));
                }
                Ok(NodeKind::Speech {
                    speaker,
                    thought,
                    text: self.text(if thought { 3 } else { 2 }, false)?,
                })
            }
        }
    }
}

fn children_at<'a>(nodes: &'a mut Vec<Node>, path: &[usize]) -> &'a mut Vec<Node> {
    if let Some((index, rest)) = path.split_first() {
        children_at(&mut nodes[*index].children, rest)
    } else {
        nodes
    }
}

fn close_scopes(nodes: &mut Vec<Node>, path: &[usize], depth: usize, end: usize) {
    for level in (depth..path.len()).rev() {
        let siblings = children_at(nodes, &path[..level]);
        siblings[path[level]].span.end = end;
    }
}

/// Parse incomplete documents and recover at the next line. Indentation is two spaces.
pub fn parse(text: &str) -> ParsedDocument {
    let source = Source::new(text);
    let (tokens, mut diagnostics) = lex(&source);
    let mut story = Story::default();
    let mut path = Vec::new();
    let mut parents: Vec<NodeKind> = Vec::new();
    let mut token_cursor = 0;
    for index in 0..source.line_count() {
        let span = source.line_span(index).unwrap();
        let raw = source.slice(span).unwrap();
        let start = token_cursor;
        while token_cursor < tokens.len() && tokens[token_cursor].span.start < span.end {
            token_cursor += 1;
        }
        let significant = tokens[start..token_cursor]
            .iter()
            .filter(|t| {
                !matches!(
                    t.kind,
                    TokenKind::Whitespace | TokenKind::Comment | TokenKind::Newline
                )
            })
            .collect::<Vec<_>>();
        if token_cursor < tokens.len() && tokens[token_cursor].kind == TokenKind::Newline {
            token_cursor += 1;
        }
        if significant.is_empty() {
            continue;
        }
        let indentation = raw.len() - raw.trim_start().len();
        let content = Span::new(span.start + indentation, span.end);
        let line = Line {
            source: &source,
            tokens: significant,
            span: content,
        };
        if !raw[..indentation].bytes().all(|byte| byte == b' ') || !indentation.is_multiple_of(2) {
            diagnostics.push(Diagnostic::error(
                "SF101",
                "Indent with two spaces per level; tabs are not allowed",
                Span::new(span.start, span.start + indentation),
            ));
            continue;
        }
        let depth = indentation / 2;
        close_scopes(&mut story.scenes, &path, depth, span.start);
        path.truncate(depth);
        parents.truncate(depth);
        if let Some(include) = parse_include(raw, span.start) {
            match include {
                Ok(path) => story.includes.push(path),
                Err(error) => diagnostics.push(error),
            }
            continue;
        }
        if line.raw(0) == "flag"
            && (depth == 0
                || !(line.raw(1) == ":" || (line.raw(1) == "think" && line.raw(2) == ":")))
        {
            let declaration = if depth != 0 {
                Err(line.error("Declare story flags at the top level"))
            } else {
                line.count(3).and_then(|_| {
                    Ok(Flag {
                        name: line.name(1)?,
                        initial: line.boolean(2)?,
                    })
                })
            };
            match declaration {
                Ok(flag) => story.flags.push(flag),
                Err(error) => diagnostics.push(error),
            }
            continue;
        }
        if line.raw(0) == "story" {
            if depth != 0 || story.title.is_some() || !story.scenes.is_empty() {
                diagnostics.push(
                    line.error("Story title must appear once, before scenes, at the top level"),
                );
            } else {
                match line.count(2).and_then(|_| line.text(1, true)) {
                    Ok(title) => story.title = Some(title),
                    Err(e) => diagnostics.push(e),
                }
            }
            continue;
        }
        let kind = match line.node() {
            Ok(kind) => kind,
            Err(e) => {
                diagnostics.push(e);
                continue;
            }
        };
        let valid = depth == parents.len()
            && match (&kind, parents.last()) {
                (NodeKind::Scene { .. }, None) => true,
                (NodeKind::Beat { .. }, Some(NodeKind::Scene { .. })) => true,
                (NodeKind::Option { .. }, Some(NodeKind::Choice { .. })) => true,
                (NodeKind::Scene { .. } | NodeKind::Beat { .. } | NodeKind::Option { .. }, _) => {
                    false
                }
                (_, Some(NodeKind::Scene { .. } | NodeKind::Beat { .. })) => true,
                _ => false,
            };
        if !valid {
            diagnostics.push(Diagnostic::error(
                "SF102",
                "Statement is outside its allowed scope or skips an indentation level",
                content,
            ));
            continue;
        }
        let opens = matches!(
            kind,
            NodeKind::Scene { .. } | NodeKind::Beat { .. } | NodeKind::Choice { .. }
        );
        let siblings = children_at(&mut story.scenes, &path);
        let at = siblings.len();
        siblings.push(Node {
            span: content,
            kind: kind.clone(),
            children: Vec::new(),
        });
        if opens {
            path.push(at);
            parents.push(kind);
        }
    }
    close_scopes(&mut story.scenes, &path, 0, source.text().len());
    ParsedDocument {
        source,
        tokens,
        story,
        diagnostics,
    }
}
