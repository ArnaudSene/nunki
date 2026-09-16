//! The launch script (SPEC 4.2, "les services et le lancement de
//! l'application", rule 2): who declares it, where it is read from, and what
//! `nunki` says when it is not there.

use std::path::{Path, PathBuf};

use nunki::launch::{self, Declared, Launch, LaunchError};
use nunki::mission::{Header, Integration, Lot, Security};
use nunki::project::{Config, Project, ProtectedPaths};
use nunki::slot::Slot;

fn config() -> Config {
    Config {
        root: None,
        harness: "claude-code".to_string(),
        forge: vec!["github.com".to_string()],
        stacks: vec!["rust".to_string()],
        protected_branches: vec!["main".to_string()],
        protected_paths: ProtectedPaths::default(),
        account: None,
        model: None,
        bounds: Default::default(),
        credentials: None,
        run: None,
        services_file: None,
        permission_mode: "auto".to_string(),
        forge_protection: Default::default(),
    }
}

fn header() -> Header {
    Header {
        branch: "feat/x".to_string(),
        base: "dev".to_string(),
        lots: vec![Lot {
            id: "L1".to_string(),
            title: "one".to_string(),
        }],
        integration: Integration::Services {
            services: Vec::new(),
            wiring: Vec::new(),
        },
        security: Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    }
}

/// A slot with one commit, so `git rev-parse HEAD` has an answer: the
/// message of a missing script names the commit it looked at.
fn slot(dir: &Path) -> Slot {
    let tree = dir.join("slot");
    std::fs::create_dir_all(&tree).unwrap();
    for args in [
        vec!["init", "-q", "-b", "feat/x"],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "T"],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(&args)
                .current_dir(&tree)
                .status()
                .unwrap()
                .success()
        );
    }
    std::fs::write(tree.join("README.md"), "x\n").unwrap();
    for args in [vec!["add", "-A"], vec!["commit", "-qm", "one"]] {
        assert!(
            std::process::Command::new("git")
                .args(&args)
                .current_dir(&tree)
                .status()
                .unwrap()
                .success()
        );
    }
    Slot {
        name: "one".to_string(),
        tree,
    }
}

fn write_script(tree: &Path, at: &str) {
    let path = tree.join(at);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "#!/bin/sh\nexec sleep infinity\n").unwrap();
    executable(&path);
}

#[cfg(unix)]
fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn executable(_path: &Path) {}

/// The stack's own launch script, where it lives: in the project's home.
fn stack_script(project: &Project) {
    write_script(&project.fragment("rust"), launch::SCRIPT);
}

#[test]
fn the_stacks_script_is_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot(dir.path());
    let project = Project::at(dir.path().join("repo"), config(), dir.path().join("nunki"));
    stack_script(&project);

    // Run where it is mounted, not where it is on the host.
    assert_eq!(
        launch::resolve(&project, &slot, "rust", &header()).unwrap(),
        Launch::Script {
            path: format!("{}/{}", nunki::run::STACK_AT, launch::SCRIPT),
            declared: Declared::Stack,
        }
    );
}

#[test]
fn nunki_yaml_replaces_the_stacks_script() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot(dir.path());
    // Both exist, so what is chosen says which declaration won and not which
    // file happened to be there.
    write_script(&slot.tree, "bin/serve");
    let mut config = config();
    config.run = Some("bin/serve".to_string());
    let project = Project::at(dir.path().join("repo"), config, dir.path().join("nunki"));
    stack_script(&project);

    assert_eq!(
        launch::resolve(&project, &slot, "rust", &header()).unwrap(),
        Launch::Script {
            path: "bin/serve".to_string(),
            declared: Declared::Config,
        }
    );
}

#[test]
fn the_mission_header_beats_nunki_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot(dir.path());
    write_script(&slot.tree, "bin/serve");
    write_script(&slot.tree, "bin/serve-this-mission");
    let mut config = config();
    config.run = Some("bin/serve".to_string());
    let project = Project::at(dir.path().join("repo"), config, dir.path().join("nunki"));
    stack_script(&project);
    let mut header = header();
    header.run = Some("bin/serve-this-mission".to_string());

    assert_eq!(
        launch::resolve(&project, &slot, "rust", &header).unwrap(),
        Launch::Script {
            path: "bin/serve-this-mission".to_string(),
            declared: Declared::Header,
        }
    );
}

