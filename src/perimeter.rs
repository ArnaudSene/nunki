//! The allowlist a firewall sidecar enforces (SPEC 4.1 bis).
//!
//! Computed by `hq`, never by an agent and never read back from the slot:
//! the frozen mission header, the stack fragment and the harness adapter are
//! the only three sources. Rule 6 of 4.1 bis is the whole of it — the
//! coder's list is what his stack needs plus what his harness needs, and
//! nothing else; the integrator's and the security agent's add the services
//! the mission declares, address by address.

use std::collections::BTreeSet;
use std::net::IpAddr;

use crate::harness::Role;
use crate::mission::Service;

/// Which profile a role runs in (SPEC 4.1, mounts per profile).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// The coder: no external service, no credentials.
    Mission,
    /// The integrator and the security agent: the mission's services, its
    /// test credentials.
    System,
}

impl Profile {
    pub fn of(role: Role) -> Self {
        match role {
            Role::Coder => Profile::Mission,
            Role::Integrator | Role::Security => Profile::System,
        }
    }
}

/// What the sidecar is allowed to resolve and to reach. Sorted and
/// deduplicated, because the Compose file is regenerated at every launch and
/// a diff must mean a real change (SPEC 4.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Perimeter {
    /// Names the filtering resolver answers for. Everything else is
    /// NXDOMAIN.
    pub domains: BTreeSet<String>,
    /// Addresses reachable without a name: a declared service, a declared
    /// host port. No private range is ever opened implicitly (rule 5).
    pub addresses: BTreeSet<String>,
}

impl Perimeter {
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty() && self.addresses.is_empty()
    }
}

/// The three sources, and the forge that must appear in none of them.
#[derive(Debug, Clone, Default)]
pub struct Sources<'a> {
    /// `.hq/stacks/<name>/allow.txt`: the package registries the stack needs.
    pub stack: &'a [String],
    /// `Provisioning::domains` from the harness adapter: the model API.
    pub harness: &'a [String],
    /// The services of the frozen mission header. Ignored for a mission
    /// profile, which must not carry any.
    pub services: &'a [Service],
    /// The project's forge, from `hq.yaml`. No agent may reach it (SPEC 3.1,
    /// 3.2): the clone's origin is unreachable, and an allowlist that names
    /// the forge would undo that.
    pub forge: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PerimeterError {
    #[error(
        "the {profile} profile's allowlist names the project's forge ({domain}), \
         declared by {declared_by} — an agent must never reach the forge (SPEC 3.1)"
    )]
    Forge {
        profile: &'static str,
        domain: String,
        declared_by: &'static str,
    },
    #[error(
        "the mission profile carries {count} service(s); only the system profile may (SPEC 4.1)"
    )]
    ServicesOnMissionProfile { count: usize },
    #[error("{declared_by} declared an empty entry")]
    Empty { declared_by: &'static str },
}

/// Build the allowlist for a role.
pub fn compute(role: Role, src: &Sources) -> Result<Perimeter, PerimeterError> {
    let profile = Profile::of(role);
    let profile_name = match profile {
        Profile::Mission => "mission",
        Profile::System => "system",
    };

    if profile == Profile::Mission && !src.services.is_empty() {
        return Err(PerimeterError::ServicesOnMissionProfile {
            count: src.services.len(),
        });
    }

    let mut out = Perimeter::default();
    let mut take = |entries: &[String], declared_by: &'static str| -> Result<(), PerimeterError> {
        for entry in entries {
            let entry = entry.trim();
            if entry.is_empty() {
                return Err(PerimeterError::Empty { declared_by });
            }
            if is_address(entry) {
                out.addresses.insert(entry.to_string());
            } else {
                let domain = normalise_domain(entry);
                if let Some(hit) = forge_hit(&domain, src.forge) {
                    return Err(PerimeterError::Forge {
                        profile: profile_name,
                        domain: hit,
                        declared_by,
                    });
                }
                out.domains.insert(domain);
            }
        }
        Ok(())
    };

    take(src.stack, "the stack fragment")?;
    take(src.harness, "the harness adapter")?;
    if profile == Profile::System {
        for service in src.services {
            take(&service.reach, "a mission service")?;
        }
    }
    Ok(out)
}

/// A bare address or a CIDR block, as opposed to a name. Anything the
/// resolver would have to answer for is a domain.
fn is_address(entry: &str) -> bool {
    let host = entry.split_once('/').map_or(entry, |(addr, _)| addr);
    host.parse::<IpAddr>().is_ok()
}

/// Lowercased, trailing dot dropped. Comparison of names is on this form.
fn normalise_domain(entry: &str) -> String {
    entry.trim_end_matches('.').to_ascii_lowercase()
}

/// A domain is the forge if it is the forge, or a subdomain of it.
fn forge_hit(domain: &str, forge: &[String]) -> Option<String> {
    forge.iter().find_map(|f| {
        let f = normalise_domain(f.trim());
        if f.is_empty() {
            return None;
        }
        if domain == f || domain.ends_with(&format!(".{f}")) {
            Some(domain.to_string())
        } else {
            None
        }
    })
}
