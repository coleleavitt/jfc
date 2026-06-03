//! Bimodal (name-based) taint source/sink inference.
//!
//! From *Fluffy* and the *fluentTQL* line of work: an identifier's **natural
//! language** is a strong prior for whether it is a taint *source* (untrusted
//! input) or a *sink* (a dangerous operation). `read_user_input` smells like a
//! source; `exec_sql` smells like a sink. Used to *auto-seed* and *rank* a
//! dataflow taint analysis ([`crate::taint_v2`]) instead of hand-writing the
//! source/sink config, and to surface the most interesting finding: a flow
//! whose **name says safe but whose dataflow says dangerous**.
//!
//! This module is purely lexical — it scores identifiers against two token
//! lexicons. It contributes the *prior*; the dataflow reachability that
//! confirms an actual source→sink path is [`crate::taint_v2`]'s job. Keeping the
//! two separate is deliberate: the name heuristic is cheap and seeds candidates,
//! the dataflow is precise and confirms them.
//!
//! Identifier splitting handles `snake_case`, `camelCase`, `PascalCase`, and
//! `kebab/dot`-separated names, lowercasing each sub-token before matching.

/// Source-ish sub-tokens: untrusted / externally-influenced data.
const SOURCE_TOKENS: &[&str] = &[
    "input",
    "request",
    "req",
    "param",
    "params",
    "arg",
    "args",
    "argv",
    "recv",
    "read",
    "env",
    "user",
    "untrusted",
    "stdin",
    "body",
    "query",
    "cookie",
    "header",
    "payload",
    "form",
    "upload",
    "external",
    "remote",
    "raw",
];

/// Sink-ish sub-tokens: operations that are dangerous with tainted data.
const SINK_TOKENS: &[&str] = &[
    "exec",
    "execute",
    "query",
    "system",
    "eval",
    "write",
    "command",
    "cmd",
    "render",
    "html",
    "sql",
    "spawn",
    "shell",
    "popen",
    "deserialize",
    "load",
    "send",
    "open",
    "delete",
    "remove",
    "run",
];

/// The name-based classification of one identifier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NameClass {
    /// Fraction of sub-tokens that look source-ish, in `[0, 1]`.
    pub source_score: f64,
    /// Fraction of sub-tokens that look sink-ish, in `[0, 1]`.
    pub sink_score: f64,
}

impl NameClass {
    /// Whether the name leans source (source_score strictly greater).
    pub fn looks_like_source(&self) -> bool {
        self.source_score > self.sink_score && self.source_score > 0.0
    }
    /// Whether the name leans sink (sink_score strictly greater).
    pub fn looks_like_sink(&self) -> bool {
        self.sink_score > self.source_score && self.sink_score > 0.0
    }
}

/// Split an identifier into lowercased sub-tokens across `snake_case`,
/// `camelCase`/`PascalCase`, digits, and `kebab`/`.`/`/` separators.
///
/// Examples: `read_user_input` → `[read, user, input]`;
/// `execSQLQuery` → `[exec, sql, query]`; `HTTPRequest` → `[http, request]`.
pub fn split_identifier(name: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = name.chars().collect();

    let flush = |cur: &mut String, tokens: &mut Vec<String>| {
        if !cur.is_empty() {
            tokens.push(std::mem::take(cur).to_ascii_lowercase());
        }
    };

    for i in 0..chars.len() {
        let c = chars[i];
        if !c.is_alphanumeric() {
            // Separator (`_`, `-`, `.`, `/`, …) ends the current token.
            flush(&mut cur, &mut tokens);
            continue;
        }
        if c.is_ascii_uppercase() && i > 0 {
            let prev = chars[i - 1];
            // Boundary on lower→Upper (camelCase) ...
            let lower_to_upper = prev.is_ascii_lowercase() || prev.is_ascii_digit();
            // ... or Upper→Upper→lower (acronym end: HTTPRequest -> HTTP|Request).
            let acronym_end = prev.is_ascii_uppercase()
                && i + 1 < chars.len()
                && chars[i + 1].is_ascii_lowercase();
            if lower_to_upper || acronym_end {
                flush(&mut cur, &mut tokens);
            }
        }
        cur.push(c);
    }
    flush(&mut cur, &mut tokens);
    tokens
}

/// Classify an identifier by its name. The score is the fraction of sub-tokens
/// matching each lexicon, so a focused name like `exec` scores higher than a
/// long name with one incidental match.
pub fn classify_name(name: &str) -> NameClass {
    let tokens = split_identifier(name);
    if tokens.is_empty() {
        return NameClass {
            source_score: 0.0,
            sink_score: 0.0,
        };
    }
    let n = tokens.len() as f64;
    let src = tokens
        .iter()
        .filter(|t| SOURCE_TOKENS.contains(&t.as_str()))
        .count();
    let sink = tokens
        .iter()
        .filter(|t| SINK_TOKENS.contains(&t.as_str()))
        .count();
    NameClass {
        source_score: src as f64 / n,
        sink_score: sink as f64 / n,
    }
}

