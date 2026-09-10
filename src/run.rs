//! Starting a run (SPEC 4.3, "un run par lot").
//!
//! What this module does, in order, and every step is a decision written
//! down somewhere: take the slot's lock; put the slot on the mission's
//! branch; freeze the header into the state, because from here on `hq` reads
//! its own copy and never the file again; lift the role's profile; and launch
//! the harness **inside** the agent's container, recording the handle so a
//! restarted `hq` can find the run again.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::compose::{AGENT_SERVICE, FIREWALL_SERVICE, NamedVolume, Plan, UserIds};
use crate::engine::spawn::ContainerSpawner;
use crate::engine::{Engine, EngineError};
use crate::git;
use crate::harness::{
    Exposure, Harness, Role, RunHandle, RunRequest, SessionId, Workspace, claude_code,
};
use crate::image;
use crate::mission::dir::{self as mission_dir, Paths};
use crate::mission::flow::Flow;
use crate::perimeter::{Profile, Sources, compute};
use crate::project::Project;
use crate::role;
use crate::slot::Slot;
use crate::state::{MissionState, Store, lock::SlotLock};

/// Where the mission folder and the tree are mounted, in every profile.
pub const TREE_AT: &str = "/work/tree";
pub const MISSION_AT: &str = "/work/mission";

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("mission {0} has already started; `hq mission status {0}` says where it is")]
    AlreadyStarted(String),
    #[error(transparent)]
    Account(#[from] crate::account::AccountError),
    #[error(
        "account {account:?} authenticates {has}, but this project runs {wants} — \
         name an account for {wants}, or change the project's harness"
    )]
    WrongHarness {
        account: String,
        has: String,
        wants: String,
    },
    #[error("no home directory: accounts live under ~/.hq")]
    NoHome,
    #[error("the images are missing — `hq slot rebuild` builds them ({0})")]
    NoImages(String),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Git(#[from] git::GitError),
    #[error(transparent)]
    Mission(#[from] mission_dir::MissionDirError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(transparent)]
    Lock(#[from] crate::state::LockError),
    #[error(transparent)]
    Compose(#[from] crate::compose::ComposeError),
    #[error(transparent)]
    Perimeter(#[from] crate::perimeter::PerimeterError),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("the run could not be launched: {0}")]
    Launch(String),
    #[error(
        "hq.yaml names {0} as the project's services file, and the slot's tree has no \
         such file on this commit"
    )]
    NoServicesFile(PathBuf),
    #[error("{0} is not a Compose file hq can read: {1}")]
    BadServicesFile(PathBuf, String),
    #[error(transparent)]
    Application(#[from] crate::launch::LaunchError),
    #[error(
        "the security profile is up but {0:?} is not writable in it — the stack declares \
         it in writable.txt, and a named volume takes its ownership from the image, so \
         an image built before that line yields a root-owned directory. \
         `hq slot rebuild` builds it again{1}"
    )]
    NotWritable(String, String),
}

/// Which account a mission spends, and the token to pass. The mission's
/// header wins over the project's declaration, which wins over the index's
/// default — and the token reaches the container as an environment variable
/// at launch, never written into a slot (SPEC 3.3, 4.3).
pub fn account_for(
    project: &Project,
    from_mission: Option<&str>,
) -> Result<(String, crate::account::Account, String), RunError> {
    let hq_home = project.hq_home();
    let accounts = crate::account::Accounts::load(&hq_home)?;
    let (name, account) =
        accounts.choose(&hq_home, from_mission, project.config.account.as_deref())?;
    if account.harness != project.config.harness {
        return Err(RunError::WrongHarness {
            account: name,
            has: account.harness,
            wants: project.config.harness.clone(),
        });
    }
    let token = account.token(&hq_home, &name)?;
    Ok((name, account, token))
}

/// Start the mission's first run: the coder, on its first lot.
pub fn start(
    project: &Project,
    id: &str,
    slot: &Slot,
    engine: Arc<dyn Engine>,
    engine_bin: &str,
) -> Result<MissionState, RunError> {
    let store = Store::open(&project.hq_root)?;
    if store.load(id).is_ok() {
        return Err(RunError::AlreadyStarted(id.to_string()));
    }

    // The lock is taken before anything is changed: two `hq` on one slot is
    // two agents in one tree (SPEC 4.2).
    let _lock = SlotLock::acquire(&project.hq_root.join("locks"), &slot.name, "mission start")?;

    // The header is read once, here, and frozen: from now on `hq` uses its
    // own copy, so nothing an agent writes can change the perimeter, the
    // bounds or the lots (SPEC 4.1).
    let header = mission_dir::read_header(&project.hq_root, id)?;
    let paths = Paths::of(&project.hq_root, id);

    let lot = header
        .lots
        .first()
        .map(|l| l.id.clone())
        .unwrap_or_else(|| "L1".to_string());

    let launched = launch(&Launching {
        project,
        slot,
        engine,
        engine_bin,
        paths: &paths,
        header: &header,
        role: Role::Coder,
        lot,
        attempt: 1,
    })?;

    let mut state = MissionState {
        id: id.to_string(),
        slot: slot.name.clone(),
        flow: Flow::new(header).map_err(crate::state::StateError::Flow)?,
        run: Some(launched.run),
        app: launched.app,
        updated_at: String::new(),
    };
    store.save(&state)?;
    state.updated_at = crate::state::now_rfc3339();
    Ok(state)
}

/// Everything one launch needs that its caller already holds. Gathered as a
/// struct because `launch` takes no lock and asks the store nothing: the
/// caller has both, and a second `SlotLock::acquire` under `hq verify` would
/// be `hq` refusing itself (SPEC 4.2, "`hq exec` lancé par `verify`
/// s'exécute **sous** son verrou").
pub struct Launching<'a> {
    pub project: &'a Project,
    pub slot: &'a Slot,
    pub engine: Arc<dyn Engine>,
    pub engine_bin: &'a str,
    pub paths: &'a Paths,
    /// The **frozen** header, never the file.
    pub header: &'a crate::mission::Header,
    pub role: Role,
    /// What this run is for, as the harness records it.
    pub lot: String,
    pub attempt: u32,
}

