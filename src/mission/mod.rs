//! Missions: the structured header (SPEC 4.1), verdicts (SPEC 4.4) and the
//! flow (SPEC 4.5).

pub mod dir;
pub mod flow;
pub mod journal;

use serde::{Deserialize, Serialize};

use crate::harness::Role;

/// A lot: a small unit of work with its own proof, done in one run (SPEC
/// 4.3, "un run par lot").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lot {
    pub id: String,
    pub title: String,
}

/// Where a mission's external services live (SPEC 2, "Où vivent les services
/// externes"). Only what the engine needs to build the allowlist is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    /// Domains or addresses the system profile may reach for this service.
    pub reach: Vec<String>,
    /// A real provider — a third-party API's test tier — that two missions
    /// must not exercise at once: an integration run that uses it locks it
    /// for the project's other missions (SPEC 7). The
    /// human says so at framing; nothing is guessed from the domains.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shared: bool,
}

/// The `integration` field of the mission header (SPEC 2, mission shapes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Integration {
    /// No integration mission; the reason is written at framing.
    None { reason: String },
    /// An integration mission on the system profile, reaching these
    /// services, and allowed to commit exactly `wiring`.
    Services {
        services: Vec<Service>,
        /// The paths the integrator's commits may touch — configuration,
        /// system tests, fixtures, migrations. SPEC 4.4 defines "wiring"
        /// mechanically as this list and nothing else: **every commit
        /// outside it is out of perimeter**. It is the integrator's gate 4,
        /// and it is an allowlist, the inverse of the coder's.
        ///
        /// Empty means the integrator may commit nothing, which is what the
        /// rule says when nothing is declared. The gate says so by name
        /// rather than failing obscurely.
        #[serde(default)]
        wiring: Vec<String>,
    },
}

/// The `security` field of the mission header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// Mechanical gates only (dependency audit, secret scan, static analysis).
    Gates,
    /// The gates, plus the security agent mission.
    Agent,
}

/// How much a mission's verification asks of it: what gate 7 demands, how
/// many security rounds are played, which roles run (SPEC 2, 4.4 gate 7,
/// 4.5). `critical` is what every mission got before a rigor could be
/// declared, and it stays the default, so a header or a `nunki.yaml` that
/// says nothing keeps that treatment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rigor {
    /// The coder and the mechanical gates only: no mutation campaign, no
    /// security agent, no integration.
    Prototype,
    /// Gate 7 passes on the project's share of killed mutants rather than
    /// on an outcome per survivor, and the security agent plays one round.
    Standard,
    /// An outcome for every survivor, and up to three security rounds.
    #[default]
    Critical,
}

impl Rigor {
    /// The rigor `mission new` freezes into a header: the flag when one is
    /// given, else the project's `rigor:` in `nunki.yaml`, else `critical`.
    pub fn chosen(flag: Option<Rigor>, project: Option<Rigor>) -> Rigor {
        flag.or(project).unwrap_or_default()
    }

    /// How many rounds the security agent may play on a mission of this
    /// rigor (SPEC 4.5). A round is one verdict it concludes; once they are
    /// spent, green gates bring the mission back to the last `FINDINGS`, or
    /// verify it when the last verdict was a `CLEAR`.
    pub fn max_security_rounds(self) -> u32 {
        match self {
            Rigor::Prototype => 0,
            Rigor::Standard => 1,
            Rigor::Critical => 3,
        }
    }

    /// Whether a mission framed at this rigor may declare services or call
    /// the security agent. A prototype runs the coder and the mechanical
    /// gates only, so asking it for either is a contradiction said at
    /// framing rather than discovered at verification.
    pub fn admits(self, services: bool, security_agent: bool) -> Result<(), RigorError> {
        if self != Rigor::Prototype {
            return Ok(());
        }
        if services {
            return Err(RigorError::PrototypeWithServices);
        }
        if security_agent {
            return Err(RigorError::PrototypeWithSecurityAgent);
        }
        Ok(())
    }
}

impl std::fmt::Display for Rigor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Rigor::Prototype => "prototype",
            Rigor::Standard => "standard",
            Rigor::Critical => "critical",
        })
    }
}

impl std::str::FromStr for Rigor {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "prototype" => Ok(Rigor::Prototype),
            "standard" => Ok(Rigor::Standard),
            "critical" => Ok(Rigor::Critical),
            other => Err(format!(
                "a rigor is prototype, standard or critical, and {other:?} is none of them"
            )),
        }
    }
}

