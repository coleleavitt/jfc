use jfc_provider::ToolDef;

pub(super) fn intent_tool_matches(intent: &str, all: &[ToolDef], limit: usize) -> Vec<String> {
    let terms = intent_terms(intent);
    if terms.is_empty() {
        return Vec::new();
    }
    let explicit_commit_message_intent = explicitly_requests_commit_message(intent);

    let docs: Vec<(String, String)> = all
        .iter()
        .filter(|tool| !super::super::defs::is_model_hidden_builtin_tool_name(&tool.name))
        .filter(|tool| !is_commit_message_tool(&tool.name) || explicit_commit_message_intent)
        .map(|tool| {
            let schema = tool
                .input_schema
                .get("properties")
                .map(|value| value.to_string())
                .unwrap_or_default();
            (
                tool.name.clone(),
                format!("{} {} {}", tool.name, tool.description, schema),
            )
        })
        .collect();
    let index = jfc_core::ToolIndex::build(docs);

    let query = terms.join(" ");
    let hits = index.search(&query, limit);
    if !hits.is_empty() {
        return hits.into_iter().map(|(name, _score)| name).collect();
    }

    let mut scored: Vec<(usize, String)> = all
        .iter()
        .filter(|tool| !super::super::defs::is_model_hidden_builtin_tool_name(&tool.name))
        .filter(|tool| !is_commit_message_tool(&tool.name) || explicit_commit_message_intent)
        .filter_map(|tool| {
            let score = intent_score(tool, &terms);
            (score >= 4).then(|| (score, tool.name.clone()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, name)| name)
        .collect()
}

fn intent_score(tool: &ToolDef, terms: &[String]) -> usize {
    let name = tool.name.to_ascii_lowercase();
    let description = tool.description.to_ascii_lowercase();
    let schema = tool
        .input_schema
        .get("properties")
        .map(|value| value.to_string().to_ascii_lowercase())
        .unwrap_or_default();

    let mut score = 0usize;
    for term in terms {
        if name == *term {
            score += 10;
        } else if name.contains(term) {
            score += 6;
        }
        if description.contains(term) {
            score += 3;
        }
        if schema.contains(term) {
            score += 1;
        }
    }
    score
}

fn intent_terms(intent: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in intent
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
    {
        let term = raw.trim();
        if term.len() < 3 || INTENT_STOPWORDS.contains(&term) {
            continue;
        }
        terms.push(term.to_owned());
    }
    super::dedup_preserve_order(terms)
}

fn is_commit_message_tool(name: &str) -> bool {
    name.eq_ignore_ascii_case("SuggestCommitMessage")
        || name.eq_ignore_ascii_case("suggest_commit_message")
}

fn explicitly_requests_commit_message(intent: &str) -> bool {
    let lower = intent.to_ascii_lowercase();
    let normalized = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = normalized.trim();
    trimmed.contains("commit message")
        || trimmed.contains("conventional commit")
        || trimmed.contains("suggest commit")
        || trimmed.contains("generate commit")
        || trimmed.contains("write commit")
        || trimmed.contains("draft commit")
}

const INTENT_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "onto", "all", "any", "can",
    "could", "would", "should", "please", "thank", "you", "use", "using", "tool", "tools", "task",
    "tasks", "make", "made", "work", "works", "working", "need", "needs", "want", "wants", "about",
    "what", "when", "where", "why", "how", "fix", "add", "do", "run", "get", "set", "list", "show",
    "tell", "find", "read", "write", "edit", "update", "create", "delete", "remove",
];
