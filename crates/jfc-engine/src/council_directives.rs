//! Bot control directives for the RoundTable-style [`CouncilSession`].
//!
//! A council seat ends its turn with zero or more *control lines* that the
//! orchestrator interprets and strips from the visible reply — mirroring the
//! RoundTable web client's directive grammar (`STANCE`, `CHALLENGE`, `DM`,
//! `CALL KICK VOTE`, `CALL OPERATOR VOTE`, `LEAVE TABLE`, `FLAG CLAIM`,
//! `GENERATE IMAGE`, `PASS`, `ASSIGN`).
//!
//! This module is intentionally pure and synchronous: it only parses text into
//! structured [`Directive`]s and returns the cleaned prose. Name resolution,
//! allowance accounting, and side effects live in
//! [`crate::council_session`], which knows the live roster.
//!
//! [`CouncilSession`]: crate::council_session::CouncilSession

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// A seat's declared stance on the central question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    For,
    Against,
    Undecided,
}

impl Stance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::For => "FOR",
            Self::Against => "AGAINST",
            Self::Undecided => "UNDECIDED",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "FOR" => Some(Self::For),
            "AGAINST" => Some(Self::Against),
            "UNDECIDED" => Some(Self::Undecided),
            _ => None,
        }
    }
}

/// One control directive emitted by a seat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Directive {
    /// `STANCE: FOR|AGAINST|UNDECIDED | 0-100`
    Stance {
        position: Stance,
        confidence: Option<u8>,
    },
    /// `CHALLENGE: <Name> | <question>`
    Challenge { target: String, question: String },
    /// `DM @<Name>: <opening>` (target may be `Operator`).
    Dm { target: String, opening: String },
    /// `CALL KICK VOTE: <Name>`
    KickVote { target: String },
    /// `CALL OPERATOR VOTE: <reason>`
    OperatorVote { reason: String },
    /// `LEAVE TABLE: <reason>`
    LeaveTable { reason: String },
    /// `FLAG CLAIM: <Name> | <claim>` (name optional).
    FlagClaim {
        target: Option<String>,
        claim: String,
    },
    /// `GENERATE IMAGE: <prompt>` (optional trailing aspect ratio).
    GenerateImage {
        prompt: String,
        aspect_ratio: Option<String>,
    },
    /// `PASS` or `PASS: <reason>`
    Pass { reason: Option<String> },
    /// `ASSIGN @<Name>: <role-or-profession>`
    Assign { target: String, value: String },
}

impl Directive {
    /// `true` when the DM is addressed to the operator rather than a seat.
    pub fn is_operator_dm(target: &str) -> bool {
        matches!(
            target.trim().to_ascii_lowercase().as_str(),
            "operator" | "op" | "you" | "the operator" | "human"
        )
    }
}

/// A parsed seat reply: the visible prose with all control lines removed, plus
/// the structured directives recovered from it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedReply {
    /// The reply with every recognised directive line stripped.
    pub cleaned: String,
    pub directives: Vec<Directive>,
}

impl ParsedReply {
    pub fn stance(&self) -> Option<(Stance, Option<u8>)> {
        self.directives.iter().find_map(|d| match d {
            Directive::Stance {
                position,
                confidence,
            } => Some((*position, *confidence)),
            _ => None,
        })
    }

    pub fn passed(&self) -> Option<Option<&str>> {
        self.directives.iter().find_map(|d| match d {
            Directive::Pass { reason } => Some(reason.as_deref()),
            _ => None,
        })
    }

