use crate::backend::AppBackend;
use crate::mode::RuntimeMode;
use crate::runtime::actions::ActionRegistry;
use crate::runtime::errors::ExecutionError;
use crate::runtime::python_host::PythonCompatibilityHost;
use crate::runtime::talon_ast::TalonBodyStmt;
use crate::runtime::talon_parser::split_args;
use crate::speech::MatchedCommand;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeValue {
    String(String),
    Number(String),
    Bool(bool),
}

impl RuntimeValue {
    pub fn as_str(&self) -> &str {
        match self {
            RuntimeValue::String(s) | RuntimeValue::Number(s) => s.as_str(),
            RuntimeValue::Bool(true) => "true",
            RuntimeValue::Bool(false) => "false",
        }
    }

    pub fn from_raw(s: String) -> Self {
        if s == "true" {
            RuntimeValue::Bool(true)
        } else if s == "false" {
            RuntimeValue::Bool(false)
        } else if s.parse::<f64>().is_ok() {
            RuntimeValue::Number(s)
        } else {
            RuntimeValue::String(s)
        }
    }
}

pub struct ExecutionContext<'a> {
    pub mode: RuntimeMode,
    pub action_registry: &'a ActionRegistry,
    pub python_host: &'a PythonCompatibilityHost,
    pub backends: &'a mut [Box<dyn AppBackend>],
    pub invoked_backend: Option<String>,
}

/// Executes a matched Talon command against registered backends, the embedded Python host,
/// and the built-in action registry.
pub fn execute_matched_command(
    matched: &MatchedCommand,
    ctx: &mut ExecutionContext<'_>,
) -> Result<(), ExecutionError> {
    if let Some(req_mode) = matched.command.context.mode {
        if req_mode != RuntimeMode::Global && req_mode != ctx.mode {
            return Err(ExecutionError::ContextMismatch(format!(
                "command requires mode `{req_mode}`, active mode is `{}`",
                ctx.mode
            )));
        }
    }

    let mut vars = matched.captures.clone();

    for stmt in &matched.command.body {
        match stmt {
            TalonBodyStmt::Assign { var, expr } => {
                let value = eval_expr(expr, &vars, ctx)?;
                vars.insert(var.clone(), value);
            }
            TalonBodyStmt::ActionInvoke { name, args } => {
                let mut eval_args = Vec::with_capacity(args.len());
                for arg in args {
                    eval_args.push(eval_expr(arg, &vars, ctx)?);
                }
                let _ = invoke_action(name, &eval_args, ctx)?;
            }
            TalonBodyStmt::TagActivate { .. } | TalonBodyStmt::Setting { .. } => {}
        }
    }

    Ok(())
}

fn eval_expr(
    raw: &str,
    vars: &HashMap<String, String>,
    ctx: &mut ExecutionContext<'_>,
) -> Result<String, ExecutionError> {
    let expr = raw.trim();
    if expr.is_empty() {
        return Ok(String::new());
    }

    // Quoted string literal with escapes and `{var}` interpolation.
    if let Some(quote) = expr.chars().next().filter(|c| *c == '"' || *c == '\'') {
        if expr.len() >= 2 && expr.ends_with(quote) {
            let inner = &expr[1..expr.len() - 1];
            let unescaped = unescape_string(inner);
            return Ok(interpolate_vars(&unescaped, vars));
        }
    }

    // Simple binary arithmetic (`a - b` or `a + b`).
    if let Some(val) = try_eval_binary_op(expr, vars) {
        return Ok(val);
    }

    // Nested action invocation (`name(arg1, arg2)`).
    if let Some(open_idx) = expr.find('(') {
        if expr.ends_with(')') && !expr[..open_idx].contains(' ') {
            let fn_name = expr[..open_idx].trim();
            let inner = &expr[open_idx + 1..expr.len() - 1];
            let arg_exprs = split_args(inner);
            let mut eval_args = Vec::with_capacity(arg_exprs.len());
            for a in arg_exprs {
                eval_args.push(eval_expr(&a, vars, ctx)?);
            }
            return Ok(invoke_action(fn_name, &eval_args, ctx)?.unwrap_or_default());
        }
    }

    // Local/capture variable reference.
    if let Some(bound) = vars.get(expr) {
        return Ok(bound.clone());
    }
    if let Some(short) = expr.rsplit('.').next() {
        if let Some(bound) = vars.get(short) {
            return Ok(bound.clone());
        }
    }

    Ok(expr.to_string())
}

