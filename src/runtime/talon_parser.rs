use crate::mode::RuntimeMode;
use crate::runtime::talon_ast::{
    SpokenRule, SpokenToken, TalonBodyStmt, TalonCommand, TalonContextHeader, TalonFile,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalonParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for TalonParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for TalonParseError {}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Section {
    Header,
    Body,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum BodyBlock {
    None,
    Settings,
    Tag,
    Command,
    SkippedTrigger,
}

pub fn parse_talon_file(source: &str) -> Result<TalonFile, TalonParseError> {
    let mut ctx = TalonContextHeader::default();
    let mut commands: Vec<TalonCommand> = Vec::new();
    let mut activated_tags: Vec<String> = Vec::new();
    let mut settings: Vec<(String, String)> = Vec::new();
    let mut diagnostics: Vec<String> = Vec::new();

    // Headerless `.talon` files omit the `-` separator and start directly in the body.
    let has_separator = source.lines().any(|raw| {
        let trimmed = raw.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed == "-"
    });

    let mut section = if has_separator {
        Section::Header
    } else {
        Section::Body
    };
    let mut body_block = BodyBlock::None;

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        match section {
            Section::Header => {
                if trimmed == "-" {
                    section = Section::Body;
                    continue;
                }
                let clean = strip_inline_comment(trimmed);
                if !clean.is_empty() {
                    parse_header_line(&mut ctx, clean, line_no)?;
                }
            }
            Section::Body => {
                let is_indented = raw_line.starts_with(' ') || raw_line.starts_with('\t');
                if !is_indented {
                    let clean = strip_inline_comment(trimmed);
                    if clean.is_empty() {
                        continue;
                    }

                    if clean == "settings():" {
                        body_block = BodyBlock::Settings;
                        continue;
                    }

                    if clean.starts_with("tag():") {
                        let TalonBodyStmt::TagActivate { tag } =
                            parse_body_statement(clean, line_no)?
                        else {
                            unreachable!("tag(): prefix always parses as TagActivate");
                        };
                        activated_tags.push(tag);
                        body_block = BodyBlock::Tag;
                        continue;
                    }

                    let Some((lhs, rhs)) = split_top_level_colon(clean) else {
                        return Err(TalonParseError {
                            line: line_no,
                            message: "unsupported talon syntax (expected `spoken: ...`)".into(),
                        });
                    };

                    let spoken = lhs.trim();
                    if spoken.is_empty() {
                        return Err(TalonParseError {
                            line: line_no,
                            message: "empty spoken form".into(),
                        });
                    }

                    if is_skipped_trigger(spoken) {
                        diagnostics.push(format!(
                            "line {}: skipped out-of-scope trigger `{}`",
                            line_no, spoken
                        ));
                        body_block = BodyBlock::SkippedTrigger;
                        continue;
                    }

                    let rule = parse_spoken_rule(spoken).map_err(|mut err| {
                        err.line = line_no;
                        err
                    })?;

                    let mut body: Vec<TalonBodyStmt> = Vec::new();
                    let after_colon = rhs.trim();
                    if !after_colon.is_empty() {
                        if after_colon == "settings():" {
                            return Err(TalonParseError {
                                line: line_no,
                                message: "unsupported construct: settings()".into(),
                            });
                        }
                        body.push(parse_body_statement(after_colon, line_no)?);
                    }

                    commands.push(TalonCommand {
                        spoken_form: spoken.to_string(),
                        rule,
                        body,
                        context: ctx.clone(),
                        source_path: None,
                        line: line_no,
                    });
                    body_block = BodyBlock::Command;
                } else {
                    let stmt_src = strip_inline_comment(trimmed);
                    if stmt_src.is_empty() {
                        continue;
                    }

                    match body_block {
                        BodyBlock::SkippedTrigger => continue,
                        BodyBlock::Settings => {
                            let Some((key, value)) = stmt_src.split_once('=') else {
                                return Err(TalonParseError {
                                    line: line_no,
                                    message: "expected `setting = value` inside settings() block"
                                        .into(),
                                });
                            };
                            let key = key.trim();
                            let value = value.trim();
                            if key.is_empty() || value.is_empty() {
                                return Err(TalonParseError {
                                    line: line_no,
                                    message: "empty setting key or value in settings() block".into(),
                                });
                            }
                            settings.push((key.to_string(), value.to_string()));
                        }
                        BodyBlock::Tag => {
                            return Err(TalonParseError {
                                line: line_no,
                                message: "indented statement after top-level tag() is unsupported"
                                    .into(),
                            });
                        }
                        BodyBlock::None => {
                            return Err(TalonParseError {
                                line: line_no,
                                message: "indented statement without a preceding command".into(),
                            });
                        }
                        BodyBlock::Command => {
                            if stmt_src == "settings():" {
                                return Err(TalonParseError {
                                    line: line_no,
                                    message: "unsupported construct: settings()".into(),
                                });
                            }
                            let stmt = parse_body_statement(stmt_src, line_no)?;
                            let last = commands
                                .last_mut()
                                .expect("BodyBlock::Command guarantees a preceding command");
                            last.body.push(stmt);
                        }
                    }
                }
            }
        }
    }

    Ok(TalonFile {
        context: ctx,
        commands,
        activated_tags,
        settings,
        diagnostics,
    })
}

/// Parses a Talon `ctx.matches` string (header lines without `-` separator).
pub fn parse_context_matches(source: &str) -> Result<TalonContextHeader, TalonParseError> {
    let mut ctx = TalonContextHeader::default();
    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let clean = strip_inline_comment(trimmed);
        if clean.is_empty() || clean == "-" {
            continue;
        }
        parse_header_line(&mut ctx, clean, line_no)?;
    }
    Ok(ctx)
}

