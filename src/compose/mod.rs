//! Compose generation (SPEC 4.2, "la forme déclarative des conteneurs").
//!
//! `nunki` invents no format: it writes a Compose file per profile at every
//! launch, from the frozen mission header, `nunki.yaml` and the stack fragment,
//! and an engine adapter runs it. Nothing here talks to a container engine —
//! this module is a pure function from a plan to bytes, which is what makes
//! the doctrine of 4.1 bis testable without Docker.
//!
//! Three shapes are load-bearing, and each was measured against Docker
//! Compose v5.1.2 before being written here (see SPEC 4.2):
//!
//! 1. The **firewall owns the network namespace** and the agent joins it
//!    (`network_mode: service:firewall`), not the other way round: in the
//!    other direction the agent starts first and has a network before any
//!    rule is laid.
//! 2. The agent therefore cannot declare `networks:` — Compose rejects the
//!    two together — so the firewall is what attaches to the project's
//!    service network.
//! 3. Every profile file **re-declares the project's services identically**,
//!    under a project name stable for the slot, so switching profiles leaves
//!    their containers untouched.

pub mod model;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use crate::engine::Dialect;
use crate::engine::spawn::RUN_DIR;
use crate::harness::Role;
use crate::perimeter::{Perimeter, Profile};

use model::{Document, Healthcheck, Service, depends_on_healthy};

/// The sidecar's service name in every generated file.
pub const FIREWALL_SERVICE: &str = "firewall";
/// The agent's service name in every generated file.
pub const AGENT_SERVICE: &str = "agent";
/// The only files an agent may write in the mission folder (SPEC 4.1).
///
/// `MUTANTS.triage.json` is the fourth, added with gate 7 on 2026-09-10: the
/// coder answers a campaign's survivors there, and it is a **different file**
/// from `MUTANTS.json` on purpose. What decides who wrote a line is this
/// list, not the line — so the outcome no machine can check lives in the
/// HQ's file, which is mounted read-only here.
pub const AGENT_WRITABLE: [&str; 4] =
    ["JOURNAL.md", "PR.md", "VERDICT.json", "MUTANTS.triage.json"];

/// The uid and gid of the human, passed through so that files the agent
/// creates belong to them on the host (SPEC 4.2 bis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserIds {
    pub uid: u32,
    pub gid: u32,
}

/// A named volume: the slot's caches, the harness config directory, the
/// writable directories a read-only tree still needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedVolume {
    pub name: String,
    /// Where it is mounted inside the container.
    pub at: PathBuf,
}

