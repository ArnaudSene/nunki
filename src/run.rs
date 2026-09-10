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
use crate::harness::{Exposure, Harness, Role, RunRequest, SessionId, Workspace, claude_code};
use crate::image;
use crate::mission::dir::{self as mission_dir, Paths};
use crate::mission::flow::Flow;
use crate::perimeter::{Sources, compute};
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

    // Which subscription this mission spends, decided here and frozen with
    // the header it was read from.
    let (account_name, _account, token) = account_for(project, header.account.as_deref())?;

    // Only now the machine: what the project declares is cheap to check and
    // belongs to the mission, while a missing image belongs to this machine.
    // Saying "build your images" to someone whose account is wrong helps
    // nobody.
    let stack = project
        .config
        .stacks
        .first()
        .cloned()
        .unwrap_or_else(|| "rust".to_string());
    let images = image::names(project, &stack);
    for image in [&images.agent, &images.firewall] {
        if !image::present(engine_bin, image) {
            return Err(RunError::NoImages(image.clone()));
        }
    }

    branch(slot, &header.branch, &header.base)?;

    let prompt = paths.dir.join(role::PROMPT_FILE);
    std::fs::write(&prompt, role::prompt(Role::Coder))
        .map_err(|e| RunError::Io(prompt.clone(), e))?;

    let lot = header
        .lots
        .first()
        .map(|l| l.id.clone())
        .unwrap_or_else(|| "L1".to_string());

    let plan = plan(project, slot, &stack, &images, &paths, &token, &header)?;
    let _ = &account_name;
    let file = profile_path(project, &slot.name);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| RunError::Io(dir.to_path_buf(), e))?;
    }
    std::fs::write(&file, crate::compose::generate(&plan, engine.dialect())?)
        .map_err(|e| RunError::Io(file.clone(), e))?;
    // The generated profile carries the subscription token in the agent's
    // environment, so the file is the human's alone. It lives at the HQ and
    // never in the repository, and it is not readable by anyone else.
    restrict(&file)?;

    let compose_project = crate::compose::project_name(&slot.name)?;
    engine.up(&file, &compose_project)?;

    let session = SessionId(session_id());
    let spawner = ContainerSpawner::new(
        engine.clone(),
        file.clone(),
        &compose_project,
        AGENT_SERVICE,
    )
    .identified_by(&session.0);
    let harness = claude_code::ClaudeCode::new(Default::default(), Box::new(spawner));
    std::fs::create_dir_all(paths.dir.join("runs"))
        .map_err(|e| RunError::Io(paths.dir.join("runs"), e))?;
    let request = RunRequest {
        role: Role::Coder,
        workspace: Workspace {
            tree: PathBuf::from(TREE_AT),
            mission_dir: PathBuf::from(MISSION_AT),
        },
        lot,
        attempt: 1,
        session,
        resume: false,
        // On the host: the container has nowhere to write a log, and the
        // mission folder is read-only but for three files (SPEC 4.1).
        runs_dir: paths.dir.join("runs"),
    };
    let handle = harness
        .launch(
            &request,
            // What the role may do through the harness. The container is
            // what restrains it; this only keeps the ordinary work possible.
            &harness.guards(Role::Coder),
            &Exposure::SystemPromptFile(PathBuf::from(MISSION_AT).join(role::PROMPT_FILE)),
        )
        .map_err(|e| RunError::Launch(e.to_string()))?;

    let mut state = MissionState {
        id: id.to_string(),
        slot: slot.name.clone(),
        flow: Flow::new(header).map_err(crate::state::StateError::Flow)?,
        run: Some(handle),
        updated_at: String::new(),
    };
    store.save(&state)?;
    state.updated_at = crate::state::now_rfc3339();
    Ok(state)
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
) -> Result<Plan, RunError> {
    let harness_domains = {
        claude_code::ClaudeCode::new(
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
            harness: &harness_domains,
            services: &[],
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
    environment.insert("HQ_ROLE".to_string(), "coder".to_string());
    environment.insert("HQ_BRANCH".to_string(), header.branch.clone());

    Ok(Plan {
        slot: slot.name.clone(),
        role: Role::Coder,
        image: images.agent.clone(),
        firewall_image: images.firewall.clone(),
        user: UserIds { uid, gid },
        tree: slot.tree.clone(),
        tree_at: PathBuf::from(TREE_AT),
        mission_dir: paths.dir.clone(),
        mission_dir_at: PathBuf::from(MISSION_AT),
        credentials: Vec::new(),
        volumes: vec![
            // The clean copy of HEAD that `hq exec` replays proofs on, and
            // its build cache with it: warmed once per slot and kept
            // (SPEC 4.2, 4.4 gate 7).
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
        ],
        environment,
        // The container stays up between runs; the harness is exec'd into it.
        command: vec!["sleep".to_string(), "infinity".to_string()],
        perimeter,
        project_services: None,
        project_networks: None,
    })
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