    pub fn first<'a>(&'a self, pred: impl Fn(&&'a Directive) -> bool) -> Option<&'a Directive> {
        self.directives.iter().find(pred)
    }
}

// ── Regexes (compiled once) ────────────────────────────────────────────────
// Anchored to a line start with optional leading markdown/quote markers. We use
// `[ \t>*_]*` (not `\s*`) so the leading class can't swallow newlines.

static RE_STANCE: LazyLock<Regex> = LazyLock::new(|| {
    // Accept up to 4 digits so an out-of-range confidence (e.g. 1000) still
    // matches; the value is clamped to 100 downstream rather than discarding
    // the whole STANCE line.
    Regex::new(r"(?im)^[ \t>*_]*STANCE\s*:\s*(FOR|AGAINST|UNDECIDED)\s*\|\s*(\d{1,4})\s*%?\s*$")
        .expect("stance regex")
});
static RE_CHALLENGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t>*_]*CHALLENGE\s*:\s*([^|\n]+?)\s*\|\s*([^\n]+?)\s*$")
        .expect("challenge regex")
});
static RE_DM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t>*_]*DM\s+@?\s*([^:\n]+?)\s*:\s*([^\n]+?)\s*$").expect("dm regex")
});
static RE_KICK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t>*_]*CALL\s+KICK\s+VOTE\s*[:\-]?\s*([^\n]+?)\s*$").expect("kick regex")
});
static RE_OP_VOTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t>*_]*CALL\s+OPERATOR\s+VOTE\s*[:\-]?\s*([^\n]*?)\s*$")
        .expect("operator vote regex")
});
static RE_LEAVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t>*_]*LEAVE\s+TABLE\s*[:\-]?\s*([^\n]*?)\s*$").expect("leave regex")
});
static RE_FLAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t>*_]*FLAG\s+CLAIM\s*[:\-]?\s*([^\n]+?)\s*$").expect("flag regex")
});
static RE_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t>*_]*GENERATE\s+IMAGE\s*[:\-]?\s*([^\n]+?)\s*$").expect("image regex")
});
static RE_PASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t>*_]*PASS(?:\s+TURN)?\s*([:\-]\s*[^\n]*)?\s*$").expect("pass regex")
});
static RE_ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t>*_]*ASSIGN\s+@?\s*([^:\n]+?)\s*:\s*([^\n]+?)\s*$")
        .expect("assign regex")
});
static RE_ASPECT_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:aspect(?:\s*ratio)?|ar)\s*[:\-]?\s*[\[(]?\s*(\d{1,2}:\d{1,2})\s*[\])]?")
        .expect("aspect label regex")
});
static RE_ASPECT_TRAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s,;—-])(\d{1,2}:\d{1,2})\s*$").expect("aspect trail regex")
});

const VALID_RATIOS: &[&str] = &[
    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9", "1:4", "4:1", "1:8",
    "8:1",
];

fn strip_wrapping(s: &str) -> String {
    s.trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '`' | '*' | '_'))
        .trim()
        .to_owned()
}

/// Accumulator threaded through the per-directive parsers: collects the matched
/// line spans (to strip from the prose) and the structured directives.
struct DirectiveSink {
    directives: Vec<(usize, Directive)>,
    spans: Vec<(usize, usize)>,
}

impl DirectiveSink {
    fn new() -> Self {
        Self {
            directives: Vec::new(),
            spans: Vec::new(),
        }
    }

    /// Record a matched full-line span and, when `make` yields one, its directive.
    fn capture(&mut self, m: regex::Match<'_>, directive: Option<Directive>) {
        let start = m.start();
        self.spans.push((m.start(), m.end()));
        if let Some(d) = directive {
            self.directives.push((start, d));
        }
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max - 1).collect::<String>() + "…"
    } else {
        s.to_owned()
    }
}

fn parse_stance(reply: &str, sink: &mut DirectiveSink) {
    for c in RE_STANCE.captures_iter(reply) {
        let directive = Stance::parse(&c[1]).map(|position| Directive::Stance {
            position,
            confidence: c[2].parse::<u32>().ok().map(|n| n.min(100) as u8),
        });
        sink.capture(c.get(0).unwrap(), directive);
    }
}

fn parse_challenge(reply: &str, sink: &mut DirectiveSink) {
    for c in RE_CHALLENGE.captures_iter(reply) {
        let target = strip_wrapping(&c[1]);
        let question = truncate_chars(&strip_wrapping(&c[2]), 240);
        let directive = (!target.is_empty() && !question.is_empty())
            .then_some(Directive::Challenge { target, question });
        sink.capture(c.get(0).unwrap(), directive);
    }
}

fn parse_dm(reply: &str, sink: &mut DirectiveSink) {
    for c in RE_DM.captures_iter(reply) {
        let target = strip_wrapping(&c[1]);
        let opening = truncate_chars(&strip_wrapping(&c[2]), 400);
        let directive = (!target.is_empty() && !opening.is_empty())
            .then_some(Directive::Dm { target, opening });
        sink.capture(c.get(0).unwrap(), directive);
    }
}

fn parse_votes(reply: &str, sink: &mut DirectiveSink) {
    for c in RE_KICK.captures_iter(reply) {
        let target = strip_wrapping(&c[1]);
        let directive = (!target.is_empty()).then_some(Directive::KickVote { target });
        sink.capture(c.get(0).unwrap(), directive);
    }
    for c in RE_OP_VOTE.captures_iter(reply) {
        let reason = strip_wrapping(&c[1]);
        sink.capture(c.get(0).unwrap(), Some(Directive::OperatorVote { reason }));
    }
    for c in RE_LEAVE.captures_iter(reply) {
        let reason = strip_wrapping(&c[1]);
        sink.capture(c.get(0).unwrap(), Some(Directive::LeaveTable { reason }));
    }
}