/// `none` is how a project with no executable — a library — says there is
/// nothing to start, and the security agent then works on the code and the
/// build artefact (SPEC 4.2).
#[test]
fn none_means_there_is_nothing_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot(dir.path());

    let mut library = config();
    library.run = Some("none".to_string());
    let project = Project::at(dir.path().join("repo"), library, dir.path().join("nunki"));
    stack_script(&project);
    assert_eq!(
        launch::resolve(&project, &slot, "rust", &header()).unwrap(),
        Launch::Nothing {
            declared: Declared::Config
        }
    );

    // And a mission may say it for itself, over a project that declares one.
    let mut other = config();
    other.run = Some("bin/serve".to_string());
    let project = Project::at(dir.path().join("repo"), other, dir.path().join("nunki"));
    let mut header = header();
    header.run = Some("none".to_string());
    assert_eq!(
        launch::resolve(&project, &slot, "rust", &header).unwrap(),
        Launch::Nothing {
            declared: Declared::Header
        }
    );
}

/// A launch script the project declares is the project's: the integrator may
/// amend it and commit it, and it is **that** version `nunki` uses afterwards.
/// So the file is read from the slot's tree, and a copy in the repository the
/// slot was cloned from is not an answer.
#[test]
fn a_declared_script_is_read_from_the_slots_tree_and_not_from_the_repository() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot(dir.path());
    let repo = dir.path().join("repo");
    write_script(&repo, "bin/serve");
    let mut config = config();
    config.run = Some("bin/serve".to_string());
    let project = Project::at(repo, config, dir.path().join("nunki"));

    let err = launch::resolve(&project, &slot, "rust", &header()).unwrap_err();
    assert!(
        matches!(err, LaunchError::Missing { .. }),
        "the repository's copy is not the slot's: {err}"
    );

    // Put it in the slot — the integrator committing it — and it resolves.
    write_script(&slot.tree, "bin/serve");
    assert!(matches!(
        launch::resolve(&project, &slot, "rust", &header()).unwrap(),
        Launch::Script { .. }
    ));
}

/// The stack's default is not the project's: it lives in the project's home,
/// out of the tree, and a file of the same name committed in the tree is not
/// it — which is what keeps it out of an agent's reach.
#[test]
fn the_stacks_default_is_read_from_the_home_and_never_from_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot(dir.path());
    write_script(&slot.tree, ".nunki/stacks/rust/run.sh");
    let project = Project::at(dir.path().join("repo"), config(), dir.path().join("nunki"));

    let err = launch::resolve(&project, &slot, "rust", &header()).unwrap_err();
    assert!(
        matches!(err, LaunchError::NoStackScript { .. }),
        "a copy in the tree is not the stack's: {err}"
    );
    assert!(
        err.to_string().contains("run: none"),
        "it says how to declare nothing: {err}"
    );

    stack_script(&project);
    assert!(matches!(
        launch::resolve(&project, &slot, "rust", &header()).unwrap(),
        Launch::Script {
            declared: Declared::Stack,
            ..
        }
    ));
}

#[test]
fn a_missing_script_names_the_declaration_and_the_commit() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot(dir.path());
    let mut config = config();
    config.run = Some("bin/serve".to_string());
    let project = Project::at(dir.path().join("repo"), config, dir.path().join("nunki"));

    let err = launch::resolve(&project, &slot, "rust", &header()).unwrap_err();
    let said = err.to_string();
    assert!(said.contains("nunki.yaml"), "{said}");
    assert!(said.contains("bin/serve"), "{said}");
    let head = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&slot.tree)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(said.contains(head.trim()), "{said} does not name {head}");
}

/// `nunki` runs the script; it does not guess an interpreter for it. A file
/// without the bit is a launch that would fail inside a container, in a log
/// nobody is reading.
#[test]
fn a_script_that_is_not_executable_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot(dir.path());
    let project = Project::at(dir.path().join("repo"), config(), dir.path().join("nunki"));
    let path = project.fragment("rust").join(launch::SCRIPT);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "#!/bin/sh\nexec sleep infinity\n").unwrap();

    let err = launch::resolve(&project, &slot, "rust", &header()).unwrap_err();
    assert!(matches!(err, LaunchError::NotExecutable { .. }), "{err}");
}