/// What a launch left behind, for the caller to persist.
pub struct Launched {
    pub run: RunHandle,
    /// The application `hq` started for this role, when the mission has one
    /// to start. Kept apart from the run: "the agent is up" and "the
    /// deliverable is up" are two facts, and a single handle would report
    /// them as one.
    pub app: Option<RunHandle>,
    /// How the application was declared, so the caller can say it. `None`
    /// for the coder, who has nothing to start.
    pub launch: Option<crate::launch::Launch>,
}

/// Lift the role's profile and launch its run, on a slot whose lock the
/// caller already holds.
///
/// The order is the one SPEC 4.2 fixes and it is not interchangeable:
///
/// 1. resolve the launch script — **before** anything is taken down, so a
///    mission that cannot start its application says so while the previous
///    role's container is still up;
/// 2. switch the profile: stop and remove the agent and its sidecar, and
///    **only** those. The project's services were lifted once for the slot
///    and keep the state the previous role left in them — migrations played,
///    fixtures laid (rule 1);
/// 3. start the application, in the new profile, **before** the agent (rule
///    2);
/// 4. launch the agent.
pub fn launch(l: &Launching) -> Result<Launched, RunError> {
    let Launching {
        project,
        slot,
        engine,
        engine_bin,
        paths,
        header,
        role,
        ..
    } = l;
    let role = *role;

    // Which subscription this mission spends. Checked before the machine:
    // what the project declares is cheap to check and belongs to the
    // mission, while a missing image belongs to this machine. Saying "build
    // your images" to someone whose account is wrong helps nobody.
    let (_account_name, _account, token) = account_for(project, header.account.as_deref())?;

    let stack = stack_of(project);
    let images = image::names(project, &stack);
    for image in [&images.agent, &images.firewall] {
        if !image::present(engine_bin, image) {
            return Err(RunError::NoImages(image.clone()));
        }
    }

    // The slot on the mission's branch, before anything is written into it.
    // Idempotent: a role that follows another finds the branch already
    // checked out and this does nothing.
    branch(slot, &header.branch, &header.base)?;

    // Step 1. The coder starts nothing: there is nothing to test or attack
    // yet, and SPEC 4.2's table says so for the "code seul" shape.
    let declared = match role {
        Role::Coder => None,
        Role::Integrator | Role::Security => {
            Some(crate::launch::resolve(project, slot, &stack, header)?)
        }
    };

    let prompt = paths.dir.join(role::PROMPT_FILE);
    std::fs::write(&prompt, role::prompt(role)).map_err(|e| RunError::Io(prompt.clone(), e))?;

    let plan = plan(project, slot, &stack, &images, paths, &token, header, role)?;
    let file = profile_path(project, &slot.name);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| RunError::Io(dir.to_path_buf(), e))?;
    }
    let compose_project = crate::compose::project_name(&slot.name)?;

    // Step 2. The switch, read from the file that is up — the one that names
    // the services to remove. `stop` and never `down`: `down` would take the
    // project's services with it.
    if file.is_file() {
        engine.stop(&file, &compose_project, &SERVICES)?;
    }

    std::fs::write(&file, crate::compose::generate(&plan, engine.dialect())?)
        .map_err(|e| RunError::Io(file.clone(), e))?;
    // The generated profile carries the subscription token in the agent's
    // environment, so the file is the human's alone. It lives at the HQ and
    // never in the repository, and it is not readable by anyone else.
    restrict(&file)?;

    // A volume can only be mounted at a path the read-only bind already
    // carries: runc creates the mount point in the assembled root filesystem,
    // and under a `:ro` bind it cannot ("create mountpoint for
    // /work/tree/target: read-only file system", measured 2026-09-10). So the
    // directory is made in the slot's tree first. It is empty and, being
    // whatever the project ignores, invisible to `git status` — gate 1 stays
    // green.
    let writable = match role {
        Role::Security => project.stack_writable(&stack),
        _ => Vec::new(),
    };
    seed_writable(slot, &writable)?;

    engine.up(&file, &compose_project)?;
    // And the ownership of that volume comes from the **image**, not from the
    // tree: an image built before the stack declared this directory yields a
    // root-owned volume, silently, and the first build inside the container
    // fails on a permission nobody is watching. `image::present` cannot see
    // that — the tag is the same — so the only honest check is to ask the
    // container that is now up.
    if !writable.is_empty() {
        writable_or_rebuild(engine.clone(), &file, &compose_project, &writable)?;
    }

    // Step 3. The application, in the profile of the role that will test or
    // attack it, before that role is launched.
    let runs = paths.dir.join("runs");
    let app = match &declared {
        Some(declared) => {
            crate::launch::start(project, slot, engine.clone(), role, &runs, declared)?
        }
        None => None,
    };

    // Step 4. The agent.
    let session = SessionId(session_id());
    let spawner = ContainerSpawner::new(
        engine.clone(),
        file.clone(),
        &compose_project,
        AGENT_SERVICE,
    )
    .identified_by(&session.0);
    let harness = claude_code::ClaudeCode::new(Default::default(), Box::new(spawner));
    std::fs::create_dir_all(&runs).map_err(|e| RunError::Io(runs.clone(), e))?;
    let request = RunRequest {
        role,
        workspace: Workspace {
            tree: PathBuf::from(TREE_AT),
            mission_dir: PathBuf::from(MISSION_AT),
        },
        lot: l.lot.clone(),
        attempt: l.attempt,
        session,
        resume: false,
        // On the host: the container has nowhere to write a log, and the
        // mission folder is read-only but for the agent's own files
        // (SPEC 4.1).
        runs_dir: runs,
    };
    let run = harness
        .launch(
            &request,
            // What the role may do through the harness. The container is
            // what restrains it; this only keeps the ordinary work possible.
            &harness.guards(role),
            &Exposure::SystemPromptFile(PathBuf::from(MISSION_AT).join(role::PROMPT_FILE)),
        )
        .map_err(|e| RunError::Launch(e.to_string()))?;

    Ok(Launched {
        run,
        app,
        launch: declared,
    })
}

