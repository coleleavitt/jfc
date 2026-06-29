pub(crate) fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for term in query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|s| stem(s.trim()))
        .filter(|s| s.len() >= 3 && !is_stopword(s))
    {
        if !terms.iter().any(|existing| existing == &term) {
            terms.push(term);
        }
    }
    terms
}

pub(crate) fn score_text(text: &str, terms: &[String]) -> usize {
    if terms.is_empty() {
        return 0;
    }
    let text_lc = text.to_ascii_lowercase();
    text_lc
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|s| stem(s.trim()))
        .filter(|s| s.len() >= 3)
        .filter(|token| terms.iter().any(|term| term == token))
        .count()
}

pub(crate) fn best_line<'a>(text: &'a str, terms: &[String]) -> Option<&'a str> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .max_by_key(|line| score_text(line, terms))
        .filter(|line| score_text(line, terms) > 0)
}

fn stem(token: &str) -> String {
    let mut s = token.to_ascii_lowercase();
    if s.len() > 6 && s.ends_with("tion") {
        s.truncate(s.len() - 3);
        return s;
    }
    for suffix in ["ations", "ation", "ions", "ing", "ers", "ed", "es", "s"] {
        if s.len() > suffix.len() + 3 && s.ends_with(suffix) {
            s.truncate(s.len() - suffix.len());
            break;
        }
    }
    s
}

fn is_stopword(term: &str) -> bool {
    matches!(
        term,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "what"
            | "when"
            | "where"
            | "why"
            | "how"
            | "into"
            | "about"
            | "there"
            | "their"
            | "have"
            | "has"
            | "was"
            | "were"
            | "are"
            | "not"
            | "you"
            | "your"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_terms_dedupes_and_stems_normal() {
        assert_eq!(
            query_terms("the compaction compacted loops"),
            vec!["compact", "loop"]
        );
    }

    #[test]
    fn score_text_matches_stems_normal() {
        let terms = query_terms("compaction loops");
        assert!(score_text("compact retry loop compacted", &terms) >= 2);
    }
}