/// `nunki init` must ship one, or every fresh project's first integration
/// mission fails on a file nobody was told to write.
#[test]
fn the_rust_fragment_ships_a_launch_script() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    nunki::init::init(&root, &dir.path().join("nunki"), &["rust".to_string()]).unwrap();

    let script = dir.path().join("nunki/stacks/rust").join(launch::SCRIPT);
    assert!(script.is_file(), "{} is missing", script.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "{script:?} is not executable: {mode:o}");
    }
    // And it must not `exec`: the wrapper that spawns it puts the run's
    // identifier on the command line, `nunki` recognises the process by it, and
    // a script that replaces itself replaces that command line — reporting an
    // application that has stopped (see the live test below).
    let body = std::fs::read_to_string(&script).unwrap();
    let command = body
        .lines()
        .rfind(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .unwrap_or_default();
    assert!(!command.trim_start().starts_with("exec "), "{body}");
}

/// The path the stack's default resolves to is the one the profile mounts the
/// script at: a launch that named a path nothing is mounted on would start
/// nothing, in a log nobody is reading.
#[test]
fn the_default_path_is_where_the_profile_mounts_the_script() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot(dir.path());
    let project = Project::at(dir.path().join("repo"), config(), dir.path().join("nunki"));
    stack_script(&project);

    let Launch::Script { path, .. } = launch::resolve(&project, &slot, "rust", &header()).unwrap()
    else {
        panic!("the stack ships a script, so there is one to start");
    };
    let mounted = nunki::run::stack_scripts(&project, "rust");
    assert!(
        mounted.iter().any(|(host, at)| {
            host == &project.fragment("rust").join(launch::SCRIPT) && at == Path::new(&path)
        }),
        "{path} is not where {mounted:?} puts it"
    );
}

// --- with a real engine ----------------------------------------------------

