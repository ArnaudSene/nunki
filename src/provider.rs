//! The lock on a real provider (SPEC 7).
//!
//! System tests against a real third-party API are slow, flaky and have side
//! effects, and two missions running them at once share the same test tier
//! and trample each other. So a service the mission's header declares
//! `shared: true` — a real provider, as the human says at framing (decided
//! by Arnaud on 2026-09-11) — is locked, for the other missions of the
//! project, for as long as an integration run uses it.
//!
//! The lock is **derived from hq's state**, not kept beside it: a provider is
//! taken while another mission stands on its integration stage with a run
//! recorded and declares that provider. A run read back is forgotten with its
//! transition, so the lock lifts with it — and a mission ended, reframed or
//! crashed leaves no lock behind, because there is no file to leave. The only
//! file is a short guard around the check and the launch, so two monitors
//! cannot both find a provider free and both launch on it.

use std::path::Path;

use crate::mission::flow::Stage;
use crate::mission::{Header, Integration};
use crate::state::{LockError, SlotLock, StateError, Store};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Lock(#[from] LockError),
}

/// What a launch that needs the providers gets.
#[derive(Debug)]
pub enum Claim {
    /// Every shared provider is free. The guards are held until the launch
    /// is recorded, then dropped: from then on the recorded run is the lock.
    Free(Vec<SlotLock>),
    /// One is taken, and by whom — a mission, or another `hq` claiming it
    /// this very moment.
    Busy { provider: String, by: String },
}

/// The providers a header declares shared, in the order declared.
pub fn shared(header: &Header) -> Vec<&str> {
    match &header.integration {
        Integration::Services { services, .. } => services
            .iter()
            .filter(|service| service.shared)
            .map(|service| service.name.as_str())
            .collect(),
        Integration::None { .. } => Vec::new(),
    }
}

/// The mission, other than `me`, whose integration run holds `provider`.
/// A state that cannot be read holds nothing: it cannot be running a run
/// `hq` knows of.
pub fn holder(store: &Store, me: &str, provider: &str) -> Result<Option<String>, StateError> {
    for id in store.missions()? {
        if id == me {
            continue;
        }
        let Ok(state) = store.load(&id) else {
            continue;
        };
        if matches!(state.flow.stage(), Stage::Integration { .. })
            && state.run.is_some()
            && shared(state.flow.header()).contains(&provider)
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// The first provider `header` shares that another mission holds, and who.
pub fn blocked(
    store: &Store,
    me: &str,
    header: &Header,
) -> Result<Option<(String, String)>, StateError> {
    for provider in shared(header) {
        if let Some(by) = holder(store, me, provider)? {
            return Ok(Some((provider.to_string(), by)));
        }
    }
    Ok(None)
}

/// Claim every provider `header` shares for mission `me`, under `verb`.
pub fn claim(
    store: &Store,
    hq_root: &Path,
    me: &str,
    header: &Header,
    verb: &str,
) -> Result<Claim, ProviderError> {
    let locks = hq_root.join("locks");
    let mut guards = Vec::new();
    for provider in shared(header) {
        match SlotLock::acquire(&locks, &guard_name(provider), verb) {
            Ok(guard) => guards.push(guard),
            Err(LockError::Held { verb, pid, .. }) => {
                return Ok(Claim::Busy {
                    provider: provider.to_string(),
                    by: format!("`hq {verb}` (pid {pid}), claiming it now"),
                });
            }
            Err(e) => return Err(e.into()),
        }
        if let Some(by) = holder(store, me, provider)? {
            return Ok(Claim::Busy {
                provider: provider.to_string(),
                by: format!("mission {by}"),
            });
        }
    }
    Ok(Claim::Free(guards))
}

/// The guard's file name: a provider's name is the human's to choose, and a
/// file name is not.
fn guard_name(provider: &str) -> String {
    let safe: String = provider
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("provider-{safe}")
}