fn parse_flag(reply: &str, sink: &mut DirectiveSink) {
    for c in RE_FLAG.captures_iter(reply) {
        let body = strip_wrapping(&c[1]);
        let (target, claim) = match body.split_once('|') {
            Some((name, claim)) => {
                let name = strip_wrapping(name);
                ((!name.is_empty()).then_some(name), strip_wrapping(claim))
            }
            None => (None, body),
        };
        let claim = claim.chars().take(300).collect::<String>();
        let directive = (!claim.is_empty()).then_some(Directive::FlagClaim { target, claim });
        sink.capture(c.get(0).unwrap(), directive);
    }
}

fn parse_assigns(reply: &str, sink: &mut DirectiveSink) {
    for c in RE_ASSIGN.captures_iter(reply) {
        let target = strip_wrapping(&c[1]);
        let value = strip_wrapping(&c[2]);
        let directive = (!target.is_empty() && !value.is_empty())
            .then_some(Directive::Assign { target, value });
        sink.capture(c.get(0).unwrap(), directive);
    }
}

fn parse_image(reply: &str, sink: &mut DirectiveSink) {
    for c in RE_IMAGE.captures_iter(reply) {
        let (prompt, aspect_ratio) = split_image_prompt(&strip_wrapping(&c[1]));
        let directive = (prompt.chars().count() >= 3).then_some(Directive::GenerateImage {
            prompt,
            aspect_ratio,
        });
        sink.capture(c.get(0).unwrap(), directive);
    }
}

fn parse_pass(reply: &str, sink: &mut DirectiveSink) {
    for c in RE_PASS.captures_iter(reply) {
        let reason = c
            .get(1)
            .map(|g| {
                g.as_str()
                    .trim_start_matches([' ', ':', '-'])
                    .trim()
                    .trim_end_matches(['"', '\'', '`', '*', '_'])
                    .to_owned()
            })
            .filter(|r| !r.is_empty());
        sink.capture(c.get(0).unwrap(), Some(Directive::Pass { reason }));
    }
}

/// Parse every recognised directive from `reply`, returning the cleaned prose
/// and the structured directives in source order.
pub fn parse_directives(reply: &str) -> ParsedReply {
    let mut sink = DirectiveSink::new();
    parse_stance(reply, &mut sink);
    parse_challenge(reply, &mut sink);
    parse_dm(reply, &mut sink);
    parse_votes(reply, &mut sink);
    parse_flag(reply, &mut sink);
    parse_assigns(reply, &mut sink);
    parse_image(reply, &mut sink);
    parse_pass(reply, &mut sink);
    sink.directives.sort_by_key(|(start, _)| *start);
    ParsedReply {
        cleaned: strip_spans(reply, &mut sink.spans),
        directives: sink
            .directives
            .into_iter()
            .map(|(_, directive)| directive)
            .collect(),
    }
}

/// Pull an optional trailing/labelled aspect ratio out of an image prompt,
/// returning the cleaned prompt and the ratio when one is recognised.
fn split_image_prompt(raw: &str) -> (String, Option<String>) {
    let valid = |r: &str| VALID_RATIOS.contains(&r);
    if let Some(c) = RE_ASPECT_LABEL.captures(raw) {
        let ratio = c[1].to_owned();
        if valid(&ratio) {
            let full = c.get(0).unwrap().as_str();
            let cleaned = raw.replacen(full, " ", 1);
            return (tidy_prompt(&cleaned), Some(ratio));
        }
    }
    if let Some(c) = RE_ASPECT_TRAIL.captures(raw) {
        let ratio = c[1].to_owned();
        if valid(&ratio) {
            let full = c.get(0).unwrap().as_str();
            let cleaned = raw.replacen(full, "", 1);
            return (tidy_prompt(&cleaned), Some(ratio));
        }
    }
    (raw.trim().to_owned(), None)
}

fn tidy_prompt(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches([',', ';', ':', '-', ' '])
        .trim()
        .to_owned()
}

