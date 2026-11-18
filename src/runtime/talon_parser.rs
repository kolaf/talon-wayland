use crate::mode::RuntimeMode;
use crate::runtime::talon_ast::{TalonBodyStmt, TalonCommand, TalonContextHeader, TalonFile};

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

pub fn parse_talon_file(source: &str) -> Result<TalonFile, TalonParseError> {
    let mut ctx = TalonContextHeader::default();
    let mut commands: Vec<TalonCommand> = Vec::new();

    enum Section {
        Header,
        Body,
    }
    let mut section = Section::Header;

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }

        match section {
            Section::Header => {
                if trimmed == "-" {
                    section = Section::Body;
                    continue;
                }
                parse_header_line(&mut ctx, trimmed, line_no)?;
            }
            Section::Body => {
                if !raw_line.starts_with(' ') && !raw_line.starts_with('\t') {
                    // Unindented line: either a top-level statement (eg `tag(): ...`) or a command header.
                    if trimmed.starts_with("tag():") {
                        let body = parse_body_statement(trimmed, line_no)?;
                        commands.push(TalonCommand {
                            spoken_form: "__talon_stmt_tag_activate".to_string(),
                            body: vec![body],
                            context: ctx.clone(),
                            source_path: None,
                            line: line_no,
                        });
                        continue;
                    }

                    let Some((lhs, rhs)) = trimmed.split_once(':') else {
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
                    let after_colon = rhs;
                    let mut body: Vec<TalonBodyStmt> = Vec::new();
                    if !after_colon.trim().is_empty() {
                        body.push(parse_body_statement(after_colon.trim(), line_no)?);
                    }

                    commands.push(TalonCommand {
                        spoken_form: spoken.to_string(),
                        body,
                        context: ctx.clone(),
                        source_path: None,
                        line: line_no,
                    });
                } else {
                    // Indented body line; attach to the most recent command.
                    let stmt_src = trimmed;

                    // Explicitly disallow settings blocks in v1 subset.
                    // `settings():` is a top-level statement that takes an indented block.
                    if stmt_src == "settings():" {
                        return Err(TalonParseError {
                            line: line_no,
                            message: "unsupported construct: settings()".into(),
                        });
                    }
                    let stmt = parse_body_statement(stmt_src, line_no)?;
                    let Some(last) = commands.last_mut() else {
                        return Err(TalonParseError {
                            line: line_no,
                            message: "indented statement without a preceding command".into(),
                        });
                    };
                    // If last is a top-level tag() pseudo-command, treat this as unsupported to avoid guessing.
                    if last.spoken_form == "__talon_stmt_tag_activate" {
                        return Err(TalonParseError {
                            line: line_no,
                            message: "indented statement after top-level tag() is unsupported".into(),
                        });
                    }
                    last.body.push(stmt);
                }
            }
        }
    }

    Ok(TalonFile {
        context: ctx,
        commands,
    })
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
    let key = key.trim();
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
        "os" => ctx.os = Some(value.to_string()),
        "mode" => {
            ctx.mode = Some(parse_mode_value(value, line_no)?);
        }
        "tag" => ctx.tags.push(value.to_string()),
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

fn parse_mode_value(value: &str, line_no: usize) -> Result<RuntimeMode, TalonParseError> {
    // Subset: keep it strict; accept canonical talon-ish words.
    match value {
        "global" => Ok(RuntimeMode::Global),
        "code" => Ok(RuntimeMode::Code),
        "browser" => Ok(RuntimeMode::Browser),
        "terminal" => Ok(RuntimeMode::Terminal),
        _ => Err(TalonParseError {
            line: line_no,
            message: format!("unsupported mode value `{}`", value),
        }),
    }
}

fn parse_body_statement(stmt_src: &str, line_no: usize) -> Result<TalonBodyStmt, TalonParseError> {
    // Supported statement forms for v1:
    // - tag activation: `tag(): user.foo`
    // - action invocation: `name(...)` where name can include dots (eg `user.rango_command_without_target`)
    // - implicit string: "hello" (double quoted only)
    //
    // Explicitly unsupported (must error): settings blocks, assignments, key bindings, etc.
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

    if stmt_src.starts_with('"') {
        if !stmt_src.ends_with('"') || stmt_src.len() < 2 {
            return Err(TalonParseError {
                line: line_no,
                message: "unterminated string literal".into(),
            });
        }
        let inner = &stmt_src[1..stmt_src.len() - 1];
        // v1: no escape parsing
        return Ok(TalonBodyStmt::ActionInvoke {
            name: "insert".to_string(),
            args: vec![format!("\"{}\"", inner)],
        });
    }

    // assignment inside body (eg `x = 1`) is unsupported in v1.
    // NOTE: this will also reject settings assignment lines like `key_wait = 1.5`.
    if stmt_src.contains('=') && !stmt_src.contains('(') {
        return Err(TalonParseError {
            line: line_no,
            message: "unsupported construct: assignment".into(),
        });
    }

    let Some(open_idx) = stmt_src.find('(') else {
        return Err(TalonParseError {
            line: line_no,
            message: "unsupported statement (expected action invocation, tag(), or string literal)".into(),
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

fn split_args(inner: &str) -> Vec<String> {
    let inner = inner.trim();
    if inner.is_empty() {
        return Vec::new();
    }
    // v1 subset: args are treated as raw strings, no quoting/escaping support.
    inner
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}