fn invoke_action(
    name: &str,
    args: &[String],
    ctx: &mut ExecutionContext<'_>,
) -> Result<Option<String>, ExecutionError> {
    if name.starts_with("user.unsupported") || name.starts_with("unsupported") {
        return Err(ExecutionError::UnsupportedFeature(name.to_string()));
    }

    let runtime_vals: Vec<RuntimeValue> =
        args.iter().cloned().map(RuntimeValue::from_raw).collect();

    // 1. Check active mode's backend first (if a backend owns this action).
    if let Some(handled) = try_dispatch_to_backend(name, &runtime_vals, ctx)? {
        ctx.invoked_backend = Some(handled);
        return Ok(None);
    }

    // 2. Check embedded Python host (module actions & context overrides).
    if ctx.python_host.resolve_action_impl(name, ctx.mode).is_some() {
        let (ret, trace) = ctx
            .python_host
            .invoke_action_with_trace(name, ctx.mode, args)
            .map_err(|e| ExecutionError::UnsupportedFeature(e.to_string()))?;

        for recorded in trace {
            let rec_vals: Vec<RuntimeValue> = recorded
                .args
                .iter()
                .cloned()
                .map(RuntimeValue::from_raw)
                .collect();
            if let Some(handled) = try_dispatch_to_backend(&recorded.name, &rec_vals, ctx)? {
                ctx.invoked_backend = Some(handled);
            } else if ctx.action_registry.resolve(&recorded.name).is_err()
                && (recorded.name.starts_with("user.unsupported")
                    || recorded.name.starts_with("unsupported"))
            {
                return Err(ExecutionError::UnsupportedFeature(recorded.name));
            }
        }
        return Ok(ret);
    }

    // 3. Check built-in action registry.
    if ctx.action_registry.resolve(name).is_ok() {
        return Ok(None);
    }

    Err(ExecutionError::ActionNotFound(name.to_string()))
}

fn try_dispatch_to_backend(
    name: &str,
    args: &[RuntimeValue],
    ctx: &mut ExecutionContext<'_>,
) -> Result<Option<String>, ExecutionError> {
    // Prefer backend matching the active mode first, then any backend that explicitly owns the action.
    for backend in ctx.backends.iter_mut() {
        if backend.target_mode() == ctx.mode && backend.handles_action(name) {
            backend.execute_action(name, args)?;
            return Ok(Some(backend.name().to_string()));
        }
    }

    for backend in ctx.backends.iter_mut() {
        if backend.handles_action(name) {
            backend.execute_action(name, args)?;
            return Ok(Some(backend.name().to_string()));
        }
    }

    Ok(None)
}

fn try_eval_binary_op(expr: &str, vars: &HashMap<String, String>) -> Option<String> {
    for (op, is_add) in [(" + ", true), (" - ", false)] {
        if let Some((lhs, rhs)) = expr.split_once(op) {
            let l_str = resolve_operand(lhs.trim(), vars);
            let r_str = resolve_operand(rhs.trim(), vars);
            if let (Ok(l_num), Ok(r_num)) = (l_str.parse::<i64>(), r_str.parse::<i64>()) {
                let res = if is_add { l_num + r_num } else { l_num - r_num };
                return Some(res.to_string());
            }
            if is_add {
                return Some(format!("{l_str}{r_str}"));
            }
        }
    }
    None
}

fn resolve_operand(tok: &str, vars: &HashMap<String, String>) -> String {
    if let Some(val) = vars.get(tok) {
        return val.clone();
    }
    if tok.len() >= 2
        && ((tok.starts_with('"') && tok.ends_with('"'))
            || (tok.starts_with('\'') && tok.ends_with('\'')))
    {
        return tok[1..tok.len() - 1].to_string();
    }
    tok.to_string()
}

fn unescape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn interpolate_vars(s: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        if let Some(close) = after_open.find('}') {
            let key = after_open[..close].trim();
            if let Some(val) = vars
                .get(key)
                .or_else(|| key.rsplit('.').next().and_then(|short| vars.get(short)))
            {
                out.push_str(val);
            } else {
                out.push('{');
                out.push_str(key);
                out.push('}');
            }
            rest = &after_open[close + 1..];
        } else {
            out.push_str(&rest[open..]);
            return out;
        }
    }
    out.push_str(rest);
    out
}