/// Everything the generator needs. Assembled by the engine from the frozen
/// header, `nunki.yaml` and the stack fragment — never from the slot.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The session the project is registered under: half of the Compose
    /// project name, and what tells two projects' slots apart.
    pub session: String,
    /// Slot identifier; the other half.
    pub slot: String,
    pub role: Role,
    /// The stack image for the agent container.
    pub image: String,
    /// The sidecar image.
    pub firewall_image: String,
    pub user: UserIds,
    /// The slot's tree on the host, and where it is mounted.
    pub tree: PathBuf,
    pub tree_at: PathBuf,
    /// The mission folder at the HQ, and where it is mounted.
    pub mission_dir: PathBuf,
    pub mission_dir_at: PathBuf,
    /// The stack's scripts, from the project's home, mounted read-only one
    /// file at a time. Given as (host path, path in the container).
    pub stack_scripts: Vec<(PathBuf, PathBuf)>,
    /// Test credential files, mounted read-only on a system profile only
    /// (SPEC 3.1). Given as (host path, path in the container).
    pub credentials: Vec<(PathBuf, PathBuf)>,
    /// The advisory database gate 8 reads, mounted read-only, or `None` when
    /// the stack declares none. Filled by the host and never by `nunki`: the
    /// tool owns its own layout (SPEC 4.4, gate 8).
    pub advisories: Option<(PathBuf, PathBuf)>,
    pub volumes: Vec<NamedVolume>,
    pub environment: BTreeMap<String, String>,
    pub command: Vec<String>,
    pub perimeter: Perimeter,
    /// The project's own `services:` block, merged verbatim: `include:` is
    /// unusable on one of the two engines, and re-typing it would lose keys
    /// (SPEC 4.2, engine table).
    pub project_services: Option<Value>,
    /// The project's own `networks:` block, merged verbatim. The firewall
    /// attaches to every network it declares — it is the one that can, the
    /// agent having `network_mode` instead (measured). A project that
    /// declares none needs none: its services and the firewall both land on
    /// the generated default network, and reach each other there (measured).
    pub project_networks: Option<Value>,
    /// The project's own `volumes:` block, merged verbatim beside nunki's.
    ///
    /// Not optional in practice: a service that names a volume the document
    /// does not declare makes the whole project invalid — `service "db"
    /// refers to undefined volume dbdata: invalid compose project`, measured
    /// on Compose v5.1.2, 2026-09-10. And it is exactly where a project keeps
    /// what must survive a profile switch, so dropping the block would drop
    /// the state SPEC 4.2's first rule exists to keep.
    pub project_volumes: Option<Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    #[error(
        "slot {0:?} yields no usable Compose project name (lowercase letters, digits, - and _)"
    )]
    SlotName(String),
    #[error("the project declares a service named {0:?}, which nunki reserves")]
    ReservedService(String),
    #[error("the project's services must be a YAML mapping, found {0}")]
    ProjectServicesShape(&'static str),
    #[error("the project's networks must be a YAML mapping, found {0}")]
    ProjectNetworksShape(&'static str),
    #[error("the project's volumes must be a YAML mapping, found {0}")]
    ProjectVolumesShape(&'static str),
    #[error("the project declares a volume named {0:?}, which nunki reserves for the slot")]
    ReservedVolume(String),
    #[error("{what} must be an absolute path, found {path:?}")]
    RelativePath { what: &'static str, path: PathBuf },
    #[error(
        "the mission profile carries {0} credential file(s); only the system profile may (SPEC 4.1)"
    )]
    CredentialsOnMissionProfile(usize),
    #[error("no domain and no address is allowed: the agent would have no allowlist at all")]
    EmptyPerimeter,
}

/// Generate the Compose file for one profile. The dialect comes from the
/// engine adapter: how a namespace is shared is the one thing on the 4.2
/// engine table this file needs, and it must not be spelled here.
pub fn generate(plan: &Plan, dialect: &Dialect) -> Result<String, ComposeError> {
    let document = build(plan, dialect)?;
    Ok(serde_yaml_ng::to_string(&document).expect("the document serialises"))
}

/// The Compose project name: the **session** and the slot, in that order.
///
/// Stable across the three profiles, which is what keeps the project's
/// services up between them. Compose refuses anything but lowercase letters,
/// digits, `-` and `_`.
///
/// The session is in it because the slot alone is not unique. Measured on
/// 2026-09-16: `test-nunki` and `notes-api` both had a slot called `one`, so
/// both were the Compose project `nunki-one` — one set of containers, one
/// network, one harness volume holding both projects' sessions. Starting a
/// mission on either would have recreated the other's containers under a
/// running agent, and each HQ's own `locks/one` would have said the slot was
/// free. The session comes first so that `docker ps` groups a project's
/// containers together, which is the reading that was impossible before.
pub fn project_name(session: &str, slot: &str) -> Result<String, ComposeError> {
    // The slot is judged on its own, as it was before the session joined it:
    // prefixed, a slot named `_1` or `///` would be legal for Compose by the
    // session's grace rather than by being a usable name, and `///` would
    // have stopped being refused at all.
    let slot = legal(slot).ok_or_else(|| ComposeError::SlotName(slot.to_string()))?;
    let session = legal(short(session)).ok_or_else(|| ComposeError::SlotName(slot.clone()))?;
    Ok(format!("nunki-{session}-{slot}"))
}

/// A name Compose accepts, or nothing: lowercase letters, digits, `-` and
/// `_`, starting on a letter or a digit — "must consist only of lowercase
/// alphanumeric characters, hyphens, and underscores as well as start with a
/// letter or number", measured.
fn legal(part: &str) -> Option<String> {
    let mut name: String = part
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    while name.starts_with(|c: char| !c.is_ascii_alphanumeric()) {
        name.remove(0);
    }
    (!name.is_empty()).then_some(name)
}

/// As much of a session as a name needs: the first eight characters.
///
/// A whole identifier would make every container name forty characters of
/// hexadecimal, and `docker ps` is read by a human. Eight is what `nunki`
/// already shows of a campaign's fingerprint, and the ledger holds the whole
/// one — two sessions that agreed on eight characters would still have two
/// homes and two HQs; they would share a Compose project, which is the thing
/// this function exists to stop, so it is worth saying that it is a
/// one-in-four-billion accident and not a design.
fn short(session: &str) -> &str {
    &session[..8.min(session.len())]
}

fn build(plan: &Plan, dialect: &Dialect) -> Result<Document, ComposeError> {
    let profile = Profile::of(plan.role);

    if profile == Profile::Mission && !plan.credentials.is_empty() {
        return Err(ComposeError::CredentialsOnMissionProfile(
            plan.credentials.len(),
        ));
    }
    if plan.perimeter.is_empty() {
        return Err(ComposeError::EmptyPerimeter);
    }
    absolute("the slot tree", &plan.tree)?;
    absolute("the mission folder", &plan.mission_dir)?;
    for (host, _) in &plan.credentials {
        absolute("a credential file", host)?;
    }

    let networks = match &plan.project_networks {
        None | Some(Value::Null) => Mapping::new(),
        Some(Value::Mapping(map)) => map.clone(),
        Some(Value::Sequence(_)) => return Err(ComposeError::ProjectNetworksShape("a sequence")),
        Some(_) => return Err(ComposeError::ProjectNetworksShape("a scalar")),
    };
    let attached: Vec<String> = networks
        .keys()
        .map(|k| k.as_str().unwrap_or_default().to_string())
        .collect();

    let mut services = Mapping::new();
    services.insert(
        Value::from(FIREWALL_SERVICE),
        firewall(plan, &attached).to_value(),
    );
    services.insert(Value::from(AGENT_SERVICE), agent(plan, dialect)?.to_value());

    if let Some(project) = &plan.project_services {
        let declared = match project {
            Value::Mapping(map) => map,
            Value::Null => return document(plan, services, networks),
            Value::Sequence(_) => return Err(ComposeError::ProjectServicesShape("a sequence")),
            _ => return Err(ComposeError::ProjectServicesShape("a scalar")),
        };
        for (key, value) in declared {
            if key == FIREWALL_SERVICE || key == AGENT_SERVICE {
                return Err(ComposeError::ReservedService(
                    key.as_str().unwrap_or("?").to_string(),
                ));
            }
            services.insert(key.clone(), value.clone());
        }
    }

    document(plan, services, networks)
}

fn document(plan: &Plan, services: Mapping, networks: Mapping) -> Result<Document, ComposeError> {
    let mut volumes = Mapping::new();
    for volume in &plan.volumes {
        volumes.insert(Value::from(volume.name.clone()), Value::Null);
    }
    match &plan.project_volumes {
        None | Some(Value::Null) => {}
        Some(Value::Mapping(declared)) => {
            for (key, value) in declared {
                let name = key.as_str().unwrap_or_default();
                // nunki's own volumes are named `nunki-<slot>-…`; a project taking
                // one of those names would have the slot's build cache or
                // the harness's sessions handed to a service.
                if volumes.contains_key(key) || name.starts_with(&format!("nunki-{}-", plan.slot)) {
                    return Err(ComposeError::ReservedVolume(name.to_string()));
                }
                volumes.insert(key.clone(), value.clone());
            }
        }
        Some(Value::Sequence(_)) => return Err(ComposeError::ProjectVolumesShape("a sequence")),
        Some(_) => return Err(ComposeError::ProjectVolumesShape("a scalar")),
    }
    Ok(Document {
        name: project_name(&plan.session, &plan.slot)?,
        services,
        volumes,
        networks,
    })
}

/// The sidecar: the only container with power, and the one that owns the
/// network namespace (SPEC 4.1 bis, rules 1 and 3).
fn firewall(plan: &Plan, attached: &[String]) -> Service {
    let mut environment = BTreeMap::new();
    environment.insert(
        "HQ_ALLOW_DOMAINS".to_string(),
        plan.perimeter
            .domains
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(","),
    );
    environment.insert(
        "HQ_ALLOW_ADDRESSES".to_string(),
        plan.perimeter
            .addresses
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(","),
    );

    Service {
        image: plan.firewall_image.clone(),
        cap_drop: vec!["ALL".to_string()],
        // NET_ADMIN and NET_RAW to lay the rules; SETUID and SETGID because
        // the resolver drops to its own uid, which is what the ruleset uses
        // to tell its queries from the agent's ("failed to change group-id
        // to dip: Operation not permitted", measured). Four capabilities in
        // the container built to hold them, none in the agent's.
        cap_add: vec![
            "NET_ADMIN".to_string(),
            "NET_RAW".to_string(),
            "SETUID".to_string(),
            "SETGID".to_string(),
        ],
        security_opt: vec!["no-new-privileges:true".to_string()],
        networks: attached.to_vec(),
        healthcheck: Some(Healthcheck {
            test: vec!["CMD".to_string(), "/usr/local/bin/fw-ready".to_string()],
            interval: "1s".to_string(),
            timeout: "2s".to_string(),
            retries: 30,
            start_period: "1s".to_string(),
        }),
        environment,
        init: Some(true),
        restart: Some("no".to_string()),
        ..Service::default()
    }
}

/// The agent: no capability, the human's uid, and no way to declare a
/// network of its own — it lives in the firewall's namespace.
fn agent(plan: &Plan, dialect: &Dialect) -> Result<Service, ComposeError> {
    let mut volumes = Vec::new();

    let tree_mode = match plan.role {
        // The security agent reads the tree and never writes it; what an
        // execution must still write is declared by the stack and mounted as
        // named volumes (SPEC 4.2, services and launch, rule 3).
        Role::Security => "ro",
        Role::Coder | Role::Integrator => "rw",
    };
    volumes.push(mount(&plan.tree, &plan.tree_at, tree_mode));

    // The mission folder is the agent's only channel with the HQ, and it is
    // read-only but for the agent's own files (SPEC 4.1). A directory
    // mounted rw would
    // hand back `MISSION.md` — the header the whole run is framed by.
    volumes.push(mount(&plan.mission_dir, &plan.mission_dir_at, "ro"));
    for file in AGENT_WRITABLE {
        volumes.push(mount(
            &plan.mission_dir.join(file),
            &plan.mission_dir_at.join(file),
            "rw",
        ));
    }

    // The scripts that judge the agent and start its application. They live
    // in the project's home, not in the tree, and read-only is what keeps
    // them out of the agent's reach — by construction, not by a gate.
    for (host, at) in &plan.stack_scripts {
        volumes.push(mount(host, at, "ro"));
    }

    for (host, at) in &plan.credentials {
        volumes.push(mount(host, at, "ro"));
    }
    // Read-only, like everything that judges the agent: a database it could
    // write is one it could empty.
    if let Some((host, at)) = &plan.advisories {
        volumes.push(mount(host, at, "ro"));
    }
    for volume in &plan.volumes {
        volumes.push(format!("{}:{}", volume.name, display(&volume.at)));
    }

    Ok(Service {
        image: plan.image.clone(),
        user: Some(format!("{}:{}", plan.user.uid, plan.user.gid)),
        working_dir: Some(display(&plan.tree_at)),
        network_mode: Some(
            dialect.netns_ref(&project_name(&plan.session, &plan.slot)?, FIREWALL_SERVICE),
        ),
        cap_drop: vec!["ALL".to_string()],
        security_opt: vec!["no-new-privileges:true".to_string()],
        depends_on: Some(depends_on_healthy(FIREWALL_SERVICE)),
        volumes,
        // The one place the agent may write that is neither the tree nor the
        // mission folder: its own runtime files, starting with the process id
        // a run publishes so `nunki` can stop it from inside (see
        // `engine::spawn`). A tmpfs, so it dies with the container; owned by
        // the agent, so nothing else in the pair can touch it.
        tmpfs: vec![format!(
            "{RUN_DIR}:uid={},gid={},mode=0700",
            plan.user.uid, plan.user.gid
        )],
        environment: plan.environment.clone(),
        init: Some(true),
        restart: Some("no".to_string()),
        command: plan.command.clone(),
        ..Service::default()
    })
}

fn mount(host: &Path, at: &Path, mode: &str) -> String {
    format!("{}:{}:{mode}", display(host), display(at))
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn absolute(what: &'static str, path: &Path) -> Result<(), ComposeError> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(ComposeError::RelativePath {
            what,
            path: path.to_path_buf(),
        })
    }
}
