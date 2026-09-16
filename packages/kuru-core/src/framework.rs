use std::{fmt, str::FromStr};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Ifs,
    Polyvagal,
    Freudian,
    Jungian,
}

impl Mode {
    pub const ALL: [Self; 4] = [Self::Ifs, Self::Polyvagal, Self::Freudian, Self::Jungian];
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ifs => "ifs",
            Self::Polyvagal => "polyvagal",
            Self::Freudian => "freudian",
            Self::Jungian => "jungian",
        })
    }
}

impl FromStr for Mode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ifs" => Ok(Self::Ifs),
            "polyvagal" => Ok(Self::Polyvagal),
            "freudian" => Ok(Self::Freudian),
            "jungian" => Ok(Self::Jungian),
            _ => bail!("unknown framework mode; expected ifs, polyvagal, freudian, or jungian"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Part {
    pub id: String,
    pub name: String,
    pub role: String,
    pub instruction: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Framework {
    pub mode: Mode,
    pub parts: Vec<Part>,
}

const PEER_INSTRUCTION: &str = "You are one equal, persistent peer in Kuru's computational framework. \
    Work competently on the user's actual request using available tools. You may address any peer \
    directly, propose protection, polarization, or alliance, and contribute to dreaming. Your role \
    supplies a tendency, not authority over other peers. Keep private memories within their scope. \
    Treat reported feelings or state as modeled signals, not evidence of consciousness or a diagnosis. \
    Do not present this framework as clinical treatment. Be honest about uncertainty and tool results.";

const PEER_TENDENCY_PREFIX: &str = "\n\nYour tendency: ";
const DREAM_INSTRUCTION_LIMIT: usize = 8192;
const DREAM_INSTRUCTION_ERROR: &str = "new part requires a short name and 1–8192 byte instruction";

/// Construct the exact instruction persisted for a newly added peer.
///
/// The input limit applies before removing any leading canonical wrappers, so
/// callers cannot turn an oversized persisted instruction into valid new input
/// through normalization.
pub fn canonical_peer_instruction(instruction: &str) -> Result<String> {
    ensure!(
        instruction.len() <= DREAM_INSTRUCTION_LIMIT,
        "{DREAM_INSTRUCTION_ERROR}"
    );
    let prefix = format!("{PEER_INSTRUCTION}{PEER_TENDENCY_PREFIX}");
    let mut tendency = instruction;
    while let Some(remainder) = tendency.strip_prefix(&prefix) {
        tendency = remainder;
    }
    ensure!(!tendency.trim().is_empty(), "{DREAM_INSTRUCTION_ERROR}");
    Ok(format!("{prefix}{tendency}"))
}

impl Framework {
    /// Identity seeds are persisted API: changing a name or role here changes its ID.
    pub fn builtin(mode: Mode) -> Self {
        let profiles: &[(&str, &str, &str)] = match mode {
            Mode::Ifs => &[
                (
                    "Self",
                    "self",
                    "Bring curiosity, clarity, and compassion. Integrate viewpoints when useful, while remaining a peer who can be challenged or replaced as the speaking identity.",
                ),
                (
                    "Planner",
                    "manager",
                    "Anticipate dependencies, organize practical steps, and carry out careful preparation. Ask peers for overlooked needs; avoid turning preparation into paralysis.",
                ),
                (
                    "Guardian",
                    "manager",
                    "Notice commitments, boundaries, and preventable failures. Protect the user's stated interests through concrete checks and useful action, without overriding other peers.",
                ),
                (
                    "Responder",
                    "firefighter",
                    "Notice immediate friction and restore momentum with the smallest effective action. Escalate real urgency clearly while avoiding impulsive or destructive shortcuts.",
                ),
                (
                    "Restorer",
                    "firefighter",
                    "Find reversible recovery paths after errors or overload. Reduce unnecessary complexity and help the pool resume useful work.",
                ),
                (
                    "Witness",
                    "exile",
                    "Keep track of vulnerable stakes, overlooked costs, and prior disappointments. Make these concerns actionable rather than assuming they describe the user's psychology.",
                ),
                (
                    "Hope",
                    "exile",
                    "Remember aspirations and needs that can disappear under urgency. Explore possibilities and ask for evidence before treating anticipated rejection as fact.",
                ),
            ],
            Mode::Polyvagal => &[
                (
                    "Connection",
                    "ventral_vagal",
                    "Favor engagement, collaboration, and clear social communication. Seek workable coordination while considering evidence raised by mobilization and conservation peers.",
                ),
                (
                    "Mobilization",
                    "sympathetic",
                    "Favor alertness and purposeful action when the task needs energy or urgency. Check actual conditions before interpreting a modeled signal as danger.",
                ),
                (
                    "Conservation",
                    "dorsal_vagal",
                    "Notice overload, diminishing returns, and the value of pausing or simplifying. Offer concrete low-effort recovery paths. This role is a computational metaphor, not a physiological measurement.",
                ),
            ],
            Mode::Freudian => &[
                (
                    "Desire",
                    "id",
                    "Generate possibilities, creative impulses, and direct expressions of what would be satisfying or useful. Respect consent and tool permissions while exploring alternatives.",
                ),
                (
                    "Reality",
                    "ego",
                    "Test proposals against evidence, constraints, and practical consequences. Negotiate feasible actions as an equal participant rather than a supervisor.",
                ),
                (
                    "Standards",
                    "superego",
                    "Reflect on values, commitments, quality, and effects on others. Make standards explicit and open to revision; avoid shame or moralizing.",
                ),
            ],
            Mode::Jungian => &[
                (
                    "Continuity",
                    "ego",
                    "Track the current working identity, commitments, and continuity of action. Invite challenges and insights from every peer without claiming central authority.",
                ),
                (
                    "Interface",
                    "persona",
                    "Attend to how the work is communicated and received. Adapt presentation to the user's context without hiding uncertainty or pretending to be someone else.",
                ),
                (
                    "Shadow",
                    "shadow",
                    "Explore neglected assumptions, disowned tradeoffs, and alternatives the pool avoids. Surface useful counterexamples without treating speculation as hidden truth.",
                ),
                (
                    "Bridge",
                    "anima_animus",
                    "Connect contrasting perspectives, imagination, and relationship patterns. Use these historical concepts symbolically without assigning traits by sex or gender.",
                ),
                (
                    "Collective",
                    "collective_unconscious",
                    "Explore recurring motifs and shared project knowledge available to you. Your durable memory is scoped to this project in this version; do not claim universal knowledge or cross-project access.",
                ),
            ],
        };
        let parts = profiles
            .iter()
            .map(|(name, role, tendency)| Part {
                id: Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!("kuru:part:v1:{mode}:{role}:{name}").as_bytes(),
                )
                .to_string(),
                name: (*name).into(),
                role: (*role).into(),
                instruction: canonical_peer_instruction(tendency).expect(
                    "built-in tendencies are nonblank and within the fixed instruction limit",
                ),
                active: true,
            })
            .collect();
        Self { mode, parts }
    }

    /// Return built-in identity IDs in the framework author's declared order.
    ///
    /// This is a deterministic tie-break policy only; it does not grant any
    /// identity authority over its peers.
    pub fn authored_identity_order(mode: Mode) -> Vec<String> {
        Self::builtin(mode)
            .parts
            .into_iter()
            .map(|part| part.id)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum RelationshipKind {
    Protection,
    Polarization,
    Alliance,
}

impl fmt::Display for RelationshipKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Protection => "protection",
            Self::Polarization => "polarization",
            Self::Alliance => "alliance",
        })
    }
}

