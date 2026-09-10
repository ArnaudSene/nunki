//! Probing the perimeter from inside a container (SPEC 4.1 bis, rule 7).
//!
//! This is the half of `hq check` that cannot be answered by reading files:
//! whether the fence actually holds. It lifts the mission profile of a slot,
//! tries to get out by every route the battery knows, and takes it down
//! again. What it runs is [`crate::perimeter::probes`] — the same list the
//! firewall's own tests use, so the verb and the tests cannot drift apart.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::check::{Check, Verdict};
use crate::compose::{AGENT_SERVICE, AGENT_WRITABLE, FIREWALL_SERVICE, NamedVolume, Plan, UserIds};
use crate::engine::{Engine, EngineError};
use crate::harness::Role;
use crate::image::{self, Images};
use crate::perimeter::{Sources, compute};
use crate::project::Project;
use crate::slot::Slot;

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("the slot's images are missing: {0} — `hq slot rebuild {1}` builds them")]
    NoImages(String, String),
    #[error("the profile did not come up: {0}")]
    Up(#[from] EngineError),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("the profile could not be generated: {0}")]
    Compose(#[from] crate::compose::ComposeError),
    #[error("the allowlist could not be computed: {0}")]
    Perimeter(#[from] crate::perimeter::PerimeterError),
}

/// Lift the mission profile of `slot` and try to get out of it.
///
/// Everything it creates, it removes: the profile is taken down before this
/// returns, whatever the probes said.
pub fn mission_profile(
    project: &Project,
    slot: &Slot,
    stack: &str,
    engine: Arc<dyn Engine>,
    engine_bin: &str,
) -> Result<Vec<Check>, ProbeError> {
    let images = image::names(project, stack);
    for image in [&images.agent, &images.firewall, &images.prober] {
        if !image::present(engine_bin, image) {
            return Err(ProbeError::NoImages(image.clone(), slot.name.clone()));
        }
    }

    let scratch = project.hq_root.join("checks").join(&slot.name);
    std::fs::create_dir_all(&scratch).map_err(|e| ProbeError::Io(scratch.clone(), e))?;
    for file in AGENT_WRITABLE {
        let path = scratch.join(file);
        if !path.exists() {
            std::fs::write(&path, "").map_err(|e| ProbeError::Io(path, e))?;
        }
    }

    let mut plan = plan(project, slot, stack, &images, &scratch)?;
    // The probes run from a container of their own, in the very namespace
    // the agent joins: the rules and the resolver belong to the namespace,
    // not to a container, and a stack image has no reason to carry
    // `nslookup`. Measured — the Debian Rust image has none, and every
    // positive probe came back "refused".
    plan.project_services = Some(prober_service(&images.prober, plan.user)?);
    let file = project
        .hq_root
        .join("checks")
        .join(format!("{}-mission.yml", slot.name));
    std::fs::write(&file, crate::compose::generate(&plan, engine.dialect())?)
        .map_err(|e| ProbeError::Io(file.clone(), e))?;

    let compose_project = compose_project(&slot.name)?;
    let _ = engine.down(&file, &compose_project, true);
    engine.up(&file, &compose_project)?;

    let allowed = plan
        .perimeter
        .domains
        .iter()
        .next()
        .cloned()
        .unwrap_or_else(|| "example.com".to_string());
    let checks = crate::perimeter::probes(&allowed, &[])
        .into_iter()
        .map(|probe| {
            let out = engine.exec(
                &file,
                &compose_project,
                PROBER_SERVICE,
                &["sh".to_string(), "-c".to_string(), probe.script.clone()],
            );
            let reached = out.map(|o| o.ok()).unwrap_or(false);
            let what = format!("from inside the mission profile: {}", probe.what);
            if reached == probe.expected {
                Check {
                    what,
                    verdict: Verdict::Green(
                        if reached { "reached" } else { "refused" }.to_string(),
                    ),
                }
            } else {
                Check {
                    what,
                    verdict: Verdict::Red(format!(
                        "expected {}, got {}",
                        say(probe.expected),
                        say(reached)
                    )),
                }
            }
        })
        .collect();

    engine.down(&file, &compose_project, true)?;
    Ok(checks)
}

fn say(reached: bool) -> &'static str {
    if reached { "reached" } else { "refused" }
}

fn plan(
    project: &Project,
    slot: &Slot,
    stack: &str,
    images: &Images,
    scratch: &std::path::Path,
) -> Result<Plan, ProbeError> {
    let harness = {
        use crate::harness::Harness;
        crate::harness::claude_code::ClaudeCode::new(
            Default::default(),
            Box::new(crate::harness::spawn::LocalSpawner),
        )
        .provision()
        .domains
    };
    let perimeter = compute(
        Role::Coder,
        &Sources {
            stack: &project.stack_domains(stack).unwrap_or_default(),
            harness: &harness,
            services: &[],
            forge: &project.config.forge,
        },
    )?;
    let (uid, gid) = image::host_ids();

    Ok(Plan {
        slot: format!("{}-check", slot.name),
        role: Role::Coder,
        image: images.agent.clone(),
        firewall_image: images.firewall.clone(),
        user: UserIds { uid, gid },
        tree: slot.tree.clone(),
        tree_at: PathBuf::from("/work/tree"),
        mission_dir: scratch.to_path_buf(),
        mission_dir_at: PathBuf::from("/work/mission"),
        credentials: Vec::new(),
        volumes: Vec::<NamedVolume>::new(),
        environment: BTreeMap::new(),
        command: vec!["sleep".to_string(), "600".to_string()],
        perimeter,
        project_services: None,
        project_networks: None,
        project_volumes: None,
    })
}

/// The service the battery is run from.
pub const PROBER_SERVICE: &str = "prober";

/// The Compose project a probe run uses: the slot's, with `-check` appended.
///
/// A name of its own, deliberately. A slot's services are levied once and
/// kept between profiles (SPEC 4.2); probing under the slot's own name would
/// take them down at the end of the check, along with the migrations and
/// fixtures a mission had left in them.
pub fn compose_project(slot: &str) -> Result<String, crate::compose::ComposeError> {
    crate::compose::project_name(&format!("{slot}-check"))
}

/// The prober, declared the way a project declares a service so that it goes
/// through the generator's own merge rather than a second code path.
fn prober_service(image: &str, user: UserIds) -> Result<serde_yaml_ng::Value, ProbeError> {
    let yaml = format!(
        "{PROBER_SERVICE}:\n  \
           image: {image}\n  \
           network_mode: \"service:{FIREWALL_SERVICE}\"\n  \
           cap_drop: [ALL]\n  \
           security_opt: [\"no-new-privileges:true\"]\n  \
           user: \"{}:{}\"\n  \
           depends_on:\n    \
             {FIREWALL_SERVICE}:\n      \
               condition: service_healthy\n  \
           command: [\"sleep\", \"600\"]\n",
        user.uid, user.gid
    );
    Ok(serde_yaml_ng::from_str(&yaml).expect("the prober service is valid YAML"))
}

/// The services a probe run lifts, named for a caller that wants to stop
/// them.
pub const SERVICES: [&str; 3] = [AGENT_SERVICE, FIREWALL_SERVICE, PROBER_SERVICE];
