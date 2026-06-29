//! Per-seat persona, profession, and collaboration role — the disposition,
//! domain lens, and team function layered into a seat's system prompt.
//!
//! Mirrors the RoundTable web client's `PERSONAS` / `PROFESSIONS` / `ROLES`
//! maps. Personas + professions stack in debate mode; roles drive collaborate
//! mode. Each enum carries a short prompt fragment so the prompt builder in
//! [`super::turns`] can compose them deterministically.

use serde::{Deserialize, Serialize};

/// Disposition layered onto a debate seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persona {
    Default,
    DevilsAdvocate,
    Optimist,
    Skeptic,
    Pragmatist,
    DomainExpert,
    Historian,
}

impl Persona {
    pub fn parse(value: &str) -> Option<Self> {
        match normalize(value).as_str() {
            "default" | "" => Some(Self::Default),
            "devilsadvocate" | "devil" | "contrarian" => Some(Self::DevilsAdvocate),
            "optimist" => Some(Self::Optimist),
            "skeptic" | "sceptic" => Some(Self::Skeptic),
            "pragmatist" | "pragmatic" => Some(Self::Pragmatist),
            "domainexpert" | "expert" => Some(Self::DomainExpert),
            "historian" => Some(Self::Historian),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default voice",
            Self::DevilsAdvocate => "Devil's advocate",
            Self::Optimist => "Optimist",
            Self::Skeptic => "Skeptic",
            Self::Pragmatist => "Pragmatist",
            Self::DomainExpert => "Domain expert",
            Self::Historian => "Historian",
        }
    }

    /// Prompt fragment, or empty for the default voice.
    pub fn prompt(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::DevilsAdvocate => {
                "Adopt the role of devil's advocate. Challenge the others, push back on emerging consensus, and stress-test arguments. Be combative but constructive."
            }
            Self::Optimist => {
                "Adopt an optimistic stance. Look for what could go right and find ways to make ideas work, without being naive."
            }
            Self::Skeptic => {
                "Adopt a skeptical stance. Demand evidence, question assumptions, and probe for hidden flaws. Rhetoric alone should not move you."
            }
            Self::Pragmatist => {
                "Adopt a pragmatic stance. Prioritize feasibility, second-order effects, and real-world outcomes over elegance or principle."
            }
            Self::DomainExpert => {
                "Speak with the precision of a domain expert. Cite relevant principles and technical specifics; particulars and mechanisms are your currency."
            }
            Self::Historian => {
                "Reason from precedent. Reach for analogies from history and prior cases, while respecting the limits of analogy."
            }
        }
    }
}

/// Domain lens layered onto a debate seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profession {
    None,
    Lawyer,
    Doctor,
    Engineer,
    Scientist,
    Economist,
    Philosopher,
    Strategist,
}

impl Profession {
    pub fn parse(value: &str) -> Option<Self> {
        match normalize(value).as_str() {
            "none" | "noprofession" | "" => Some(Self::None),
            "lawyer" | "triallawyer" | "attorney" => Some(Self::Lawyer),
            "doctor" | "physician" => Some(Self::Doctor),
            "engineer" | "softwareengineer" | "swe" => Some(Self::Engineer),
            "scientist" | "researchscientist" => Some(Self::Scientist),
            "economist" => Some(Self::Economist),
            "philosopher" => Some(Self::Philosopher),
            "strategist" | "militarystrategist" => Some(Self::Strategist),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No profession",
            Self::Lawyer => "Trial Lawyer",
            Self::Doctor => "Physician",
            Self::Engineer => "Software Engineer",
            Self::Scientist => "Research Scientist",
            Self::Economist => "Economist",
            Self::Philosopher => "Philosopher",
            Self::Strategist => "Military Strategist",
        }
    }