/// A framing the chosen rigor contradicts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RigorError {
    #[error(
        "a prototype runs the coder and the mechanical gates only, so it declares no \
         service: drop --service, or frame the mission at standard or critical"
    )]
    PrototypeWithServices,
    #[error(
        "a prototype runs the coder and the mechanical gates only, so it calls no \
         security agent: drop --security-agent, or frame the mission at standard or critical"
    )]
    PrototypeWithSecurityAgent,
}

/// The three shapes of SPEC 2, derived from the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    CodeOnly,
    CodeAndSecurity,
    Full,
}

/// Bounds and cadences (SPEC 4.3 and 4.5). Project defaults come from
/// `nunki.yaml`; the mission header overrides them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    /// Returns to the coder allowed after a red verdict. Zero means the first
    /// red goes to the human.
    pub max_volets: u32,
    /// Attempts allowed per lot before the human is called.
    pub attempts_per_lot: u32,
    /// Minutes between the agent's checkpoint blocks.
    pub checkpoint_minutes: u32,
    /// Minutes between liveness checks.
    pub check_minutes: u32,
    /// Consecutive checks without change that count as a stall.
    pub stall_checks: u32,
    /// Hours on one lot, with progress, after which the human is told.
    pub long_lot_hours: u32,
    /// Minutes a mutation campaign is given before `nunki` stops it (SPEC 4.4,
    /// gate 7: "elle est longue, donc elle se lance et se guette comme un
    /// run, avec un délai paramétré").
    #[serde(default = "default_mutation_minutes")]
    pub mutation_minutes: u32,
    /// Hours `nunki` waits out a harness that keeps failing — quota, network,
    /// crash — before it holds the mission and hands it to the human (SPEC
    /// 4.3). Counted from the first failure in a row; zero sends the first
    /// one to the human.
    #[serde(default = "default_harness_wait_hours")]
    pub harness_wait_hours: u32,
    /// Runs a mission may spend before `nunki` holds it (SPEC 7). No default:
    /// runs are already bounded by the attempts and the volets, and a fixed
    /// number would cut legitimate missions short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u32>,
    /// Tokens a mission may spend before `nunki` holds it, the four kinds
    /// summed (SPEC 7). No default until real missions have been measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Percent of the subscription's five-hour window past which `nunki`
    /// launches no run until the window resets (SPEC 4.3).
    #[serde(default = "default_five_hour_stop_percent")]
    pub five_hour_stop_percent: u32,
    /// Percent of the subscription's weekly window past which `nunki` launches
    /// no run until the window resets (SPEC 4.3).
    #[serde(default = "default_weekly_stop_percent")]
    pub weekly_stop_percent: u32,
}

/// The rest of the five hours is left to a supervisor that shares the
/// account, so a human can still step in.
fn default_five_hour_stop_percent() -> u32 {
    90
}

/// A fifth of the week stays outside the agents, whatever they
/// do.
fn default_weekly_stop_percent() -> u32 {
    80
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            five_hour_stop_percent: default_five_hour_stop_percent(),
            weekly_stop_percent: default_weekly_stop_percent(),
            max_volets: 3,
            attempts_per_lot: 3,
            checkpoint_minutes: 45,
            check_minutes: 15,
            stall_checks: 3,
            long_lot_hours: 8,
            mutation_minutes: default_mutation_minutes(),
            harness_wait_hours: default_harness_wait_hours(),
            max_runs: None,
            max_tokens: None,
        }
    }
}

/// Long enough to outlast a subscription window that has run dry overnight,
/// so `nunki` picks the mission up by itself when it reopens —
/// a shorter wait would wake the human instead.
fn default_harness_wait_hours() -> u32 {
    6
}

/// Long enough for a real campaign on a lot's worth of files, short enough
/// that a hung one does not hold a slot for a working day.
fn default_mutation_minutes() -> u32 {
    45
}