fn parse_header_line(
    ctx: &mut TalonContextHeader,
    line: &str,
    line_no: usize,
) -> Result<(), TalonParseError> {
    let Some((key, value)) = line.split_once(':') else {
        return Err(TalonParseError {
            line: line_no,
            message: "unsupported talon syntax in header".into(),
        });
    };
    let raw_key = key.trim();
    let key = raw_key
        .strip_prefix("and ")
        .map(str::trim_start)
        .unwrap_or(raw_key);
    let value = value.trim();
    if value.is_empty() {
        return Err(TalonParseError {
            line: line_no,
            message: format!("empty value for header `{}`", key),
        });
    }

    match key {
        "app" => ctx.app = Some(value.to_string()),
        "app.name" => ctx.app_name = Some(value.to_string()),
        "app.exe" => ctx.app_exe = Some(value.to_string()),
        "app.bundle" => ctx.app_bundle = Some(value.to_string()),
        "title" => ctx.title = Some(value.to_string()),
        "hostname" => ctx.hostname = Some(value.to_string()),
        "os" => ctx.os = Some(value.to_string()),
        "mode" => parse_mode_header(ctx, value, line_no)?,
        "not mode" => ctx.not_modes.push(value.to_string()),
        "tag" => ctx.tags.push(value.to_string()),
        "not tag" => ctx.not_tags.push(value.to_string()),
        "code.language" => ctx.code_language = Some(value.to_string()),
        _ => {
            return Err(TalonParseError {
                line: line_no,
                message: format!("unsupported context header `{}`", key),
            });
        }
    }
    Ok(())
}

fn parse_mode_header(
    ctx: &mut TalonContextHeader,
    value: &str,
    line_no: usize,
) -> Result<(), TalonParseError> {
    match value {
        "global" => {
            ctx.mode = Some(RuntimeMode::Global);
            ctx.talon_modes.push(value.to_string());
            Ok(())
        }
        "code" => {
            ctx.mode = Some(RuntimeMode::Code);
            ctx.talon_modes.push(value.to_string());
            Ok(())
        }
        "browser" => {
            ctx.mode = Some(RuntimeMode::Browser);
            ctx.talon_modes.push(value.to_string());
            Ok(())
        }
        "terminal" => {
            ctx.mode = Some(RuntimeMode::Terminal);
            ctx.talon_modes.push(value.to_string());
            Ok(())
        }
        "command" | "dictation" | "sleep" | "all" => {
            ctx.talon_modes.push(value.to_string());
            Ok(())
        }
        v if v.starts_with("user.") && v.len() > "user.".len() => {
            ctx.talon_modes.push(v.to_string());
            Ok(())
        }
        _ => Err(TalonParseError {
            line: line_no,
            message: format!("unsupported mode value `{}`", value),
        }),
    }
}