/// Whether a confirmed dataflow `source → sink` flow is **surprising**: the
/// names give no hint that it is dangerous, so a human reviewer would likely
/// miss it. These are the highest-value findings to rank first — a real flow
/// the naming convention hides.
///
/// A flow is surprising when neither endpoint is named like what it is: the
/// source isn't named source-ish *and* the sink isn't named sink-ish. If either
/// end "announces itself", the flow is expected and ranked lower.
pub fn is_surprising(source_name: &str, sink_name: &str) -> bool {
    let s = classify_name(source_name);
    let k = classify_name(sink_name);
    !s.looks_like_source() && !k.looks_like_sink()
}

/// Boost applied to a surprising flow's priority. Strictly greater than the
/// maximum possible name `evidence` (`source_score + sink_score` ∈ [0, 2]) so a
/// surprising flow always out-ranks any well-named one — the Fluffy insight is
/// that name/behaviour *mismatches* are the highest-value findings, since a
/// reviewer reading the names would never suspect them.
const SURPRISE_BOOST: f64 = 10.0;

/// Rank a *confirmed* (dataflow-proven) flow for review: surprising flows
/// first, then by combined name evidence within each group. Higher is more
/// worth surfacing. Deterministic.
///
/// Because [`flow_priority`] is applied to flows the dataflow has already
/// confirmed are real, name evidence no longer measures "is this real" but
/// "would a human catch it". Innocuously-named (surprising) flows are the ones a
/// reviewer would miss, so they get [`SURPRISE_BOOST`] and sort to the top.
pub fn flow_priority(source_name: &str, sink_name: &str) -> f64 {
    let s = classify_name(source_name);
    let k = classify_name(sink_name);
    // Evidence ∈ [0, 2]: how strongly the names themselves suggest a src→sink.
    let evidence = s.source_score + k.sink_score;
    let surprise = if is_surprising(source_name, sink_name) {
        SURPRISE_BOOST
    } else {
        0.0
    };
    evidence + surprise
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: identifier splitting handles snake, camel, Pascal, acronyms.
    #[test]
    fn split_identifier_cases_normal() {
        assert_eq!(
            split_identifier("read_user_input"),
            vec!["read", "user", "input"]
        );
        assert_eq!(
            split_identifier("execSqlQuery"),
            vec!["exec", "sql", "query"]
        );
        assert_eq!(split_identifier("HTTPRequest"), vec!["http", "request"]);
        assert_eq!(
            split_identifier("render-html.now"),
            vec!["render", "html", "now"]
        );
    }

    // Normal: a source-ish name scores source, a sink-ish name scores sink.
    #[test]
    fn classify_source_and_sink_normal() {
        let src = classify_name("read_user_input");
        assert!(src.looks_like_source());
        assert!(src.source_score > 0.0 && src.sink_score == 0.0);

        let sink = classify_name("exec_sql_command");
        assert!(sink.looks_like_sink());
        assert!(sink.sink_score > 0.0);
    }

    // Robust: a neutral name leans neither way.
    #[test]
    fn neutral_name_is_unscored_robust() {
        let c = classify_name("compute_average");
        assert!(!c.looks_like_source() && !c.looks_like_sink());
        assert_eq!(c.source_score, 0.0);
        assert_eq!(c.sink_score, 0.0);
    }

    // Robust: an empty / symbol-only identifier never panics and scores zero.
    #[test]
    fn empty_identifier_is_safe_robust() {
        let c = classify_name("___");
        assert_eq!(c.source_score, 0.0);
        assert_eq!(c.sink_score, 0.0);
        assert!(split_identifier("").is_empty());
    }

    // Normal: a flow from a clearly-named source to a clearly-named sink is NOT
    // surprising (the names announce the danger).
    #[test]
    fn well_named_flow_not_surprising_normal() {
        assert!(!is_surprising("user_input", "exec_command"));
    }

    // Robust: a flow between two innocuously-named endpoints IS surprising —
    // exactly the case the dataflow catches but a reviewer misses.
    #[test]
    fn innocuous_flow_is_surprising_robust() {
        assert!(is_surprising("config_value", "apply_setting"));
    }

    // Robust: a surprising (innocuously-named) flow out-ranks even a fully
    // self-announcing source→sink flow — the boost dominates name evidence.
    #[test]
    fn surprising_flows_rank_first_robust() {
        let surprising = flow_priority("config_value", "apply_setting");
        // Maximally well-named: full source evidence + full sink evidence = 2.0.
        let well_named = flow_priority("user_input", "exec_command");
        assert!(surprising > well_named);
        // Among NON-surprising flows (here the source self-announces, so neither
        // is boosted), more name evidence ranks higher: exec_command (sink 1.0)
        // beats write_log (sink 0.5).
        let weaker_sink = flow_priority("user_input", "write_log");
        assert!(well_named > weaker_sink);
        assert!(weaker_sink < surprising);
    }
}