/// Make the declared directories exist in the slot's tree, so the profile can
/// mount a volume over each of them.
///
/// Public because it is a write into a slot, and a write into a slot is
/// something a test should be able to hold to account. What it writes is an
/// empty directory that the project already ignores — git tracks files, not
/// directories, so gate 1 stays green — and it writes nothing else.
pub fn seed_writable(slot: &Slot, writable: &[String]) -> Result<(), RunError> {
    for path in writable {
        let at = slot.tree.join(path);
        std::fs::create_dir_all(&at).map_err(|e| RunError::Io(at, e))?;
    }
    Ok(())
}

/// Ask the container that is up whether it can really write where the stack
/// says it must. Named here rather than discovered later: this is the one
/// failure of the security profile that produces no error of its own.
///
/// Public because it is the enforcement of a measurement, and a test that
/// cannot lift a stale image beside a good one cannot prove it enforces
/// anything.
pub fn writable_or_rebuild(
    engine: Arc<dyn Engine>,
    file: &std::path::Path,
    compose_project: &str,
    writable: &[String],
) -> Result<(), RunError> {
    let mut argv = vec![
        "sh".to_string(),
        "-c".to_string(),
        "for d in \"$@\"; do test -w \"$d\" || { echo \"$d\"; exit 1; }; done".to_string(),
        "sh".to_string(),
    ];
    argv.extend(writable.iter().map(|p| format!("{TREE_AT}/{p}")));
    let out = engine.exec(file, compose_project, AGENT_SERVICE, &argv)?;
    if out.ok() {
        return Ok(());
    }
    Err(RunError::NotWritable(
        out.stdout.trim().to_string(),
        [out.stderr.trim()]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    ))
}