fn is_skipped_trigger(lhs: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "parrot(",
        "gamepad(",
        "noise(",
        "face(",
        "deck(",
        "key(",
    ];
    let trimmed = lhs.trim();
    PREFIXES.iter().any(|p| trimmed.starts_with(p)) && trimmed.ends_with(')')
}

fn parse_body_statement(stmt_src: &str, line_no: usize) -> Result<TalonBodyStmt, TalonParseError> {
    let stmt_src = stmt_src.trim();
    if stmt_src.is_empty() {
        return Err(TalonParseError {
            line: line_no,
            message: "empty statement".into(),
        });
    }

    if stmt_src == "settings():" || stmt_src.starts_with("settings():") {
        return Err(TalonParseError {
            line: line_no,
            message: "unsupported construct: settings()".into(),
        });
    }

    if let Some(rest) = stmt_src.strip_prefix("tag():") {
        let tag = rest.trim();
        if tag.is_empty() {
            return Err(TalonParseError {
                line: line_no,
                message: "tag activation missing tag name".into(),
            });
        }
        return Ok(TalonBodyStmt::TagActivate {
            tag: tag.to_string(),
        });
    }

    if let Some(quote) = stmt_src.chars().next().filter(|c| *c == '"' || *c == '\'') {
        if !is_complete_quoted_literal(stmt_src, quote) {
            return Err(TalonParseError {
                line: line_no,
                message: "unterminated string literal".into(),
            });
        }
        return Ok(TalonBodyStmt::ActionInvoke {
            name: "insert".to_string(),
            args: vec![stmt_src.to_string()],
        });
    }

    if let Some(eq_idx) = find_top_level_assignment(stmt_src) {
        let lhs = stmt_src[..eq_idx].trim();
        let rhs = stmt_src[eq_idx + 1..].trim();
        if !is_valid_identifier(lhs) || rhs.is_empty() {
            return Err(TalonParseError {
                line: line_no,
                message: "unsupported construct: invalid assignment".into(),
            });
        }
        return Ok(TalonBodyStmt::Assign {
            var: lhs.to_string(),
            expr: rhs.to_string(),
        });
    }

    let Some(open_idx) = stmt_src.find('(') else {
        return Err(TalonParseError {
            line: line_no,
            message: "unsupported statement (expected action invocation, tag(), or string literal)"
                .into(),
        });
    };
    if !stmt_src.ends_with(')') {
        return Err(TalonParseError {
            line: line_no,
            message: "unterminated action invocation".into(),
        });
    }
    let name = stmt_src[..open_idx].trim();
    if name.is_empty() {
        return Err(TalonParseError {
            line: line_no,
            message: "missing action name".into(),
        });
    }
    let inner = &stmt_src[open_idx + 1..stmt_src.len() - 1];
    let args = split_args(inner);
    Ok(TalonBodyStmt::ActionInvoke {
        name: name.to_string(),
        args,
    })
}

fn is_complete_quoted_literal(s: &str, quote: char) -> bool {
    let mut chars = s.chars();
    if chars.next() != Some(quote) {
        return false;
    }
    let mut escaped = false;
    let mut closed = false;
    for ch in chars {
        if closed {
            return false;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            closed = true;
        }
    }
    closed
}