/// The two rules of SPEC 4.2 that only a container can prove:
///
/// 1. the project's services are lifted once per slot and **never stopped
///    between two profiles** — what the integrator laid down in them
///    survives the switch;
/// 2. `nunki` starts the application in the profile of the role that will test
///    it, before that role, and can tell afterwards that it is running.
///
/// It also measures the thing that makes rule 2 work at all: the run is
/// recognised by the identifier on its command line, so a launch script that
/// `exec`s replaces that command line and reports an application that has
/// stopped. Both shapes are run here, and they must not answer the same.
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_the_services_survive_a_switch_and_the_application_starts_in_the_profile() {
    use nunki::compose::{NamedVolume, Plan, UserIds, generate};
    use nunki::engine::docker::Docker;
    use nunki::engine::{Engine, Liveness};
    use nunki::harness::Role;
    use nunki::harness::spawn::{Presence, Spawned, Spawner};
    use nunki::perimeter::{Sources, compute};
    use std::sync::Arc;

    let docker = Docker::real();
    let slot_name = "launchlive";
    let compose_project =
        nunki::compose::project_name("11111111-2222-4333-8444-555555555555", slot_name).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let context = dir.path().join("firewall");
    std::fs::create_dir_all(&context).unwrap();
    nunki::firewall::materialise(&context).unwrap();
    assert!(
        std::process::Command::new("docker")
            .args(["build", "-q", "-t", "nunki/firewall:test"])
            .arg(&context)
            .status()
            .expect("docker is on the path")
            .success()
    );

    let mut slot = slot(dir.path());
    slot.name = slot_name.to_string();
    // Two launch scripts, identical but for the one line under measurement.
    std::fs::write(
        slot.tree.join("run.sh"),
        "#!/bin/sh\nset -eu\necho \"$1\" > /work/tree/app.marker\nsleep 600\n",
    )
    .unwrap();
    executable(&slot.tree.join("run.sh"));
    std::fs::write(
        slot.tree.join("run-exec.sh"),
        "#!/bin/sh\nset -eu\nexec sleep 600\n",
    )
    .unwrap();
    executable(&slot.tree.join("run-exec.sh"));

    let mission_dir = dir.path().join("mission");
    std::fs::create_dir_all(&mission_dir).unwrap();
    for f in nunki::compose::AGENT_WRITABLE {
        std::fs::write(mission_dir.join(f), "").unwrap();
    }

    let hq_root = dir.path().join("nunki");
    let mut declared = config();
    declared.run = Some("run.sh".to_string());
    let project = Project::at(dir.path().join("repo"), declared, hq_root.clone());
    let file = nunki::run::profile_path(&project, slot_name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();

    let profile = |role: Role| {
        let services = match role {
            Role::Coder => Vec::new(),
            _ => vec![nunki::mission::Service {
                name: "db".to_string(),
                reach: vec!["db".to_string()],
                shared: false,
            }],
        };
        let perimeter = compute(
            role,
            &Sources {
                stack: &["example.com".to_string()],
                harness: &[],
                services: &services,
                forge: &[],
            },
        )
        .unwrap();
        let (uid, gid) = nunki::image::host_ids();
        Plan {
            session: "11111111-2222-4333-8444-555555555555".into(),
            slot: slot_name.to_string(),
            role,
            image: "alpine:3.20".to_string(),
            firewall_image: "nunki/firewall:test".to_string(),
            user: UserIds { uid, gid },
            tree: slot.tree.clone(),
            tree_at: PathBuf::from("/work/tree"),
            mission_dir: mission_dir.clone(),
            mission_dir_at: PathBuf::from("/work/mission"),
            stack_scripts: Vec::new(),
            credentials: Vec::new(),
            volumes: Vec::<NamedVolume>::new(),
            environment: Default::default(),
            command: vec!["sleep".to_string(), "600".to_string()],
            perimeter,
            // Re-declared identically in both profiles — that identity is
            // what makes the switch a no-op for them. The named volume is
            // the point: it is the state the integrator leaves behind.
            project_services: Some(
                serde_yaml_ng::from_str(
                    "db:\n  image: alpine:3.20\n  command: [\"sleep\", \"600\"]\n  \
                     volumes:\n    - dbdata:/state\n",
                )
                .unwrap(),
            ),
            project_networks: None,
            // Without this the whole project is invalid — `service "db"
            // refers to undefined volume dbdata` (measured, and the reason
            // the block is merged at all).
            project_volumes: Some(serde_yaml_ng::from_str("dbdata: null\n").unwrap()),
        }
    };
    let write = |role: Role| {
        std::fs::write(&file, generate(&profile(role), docker.dialect()).unwrap()).unwrap();
    };

    let _ = docker.down(&file, &compose_project, true);
    write(Role::Coder);
    let _ = docker.down(&file, &compose_project, true);

    // The coder's profile, with the project's service beside it.
    docker.up(&file, &compose_project).unwrap();
    let db_before = docker
        .container_of(&file, &compose_project, "db")
        .unwrap()
        .expect("the project's service has a container");
    docker
        .exec(
            &file,
            &compose_project,
            "db",
            &[
                "sh".to_string(),
                "-c".to_string(),
                "echo 'migrations played' > /state/left-behind".to_string(),
            ],
        )
        .unwrap();

    // The switch: the agent and its sidecar go, and only those.
    docker
        .stop(&file, &compose_project, &nunki::run::SERVICES)
        .unwrap();
    write(Role::Integrator);
    docker.up(&file, &compose_project).unwrap();

    // Rule 1, measured: same container, same data.
    let db_after = docker
        .container_of(&file, &compose_project, "db")
        .unwrap()
        .expect("the project's service still has a container");
    assert_eq!(
        db_before, db_after,
        "the project's service was replaced by the switch"
    );
    assert_eq!(docker.liveness(&db_after).unwrap(), Liveness::Running);
    let left = docker
        .exec(
            &file,
            &compose_project,
            "db",
            &["cat".to_string(), "/state/left-behind".to_string()],
        )
        .unwrap();
    assert!(
        left.stdout.contains("migrations played"),
        "what the previous role laid down did not survive: {left:?}"
    );

    // Rule 2: the application starts in this profile, and `nunki` can tell.
    let engine: Arc<dyn Engine> = Arc::new(Docker::real());
    let runs = dir.path().join("runs");
    let launch = launch::resolve(&project, &slot, "rust", &header()).unwrap();
    assert_eq!(
        launch,
        Launch::Script {
            path: "run.sh".to_string(),
            declared: Declared::Config
        }
    );
    let handle = launch::start(
        &project,
        &slot,
        engine.clone(),
        nunki::harness::Role::Integrator,
        &runs,
        &launch,
    )
    .unwrap()
    .expect("a script means a started application");

    let spawner = nunki::engine::spawn::ContainerSpawner::new(
        engine.clone(),
        file.clone(),
        &compose_project,
        nunki::compose::AGENT_SERVICE,
    )
    .identified_by(&handle.session.0);
    let spawned = Spawned {
        pid: handle.pid,
        container: handle.container.clone(),
    };
    assert!(
        matches!(spawner.alive(&spawned).unwrap(), Presence::Running),
        "the application is up and nunki can say so"
    );
    // It really ran the project's script, in the tree, as the agent.
    let marker = slot.tree.join("app.marker");
    let mut waited = 0;
    while !marker.is_file() && waited < 50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        waited += 1;
    }
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().trim(),
        handle.session.0,
        "the script was called with the run's identifier"
    );

    // The measurement behind the fragment's comment: a script that `exec`s
    // loses the identifier from its command line, and the very same check
    // then answers "ended" on a process that is alive.
    let mut execs = config();
    execs.run = Some("run-exec.sh".to_string());
    let execing = Project::at(dir.path().join("repo"), execs, hq_root);
    let other = launch::resolve(&execing, &slot, "rust", &header()).unwrap();
    let exec_handle = launch::start(
        &execing,
        &slot,
        engine.clone(),
        nunki::harness::Role::Integrator,
        &runs,
        &other,
    )
    .unwrap()
    .unwrap();
    let exec_spawner = nunki::engine::spawn::ContainerSpawner::new(
        engine,
        file.clone(),
        &compose_project,
        nunki::compose::AGENT_SERVICE,
    )
    .identified_by(&exec_handle.session.0);
    let exec_spawned = Spawned {
        pid: exec_handle.pid,
        container: exec_handle.container.clone(),
    };
    // The process is there — `kill -0` on it succeeds — and the check still
    // says ended, because the command line no longer carries the identifier.
    let still_there = docker
        .exec(
            &file,
            &compose_project,
            nunki::compose::AGENT_SERVICE,
            &[
                "sh".to_string(),
                "-c".to_string(),
                format!("kill -0 {} && echo alive", exec_handle.pid.unwrap()),
            ],
        )
        .unwrap();
    assert!(still_there.stdout.contains("alive"), "{still_there:?}");
    assert!(
        matches!(exec_spawner.alive(&exec_spawned).unwrap(), Presence::Ended),
        "this is why the stack's run.sh does not exec"
    );

    docker.down(&file, &compose_project, true).unwrap();
}

