use super::{Classification, Intent};

pub fn classify(message: &str) -> Classification {
    let lower = message.to_lowercase();

    // ── Doc-request intents (highest precision; checked first) ──
    // These trigger a one-shot toast suggesting `/plan` etc., so a
    // false positive is annoying but recoverable. We use high-bar
    // multi-word phrases ("draft a plan", not just "plan") to keep
    // the noise floor low.
    if let Some(intent) = classify_doc_intent(&lower) {
        return Classification {
            intent,
            confidence: 0.9,
        };
    }

    // ── Auto-Plan-Mode intent (planning posture, not a file) ──
    // Checked before graph intents because "should I refactor X" is
    // both a RefactorRisk signal AND an AutoPlanMode signal — the
    // permission flip is the higher-leverage answer, and the graph
    // injection still runs as a side-effect via the explicit
    // RefactorRisk classifier when the user follow-up confirms.
    if classify_auto_plan_mode(&lower) {
        return Classification {
            intent: Intent::AutoPlanModeRequest,
            confidence: 0.85,
        };
    }

    // ── Graph-flavored intents (high-precision) ──
    if let Some(intent) = classify_graph_intent(&lower) {
        return Classification {
            intent,
            confidence: 0.85,
        };
    }

    let mut scores: [(Intent, f32); 6] = [
        (Intent::Research, 0.0),
        (Intent::Implementation, 0.0),
        (Intent::Investigation, 0.0),
        (Intent::Fix, 0.0),
        (Intent::Evaluation, 0.0),
        (Intent::Chat, 0.0),
    ];

    // Research keywords
    for kw in &[
        "find",
        "search",
        "where",
        "which file",
        "locate",
        "look up",
        "grep",
        "show me",
    ] {
        if lower.contains(kw) {
            scores[0].1 += 1.0;
        }
    }

    // Implementation keywords
    for kw in &[
        "create",
        "add",
        "implement",
        "build",
        "write",
        "make",
        "generate",
        "new",
    ] {
        if lower.contains(kw) {
            scores[1].1 += 1.0;
        }
    }

    // Investigation keywords
    for kw in &[
        "explain",
        "how does",
        "what does",
        "understand",
        "trace",
        "follow",
        "read",
    ] {
        if lower.contains(kw) {
            scores[2].1 += 1.0;
        }
    }

    // Fix keywords
    for kw in &[
        "fix", "bug", "error", "broken", "failing", "crash", "issue", "wrong",
    ] {
        if lower.contains(kw) {
            scores[3].1 += 1.0;
        }
    }

    // Evaluation keywords
    for kw in &[
        "review", "check", "audit", "evaluate", "assess", "quality", "test",
    ] {
        if lower.contains(kw) {
            scores[4].1 += 1.0;
        }
    }

    // Find best match
    let total: f32 = scores.iter().map(|(_, score)| score).sum();
    if total == 0.0 {
        return Classification {
            intent: Intent::Chat,
            confidence: 0.5,
        };
    }

    // Defensive: `partial_cmp` on f32 returns `None` only for NaN, which the
    // current keyword-counting scoring path can't produce (all inputs are
    // u32 → f32 conversions). We still fall back to `Ordering::Equal`
    // instead of unwrapping so any future scoring change that introduces
    // floating-point math (e.g. weighted normalization, decay) won't
    // panic at the comparison site if it accidentally yields NaN.
    let (best_intent, best_score) = scores
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap();

    let confidence = best_score / total;
    if confidence < 0.4 {
        Classification {
            intent: Intent::Chat,
            confidence: 0.3,
        }
    } else {
        Classification {
            intent: *best_intent,
            confidence,
        }
    }
}

/// Detect graph-flavored intents from already-lowercased text.
///
/// Returns `None` to let the generic scorer take over. False positives
/// here are cheap (one extra graph query); false negatives are
/// expensive (the model never gets the structural hint).
fn classify_graph_intent(lower: &str) -> Option<Intent> {
    // RefactorRisk first — overlaps with ImpactAnalysis on "what
    // breaks" but the "safe to" phrasing is the stronger signal.
    if contains_any(
        lower,
        &[
            "safe to refactor",
            "safe to rename",
            "safe to change",
            "safe to move",
            "risk of refactor",
            "risk of rename",
            "risk of changing",
            "risk of removing",
        ],
    ) {
        return Some(Intent::RefactorRisk);
    }

    // DependencyTrace — outgoing call traversal.
    if contains_any(
        lower,
        &[
            "what does ",
            "callees",
            "reachable from",
            "trace from ",
            " trace to ",
            "what calls does",
            "outgoing calls",
        ],
    ) && contains_any(lower, &[" call", "calls", "invoke", "trace"])
    {
        return Some(Intent::DependencyTrace);
    }

    // ImpactAnalysis — incoming call traversal / blast radius.
    if contains_any(
        lower,
        &[
            "depends on",
            "callers of",
            "callers for",
            "impact of",
            "affected by",
            "what would break",
            "what breaks if",
            "what will break",
            "ripple",
            "blast radius",
            "who uses",
            "who calls",
        ],
    ) {
        return Some(Intent::ImpactAnalysis);
    }

    // EntrypointDiscovery — locate program entry / public API surface.
    if contains_any(
        lower,
        &[
            "entrypoint",
            "entry point",
            "public api",
            "main function",
            "where does it start",
            "where does this start",
            "where does the program start",
            "where do we start",
            "find main",
        ],
    ) {
        return Some(Intent::EntrypointDiscovery);
    }

    None
}