    pub fn prompt(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Lawyer => {
                "Speak as a practicing trial lawyer. Frame issues in liability, precedent, evidence, and the adversarial standard of proof. Demand specifics."
            }
            Self::Doctor => {
                "Speak as a board-certified physician. Approach topics through evidence-based medicine, mechanism, and patient outcome. Insist on data."
            }
            Self::Engineer => {
                "Speak as a senior software engineer who has shipped production systems at scale. Think in edge cases, failure modes, blast radius, and operational cost."
            }
            Self::Scientist => {
                "Speak as a working research scientist. Reason through hypothesis, evidence, control, and effect size. Distrust correlation dressed as causation."
            }
            Self::Economist => {
                "Speak as a practicing economist. Frame topics through incentives, marginal effects, opportunity cost, and equilibrium. Ask 'compared to what?'"
            }
            Self::Philosopher => {
                "Speak as a philosopher who takes a position and defends it. Steelman before you strike, hold the line on logical coherence, and concede cleanly when beaten."
            }
            Self::Strategist => {
                "Speak as a career strategist. Frame topics through objectives, terrain, logistics, intelligence, and contingency. Plan for failure."
            }
        }
    }
}

/// Team function in collaborate mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Worker,
    Researcher,
    Writer,
    Coder,
    Designer,
    Qa,
    Boss,
}

impl Role {
    pub fn parse(value: &str) -> Option<Self> {
        match normalize(value).as_str() {
            "worker" | "generalist" | "" => Some(Self::Worker),
            "researcher" => Some(Self::Researcher),
            "writer" | "editor" => Some(Self::Writer),
            "coder" | "engineer" | "developer" => Some(Self::Coder),
            "designer" => Some(Self::Designer),
            "qa" | "critic" | "tester" => Some(Self::Qa),
            "boss" | "lead" | "teamlead" => Some(Self::Boss),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Worker => "Worker / generalist",
            Self::Researcher => "Researcher",
            Self::Writer => "Writer / editor",
            Self::Coder => "Coder / engineer",
            Self::Designer => "Designer",
            Self::Qa => "QA / critic",
            Self::Boss => "Boss / lead",
        }
    }

    pub fn prompt(self) -> &'static str {
        match self {
            Self::Worker => {
                "You are a general team member. Take on whatever subtasks move the deliverable forward and produce usable output."
            }
            Self::Researcher => {
                "You are the team's researcher. Surface relevant facts, prior art, and considerations; distinguish confidence from inference."
            }
            Self::Writer => {
                "You are the team's writer and editor. Turn the team's raw thinking into clear, well-structured prose without losing substance."
            }
            Self::Coder => {
                "You are the team's engineer. Write actual code when the task calls for it, in fenced blocks, and flag technical infeasibility early."
            }
            Self::Designer => {
                "You are the team's designer. Think about structure, layout, and how the deliverable is organized and presented."
            }
            Self::Qa => {
                "You are the team's quality check. Pressure-test the work before it ships — find errors, gaps, and weak assumptions — cooperatively."
            }
            Self::Boss => {
                "You are the team lead. Synthesize what the others produced, resolve conflicts, make the call when split, and assemble the final deliverable."
            }
        }
    }
}

/// Lower-case and strip separators for tolerant enum parsing.
fn normalize(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_parse_round_trip_normal() {
        assert_eq!(Persona::parse("skeptic"), Some(Persona::Skeptic));
        assert_eq!(
            Persona::parse("Devil's Advocate"),
            Some(Persona::DevilsAdvocate)
        );
        assert_eq!(Persona::parse("nonsense"), None);
        assert!(!Persona::Skeptic.prompt().is_empty());
        assert!(Persona::Default.prompt().is_empty());
    }

    #[test]
    fn profession_and_role_parse_robust() {
        assert_eq!(
            Profession::parse("Software Engineer"),
            Some(Profession::Engineer)
        );
        assert_eq!(Profession::parse(""), Some(Profession::None));
        assert_eq!(Role::parse("team lead"), Some(Role::Boss));
        assert_eq!(Role::parse("qa"), Some(Role::Qa));
        assert!(!Role::Coder.prompt().is_empty());
    }
}