/// The stack a project's profiles are built from. One place, because three
/// modules had begun to each spell the same fallback.
pub fn stack_of(project: &Project) -> String {
    project
        .config
        .stacks
        .first()
        .cloned()
        .unwrap_or_else(|| "rust".to_string())
}

/// Where a slot's current profile is written. One file per slot, regenerated
/// at every launch (SPEC 4.2).
pub fn profile_path(project: &Project, slot: &str) -> PathBuf {
    project.hq_root.join("profiles").join(format!("{slot}.yml"))
}

/// The services a mission's profile lifts, for a caller taking them down.
pub const SERVICES: [&str; 2] = [AGENT_SERVICE, FIREWALL_SERVICE];

/// Put the slot on the mission's branch, from its base.
fn branch(slot: &Slot, branch: &str, base: &str) -> Result<(), git::GitError> {
    if git::current_branch(&slot.tree)? == branch {
        return Ok(());
    }
    // The base as the clone knows it: a fresh clone has the remote's branches
    // and only one of its own.
    let start = if git::run(&slot.tree, &["rev-parse", "--verify", base]).is_ok() {
        base.to_string()
    } else {
        format!("origin/{base}")
    };
    git::run(&slot.tree, &["checkout", "-q", "-B", branch, &start])?;
    Ok(())
}

