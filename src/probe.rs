//! Probing the perimeter from inside a container (SPEC 4.1 bis, rule 7).
//!
//! This is the half of `nunki check` that cannot be answered by reading files:
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
    #[error("the slot's images are missing: {0} — `nunki slot rebuild {1}` builds them")]
    NoImages(String, String),
    #[error("the profile did not come up: {0}")]
    Up(#[from] EngineError),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("the profile could not be generated: {0}")]
    Compose(#[from] crate::compose::ComposeError),
    #[error("the allowlist could not be computed: {0}")]
    Perimeter(#[from] crate::perimeter::PerimeterError),
    #[error("mission {0} has not started, so it has no frozen header to lift a profile from")]
    NotStarted(String),
    #[error(transparent)]
    Slot(#[from] crate::slot::SlotError),
    #[error("the system profile could not be planned: {0}")]
    Plan(#[from] crate::run::RunError),
    #[error(
        "the project's services file declares a service named {0:?}, which the check needs for its prober"
    )]
    ProberNameTaken(String),
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

    let compose_project = compose_project(&project.session(), &slot.name)?;
    let _ = engine.down(&file, &compose_project, true);
    engine.up(&file, &compose_project)?;

    let allowed = plan
        .perimeter
        .domains
        .iter()
        .next()
        .cloned()
        .unwrap_or_else(|| "example.com".to_string());
    let checks = run_battery(
        engine.as_ref(),
        &file,
        &compose_project,
        crate::perimeter::probes(&allowed, &[]),
        "the mission profile",
    );

    engine.down(&file, &compose_project, true)?;
    Ok(checks)
}

/// Run each probe from the prober and judge it: green when the fence did
/// what the probe expected, red when it did not. Shared by both profiles, so
/// "green" means the same thing in each.
fn run_battery(
    engine: &dyn Engine,
    file: &std::path::Path,
    compose_project: &str,
    probes: Vec<crate::perimeter::Probe>,
    profile: &str,
) -> Vec<Check> {
    probes
        .into_iter()
        .map(|probe| {
            let out = engine.exec(
                file,
                compose_project,
                PROBER_SERVICE,
                &["sh".to_string(), "-c".to_string(), probe.script.clone()],
            );
            let reached = out.map(|o| o.ok()).unwrap_or(false);
            let what = format!("from inside {profile}: {}", probe.what);
            if reached == probe.expected {
                Check {
                    what,
                    verdict: Verdict::Green(say(reached).to_string()),
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
        .collect()
}

/// Lift the system profile of mission `id` and try to get out of it (SPEC
/// 4.1 bis, rule 7, `nunki check --mission`).
///
/// The profile is the one `nunki verify` lifts for the integrator — planned by
/// [`crate::run::plan`] itself, from the mission's **frozen** header, so the
/// check cannot drift from the run — under a slot of its own (see
/// [`system_plan`]). Everything it lifts, it takes down again, and it does so
/// even when the profile fails to come up: a half-lifted system profile is a
/// database left running.
pub fn system_profile(
    project: &Project,
    id: &str,
    engine: Arc<dyn Engine>,
    engine_bin: &str,
) -> Result<Vec<Check>, ProbeError> {
    let state = crate::state::Store::open(&project.hq_root)
        .and_then(|s| s.load(id))
        .map_err(|_| ProbeError::NotStarted(id.to_string()))?;
    let header = state.flow.header().clone();
    let what = format!("the perimeter holds from inside the system profile of {id}");
    if !header.has_integration() {
        return Ok(vec![Check {
            what,
            verdict: Verdict::NotChecked(format!(
                "mission {id} declares no service, so it has no system profile to lift"
            )),
        }]);
    }
    let Some(stack) = project.config.stacks.first().cloned() else {
        return Ok(vec![Check {
            what,
            verdict: Verdict::NotChecked(
                "no stack declared in nunki.yaml, so no image to lift".to_string(),
            ),
        }]);
    };
    let slot = crate::slot::find(project, &state.slot)?;
    let images = image::names(project, &stack);
    for image in [&images.agent, &images.firewall, &images.prober] {
        if !image::present(engine_bin, image) {
            return Err(ProbeError::NoImages(image.clone(), slot.name.clone()));
        }
    }

    let paths = scratch_paths(project, &slot, id)?;
    let plan = system_plan(project, &slot, &stack, &images, &paths, &header)?;
    let checks_dir = project.hq_root.join("checks");
    std::fs::create_dir_all(&checks_dir).map_err(|e| ProbeError::Io(checks_dir.clone(), e))?;
    let file = checks_dir.join(format!("{}-system.yml", slot.name));
    std::fs::write(&file, crate::compose::generate(&plan, engine.dialect())?)
        .map_err(|e| ProbeError::Io(file.clone(), e))?;

    let compose_project = compose_project(&project.session(), &slot.name)?;
    let _ = engine.down(&file, &compose_project, true);
    if let Err(e) = engine.up(&file, &compose_project) {
        let _ = engine.down(&file, &compose_project, true);
        return Err(e.into());
    }

    let profile = format!("the system profile of {id}");
    let mut checks = run_battery(
        engine.as_ref(),
        &file,
        &compose_project,
        system_probes(&plan, &header),
        &profile,
    );
    // Said, not skipped: a service is reached on the port it listens on, and
    // nothing tells nunki which port that is. Resolving it is probed; reaching
    // it is not, and a green here would be one nobody measured.
    checks.push(Check {
        what: format!("from inside {profile}: a declared service is reachable on its port"),
        verdict: Verdict::NotChecked(
            "nunki does not know which port a service listens on, so it probes that the \
             service resolves, not that it answers"
                .to_string(),
        ),
    });

    engine.down(&file, &compose_project, true)?;
    Ok(checks)
}

/// The mission folder a system-profile check mounts: a scratch one under
/// the HQ's `checks/`, never the mission's own.
///
/// The profile mounts the mission folder with the agent's files writable
/// (`JOURNAL.md`, `PR.md`, …). The check's container only sleeps today, but
/// a check that bound a real mission's journal read-write would be one exec
/// away from writing into it — so it gets the same shape of folder, empty,
/// as the mission-profile probe does.
pub fn scratch_paths(
    project: &Project,
    slot: &Slot,
    id: &str,
) -> Result<crate::mission::dir::Paths, ProbeError> {
    let root = project
        .hq_root
        .join("checks")
        .join(format!("{}-system", slot.name));
    let paths = crate::mission::dir::Paths::of(&root, id);
    std::fs::create_dir_all(&paths.dir).map_err(|e| ProbeError::Io(paths.dir.clone(), e))?;
    for file in AGENT_WRITABLE {
        let path = paths.dir.join(file);
        if !path.exists() {
            std::fs::write(&path, "").map_err(|e| ProbeError::Io(path, e))?;
        }
    }
    Ok(paths)
}

/// The system profile a check lifts: [`crate::run::plan`]'s, for the
/// integrator, under a slot named `<slot>-check`.
///
/// The name is the safety of it. Everything nunki names after a slot — the
/// Compose project, the network, every named volume — becomes the check's
/// and not the slot's, so the project's services (a database with its data)
/// are lifted a second time **beside** the slot's rather than on top of
/// them: two databases on one volume is a corrupted volume. The one way
/// around it is a project volume with an explicit `name:` in its services
/// file, which Compose then shares across projects; that is the project's
/// to avoid, and it is written here so nobody has to rediscover it.
///
/// No token: a check must not spend a subscription, and nothing here runs
/// the harness — the agent's container only sleeps.
pub fn system_plan(
    project: &Project,
    slot: &Slot,
    stack: &str,
    images: &Images,
    paths: &crate::mission::dir::Paths,
    header: &crate::mission::Header,
) -> Result<Plan, ProbeError> {
    let check_slot = Slot {
        name: format!("{}-check", slot.name),
        tree: slot.tree.clone(),
    };
    let mut plan = crate::run::plan(
        project,
        &check_slot,
        stack,
        images,
        paths,
        "nunki-check-spends-no-subscription",
        header,
        Role::Integrator,
    )?;
    plan.command = vec!["sleep".to_string(), "600".to_string()];

    let serde_yaml_ng::Value::Mapping(prober) = prober_service(&images.prober, plan.user)? else {
        unreachable!("prober_service builds a mapping");
    };
    plan.project_services = Some(match plan.project_services.take() {
        None => serde_yaml_ng::Value::Mapping(prober),
        Some(serde_yaml_ng::Value::Mapping(mut services)) => {
            if services.contains_key(serde_yaml_ng::Value::from(PROBER_SERVICE)) {
                return Err(ProbeError::ProberNameTaken(PROBER_SERVICE.to_string()));
            }
            services.extend(prober);
            serde_yaml_ng::Value::Mapping(services)
        }
        // Not a mapping: handed on as it is, for the generator to refuse
        // with the message it already has for it.
        Some(other) => other,
    });
    Ok(plan)
}

/// What is tried from inside a system profile: the mission profile's
/// battery, plus the two things only a system profile has.
///
/// Each service the mission declares must resolve. Each service the
/// **project** lifts but the mission does not declare must not: the
/// project's services file is merged whole, so an undeclared database is on
/// the same network, and the fence is the only thing between it and the
/// agent. That probe would succeed without the firewall — Compose's own
/// resolver answers every service name — which is what makes it one.
pub fn system_probes(plan: &Plan, header: &crate::mission::Header) -> Vec<crate::perimeter::Probe> {
    use crate::perimeter::Probe;

    let declared: Vec<String> = match &header.integration {
        crate::mission::Integration::Services { services, .. } => {
            services.iter().flat_map(|s| s.reach.clone()).collect()
        }
        crate::mission::Integration::None { .. } => Vec::new(),
    };
    // The battery's allowed host must be one the perimeter allows for its
    // own sake — a registry, the model's API — and never a declared service:
    // a service listens where it listens, and the battery's TCP probe on 443
    // against a database would fail on the port and read as a wall that is
    // not there.
    let allowed = plan
        .perimeter
        .domains
        .iter()
        .find(|d| !declared.contains(d))
        .cloned()
        .unwrap_or_else(|| "example.com".to_string());
    let mut probes = crate::perimeter::probes(&allowed, &[]);

    let resolves =
        |name: &str| format!("nslookup {name} 2>&1 | tail -5 | grep -q 'Address: [0-9]'");
    for name in &declared {
        probes.push(Probe {
            what: format!("{name}, a service the mission declares, resolves"),
            expected: true,
            script: resolves(name),
        });
    }
    if let Some(serde_yaml_ng::Value::Mapping(services)) = &plan.project_services {
        for name in services.keys().filter_map(|k| k.as_str()) {
            if name != PROBER_SERVICE && !declared.iter().any(|d| d == name) {
                probes.push(Probe {
                    what: format!(
                        "{name}, a project service the mission does not declare, resolves"
                    ),
                    expected: false,
                    script: resolves(name),
                });
            }
        }
    }
    probes
}

fn say(reached: bool) -> &'static str {
    if reached { "reached" } else { "refused" }
}

/// The profile a probe run lifts, and **not** the one `nunki mission start`
/// lifts.
///
/// It is deliberately a mission profile without a mission: no token — a
/// check must run without spending a subscription — no frozen header, no
/// slot volumes, and a Compose project name of its own. What it must not
/// drift on is the **allowlist**, and it does not: the perimeter comes from
/// `perimeter::compute` with the same sources `run::plan` gives it, which is
/// the thing under test here. The mounts differ and are not probed. The
/// system profile is planned differently, by `run::plan` itself — see
/// [`system_plan`].
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
        session: project.session(),
        slot: format!("{}-check", slot.name),
        role: Role::Coder,
        image: images.agent.clone(),
        firewall_image: images.firewall.clone(),
        user: UserIds { uid, gid },
        tree: slot.tree.clone(),
        tree_at: PathBuf::from("/work/tree"),
        mission_dir: scratch.to_path_buf(),
        mission_dir_at: PathBuf::from("/work/mission"),
        stack_scripts: crate::run::stack_scripts(project, stack),
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
pub fn compose_project(session: &str, slot: &str) -> Result<String, crate::compose::ComposeError> {
    crate::compose::project_name(session, &format!("{slot}-check"))
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