fn find_top_level_assignment(stmt: &str) -> Option<usize> {
    let bytes = stmt.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    for (i, &b) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if (in_single || in_double) && b == b'\\' {
            escaped = true;
            continue;
        }
        if b == b'\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if b == b'"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if in_single || in_double {
            continue;
        }
        match b {
            b'(' => {
                // `=` after an opening call paren belongs to keyword/call syntax, not assignment.
                if paren_depth == 0 && bracket_depth == 0 {
                    return None;
                }
                paren_depth += 1;
            }
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'=' if paren_depth == 0 && bracket_depth == 0 => {
                let prev = i.checked_sub(1).and_then(|idx| bytes.get(idx).copied());
                let next = bytes.get(i + 1).copied();
                if !matches!(prev, Some(b'=') | Some(b'!') | Some(b'<') | Some(b'>'))
                    && next != Some(b'=')
                {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn split_top_level_colon(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut angle_depth = 0usize;

    for (i, &b) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if (in_single || in_double) && b == b'\\' {
            escaped = true;
            continue;
        }
        if b == b'\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if b == b'"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if in_single || in_double {
            continue;
        }
        match b {
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'{' => brace_depth += 1,
            b'}' => brace_depth = brace_depth.saturating_sub(1),
            b'<' => angle_depth += 1,
            b'>' => angle_depth = angle_depth.saturating_sub(1),
            b':' if paren_depth == 0
                && bracket_depth == 0
                && brace_depth == 0
                && angle_depth == 0 =>
            {
                return Some((&line[..i], &line[i + 1..]));
            }
            _ => {}
        }
    }
    None
}

fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_regex = false;
    let mut escaped = false;

    for (i, &b) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if (in_single || in_double || in_regex) && b == b'\\' {
            escaped = true;
            continue;
        }
        if b == b'\'' && !in_double && !in_regex {
            in_single = !in_single;
            continue;
        }
        if b == b'"' && !in_single && !in_regex {
            in_double = !in_double;
            continue;
        }
        if b == b'/' && !in_single && !in_double {
            in_regex = !in_regex;
            continue;
        }
        if !in_single && !in_double && !in_regex && b == b'#' {
            if i == 0 || bytes[i - 1].is_ascii_whitespace() {
                return line[..i].trim_end();
            }
        }
    }
    line.trim_end()
}

pub fn split_args(inner: &str) -> Vec<String> {
    let inner = inner.trim();
    if inner.is_empty() {
        return Vec::new();
    }

    let mut args = Vec::new();
    let mut start = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;

    for (i, ch) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if (in_single || in_double) && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if in_single || in_double {
            continue;
        }
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                let piece = inner[start..i].trim();
                if !piece.is_empty() {
                    args.push(piece.to_string());
                }
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }

    let tail = inner[start..].trim();
    if !tail.is_empty() {
        args.push(tail.to_string());
    }
    args
}

/// Parses a Talon spoken rule (e.g. `^take <user.cursorless_target> [please]$`) into a `SpokenRule` AST.
pub fn parse_spoken_rule(spoken: &str) -> Result<SpokenRule, TalonParseError> {
    let mut s = spoken.trim();
    if s.is_empty() {
        return Err(TalonParseError {
            line: 0,
            message: "empty spoken rule".into(),
        });
    }

    let anchored_start = if let Some(rest) = s.strip_prefix('^') {
        s = rest.trim_start();
        true
    } else {
        false
    };

    let anchored_end = if let Some(rest) = s.strip_suffix('$') {
        s = rest.trim_end();
        true
    } else {
        false
    };

    let mut parser = RuleParser::new(s);
    let branches = parser.parse_branches(None)?;
    if !parser.is_eof() {
        return Err(TalonParseError {
            line: 0,
            message: format!("unexpected trailing token in spoken rule `{}`", spoken),
        });
    }

    let tokens = if branches.len() == 1 {
        branches.into_iter().next().unwrap()
    } else {
        vec![SpokenToken::Alternation(branches)]
    };

    if tokens.is_empty() {
        return Err(TalonParseError {
            line: 0,
            message: "empty spoken rule".into(),
        });
    }

    Ok(SpokenRule {
        anchored_start,
        anchored_end,
        tokens,
    })
}

