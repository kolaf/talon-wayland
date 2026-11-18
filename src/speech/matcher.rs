use crate::mode::RuntimeMode;
use crate::runtime::command_inventory::CommandInventory;
use crate::runtime::python_host::PythonCompatibilityHost;
use crate::runtime::talon_ast::{SpokenRule, SpokenToken, TalonCommand};
use crate::runtime::talon_parser::parse_spoken_rule;
use crate::speech::candidate::{MatchedCommand, SpeechCandidate};
use std::collections::{HashMap, HashSet};

const MAX_PHRASE_EXPANSIONS_PER_CMD: usize = 64;

/// Builds the active phrase inventory for `mode` from `inventory`.
pub fn phrase_inventory_for_mode(inventory: &CommandInventory, mode: RuntimeMode) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut phrases = Vec::new();
    for cmd in inventory.commands_for_mode(mode) {
        if seen.insert(cmd.spoken_form.clone()) {
            phrases.push(cmd.spoken_form.clone());
        }
    }
    phrases
}

/// Builds the active phrase inventory for `mode`, expanding finite list/alternation
/// rules using lists registered in `host`.
pub fn phrase_inventory_for_mode_with_host(
    inventory: &CommandInventory,
    host: &PythonCompatibilityHost,
    mode: RuntimeMode,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut phrases = Vec::new();

    for cmd in inventory.effective_commands_for_mode(mode) {
        let rule = effective_rule(cmd);
        let expanded = expand_rule_phrases(&rule, host);
        if expanded.is_empty() {
            if seen.insert(cmd.spoken_form.clone()) {
                phrases.push(cmd.spoken_form.clone());
            }
        } else {
            for p in expanded {
                if seen.insert(p.clone()) {
                    phrases.push(p);
                }
            }
        }
    }

    phrases
}

/// Finds the best matching command across `candidates` for `mode` (and `Global` fallback).
pub fn best_command_match(
    candidates: &[SpeechCandidate],
    inventory: &CommandInventory,
    mode: RuntimeMode,
) -> Option<MatchedCommand> {
    best_command_match_with_host(candidates, inventory, None, mode)
}

/// Finds the best matching command across `candidates`, resolving lists and captures
/// against `host` when available.
pub fn best_command_match_with_host(
    candidates: &[SpeechCandidate],
    inventory: &CommandInventory,
    host: Option<&PythonCompatibilityHost>,
    mode: RuntimeMode,
) -> Option<MatchedCommand> {
    let mut sorted: Vec<(usize, &SpeechCandidate)> = candidates.iter().enumerate().collect();
    sorted.sort_by(|(idx_a, a), (idx_b, b)| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| idx_a.cmp(idx_b))
    });

    let mode_cmds = inventory.commands_for_mode(mode);
    let global_cmds = if mode != RuntimeMode::Global {
        inventory.commands_for_mode(RuntimeMode::Global)
    } else {
        Vec::new()
    };

    for (_, candidate) in sorted {
        let transcript = candidate.transcript.trim();
        if transcript.is_empty() {
            continue;
        }
        let words: Vec<&str> = transcript.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }

        if let Some(matched) = match_against_commands(&mode_cmds, transcript, &words, host, mode) {
            return Some(matched);
        }
        if !global_cmds.is_empty() {
            if let Some(matched) =
                match_against_commands(&global_cmds, transcript, &words, host, mode)
            {
                return Some(matched);
            }
        }
    }

    None
}

fn match_against_commands(
    commands: &[&TalonCommand],
    transcript: &str,
    words: &[&str],
    host: Option<&PythonCompatibilityHost>,
    mode: RuntimeMode,
) -> Option<MatchedCommand> {
    // 1. Exact literal match wins over parameterized rules.
    for cmd in commands {
        if cmd.spoken_form.trim().eq_ignore_ascii_case(transcript) {
            return Some(MatchedCommand::new((*cmd).clone(), HashMap::new()));
        }
    }

    // 2. SpokenRule match, preferring rules with more literal word constraints.
    let mut best: Option<(usize, MatchedCommand)> = None;
    for cmd in commands {
        let rule = effective_rule(cmd);
        if let Some(captures) = match_spoken_rule(&rule, words, host, mode) {
            let literal_score = count_literal_words(&rule.tokens);
            let replace = match &best {
                None => true,
                Some((best_score, _)) => literal_score > *best_score,
            };
            if replace {
                best = Some((literal_score, MatchedCommand::new((*cmd).clone(), captures)));
            }
        }
    }

    best.map(|(_, m)| m)
}

fn effective_rule(cmd: &TalonCommand) -> SpokenRule {
    if !cmd.rule.tokens.is_empty() {
        return cmd.rule.clone();
    }
    parse_spoken_rule(&cmd.spoken_form).unwrap_or_else(|_| SpokenRule {
        anchored_start: false,
        anchored_end: false,
        tokens: cmd
            .spoken_form
            .split_whitespace()
            .map(|w| SpokenToken::Word(w.to_string()))
            .collect(),
    })
}

