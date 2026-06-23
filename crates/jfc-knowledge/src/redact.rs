//! Stage 0 — deterministic, high-recall secret/PII redaction.
//!
//! Runs **before any session text is stored or shown to any extractor**. This is
//! the gate that makes the rest of the pipeline safe: poor recall here would let
//! credentials reach the DB (and, if an LLM extractor is ever enabled, a vendor).
//! Tool `output` is scrubbed harder than `input`, because command output (env
//! dumps, `cat .env`, `git remote -v`) is the primary leak vector.
//!
//! Deliberately regex-free (no extra dep): hand-rolled scanners over char
//! classes. Recall over precision — we would rather over-redact a token than
//! leak one.

/// Replacement marker for a redacted span.
const MARK: &str = "[REDACTED]";

/// Redact secrets/PII from a text span. `aggressive` (used for tool output)
/// additionally strips long high-entropy tokens that aren't a recognized format.
pub fn redact(text: &str, aggressive: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        out.push_str(&redact_line(line, aggressive));
    }
    out
}

fn redact_line(line: &str, aggressive: bool) -> String {
    // Whole-line redaction for obvious key/secret assignments.
    let lower = line.to_ascii_lowercase();
    for needle in [
        "-----begin",     // PEM private keys
        "authorization:", // headers
        "authorization\":",
    ] {
        if lower.contains(needle) {
            let nl = if line.ends_with('\n') { "\n" } else { "" };
            return format!("{MARK}{nl}");
        }
    }

    // Token-wise scan. `redact_next` carries a "the previous token was a secret
    // key with a dangling separator (`API_KEY:`)" signal so the *value* token,
    // separated by whitespace, is redacted too.
    let mut out = String::with_capacity(line.len());
    let mut tok = String::new();
    let mut redact_next = false;
    for ch in line.chars() {
        if ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';' | '(' | ')' | '<' | '>') {
            if !tok.is_empty() {
                if redact_next && !tok.is_empty() {
                    out.push_str(MARK);
                    redact_next = false;
                } else {
                    out.push_str(&redact_token(&tok, aggressive));
                    redact_next = dangling_secret_key(&tok);
                }
                tok.clear();
            }
            out.push(ch);
        } else {
            tok.push(ch);
        }
    }
    if !tok.is_empty() {
        if redact_next {
            out.push_str(MARK);
        } else {
            out.push_str(&redact_token(&tok, aggressive));
        }
    }
    out
}

/// A token like `API_KEY:` or `password=` — a secret key with an empty value,
/// signalling that the next whitespace-separated token is the secret value.
fn dangling_secret_key(tok: &str) -> bool {
    let stripped = tok.trim_end_matches([':', '=']);
    stripped.len() < tok.len() && is_secret_key(stripped)
}

fn redact_token(tok: &str, aggressive: bool) -> String {
    // key=value / key:value secrets.
    if let Some((k, v)) = split_kv(tok)
        && is_secret_key(k)
        && !v.is_empty()
    {
        return format!("{k}={MARK}");
    }
    // Known credential prefixes.
    if has_secret_prefix(tok) {
        return MARK.to_owned();
    }
    // JWT: three base64url segments separated by dots.
    if is_jwt(tok) {
        return MARK.to_owned();
    }
    // Email.
    if is_email(tok) {
        return MARK.to_owned();
    }
    // Home path normalization: /home/<user>/… or /Users/<user>/… → <HOME>/…
    if let Some(norm) = normalize_home(tok) {
        return norm;
    }
    // Aggressive: long high-entropy opaque tokens (likely secrets).
    if aggressive && is_high_entropy_token(tok) {
        return MARK.to_owned();
    }
    tok.to_owned()
}

fn split_kv(tok: &str) -> Option<(&str, &str)> {
    tok.split_once('=').or_else(|| tok.split_once(':'))
}

fn is_secret_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    const HINTS: &[&str] = &[
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "api-key",
        "access_key",
        "secret_key",
        "private_key",
        "auth",
        "credential",
        "session",
        "bearer",
        "client_secret",
    ];
    HINTS.iter().any(|h| k.contains(h))
}