/// Remove the matched directive spans from `text` and tidy whitespace.
fn strip_spans(text: &str, spans: &mut Vec<(usize, usize)>) -> String {
    if spans.is_empty() {
        return text.trim_end().to_owned();
    }
    spans.sort_by_key(|(start, _)| *start);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (start, end) in spans.iter().copied() {
        if start < cursor {
            continue; // overlapping match already covered
        }
        out.push_str(&text[cursor..start]);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    // Collapse runs of blank lines the strip may have left behind.
    let collapsed = out
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    collapsed
        .split("\n\n\n")
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stance_normal() {
        let p = parse_directives("I think the answer is yes.\nSTANCE: FOR | 70");
        assert_eq!(p.stance(), Some((Stance::For, Some(70))));
        assert_eq!(p.cleaned, "I think the answer is yes.");
    }

    #[test]
    fn stance_clamps_confidence_robust() {
        let p = parse_directives("body\nSTANCE: AGAINST | 250");
        assert_eq!(p.stance(), Some((Stance::Against, Some(100))));
        // 4-digit confidence still parses (then clamps) rather than dropping the
        // whole STANCE line.
        let big = parse_directives("body\nSTANCE: FOR | 1000");
        assert_eq!(big.stance(), Some((Stance::For, Some(100))));
        assert_eq!(big.cleaned, "body");
    }

    #[test]
    fn parses_challenge_normal() {
        let p = parse_directives("You dodged.\nCHALLENGE: Claude | What is the cost at 100x?");
        let d = p
            .directives
            .iter()
            .find(|d| matches!(d, Directive::Challenge { .. }));
        assert_eq!(
            d,
            Some(&Directive::Challenge {
                target: "Claude".into(),
                question: "What is the cost at 100x?".into()
            })
        );
        assert!(!p.cleaned.contains("CHALLENGE"));
    }

    #[test]
    fn parses_dm_and_operator_dm_robust() {
        let p = parse_directives("hi\nDM @Gemini: let's align");
        assert_eq!(
            p.directives[0],
            Directive::Dm {
                target: "Gemini".into(),
                opening: "let's align".into()
            }
        );
        let p2 = parse_directives("private\nDM @Operator: a concern");
        if let Directive::Dm { target, .. } = &p2.directives[0] {
            assert!(Directive::is_operator_dm(target));
        } else {
            panic!("expected DM");
        }
    }

    #[test]
    fn parses_kick_and_operator_vote_normal() {
        let p = parse_directives("disruptive\nCALL KICK VOTE: Grok");
        assert_eq!(
            p.directives[0],
            Directive::KickVote {
                target: "Grok".into()
            }
        );
        let p2 = parse_directives("bad faith\nCALL OPERATOR VOTE: manipulating us");
        assert_eq!(
            p2.directives[0],
            Directive::OperatorVote {
                reason: "manipulating us".into()
            }
        );
    }

    #[test]
    fn parses_flag_with_and_without_name_robust() {
        let named = parse_directives("FLAG CLAIM: GPT | The benchmark was 9.2s");
        assert_eq!(
            named.directives[0],
            Directive::FlagClaim {
                target: Some("GPT".into()),
                claim: "The benchmark was 9.2s".into()
            }
        );
        let anon = parse_directives("FLAG CLAIM: that number is wrong");
        assert_eq!(
            anon.directives[0],
            Directive::FlagClaim {
                target: None,
                claim: "that number is wrong".into()
            }
        );
    }

    #[test]
    fn parses_generate_image_with_ratio_robust() {
        let p = parse_directives("here it is\nGENERATE IMAGE: a city skyline at dusk, 9:16");
        assert_eq!(
            p.directives[0],
            Directive::GenerateImage {
                prompt: "a city skyline at dusk".into(),
                aspect_ratio: Some("9:16".into())
            }
        );
    }

    #[test]
    fn parses_pass_and_assign_normal() {
        let p = parse_directives("PASS: nothing to add");
        assert_eq!(
            p.directives[0],
            Directive::Pass {
                reason: Some("nothing to add".into())
            }
        );
        let a = parse_directives("ASSIGN @Claude: lawyer\nASSIGN @GPT: engineer");
        let assigns: Vec<_> = a
            .directives
            .iter()
            .filter(|d| matches!(d, Directive::Assign { .. }))
            .collect();
        assert_eq!(assigns.len(), 2);
    }

    #[test]
    fn recovers_multiple_directives_and_strips_all_robust() {
        let p = parse_directives(
            "Opening.\n\
             FLAG CLAIM: Alpha | first claim\n\
             DM @Beta: private note\n\
             FLAG CLAIM: Gamma | second claim\n\
             CHALLENGE: Delta | answer this?\n\
             PASS: done",
        );

        assert_eq!(p.directives.len(), 5);
        assert!(matches!(p.directives[0], Directive::FlagClaim { .. }));
        assert!(matches!(p.directives[1], Directive::Dm { .. }));
        assert!(matches!(p.directives[2], Directive::FlagClaim { .. }));
        assert!(matches!(p.directives[3], Directive::Challenge { .. }));
        assert!(matches!(p.directives[4], Directive::Pass { .. }));
        for marker in ["FLAG CLAIM", "DM @", "CHALLENGE", "PASS"] {
            assert!(
                !p.cleaned.contains(marker),
                "directive marker leaked into cleaned transcript: {}",
                p.cleaned
            );
        }
        assert_eq!(p.cleaned, "Opening.");
    }

    #[test]
    fn no_directives_leaves_text_intact_normal() {
        let p = parse_directives("Just a normal paragraph with no controls.");
        assert!(p.directives.is_empty());
        assert_eq!(p.cleaned, "Just a normal paragraph with no controls.");
    }
}