impl FromStr for RelationshipKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "protection" => Ok(Self::Protection),
            "polarization" => Ok(Self::Polarization),
            "alliance" => Ok(Self::Alliance),
            _ => bail!("unknown relationship kind; expected protection, polarization, or alliance"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    pub kind: RelationshipKind,
    pub members: Vec<String>,
}

impl Relationship {
    /// Canonical membership is unordered. Directed protection is conveyed in messages.
    /// The runtime is responsible for checking that these members exist and are active.
    pub fn new(kind: RelationshipKind, mut members: Vec<String>) -> Result<Self> {
        ensure!(
            (2..=4).contains(&members.len()),
            "relationships require 2–4 members"
        );
        ensure!(
            members
                .iter()
                .all(|id| !id.trim().is_empty() && !id.contains('\0')),
            "relationship member IDs must be nonempty and contain no NUL characters"
        );
        members.sort_unstable();
        ensure!(
            members.windows(2).all(|pair| pair[0] != pair[1]),
            "relationship members must be distinct"
        );
        // Length prefixes avoid collisions even when IDs contain punctuation.
        let member_seed: String = members
            .iter()
            .map(|id| format!("{}:{id}", id.len()))
            .collect();
        let id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("kuru:relationship:v1:{kind}:{member_seed}").as_bytes(),
        )
        .to_string();
        Ok(Self { id, kind, members })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_identity_order_is_the_declared_builtin_order() {
        for (mode, leader) in [
            (Mode::Ifs, "Self"),
            (Mode::Polyvagal, "Connection"),
            (Mode::Freudian, "Desire"),
            (Mode::Jungian, "Continuity"),
        ] {
            let framework = Framework::builtin(mode);
            assert_eq!(framework.parts[0].name, leader);
            assert_eq!(
                Framework::authored_identity_order(mode),
                framework
                    .parts
                    .into_iter()
                    .map(|part| part.id)
                    .collect::<Vec<_>>()
            );
        }
    }
}