fn count_literal_words(tokens: &[SpokenToken]) -> usize {
    tokens
        .iter()
        .map(|tok| match tok {
            SpokenToken::Word(_) => 1,
            SpokenToken::Optional(inner) => count_literal_words(inner),
            SpokenToken::Alternation(branches) => branches
                .iter()
                .map(|b| count_literal_words(b))
                .min()
                .unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn match_spoken_rule(
    rule: &SpokenRule,
    words: &[&str],
    host: Option<&PythonCompatibilityHost>,
    mode: RuntimeMode,
) -> Option<HashMap<String, String>> {
    let matches = match_sequence(&rule.tokens, words, 0, host, mode);
    matches
        .into_iter()
        .find(|(end_pos, _)| *end_pos == words.len())
        .map(|(_, bindings)| bindings)
}

fn match_sequence(
    tokens: &[SpokenToken],
    words: &[&str],
    pos: usize,
    host: Option<&PythonCompatibilityHost>,
    mode: RuntimeMode,
) -> Vec<(usize, HashMap<String, String>)> {
    if tokens.is_empty() {
        return vec![(pos, HashMap::new())];
    }

    let first = &tokens[0];
    let rest = &tokens[1..];
    let mut results = Vec::new();

    for (next_pos, first_bindings) in match_token(first, words, pos, host, mode) {
        for (end_pos, rest_bindings) in match_sequence(rest, words, next_pos, host, mode) {
            let mut merged = first_bindings.clone();
            merged.extend(rest_bindings);
            results.push((end_pos, merged));
        }
    }

    results
}

fn match_token(
    token: &SpokenToken,
    words: &[&str],
    pos: usize,
    host: Option<&PythonCompatibilityHost>,
    mode: RuntimeMode,
) -> Vec<(usize, HashMap<String, String>)> {
    match token {
        SpokenToken::Word(expected) => {
            if let Some(actual) = words.get(pos) {
                if actual.eq_ignore_ascii_case(expected) {
                    return vec![(pos + 1, HashMap::new())];
                }
            }
            Vec::new()
        }
        SpokenToken::ListRef(list_name) => match_list_ref(list_name, words, pos, host),
        SpokenToken::CaptureRef(cap_name) => match_capture_ref(cap_name, words, pos, host, mode),
        SpokenToken::Optional(inner) => {
            let mut out = match_sequence(inner, words, pos, host, mode);
            out.push((pos, HashMap::new()));
            out
        }
        SpokenToken::Alternation(branches) => {
            let mut out = Vec::new();
            for branch in branches {
                out.extend(match_sequence(branch, words, pos, host, mode));
            }
            out
        }
        SpokenToken::Repeat { token, one_or_more } => {
            match_repeat(token, *one_or_more, words, pos, host, mode)
        }
    }
}

fn match_list_ref(
    list_name: &str,
    words: &[&str],
    pos: usize,
    host: Option<&PythonCompatibilityHost>,
) -> Vec<(usize, HashMap<String, String>)> {
    if pos >= words.len() {
        return Vec::new();
    }

    let short = short_name(list_name);
    if let Some(host) = host {
        if let Some(mapping) = host.list_mapping(list_name) {
            let mut out = Vec::new();
            for (spoken_key, target_val) in mapping {
                let key_words: Vec<&str> = spoken_key.split_whitespace().collect();
                if key_words.is_empty() || pos + key_words.len() > words.len() {
                    continue;
                }
                let matches = key_words
                    .iter()
                    .zip(&words[pos..pos + key_words.len()])
                    .all(|(k, w)| k.eq_ignore_ascii_case(w));
                if matches {
                    let mut bindings = HashMap::new();
                    bindings.insert(list_name.to_string(), target_val.clone());
                    bindings.insert(short.to_string(), target_val.clone());
                    out.push((pos + key_words.len(), bindings));
                }
            }
            return out;
        }
    }

    // Fallback without host list mapping: bind single spoken word.
    let val = words[pos].to_string();
    let mut bindings = HashMap::new();
    bindings.insert(list_name.to_string(), val.clone());
    bindings.insert(short.to_string(), val);
    vec![(pos + 1, bindings)]
}

fn match_capture_ref(
    cap_name: &str,
    words: &[&str],
    pos: usize,
    host: Option<&PythonCompatibilityHost>,
    mode: RuntimeMode,
) -> Vec<(usize, HashMap<String, String>)> {
    if pos >= words.len() {
        return Vec::new();
    }

    let short = short_name(cap_name);
    if let Some(host) = host {
        if let Some(sig) = host.capture_signature(cap_name) {
            let self_ref = format!("<{cap_name}>");
            if sig != self_ref {
                if let Ok(sub_rule) = parse_spoken_rule(sig) {
                    let sub_matches = match_sequence(&sub_rule.tokens, words, pos, Some(host), mode);
                    let mut out = Vec::new();
                    let mut seen_ends = HashSet::new();
                    for (end_pos, _) in sub_matches {
                        if end_pos <= pos || !seen_ends.insert(end_pos) {
                            continue;
                        }
                        let matched_words: Vec<String> = words[pos..end_pos]
                            .iter()
                            .map(|w| (*w).to_string())
                            .collect();
                        let val = host
                            .invoke_capture(cap_name, mode, &matched_words)
                            .unwrap_or_else(|_| matched_words.join(" "));
                        let mut bindings = HashMap::new();
                        bindings.insert(cap_name.to_string(), val.clone());
                        bindings.insert(short.to_string(), val);
                        out.push((end_pos, bindings));
                    }
                    if !out.is_empty() {
                        return out;
                    }
                }
            }
        }
    }

    // Fallback / free capture (e.g. `<user.text>` or `<number>`): try 1..=remaining words.
    let mut out = Vec::new();
    for end_pos in (pos + 1)..=words.len() {
        let matched_words: Vec<String> = words[pos..end_pos]
            .iter()
            .map(|w| (*w).to_string())
            .collect();
        let val = host
            .and_then(|h| h.invoke_capture(cap_name, mode, &matched_words).ok())
            .unwrap_or_else(|| matched_words.join(" "));
        let mut bindings = HashMap::new();
        bindings.insert(cap_name.to_string(), val.clone());
        bindings.insert(short.to_string(), val);
        out.push((end_pos, bindings));
    }
    out
}

fn match_repeat(
    token: &SpokenToken,
    one_or_more: bool,
    words: &[&str],
    pos: usize,
    host: Option<&PythonCompatibilityHost>,
    mode: RuntimeMode,
) -> Vec<(usize, HashMap<String, String>)> {
    let mut out = Vec::new();
    if !one_or_more {
        out.push((pos, HashMap::new()));
    }

    let mut frontier: Vec<(usize, HashMap<String, String>)> = vec![(pos, HashMap::new())];
    while let Some((curr_pos, curr_bindings)) = frontier.pop() {
        for (next_pos, step_bindings) in match_token(token, words, curr_pos, host, mode) {
            if next_pos <= curr_pos {
                continue;
            }
            let mut merged = curr_bindings.clone();
            for (k, v) in step_bindings {
                merged
                    .entry(k)
                    .and_modify(|existing| {
                        if !existing.is_empty() && !v.is_empty() {
                            existing.push(' ');
                        }
                        existing.push_str(&v);
                    })
                    .or_insert(v);
            }
            out.push((next_pos, merged.clone()));
            frontier.push((next_pos, merged));
        }
    }

    out
}

fn short_name(full: &str) -> &str {
    full.rsplit('.').next().unwrap_or(full)
}

fn expand_rule_phrases(rule: &SpokenRule, host: &PythonCompatibilityHost) -> Vec<String> {
    let mut acc = vec![String::new()];
    for tok in &rule.tokens {
        let Some(variants) = expand_token_variants(tok, host) else {
            return Vec::new();
        };
        let mut next = Vec::new();
        for prefix in &acc {
            for v in &variants {
                let joined = match (prefix.is_empty(), v.is_empty()) {
                    (true, _) => v.clone(),
                    (false, true) => prefix.clone(),
                    (false, false) => format!("{prefix} {v}"),
                };
                next.push(joined);
                if next.len() > MAX_PHRASE_EXPANSIONS_PER_CMD {
                    return Vec::new();
                }
            }
        }
        acc = next;
    }
    acc.into_iter().filter(|s| !s.is_empty()).collect()
}

fn expand_token_variants(
    tok: &SpokenToken,
    host: &PythonCompatibilityHost,
) -> Option<Vec<String>> {
    match tok {
        SpokenToken::Word(w) => Some(vec![w.clone()]),
        SpokenToken::ListRef(name) => {
            let vals = host.list_values(name)?;
            if vals.is_empty() || vals.len() > MAX_PHRASE_EXPANSIONS_PER_CMD {
                return None;
            }
            Some(vals.to_vec())
        }
        SpokenToken::Optional(inner) => {
            let mut out = vec![String::new()];
            let sub = expand_sequence_variants(inner, host)?;
            out.extend(sub);
            Some(out)
        }
        SpokenToken::Alternation(branches) => {
            let mut out = Vec::new();
            for b in branches {
                out.extend(expand_sequence_variants(b, host)?);
                if out.len() > MAX_PHRASE_EXPANSIONS_PER_CMD {
                    return None;
                }
            }
            Some(out)
        }
        SpokenToken::CaptureRef(_) | SpokenToken::Repeat { .. } => None,
    }
}

fn expand_sequence_variants(
    seq: &[SpokenToken],
    host: &PythonCompatibilityHost,
) -> Option<Vec<String>> {
    let mut acc = vec![String::new()];
    for tok in seq {
        let variants = expand_token_variants(tok, host)?;
        let mut next = Vec::new();
        for prefix in &acc {
            for v in &variants {
                let joined = match (prefix.is_empty(), v.is_empty()) {
                    (true, _) => v.clone(),
                    (false, true) => prefix.clone(),
                    (false, false) => format!("{prefix} {v}"),
                };
                next.push(joined);
                if next.len() > MAX_PHRASE_EXPANSIONS_PER_CMD {
                    return None;
                }
            }
        }
        acc = next;
    }
    Some(acc)
}