/// The profile a run lifts, as data. Public because it **is** the run's
/// perimeter, its mounts and its secret: a test that cannot read it can only
/// check that the file was written, not what it says.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    project: &Project,
    slot: &Slot,
    stack: &str,
    images: &image::Images,
    paths: &Paths,
    token: &str,
    header: &crate::mission::Header,
    role: Role,
) -> Result<Plan, RunError> {
    let profile = Profile::of(role);
    let harness_domains = {
        claude_code::ClaudeCode::new(
            Default::default(),
            Box::new(crate::harness::spawn::LocalSpawner),
        )
        .provision()
        .domains
    };
    // A mission profile carries no service, whatever the header declares:
    // the coder has nothing to reach, and `compute` refuses one anyway. The
    // system profile adds exactly the services the frozen header declared,
    // address by address (SPEC 4.1 bis, rule 6).
    let declared: &[crate::mission::Service] = match (profile, &header.integration) {
        (Profile::System, crate::mission::Integration::Services { services, .. }) => services,
        _ => &[],
    };
    let perimeter = compute(
        role,
        &Sources {
            stack: &project.stack_domains(stack).unwrap_or_default(),
            harness: &harness_domains,
            services: declared,
            forge: &project.config.forge,
        },
    )?;
    let (uid, gid) = image::host_ids();

    let mut environment = BTreeMap::new();
    // The one secret that enters a container, and it enters at launch, never
    // through a file in the slot (SPEC 3.3, 4.3). Which variable carries it
    // is the harness's business, not this module's.
    let harness = claude_code::ClaudeCode::new(
        Default::default(),
        Box::new(crate::harness::spawn::LocalSpawner),
    );
    environment.insert(harness.token_env().to_string(), token.to_string());
    environment.insert("HQ_ROLE".to_string(), role::slug(role).to_string());
    environment.insert("HQ_BRANCH".to_string(), header.branch.clone());

    // The test credentials, on the system profile and nowhere else (SPEC
    // 3.1). `compose::build` refuses them on a mission profile, so this is
    // two guards on one rule and that is deliberate: one of them is a type
    // error away from being deleted, the other is not.
    let credentials = match profile {
        Profile::Mission => Vec::new(),
        Profile::System => credentials(project)?,
    };

    // The project's own services, merged verbatim into the system profile.
    // Not into the mission one: the coder reaches no service, so lifting a
    // database beside it would be lifting what its allowlist forbids it to
    // talk to.
    let (project_services, project_networks, project_volumes) = match profile {
        Profile::Mission => (None, None, None),
        Profile::System => project_compose(project, slot)?,
    };

    Ok(Plan {
        slot: slot.name.clone(),
        role,
        image: images.agent.clone(),
        firewall_image: images.firewall.clone(),
        user: UserIds { uid, gid },
        tree: slot.tree.clone(),
        tree_at: PathBuf::from(TREE_AT),
        mission_dir: paths.dir.clone(),
        mission_dir_at: PathBuf::from(MISSION_AT),
        credentials,
        volumes: volumes(project, slot, stack, role),
        environment,
        // The container stays up between runs; the harness is exec'd into it.
        command: vec!["sleep".to_string(), "infinity".to_string()],
        perimeter,
        project_services,
        project_networks,
        project_volumes,
    })
}

/// The named volumes a role's profile carries.
///
/// The first three are every profile's: the clean copy of `HEAD` with its
/// build cache, the harness's sessions, the package cache. The rest exist
/// only for the security agent, whose tree is mounted read-only — the stack
/// declares what an execution must still be able to write and it is mounted
/// as a volume of its own (SPEC 4.2, rule 3). What is not declared stays
/// closed.
fn volumes(project: &Project, slot: &Slot, stack: &str, role: Role) -> Vec<NamedVolume> {
    let mut volumes = vec![
        // The clean copy of HEAD that `hq exec` replays proofs on, and its
        // build cache with it: warmed once per slot and kept (SPEC 4.2,
        // 4.4 gate 7).
        crate::exec::volume(&slot.name),
        // The harness keeps its sessions here, so resuming survives a
        // rebuilt container (SPEC 4.3).
        NamedVolume {
            name: format!("hq-{}-harness", slot.name),
            at: PathBuf::from("/home/agent/.claude"),
        },
        NamedVolume {
            name: format!("hq-{}-cargo", slot.name),
            at: PathBuf::from("/home/agent/.cargo/registry"),
        },
    ];
    if role != Role::Security {
        return volumes;
    }
    for path in project.stack_writable(stack) {
        volumes.push(NamedVolume {
            name: writable_volume(&slot.name, &path),
            at: PathBuf::from(TREE_AT).join(&path),
        });
    }
    volumes
}