/// The structured header of `MISSION.md`, frozen into the state at
/// validation (SPEC 4.1). The agent cannot write it and `nunki` never re-reads
/// it during the mission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub branch: String,
    pub base: String,
    pub lots: Vec<Lot>,
    pub integration: Integration,
    pub security: Security,
    /// How much verification asks of this mission. Frozen with the rest of
    /// the header, like `bounds`: verification reads the rigor the mission
    /// was framed at, not whatever `nunki.yaml` says today. Absent — every
    /// header written before the field — means `critical`.
    #[serde(default)]
    pub rigor: Rigor,
    /// The share of tried mutants a `standard` mission must kill for gate 7,
    /// as a whole percentage, frozen from `nunki.yaml` at `mission new` with
    /// the rigor it serves: a threshold read live could be lowered under a
    /// mission already framed. Absent — every header written before it was
    /// frozen — the project's current value is read, as it was then.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::project::frozen_mutation_threshold"
    )]
    pub mutation_threshold: Option<u32>,
    /// Who decides when this mission comes back with a question — an
    /// arbitration, a verdict to accept, a push to authorise. Defaults to
    /// whoever framed it, and is said rather than assumed the moment a
    /// second person works on the project (SPEC 4.5).
    #[serde(default)]
    pub arbiter: Option<String>,
    /// Which account this mission spends. Frozen with the rest of the
    /// header: a mission cannot change the subscription it runs on halfway
    /// through, any more than it can change its perimeter (SPEC 4.3).
    #[serde(default)]
    pub account: Option<String>,
    /// Which model this mission's agents run on, refining `nunki.yaml`. Frozen
    /// with the rest of the header: a mission does not change model halfway
    /// through, any more than it changes the subscription it spends.
    #[serde(default)]
    pub model: Option<String>,
    /// How the application is started for this mission, refining `nunki.yaml`
    /// and the stack's default (SPEC 4.2, "les services et le lancement de
    /// l'application"). `none` when there is nothing to start.
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub bounds: Bounds,
}

impl Header {
    pub fn shape(&self) -> Shape {
        match (&self.integration, self.security) {
            (Integration::Services { .. }, _) => Shape::Full,
            (Integration::None { .. }, Security::Agent) => Shape::CodeAndSecurity,
            (Integration::None { .. }, Security::Gates) => Shape::CodeOnly,
        }
    }

    pub fn has_integration(&self) -> bool {
        matches!(self.integration, Integration::Services { .. })
    }

    pub fn has_security_agent(&self) -> bool {
        self.security == Security::Agent
    }
}

/// A role's verdict (SPEC 2 and 4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    Integrated,
    Broken,
    Clear,
    Findings,
}

impl Verdict {
    /// The verdict a stage waits for.
    pub fn is_green(self) -> bool {
        matches!(self, Verdict::Integrated | Verdict::Clear)
    }
}

/// `VERDICT.json`, written by the agent at the end of its last run and
/// validated by `nunki` against the real `HEAD` (SPEC 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictFile {
    pub role: Role,
    pub verdict: Verdict,
    /// Commit the role concluded on.
    pub head: String,
    /// RFC 3339 date.
    pub date: String,
    /// The report, as text: the prose deliverable. For the security role it
    /// carries every finding, ranked, whatever `findings` says.
    #[serde(default)]
    pub report: String,
    /// The security agent's findings, one entry each, with a severity: what
    /// lets `nunki` lift a report whose worst finding is below `MEDIUM` on
    /// its own ([`VerdictFile::automatic_lift`], SPEC 4.5). Absent from
    /// older verdicts and from the other roles'.
    ///
    /// Read leniently, on purpose: a list `nunki` cannot read is kept as it
    /// was written, so the verdict still reads and the mission stops at
    /// `Findings` as it did before severities existed. Refusing the whole
    /// file would turn a malformed ranking into a failed attempt instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub findings: Option<FindingList>,
}

/// The `findings` of a verdict file, as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FindingList {
    /// A list of findings, each read as an object.
    Listed(Vec<Finding>),
    /// Anything else, kept verbatim: nothing is lifted on it.
    Unreadable(serde_json::Value),
}

/// One finding in a verdict file.
///
/// The severity is kept as the agent wrote it and read by
/// [`Severity::parse`], so an unknown one is a reason not to lift rather
/// than a verdict nobody can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub title: String,
    /// Why it can be accepted: required for `LOW` and `INFO`, ignored above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why_acceptable: Option<String>,
}

/// How bad a finding is, by what it lets an attacker do. Ordered, least
/// severe first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
}