struct RuleParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> RuleParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn next_char(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn parse_branches(&mut self, stop: Option<char>) -> Result<Vec<Vec<SpokenToken>>, TalonParseError> {
        let mut branches = Vec::new();
        loop {
            let seq = self.parse_sequence(stop)?;
            if seq.is_empty() {
                return Err(TalonParseError {
                    line: 0,
                    message: "empty alternation branch in spoken rule".into(),
                });
            }
            branches.push(seq);
            if self.peek() == Some('|') {
                self.next_char();
                continue;
            }
            break;
        }
        Ok(branches)
    }

    fn parse_sequence(&mut self, stop: Option<char>) -> Result<Vec<SpokenToken>, TalonParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some(ch) if Some(ch) == stop || ch == '|' => break,
                Some(ch @ (']' | ')')) => {
                    return Err(TalonParseError {
                        line: 0,
                        message: format!("unexpected closing `{}` in spoken rule", ch),
                    });
                }
                _ => {}
            }

            let mut tok = match self.peek().unwrap() {
                '<' => self.parse_delimited('<', '>', "capture").map(SpokenToken::CaptureRef)?,
                '{' => self.parse_delimited('{', '}', "list").map(SpokenToken::ListRef)?,
                '[' => {
                    self.next_char();
                    let branches = self.parse_branches(Some(']'))?;
                    if self.next_char() != Some(']') {
                        return Err(TalonParseError {
                            line: 0,
                            message: "unclosed `[` in spoken rule".into(),
                        });
                    }
                    let inner = if branches.len() == 1 {
                        branches.into_iter().next().unwrap()
                    } else {
                        vec![SpokenToken::Alternation(branches)]
                    };
                    SpokenToken::Optional(inner)
                }
                '(' => {
                    self.next_char();
                    let branches = self.parse_branches(Some(')'))?;
                    if self.next_char() != Some(')') {
                        return Err(TalonParseError {
                            line: 0,
                            message: "unclosed `(` in spoken rule".into(),
                        });
                    }
                    SpokenToken::Alternation(branches)
                }
                '+' | '*' | '>' | '}' => {
                    let bad = self.next_char().unwrap();
                    return Err(TalonParseError {
                        line: 0,
                        message: format!("unexpected `{}` in spoken rule", bad),
                    });
                }
                _ => self.parse_word()?,
            };

            match self.peek() {
                Some('+') => {
                    self.next_char();
                    tok = SpokenToken::Repeat {
                        token: Box::new(tok),
                        one_or_more: true,
                    };
                }
                Some('*') => {
                    self.next_char();
                    tok = SpokenToken::Repeat {
                        token: Box::new(tok),
                        one_or_more: false,
                    };
                }
                _ => {}
            }

            tokens.push(tok);
        }
        Ok(tokens)
    }

    fn parse_delimited(
        &mut self,
        open: char,
        close: char,
        kind: &str,
    ) -> Result<String, TalonParseError> {
        let actual = self.next_char();
        debug_assert_eq!(actual, Some(open));
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch == close {
                let name = self.input[start..self.pos].trim().to_string();
                self.next_char();
                if name.is_empty() {
                    return Err(TalonParseError {
                        line: 0,
                        message: format!("empty {} reference in spoken rule", kind),
                    });
                }
                return Ok(name);
            }
            self.next_char();
        }
        Err(TalonParseError {
            line: 0,
            message: format!("unclosed `{}` {} reference in spoken rule", open, kind),
        })
    }

    fn parse_word(&mut self) -> Result<SpokenToken, TalonParseError> {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_whitespace()
                || matches!(
                    ch,
                    '<' | '>' | '{' | '}' | '[' | ']' | '(' | ')' | '|' | '+' | '*'
                )
            {
                break;
            }
            self.next_char();
        }
        let word = &self.input[start..self.pos];
        if word.is_empty() {
            return Err(TalonParseError {
                line: 0,
                message: "expected word token in spoken rule".into(),
            });
        }
        Ok(SpokenToken::Word(word.to_string()))
    }
}
