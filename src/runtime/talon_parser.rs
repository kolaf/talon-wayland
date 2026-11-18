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
                // For v1 subset: only support single-line commands:
                // `spoken form: action(args)`
                // Anything else errors explicitly.
                let Some((lhs, rhs)) = trimmed.split_once(':') else {
                    return Err(TalonParseError {
                        line: line_no,
                        message: "unsupported talon syntax (expected `spoken: body`)".into(),
                    });
                };
                let spoken = lhs.trim();
                if spoken.is_empty() {
                    return Err(TalonParseError {
                        line: line_no,
                        message: "empty spoken form".into(),
                    });
                }
                let body_src = rhs.trim();
                if body_src.is_empty() {
                    return Err(TalonParseError {
                        line: line_no,
                        message: "empty command body".into(),
                    });
                }
                let body = parse_body_inline(body_src, line_no)?;
                commands.push(TalonCommand {
                    spoken_form: spoken.to_string(),
                    body,
                    context: ctx.clone(),
                    source_path: None,
                    line: line_no,
                });
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

fn parse_body_inline(body_src: &str, line_no: usize) -> Result<Vec<TalonBodyStmt>, TalonParseError> {
    // Support minimal statements needed by supported repos.
    // - `tag(): user.foo`
    // - `action(arg, arg2)` or `user.action()` etc.
    if let Some(rest) = body_src.strip_prefix("tag():") {
        let tag = rest.trim();
        if tag.is_empty() {
            return Err(TalonParseError {
                line: line_no,
                message: "tag activation missing tag name".into(),
            });
        }
        return Ok(vec![TalonBodyStmt::TagActivate {
            tag: tag.to_string(),
        }]);
    }

    let Some(open_idx) = body_src.find('(') else {
        return Err(TalonParseError {
            line: line_no,
            message: "unsupported command body (expected action invocation)".into(),
        });
    };
    if !body_src.ends_with(')') {
        return Err(TalonParseError {
            line: line_no,
            message: "unterminated action invocation".into(),
        });
    }
    let name = body_src[..open_idx].trim();
    if name.is_empty() {
        return Err(TalonParseError {
            line: line_no,
            message: "missing action name".into(),
        });
    }
    let inner = &body_src[open_idx + 1..body_src.len() - 1];
    let args = split_args(inner);
    Ok(vec![TalonBodyStmt::ActionInvoke {
        name: name.to_string(),
        args,
    }])
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