/// Detect a project-doc request from already-lowercased text. Returns
/// the matching `Doc*Request` intent, or `None` to let later
/// classifiers run.
///
/// The phrasings are deliberately multi-word — matching bare "plan" or
/// "usage" would fire on almost every coding prompt. We require an
/// action verb ("draft", "write", "update", "generate", "create",
/// "refresh") paired with the doc noun, OR a direct question about the
/// doc's subject ("what's our parity status").
fn classify_doc_intent(lower: &str) -> Option<Intent> {
    let has_doc_verb = contains_any(
        lower,
        &[
            "draft ",
            "write ",
            "update ",
            "generate ",
            "create ",
            "refresh ",
            "make ",
        ],
    );

    // PARITY — also matches the direct status question, which has no
    // verb ("what's our parity status with upstream").
    if contains_any(lower, &["parity.md", "parity status", "parity report"])
        || (has_doc_verb && lower.contains("parity"))
    {
        return Some(Intent::DocParityRequest);
    }

    // ROADMAP
    if lower.contains("roadmap.md") || (has_doc_verb && lower.contains("roadmap")) {
        return Some(Intent::DocRoadmapRequest);
    }

    // PHILOSOPHY
    if lower.contains("philosophy.md")
        || (has_doc_verb && lower.contains("philosophy"))
        || contains_any(lower, &["philosophy doc", "project philosophy"])
    {
        return Some(Intent::DocPhilosophyRequest);
    }

    // USAGE — guard against "usage" appearing in "token usage" / "memory
    // usage" / "cpu usage" by requiring the doc verb or the explicit
    // file / "usage guide" / "usage docs" phrasing.
    if lower.contains("usage.md")
        || contains_any(lower, &["usage guide", "usage doc", "usage instructions"])
        || (has_doc_verb && lower.contains("usage") && !lower.contains(" usage of "))
    {
        return Some(Intent::DocUsageRequest);
    }

    // PLAN — most ambiguous, so the bar is highest. Require an explicit
    // "plan.md" OR a verb+plan pairing that isn't "plan mode" / "plan to".
    if lower.contains("plan.md")
        || contains_any(
            lower,
            &[
                "draft a plan",
                "write a plan",
                "write the plan",
                "draft the plan",
                "update the plan",
                "create a plan",
                "make a plan",
                "generate a plan",
                "refresh the plan",
                "implementation plan document",
            ],
        )
    {
        return Some(Intent::DocPlanRequest);
    }

    None
}

/// Detect a planning-posture request from already-lowercased text.
/// Returns `true` when the prompt reads as "think before you act" —
/// the dispatcher uses this (gated by `JFC_AUTO_PLAN_MODE=1`) to flip
/// the session into Plan permission mode.
///
/// Kept separate from `classify_doc_intent` because this is about the
/// *permission posture* for the upcoming work, not a request to write
/// a file. False positives flip the agent to read-only mid-session,
/// which is why the dispatcher gates it behind an opt-in env var.
pub fn classify_auto_plan_mode(lower: &str) -> bool {
    // Strong design / planning verbs paired with scope.
    let design_phrases = [
        "design a ",
        "design the ",
        "how should i implement",
        "how should we implement",
        "how would you implement",
        "plan the refactor",
        "plan the rewrite",
        "plan out ",
        "come up with a plan",
        "think through ",
        "architect the ",
        "architecture for ",
        "what's the best way to implement",
        "what is the best way to implement",
        "should i refactor",
        "should we refactor",
        "approach for refactoring",
        "before you start",
        "before we start",
        "don't write code yet",
        "do not write code yet",
        "just plan",
        "only plan",
        "plan first",
    ];
    contains_any(lower, &design_phrases)
}

#[inline]
fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}