impl Severity {
    /// The severity as the verdict file spells it, exactly: `HIGH`,
    /// `MEDIUM`, `LOW` or `INFO`. Anything else is unknown, and an unknown
    /// severity is never guessed at.
    pub fn parse(text: &str) -> Option<Severity> {
        match text {
            "HIGH" => Some(Severity::High),
            "MEDIUM" => Some(Severity::Medium),
            "LOW" => Some(Severity::Low),
            "INFO" => Some(Severity::Info),
            _ => None,
        }
    }

    /// Whether `nunki` may accept a finding of this severity on its own:
    /// below `MEDIUM`, and nothing else.
    pub fn lifted_by_nunki(self) -> bool {
        self < Severity::Medium
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Severity::High => "HIGH",
            Severity::Medium => "MEDIUM",
            Severity::Low => "LOW",
            Severity::Info => "INFO",
        })
    }
}

/// A finding `nunki` accepts on its own, with the agent's reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowFinding {
    pub severity: Severity,
    pub title: String,
    pub why_acceptable: String,
}

impl LowFinding {
    /// The finding in one line, as the lift's reason records it, one line
    /// per finding.
    pub fn line(&self) -> String {
        crate::text::one_line(&format!(
            "{} — {}: {}",
            self.severity, self.title, self.why_acceptable
        ))
    }
}

/// Why `nunki` leaves a `FINDINGS` to a human rather than lifting it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Withheld {
    #[error("the verdict is {0:?}, and only a FINDINGS is lifted")]
    NotFindings(Verdict),
    #[error("the verdict ranks no finding: it has no `findings` list")]
    NoList,
    #[error("the verdict's `findings` is not a list of findings nunki can read")]
    Unreadable,
    #[error("the verdict is FINDINGS and its `findings` list is empty")]
    Empty,
    #[error("finding {index} of the list has no title")]
    Untitled { index: usize },
    #[error(
        "finding {title:?} has severity {severity:?}, which is none of HIGH, MEDIUM, LOW, INFO"
    )]
    UnknownSeverity { title: String, severity: String },
    #[error("finding {title:?} is {severity}, and nunki lifts LOW and INFO only")]
    TooSevere { title: String, severity: Severity },
    #[error("finding {title:?} is {severity} and does not say why it is acceptable")]
    NoReason { title: String, severity: Severity },
}

impl VerdictFile {
    /// The findings `nunki` lifts on its own, or why it lifts nothing
    /// (SPEC 4.5).
    ///
    /// Fail closed: a `FINDINGS` whose list is not empty and whose every
    /// finding is `LOW` or `INFO`, titled, with a reason; a title or a reason
    /// that shows nothing once whitespace and invisible characters are set
    /// aside ([`crate::text::blank`]) is none. Anything else — from a
    /// missing list to one unknown severity — leaves the whole
    /// verdict to a human, exactly as before severities existed. Nothing is
    /// ever accepted on a guess.
    pub fn automatic_lift(&self) -> Result<Vec<LowFinding>, Withheld> {
        if self.verdict != Verdict::Findings {
            return Err(Withheld::NotFindings(self.verdict));
        }
        let listed = match &self.findings {
            None => return Err(Withheld::NoList),
            Some(FindingList::Unreadable(_)) => return Err(Withheld::Unreadable),
            Some(FindingList::Listed(listed)) => listed,
        };
        if listed.is_empty() {
            return Err(Withheld::Empty);
        }
        let mut lifted = Vec::with_capacity(listed.len());
        for (index, finding) in listed.iter().enumerate() {
            let title = finding.title.trim();
            if crate::text::blank(title) {
                return Err(Withheld::Untitled { index: index + 1 });
            }
            let Some(severity) = Severity::parse(&finding.severity) else {
                return Err(Withheld::UnknownSeverity {
                    title: title.to_string(),
                    severity: finding.severity.clone(),
                });
            };
            if !severity.lifted_by_nunki() {
                return Err(Withheld::TooSevere {
                    title: title.to_string(),
                    severity,
                });
            }
            let why = finding.why_acceptable.as_deref().map(str::trim);
            let Some(why) = why.filter(|w| !crate::text::blank(w)) else {
                return Err(Withheld::NoReason {
                    title: title.to_string(),
                    severity,
                });
            };
            lifted.push(LowFinding {
                severity,
                title: title.to_string(),
                why_acceptable: why.to_string(),
            });
        }
        Ok(lifted)
    }
}
