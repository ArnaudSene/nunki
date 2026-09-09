//! Compose generation (SPEC 4.2, "la forme déclarative des conteneurs").
//!
//! `hq` invents no format: it writes a Compose file per profile at every
//! launch, from the frozen mission header, `hq.yaml` and the stack fragment,
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

use crate::harness::Role;
use crate::perimeter::{Perimeter, Profile};

use model::{Document, Healthcheck, Service, depends_on_healthy};

/// The sidecar's service name in every generated file.
pub const FIREWALL_SERVICE: &str = "firewall";
/// The agent's service name in every generated file.
pub const AGENT_SERVICE: &str = "agent";
/// The only three files an agent may write in the mission folder (SPEC 4.1).
pub const AGENT_WRITABLE: [&str; 3] = ["JOURNAL.md", "PR.md", "VERDICT.json"];

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
/// header, `hq.yaml` and the stack fragment — never from the slot.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Slot identifier; the Compose project name is derived from it.
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
    /// Test credential files, mounted read-only on a system profile only
    /// (SPEC 3.1). Given as (host path, path in the container).
    pub credentials: Vec<(PathBuf, PathBuf)>,
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
}

#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    #[error(
        "slot {0:?} yields no usable Compose project name (lowercase letters, digits, - and _)"
    )]
    SlotName(String),
    #[error("the project declares a service named {0:?}, which hq reserves")]
    ReservedService(String),
    #[error("the project's services must be a YAML mapping, found {0}")]
    ProjectServicesShape(&'static str),
    #[error("the project's networks must be a YAML mapping, found {0}")]
    ProjectNetworksShape(&'static str),
    #[error("{what} must be an absolute path, found {path:?}")]
    RelativePath { what: &'static str, path: PathBuf },
    #[error(
        "the mission profile carries {0} credential file(s); only the system profile may (SPEC 4.1)"
    )]
    CredentialsOnMissionProfile(usize),
    #[error("no domain and no address is allowed: the agent would have no allowlist at all")]
    EmptyPerimeter,
}

/// Generate the Compose file for one profile.
pub fn generate(plan: &Plan) -> Result<String, ComposeError> {
    let document = build(plan)?;
    Ok(serde_yaml_ng::to_string(&document).expect("the document serialises"))
}

/// The Compose project name for a slot: stable across the three profiles,
/// which is what keeps the project's services up between them. Compose
/// refuses anything but lowercase letters, digits, `-` and `_`.
pub fn project_name(slot: &str) -> Result<String, ComposeError> {
    let mut name: String = slot
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
    if name.is_empty() {
        return Err(ComposeError::SlotName(slot.to_string()));
    }
    Ok(format!("hq-{name}"))
}

fn build(plan: &Plan) -> Result<Document, ComposeError> {
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
    services.insert(Value::from(AGENT_SERVICE), agent(plan).to_value());

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
    Ok(Document {
        name: project_name(&plan.slot)?,
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
fn agent(plan: &Plan) -> Service {
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
    // read-only but for three files (SPEC 4.1). A directory mounted rw would
    // hand back `MISSION.md` — the header the whole run is framed by.
    volumes.push(mount(&plan.mission_dir, &plan.mission_dir_at, "ro"));
    for file in AGENT_WRITABLE {
        volumes.push(mount(
            &plan.mission_dir.join(file),
            &plan.mission_dir_at.join(file),
            "rw",
        ));
    }

    for (host, at) in &plan.credentials {
        volumes.push(mount(host, at, "ro"));
    }
    for volume in &plan.volumes {
        volumes.push(format!("{}:{}", volume.name, display(&volume.at)));
    }

    Service {
        image: plan.image.clone(),
        user: Some(format!("{}:{}", plan.user.uid, plan.user.gid)),
        working_dir: Some(display(&plan.tree_at)),
        network_mode: Some(format!("service:{FIREWALL_SERVICE}")),
        cap_drop: vec!["ALL".to_string()],
        security_opt: vec!["no-new-privileges:true".to_string()],
        depends_on: Some(depends_on_healthy(FIREWALL_SERVICE)),
        volumes,
        environment: plan.environment.clone(),
        init: Some(true),
        restart: Some("no".to_string()),
        command: plan.command.clone(),
        ..Service::default()
    }
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