fn has_secret_prefix(tok: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "sk-",
        "sk-ant-",
        "sk-proj-",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "github_pat_",
        "AKIA",
        "ASIA",
        "AIza",
        "ya29.",
        "xoxb-",
        "xoxp-",
        "glpat-",
    ];
    PREFIXES.iter().any(|p| tok.starts_with(p)) && tok.len() >= 12
}

fn is_jwt(tok: &str) -> bool {
    let parts: Vec<&str> = tok.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            p.len() >= 8
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

fn is_email(tok: &str) -> bool {
    let Some((local, domain)) = tok.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn normalize_home(tok: &str) -> Option<String> {
    for prefix in ["/home/", "/Users/"] {
        if let Some(rest) = tok.strip_prefix(prefix) {
            // Replace the username segment with <HOME>.
            let after_user = rest.find('/').map(|i| &rest[i..]).unwrap_or("");
            return Some(format!("<HOME>{after_user}"));
        }
    }
    None
}

/// Shannon-entropy heuristic for opaque tokens: long, mixed-charset, no spaces.
fn is_high_entropy_token(tok: &str) -> bool {
    if tok.len() < 24 {
        return false;
    }
    // Must look like an opaque blob: only base64/hex-ish chars.
    if !tok
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'))
    {
        return false;
    }
    let mut has_digit = false;
    let mut has_alpha = false;
    for c in tok.chars() {
        has_digit |= c.is_ascii_digit();
        has_alpha |= c.is_ascii_alphabetic();
    }
    if !(has_digit && has_alpha) {
        return false; // pure words / pure numbers aren't secrets
    }
    shannon_bits_per_char(tok) >= 3.5
}

fn shannon_bits_per_char(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    let n = s.chars().count() as f64;
    for c in s.chars() {
        *counts.entry(c).or_insert(0u32) += 1;
    }
    -counts
        .values()
        .map(|&c| {
            let p = c as f64 / n;
            p * p.log2()
        })
        .sum::<f64>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_key_prefixes_normal() {
        assert_eq!(
            redact("key sk-ant-abc123def456ghi here", false),
            "key [REDACTED] here"
        );
        assert!(redact("token=ghp_0123456789abcdefghij", false).contains("[REDACTED]"));
    }

    #[test]
    fn redacts_kv_secrets_normal() {
        assert_eq!(redact("password=hunter2", false), "password=[REDACTED]");
        assert!(redact("API_KEY: somevalue123", false).contains("[REDACTED]"));
    }

    #[test]
    fn redacts_jwt_and_email_normal() {
        let jwt = "eyJhbGciOiJI.eyJzdWIiOiIx.SflKxwRJSMeKKF2";
        assert_eq!(redact(jwt, false), "[REDACTED]");
        assert_eq!(
            redact("ping alice@example.com now", false),
            "ping [REDACTED] now"
        );
    }

    #[test]
    fn redacts_pem_line_normal() {
        assert_eq!(
            redact("-----BEGIN RSA PRIVATE KEY-----", false),
            "[REDACTED]"
        );
    }

    #[test]
    fn normalizes_home_path_normal() {
        assert_eq!(
            redact("/home/cole/secret/file.rs", false),
            "<HOME>/secret/file.rs"
        );
        assert_eq!(redact("/Users/alice/x", false), "<HOME>/x");
    }

    #[test]
    fn aggressive_strips_high_entropy_blob_robust() {
        let blob = "aZ9kQ2mX7pL4vR8nT1wY6cB3dF5gH0jK";
        // Non-aggressive keeps it (could be a hash in normal text); aggressive (tool output) strips.
        assert_eq!(redact(blob, false), blob);
        assert_eq!(redact(blob, true), "[REDACTED]");
    }

    #[test]
    fn leaves_ordinary_text_untouched_robust() {
        let s = "let x = compute(value) + 42;";
        assert_eq!(redact(s, true), s);
        assert_eq!(redact("use ripgrep not grep", true), "use ripgrep not grep");
    }
}