/// The volume behind one writable directory. Named per profile, not per
/// slot: SPEC 4.2 says "montés comme volumes propres au profil", and a
/// `target/` shared with the coder's would hand the security agent a build
/// tree it is meant to attack from the outside.
pub fn writable_volume(slot: &str, path: &str) -> String {
    let sanitised: String = path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("hq-{slot}-security-{sanitised}")
}

/// Where the test credentials are mounted, read-only, on a system profile.
/// Under the agent's own tmpfs: it is the one writable place in a container
/// whose tree and mission folder are both restrained, and a file bound
/// underneath it stays read-only while the tmpfs above stays the agent's
/// (measured, 2026-09-10).
pub const CREDENTIALS_AT: &str = "/run/hq/credentials";

/// The credential files a system profile mounts: every file the project's
/// declared directory holds, sorted, so the generated profile is stable.
///
/// A directory that does not exist is not an error here. `hq check` is where
/// a missing credential is a finding; refusing to launch would turn "the
/// human has not put the files there yet" into "the mission cannot run",
/// which is a different sentence.
fn credentials(project: &Project) -> Result<Vec<(PathBuf, PathBuf)>, RunError> {
    let Some(dir) = &project.config.credentials else {
        return Ok(Vec::new());
    };
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| RunError::Io(dir.clone(), e))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| RunError::Io(dir.clone(), e))?;
        if !entry.path().is_file() {
            continue;
        }
        let name = entry.file_name();
        files.push((entry.path(), PathBuf::from(CREDENTIALS_AT).join(&name)));
    }
    files.sort();
    Ok(files)
}

/// The project's own `services:` and `networks:` blocks, read from the slot's
/// tree.
///
/// From the tree and not from the repository: the file is the project's, it
/// travels with the commit, and the integrator may amend it in its wiring —
/// the same rule as the launch script (SPEC 4.2). What `hq.yaml` decides is
/// **which** file; what the slot decides is what is in it.
type ProjectBlocks = (
    Option<serde_yaml_ng::Value>,
    Option<serde_yaml_ng::Value>,
    Option<serde_yaml_ng::Value>,
);

fn project_compose(project: &Project, slot: &Slot) -> Result<ProjectBlocks, RunError> {
    let Some(relative) = &project.config.services_file else {
        return Ok((None, None, None));
    };
    let file = slot.tree.join(relative);
    if !file.is_file() {
        return Err(RunError::NoServicesFile(file));
    }
    let text = std::fs::read_to_string(&file).map_err(|e| RunError::Io(file.clone(), e))?;
    let document: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text)
        .map_err(|e| RunError::BadServicesFile(file.clone(), e.to_string()))?;
    let pick = |key: &str| match &document {
        serde_yaml_ng::Value::Mapping(map) => map.get(serde_yaml_ng::Value::from(key)).cloned(),
        _ => None,
    };
    Ok((pick("services"), pick("networks"), pick("volumes")))
}

/// A v4-shaped identifier, from the clock and the process — enough to be
/// unique per run, and `hq` imposes it rather than reading one back.
fn session_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mix = now ^ (pid << 64);
    let hex = format!("{mix:032x}");
    format!(
        "{}-{}-4{}-8{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

#[cfg(unix)]
fn restrict(path: &PathBuf) -> Result<(), RunError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| RunError::Io(path.clone(), e))
}

#[cfg(not(unix))]
fn restrict(_path: &PathBuf) -> Result<(), RunError> {
    Ok(())
}