/// The security profile, with a real engine (SPEC 4.2, rule 3): the tree is
/// read-only, the directories the stack declared are open, and everything
/// else is closed. Three things only a container can answer:
///
/// 1. a named volume mounted over a subpath of a `:ro` bind takes its
///    **ownership from the image**, so an image built before the stack
///    declared that directory yields a root-owned one — silently;
/// 2. the mount point must also exist in the **bind source**, or the
///    container does not start at all;
/// 3. `nunki` can tell the two apart at launch and name `nunki slot rebuild`.
#[test]
#[ignore = "builds images and lifts containers; run by hand"]
fn live_the_security_profile_writes_only_what_the_stack_declared() {
    use nunki::compose::{NamedVolume, Plan, UserIds, generate};
    use nunki::engine::Engine;
    use nunki::engine::docker::Docker;
    use nunki::harness::Role;
    use nunki::perimeter::{Sources, compute};
    use std::sync::Arc;

    let docker = Docker::real();
    let slot_name = "seclive";
    let compose_project =
        nunki::compose::project_name("11111111-2222-4333-8444-555555555555", slot_name).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let context = dir.path().join("firewall");
    std::fs::create_dir_all(&context).unwrap();
    nunki::firewall::materialise(&context).unwrap();
    let build = |tag: &str, at: &Path| {
        assert!(
            std::process::Command::new("docker")
                .args(["build", "-q", "-t", tag])
                .arg(at)
                .status()
                .expect("docker is on the path")
                .success(),
            "building {tag}"
        );
    };
    build("nunki/firewall:test", &context);

    // Two agent images, identical but for the one line under measurement:
    // the mount point `nunki slot rebuild` puts there from `writable.txt`.
    let (uid, gid) = nunki::image::host_ids();
    for (tag, mkdir) in [
        ("nunki/sec-good:test", "RUN mkdir -p /work/tree/target\n"),
        ("nunki/sec-stale:test", ""),
    ] {
        let at = dir.path().join(tag.replace([':', '/'], "-"));
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(
            at.join("Dockerfile"),
            format!(
                "FROM alpine:3.20\n\
                 RUN addgroup -g {gid} agent 2>/dev/null || true\n\
                 RUN adduser -D -u {uid} -G $(getent group {gid} | cut -d: -f1) agent \
                 2>/dev/null || true\n\
                 RUN mkdir -p /work/tree /work/mission /run/nunki && chown -R {uid}:{gid} /work\n\
                 USER {uid}:{gid}\n{mkdir}"
            ),
        )
        .unwrap();
        build(tag, &at);
    }

    let mut slot = slot(dir.path());
    slot.name = slot_name.to_string();
    std::fs::write(slot.tree.join("secret.rs"), "fn main() {}\n").unwrap();
    // What `nunki` does before lifting the profile: the mount point has to exist
    // in the bind source, or runc cannot create it under a read-only bind.
    std::fs::create_dir_all(slot.tree.join("target")).unwrap();

    let mission_dir = dir.path().join("mission");
    std::fs::create_dir_all(&mission_dir).unwrap();
    for f in nunki::compose::AGENT_WRITABLE {
        std::fs::write(mission_dir.join(f), "").unwrap();
    }
    let project = Project::at(dir.path().join("repo"), config(), dir.path().join("nunki"));
    let file = nunki::run::profile_path(&project, slot_name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();

    let write_profile = |image: &str| {
        let perimeter = compute(
            Role::Security,
            &Sources {
                stack: &["example.com".to_string()],
                harness: &[],
                services: &[],
                forge: &[],
            },
        )
        .unwrap();
        let plan = Plan {
            session: "11111111-2222-4333-8444-555555555555".into(),
            slot: slot_name.to_string(),
            role: Role::Security,
            image: image.to_string(),
            firewall_image: "nunki/firewall:test".to_string(),
            user: UserIds { uid, gid },
            tree: slot.tree.clone(),
            tree_at: PathBuf::from("/work/tree"),
            mission_dir: mission_dir.clone(),
            mission_dir_at: PathBuf::from("/work/mission"),
            stack_scripts: Vec::new(),
            credentials: Vec::new(),
            volumes: vec![NamedVolume {
                name: nunki::run::writable_volume(slot_name, "target"),
                at: PathBuf::from("/work/tree/target"),
            }],
            environment: Default::default(),
            command: vec!["sleep".to_string(), "600".to_string()],
            perimeter,
            project_services: None,
            project_networks: None,
            project_volumes: None,
        };
        std::fs::write(&file, generate(&plan, docker.dialect()).unwrap()).unwrap();
    };

    let engine: Arc<dyn Engine> = Arc::new(Docker::real());
    let writable = vec!["target".to_string()];
    let clean = || {
        let _ = docker.down(&file, &compose_project, true);
        let _ = std::process::Command::new("docker")
            .args([
                "volume",
                "rm",
                "-f",
                &nunki::run::writable_volume(slot_name, "target"),
            ])
            .status();
    };

    // The image the stack declared its directory to: the volume is the
    // agent's, and `nunki` says so.
    write_profile("nunki/sec-good:test");
    clean();
    docker.up(&file, &compose_project).unwrap();
    nunki::run::writable_or_rebuild(engine.clone(), &file, &compose_project, &writable)
        .expect("the declared directory is writable");

    let said = docker
        .exec(
            &file,
            &compose_project,
            nunki::compose::AGENT_SERVICE,
            &[
                "sh".to_string(),
                "-c".to_string(),
                "echo built > /work/tree/target/artefact && cat /work/tree/target/artefact; \
                 (echo x > /work/tree/leak 2>/dev/null && echo TREE-WRITABLE) || \
                 echo TREE-READ-ONLY; cat /work/tree/secret.rs"
                    .to_string(),
            ],
        )
        .unwrap();
    // Open where the stack said, closed everywhere else, and the tree still
    // readable beside it — that is what "read-only for the security agent"
    // has to mean for the role to work at all.
    assert!(said.stdout.contains("built"), "{said:?}");
    assert!(said.stdout.contains("TREE-READ-ONLY"), "{said:?}");
    assert!(said.stdout.contains("fn main()"), "{said:?}");

    // The same profile on an image built before that line: the volume is
    // root's, nothing says so, and this is the check that does.
    write_profile("nunki/sec-stale:test");
    clean();
    docker.up(&file, &compose_project).unwrap();
    let err = nunki::run::writable_or_rebuild(engine, &file, &compose_project, &writable)
        .expect_err("a stale image yields a root-owned volume");
    let message = err.to_string();
    assert!(message.contains("target"), "{message}");
    assert!(message.contains("nunki slot rebuild"), "{message}");

    clean();
}
