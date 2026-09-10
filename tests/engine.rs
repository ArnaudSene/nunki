//! The container-engine boundary (SPEC 4.2): what `hq` asks an engine, and
//! what it must never ask it.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Mutex;

use hq::engine::cli::Cli;
use hq::engine::docker::{Config, Docker};
use hq::engine::fake::{Call, FakeEngine, nowhere};
use hq::engine::{Dialect, Engine, ExecOutput, Liveness, Netns};
use hq::harness::spawn::CommandSpec;

/// Records what the adapter asked for and answers from a script, so the
/// command lines can be read without an engine on the machine.
#[derive(Default)]
struct RecordingCli {
    seen: Mutex<Vec<CommandSpec>>,
    answers: Mutex<VecDeque<Output>>,
}

impl RecordingCli {
    fn answering(outputs: Vec<Output>) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            answers: Mutex::new(outputs.into()),
        }
    }

    fn lines(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.display())
            .collect()
    }
}

impl Cli for RecordingCli {
    fn run(&self, spec: &CommandSpec) -> io::Result<Output> {
        self.seen.lock().unwrap().push(spec.clone());
        Ok(self.answers.lock().unwrap().pop_front().unwrap_or_else(ok))
    }
}

fn status(code: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code << 8)
}

fn ok() -> Output {
    Output {
        status: status(0),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

fn saying(code: i32, stdout: &str, stderr: &str) -> Output {
    Output {
        status: status(code),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

fn file() -> PathBuf {
    PathBuf::from("/slots/demo/mission.yml")
}

/// An adapter whose command lines a test can read afterwards.
fn recorded(answers: Vec<Output>) -> (Docker, &'static RecordingCli) {
    let cli: &'static RecordingCli = Box::leak(Box::new(RecordingCli::answering(answers)));
    // The adapter needs ownership; the test keeps a reference to read what
    // was asked. Leaking is what makes both possible in a test binary.
    struct Shared(&'static RecordingCli);
    impl Cli for Shared {
        fn run(&self, spec: &CommandSpec) -> io::Result<Output> {
            self.0.run(spec)
        }
    }
    let config = Config {
        compose: vec!["docker".to_string(), "compose".to_string()],
        engine: "docker".to_string(),
        stop_timeout: 20,
    };
    (Docker::new(config, Box::new(Shared(cli))), cli)
}

#[test]
fn up_waits_for_health_because_that_is_what_holds_the_firewall_rule() {
    let (docker, cli) = recorded(vec![]);
    docker.up(&file(), "hq-demo").unwrap();
    let line = &cli.lines()[0];
    assert!(line.contains("-p hq-demo"), "{line}");
    assert!(line.contains("-f /slots/demo/mission.yml"), "{line}");
    assert!(
        line.ends_with("up -d --wait"),
        "without --wait an unhealthy sidecar would not stop the profile: {line}"
    );
}

#[test]
fn a_profile_switch_stops_two_services_and_never_takes_the_project_down() {
    let (docker, cli) = recorded(vec![]);
    docker
        .stop(&file(), "hq-demo", &["agent", "firewall"])
        .unwrap();
    let lines = cli.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].contains("stop --timeout 20 agent firewall"),
        "{lines:?}"
    );
    assert!(lines[1].contains("rm -f -v agent firewall"), "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.contains(" down")),
        "the project's services are levied once per slot and never stopped between profiles: {lines:?}"
    );
}

#[test]
fn down_is_explicit_about_volumes_and_never_removes_orphans() {
    let (docker, cli) = recorded(vec![]);
    docker.down(&file(), "hq-demo", false).unwrap();
    docker.down(&file(), "hq-demo", true).unwrap();
    let lines = cli.lines();
    assert!(lines[0].ends_with("down --timeout 20"), "{lines:?}");
    assert!(
        lines[1].ends_with("down --timeout 20 --volumes"),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("--remove-orphans")),
        "a profile file that omits a service must not be a reason to delete it: {lines:?}"
    );
}

#[test]
fn exec_never_allocates_a_terminal() {
    let (docker, cli) = recorded(vec![saying(0, "hello\n", "")]);
    let out = docker
        .exec(
            &file(),
            "hq-demo",
            "agent",
            &["sh".to_string(), "-c".to_string(), "echo hello".to_string()],
        )
        .unwrap();
    assert_eq!(out.stdout, "hello\n");
    assert!(out.ok());
    let line = &cli.lines()[0];
    assert!(
        line.contains("exec -T agent sh -c echo hello"),
        "an autonomous run has no terminal to allocate: {line}"
    );
}

#[test]
fn a_failing_command_names_the_verb_and_carries_the_engines_words() {
    let (docker, _) = recorded(vec![saying(1, "", "network hq-demo_default not found")]);
    let err = docker.up(&file(), "hq-demo").unwrap_err();
    let text = err.to_string();
    assert!(text.contains("up failed"), "{text}");
    assert!(text.contains("network hq-demo_default not found"), "{text}");
}

#[test]
fn liveness_reads_the_engine_and_treats_an_unknown_container_as_an_answer() {
    let (docker, _) = recorded(vec![saying(0, "true 0\n", "")]);
    assert_eq!(docker.liveness("c1").unwrap(), Liveness::Running);

    let (docker, _) = recorded(vec![saying(0, "false 137\n", "")]);
    assert_eq!(docker.liveness("c1").unwrap(), Liveness::Exited(137));

    // A machine that slept, an engine restarted without its containers, a
    // slot removed: the engine knowing nothing is a fact, not a failure.
    let (docker, _) = recorded(vec![saying(1, "", "No such object: c1")]);
    assert_eq!(docker.liveness("c1").unwrap(), Liveness::Gone);

    let (docker, _) = recorded(vec![saying(0, "perhaps\n", "")]);
    assert!(
        docker.liveness("c1").is_err(),
        "an answer we cannot read is an error, not a guess"
    );
}

#[test]
fn container_of_returns_nothing_rather_than_an_empty_name() {
    let (docker, _) = recorded(vec![saying(0, "\n", "")]);
    assert_eq!(
        docker.container_of(&file(), "hq-demo", "agent").unwrap(),
        None
    );

    let (docker, _) = recorded(vec![saying(0, "abc123\n", "")]);
    assert_eq!(
        docker.container_of(&file(), "hq-demo", "agent").unwrap(),
        Some("abc123".to_string())
    );
}

#[test]
fn the_compose_command_is_overridable_because_the_machines_are_far_apart() {
    // The development machine measured v5.1.2 against a CI runner on
    // v2.38.2 (SPEC 4.2 bis); the adapter has to be pointable at either.
    let config = Config {
        compose: vec!["/opt/compose-2.38.2".to_string()],
        engine: "podman".to_string(),
        stop_timeout: 5,
    };
    let (cli, _) = (Box::new(RecordingCli::default()), ());
    let docker = Docker::new(config, cli);
    let spec = docker.compose_command(&file(), "hq-demo", &["up"]);
    assert_eq!(spec.program, "/opt/compose-2.38.2");
    assert_eq!(
        spec.display(),
        "/opt/compose-2.38.2 -p hq-demo -f /slots/demo/mission.yml up"
    );
}

#[test]
fn the_two_engines_spell_a_shared_namespace_differently() {
    let docker = Dialect {
        netns: Netns::Service,
        host_alias: "host.docker.internal".to_string(),
        userns: None,
    };
    assert_eq!(docker.netns_ref("hq-demo", "firewall"), "service:firewall");

    // podman-compose has no `service:` form: the generated container name has
    // to be known in advance (SPEC 4.2, engine table).
    let podman = Dialect {
        netns: Netns::Container,
        host_alias: "host.containers.internal".to_string(),
        userns: Some("keep-id".to_string()),
    };
    assert_eq!(
        podman.netns_ref("hq-demo", "firewall"),
        "container:hq-demo-firewall-1"
    );
}

#[test]
fn the_fake_engine_records_what_it_was_asked() {
    let engine = FakeEngine::default()
        .with_container("agent", "c-agent")
        .with_liveness("c-agent", Liveness::Running)
        .with_exec(ExecOutput {
            status: 0,
            stdout: "ok".to_string(),
            stderr: String::new(),
        });

    engine.up(&nowhere(), "hq-demo").unwrap();
    let container = engine
        .container_of(&nowhere(), "hq-demo", "agent")
        .unwrap()
        .unwrap();
    assert_eq!(engine.liveness(&container).unwrap(), Liveness::Running);
    engine.stop(&nowhere(), "hq-demo", &["agent"]).unwrap();

    assert_eq!(
        engine.calls(),
        vec![
            Call::Up("hq-demo".to_string()),
            Call::ContainerOf("hq-demo".to_string(), "agent".to_string()),
            Call::Liveness("c-agent".to_string()),
            Call::Stop("hq-demo".to_string(), vec!["agent".to_string()]),
        ]
    );
}

#[test]
fn a_sidecar_that_never_becomes_healthy_fails_the_profile() {
    let engine = FakeEngine::default().failing_up("dependency failed to start");
    let err = engine.up(&nowhere(), "hq-demo").unwrap_err();
    assert!(
        err.to_string().contains("dependency failed to start"),
        "{err}"
    );
}

/// The property the whole profile mechanism rests on, checked against a real
/// engine: **the project's services are levied once per slot and are not
/// stopped between profiles** (SPEC 4.2). A generated file that lifts proves
/// nothing about that; only switching does.
///
/// ```text
/// cargo test --test engine -- --ignored --nocapture
/// ```
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_a_profile_switch_keeps_the_projects_services() {
    let docker = Docker::real();
    let slot = "engineswitch";
    let project = hq::compose::project_name(slot).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let context = dir.path().join("firewall");
    std::fs::create_dir_all(&context).unwrap();
    hq::firewall::materialise(&context).unwrap();
    let built = std::process::Command::new("docker")
        .args(["build", "-q", "-t", "hq/firewall:test"])
        .arg(&context)
        .output()
        .expect("docker is on the path");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let mission = write_profile(dir.path(), slot, hq::harness::Role::Coder, "mission.yml");
    let system = write_profile(
        dir.path(),
        slot,
        hq::harness::Role::Integrator,
        "system.yml",
    );

    let _ = docker.down(&mission, &project, true);

    // The coder's profile, with the project's own service beside it.
    docker.up(&mission, &project).unwrap();
    let db_before = docker
        .container_of(&mission, &project, "db")
        .unwrap()
        .expect("db has a container");
    let agent_before = docker
        .container_of(&mission, &project, "agent")
        .unwrap()
        .expect("the agent has a container");
    assert_eq!(docker.liveness(&agent_before).unwrap(), Liveness::Running);

    let who = docker
        .exec(&mission, &project, "agent", &["id".to_string()])
        .unwrap();
    assert!(
        who.stdout.starts_with("uid=501"),
        "the agent runs under the host's uid: {who:?}"
    );

    // The switch: the agent and its sidecar go, nothing else is touched.
    docker
        .stop(&mission, &project, &["agent", "firewall"])
        .unwrap();
    assert_eq!(
        docker.liveness(&agent_before).unwrap(),
        Liveness::Gone,
        "the previous agent's container is removed, not left behind"
    );
    assert_eq!(
        docker.liveness(&db_before).unwrap(),
        Liveness::Running,
        "the project's service must not have been touched"
    );

    docker.up(&system, &project).unwrap();
    let db_after = docker
        .container_of(&system, &project, "db")
        .unwrap()
        .expect("db still has a container");
    let agent_after = docker
        .container_of(&system, &project, "agent")
        .unwrap()
        .expect("the new agent has a container");

    assert_eq!(
        db_before, db_after,
        "the same container, so the migrations and fixtures the previous role \
         left in it are still there"
    );
    assert_ne!(agent_before, agent_after, "the agent is a new container");
    println!("db kept container {}", &db_after[..12]);
    println!("agent {} → {}", &agent_before[..12], &agent_after[..12]);

    docker.down(&system, &project, true).unwrap();
    assert_eq!(docker.liveness(&db_after).unwrap(), Liveness::Gone);
}

/// A profile of the same slot, written where the engine can read it.
fn write_profile(dir: &Path, slot: &str, role: hq::harness::Role, name: &str) -> PathBuf {
    use hq::compose::{AGENT_WRITABLE, NamedVolume, Plan, UserIds, generate};
    use hq::perimeter::{Sources, compute};

    let tree = dir.join("tree");
    let mission_dir = dir.join("mission");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::create_dir_all(&mission_dir).unwrap();
    for f in AGENT_WRITABLE {
        let _ = std::fs::write(mission_dir.join(f), "");
    }

    let services = match role {
        hq::harness::Role::Coder => Vec::new(),
        _ => vec![hq::mission::Service {
            name: "db".to_string(),
            reach: vec!["db".to_string()],
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

    let plan = Plan {
        slot: slot.to_string(),
        role,
        image: "alpine:3.20".to_string(),
        firewall_image: "hq/firewall:test".to_string(),
        user: UserIds { uid: 501, gid: 20 },
        tree,
        tree_at: PathBuf::from("/work/tree"),
        mission_dir,
        mission_dir_at: PathBuf::from("/work/mission"),
        credentials: Vec::new(),
        volumes: Vec::<NamedVolume>::new(),
        environment: Default::default(),
        command: vec!["sleep".to_string(), "600".to_string()],
        perimeter,
        // Re-declared identically in both profiles, which is what makes the
        // switch a no-op for it (SPEC 4.2, measured).
        project_services: Some(serde_yaml_ng::from_str("db:\n  image: nginx:alpine\n").unwrap()),
        project_networks: None,
    };

    let path = dir.join(name);
    // The engine's own dialect, not a guess: this is exactly how `hq` will
    // ask for a file it is about to run.
    std::fs::write(&path, generate(&plan, Docker::real().dialect()).unwrap()).unwrap();
    path
}

#[test]
fn a_detached_command_puts_its_options_before_the_service_name() {
    let (docker, _) = recorded(vec![]);
    let spec = docker.detached_command(
        &file(),
        "hq-demo",
        "agent",
        &["-w".to_string(), "/work/tree".to_string()],
        &["sh".to_string(), "-c".to_string(), "true".to_string()],
    );
    // `exec [options] SERVICE COMMAND`: an option after the service name is
    // read as part of the command, and the run would start in the wrong
    // directory without saying so.
    assert!(
        spec.display()
            .ends_with("exec -T -w /work/tree agent sh -c true"),
        "{}",
        spec.display()
    );
}

/// A run really started inside a container, and really stopped from inside
/// it. This is what the local spawner could not do: the pid the host holds
/// there is the engine's client, and signalling the client kills the client
/// while the agent keeps working (SPEC 4.3).
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_a_run_lives_in_the_container_and_is_signalled_from_inside_it() {
    use hq::engine::spawn::ContainerSpawner;
    use hq::harness::spawn::{Presence, Signal, Spawner};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let context = dir.path().join("firewall");
    std::fs::create_dir_all(&context).unwrap();
    hq::firewall::materialise(&context).unwrap();
    assert!(
        std::process::Command::new("docker")
            .args(["build", "-q", "-t", "hq/firewall:test"])
            .arg(&context)
            .status()
            .unwrap()
            .success()
    );

    let slot = "enginerun";
    let project = hq::compose::project_name(slot).unwrap();
    let profile = write_profile(dir.path(), slot, hq::harness::Role::Coder, "mission.yml");

    let docker: Arc<dyn Engine> = Arc::new(Docker::real());
    let _ = docker.down(&profile, &project, true);
    docker.up(&profile, &project).unwrap();

    let spawner = ContainerSpawner::new(docker.clone(), profile.clone(), &project, "agent");
    let log = dir.path().join("runs").join("session-1.jsonl");
    let mut env = std::collections::BTreeMap::new();
    env.insert("HQ_MARKER".to_string(), "carried".to_string());

    let spawned = spawner
        .spawn(
            &CommandSpec {
                program: "sh".to_string(),
                args: vec![
                    "-c".to_string(),
                    "echo started in $PWD with $HQ_MARKER; sleep 300".to_string(),
                ],
                cwd: PathBuf::from("/work/tree"),
                env,
            },
            &log,
        )
        .unwrap();

    let pid = spawned.pid.expect("the run published a pid");
    println!(
        "in-container pid {pid}, container {}",
        &spawned.container[..12]
    );
    assert!(!spawned.container.is_empty());
    assert_eq!(
        spawner.alive(&spawned).unwrap(),
        Presence::Running,
        "the run should be alive"
    );

    // The stream is captured on the host, where hq reads it — the container
    // has nowhere to write it (SPEC 4.1, mounts per profile).
    let mut waited = 0;
    let captured = loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.contains("started") || waited > 50 {
            break text;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        waited += 1;
    };
    assert!(captured.contains("started in /work/tree"), "{captured:?}");
    assert!(
        captured.contains("carried"),
        "the environment must reach the run: {captured:?}"
    );

    // The pid is the container's own, published on the agent's tmpfs — the
    // only place it may write besides the tree and its three mission files.
    let published = docker
        .exec(
            &profile,
            &project,
            "agent",
            &[
                "cat".to_string(),
                format!("{}/session-1.pid", hq::engine::spawn::RUN_DIR),
            ],
        )
        .unwrap();
    assert_eq!(published.stdout.trim(), pid.to_string());

    spawner.signal(&spawned, Signal::Interrupt).unwrap();
    let mut stopped = false;
    for _ in 0..50 {
        if spawner.alive(&spawned).unwrap() != Presence::Running {
            stopped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        stopped,
        "SIGINT from inside the container should end the run"
    );

    docker.down(&profile, &project, true).unwrap();
}

#[test]
fn a_signal_is_named_the_way_kill_expects_it() {
    use hq::harness::spawn::Signal;
    // SIGINT ends an agent's turn properly; SIGTERM leaves it unfinished
    // (SPEC 4.3). Going through a shell in the container, the difference is
    // one word.
    assert_eq!(Signal::Interrupt.name(), "INT");
    assert_eq!(Signal::Terminate.name(), "TERM");
}
