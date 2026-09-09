//! The slice of the Compose format `hq` emits (SPEC 4.2).
//!
//! Not a Compose implementation: only the keys the generator writes, in a
//! fixed order, so that regenerating an unchanged plan yields the same bytes.
//! Everything the project itself declares travels as an opaque YAML value and
//! is never re-typed.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_yaml_ng::{Mapping, Value};

/// A generated Compose file.
#[derive(Debug, Clone, Serialize)]
pub struct Document {
    /// The stable project name: the same one for every profile of a slot, so
    /// that switching profiles leaves the project's services untouched.
    pub name: String,
    /// Insertion-ordered so the generated file reads the way it was built:
    /// the firewall, the agent, then the project's own services verbatim.
    pub services: Mapping,
    #[serde(skip_serializing_if = "Mapping::is_empty")]
    pub volumes: Mapping,
}

/// A service `hq` writes itself.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Service {
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_mode: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cap_drop: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cap_add: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub security_opt: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Mapping>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub healthcheck: Option<Healthcheck>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Healthcheck {
    pub test: Vec<String>,
    pub interval: String,
    pub timeout: String,
    pub retries: u32,
    pub start_period: String,
}

/// `depends_on: { <service>: { condition: service_healthy } }` — the shape
/// that makes "a sidecar that fails is an agent that does not start" true
/// (SPEC 4.1 bis, rule 2).
pub fn depends_on_healthy(service: &str) -> Mapping {
    let mut condition = Mapping::new();
    condition.insert(Value::from("condition"), Value::from("service_healthy"));
    let mut deps = Mapping::new();
    deps.insert(Value::from(service), Value::Mapping(condition));
    deps
}

impl Service {
    pub fn to_value(&self) -> Value {
        serde_yaml_ng::to_value(self).expect("a service serialises to a mapping")
    }
}
