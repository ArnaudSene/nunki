//! The container-engine boundary (SPEC 4.2): what `nunki` asks an engine, and
//! what it must never ask it.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Mutex;

use nunki::engine::cli::Cli;
use nunki::engine::docker::{Config, Docker};
use nunki::engine::fake::{Call, FakeEngine, nowhere};
use nunki::engine::{Dialect, Engine, ExecOutput, Liveness, Netns};
use nunki::harness::spawn::CommandSpec;

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
    docker.up(&file(), "nunki-demo").unwrap();
    let line = &cli.lines()[0];
    assert!(line.contains("-p nunki-demo"), "{line}");
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
        .stop(&file(), "nunki-demo", &["agent", "firewall"])
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

/// The three gestures of SPEC 4.3 ask the engine for exactly what each one
/// means, and nothing near it. `kill` in particular is not `stop` with no
/// patience: `stop` asks and waits, and the emergency brake does neither.
#[test]
fn the_three_gestures_ask_for_exactly_what_each_one_means() {
    let (docker, cli) = recorded(vec![]);
    docker
        .pause(&file(), "nunki-demo", &["agent", "firewall"])
        .unwrap();
    docker
        .unpause(&file(), "nunki-demo", &["agent", "firewall"])
        .unwrap();
    docker
        .kill(&file(), "nunki-demo", &["agent", "firewall"])
        .unwrap();
    let lines = cli.lines();
    assert!(lines[0].ends_with("pause agent firewall"), "{lines:?}");
    assert!(lines[1].ends_with("unpause agent firewall"), "{lines:?}");
    assert!(lines[2].ends_with("kill agent firewall"), "{lines:?}");
    assert!(
        !lines[2].contains("stop") && !lines[2].contains("--timeout"),
        "the emergency brake asks nothing and waits for nothing: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains(" down")),
        "no gesture takes the project's services with it: {lines:?}"
    );
}

#[test]
fn down_is_explicit_about_volumes_and_never_removes_orphans() {
    let (docker, cli) = recorded(vec![]);
    docker.down(&file(), "nunki-demo", false).unwrap();
    docker.down(&file(), "nunki-demo", true).unwrap();
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
            "nunki-demo",
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
    let (docker, _) = recorded(vec![saying(1, "", "network nunki-demo_default not found")]);
    let err = docker.up(&file(), "nunki-demo").unwrap_err();
    let text = err.to_string();
    assert!(text.contains("up failed"), "{text}");
    assert!(
        text.contains("network nunki-demo_default not found"),
        "{text}"
    );
}

#[test]
fn liveness_reads_the_engine_and_treats_an_unknown_container_as_an_answer() {
    // Three fields, and the middle one is the one a first reading misses:
    // measured on Docker 28, a **paused** container answers `true true 0`,
    // so reading only `.State.Running` reports a frozen agent as a working
    // one — and a stall check would then read it as an agent that stopped
    // thinking.
    let (docker, _) = recorded(vec![saying(0, "true false 0\n", "")]);
    assert_eq!(docker.liveness("c1").unwrap(), Liveness::Running);

    let (docker, _) = recorded(vec![saying(0, "true true 0\n", "")]);
    assert_eq!(docker.liveness("c1").unwrap(), Liveness::Paused);

    let (docker, _) = recorded(vec![saying(0, "false false 137\n", "")]);
    assert_eq!(docker.liveness("c1").unwrap(), Liveness::Exited(137));

    // A machine that slept, an engine restarted without its containers, a
    // slot removed: the engine knowing nothing is a fact, not a failure.
    // Gone because the engine said so: it answered a listing of the
    // container, and the listing was empty.
    let (docker, cli) = recorded(vec![saying(1, "", "No such object: c1"), saying(0, "", "")]);
    assert_eq!(docker.liveness("c1").unwrap(), Liveness::Gone);
    assert_eq!(
        cli.lines().last().map(String::as_str),
        Some("docker ps -a -q --no-trunc --filter id=c1")
    );

    let (docker, _) = recorded(vec![saying(0, "perhaps\n", "")]);
    assert!(
        docker.liveness("c1").is_err(),
        "an answer we cannot read is an error, not a guess"
    );
}

/// A daemon or a socket that cannot be reached fails `inspect` too, and is
/// no answer about the container: `Gone` read on it would declare a
/// container removed on a question nobody answered — and a campaign in it
/// forgotten, its doors reopened, while it still runs.
#[test]
fn an_engine_that_cannot_be_reached_never_says_a_container_is_gone() {
    let unreachable = "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. \
                       Is the docker daemon running?";
    let (docker, _) = recorded(vec![saying(1, "", unreachable), saying(1, "", unreachable)]);
    let err = docker.liveness("c1").unwrap_err();
    assert!(err.to_string().contains("Cannot connect"), "{err}");

    // Listed, and still there: it could not be inspected, which is no
    // answer about it either.
    let (docker, _) = recorded(vec![
        saying(1, "", "permission denied"),
        saying(0, "c1\n", ""),
    ]);
    let err = docker.liveness("c1").unwrap_err();
    assert!(err.to_string().contains("inspect"), "{err}");
    assert!(err.to_string().contains("permission denied"), "{err}");
}

#[test]
fn container_of_returns_nothing_rather_than_an_empty_name() {
    let (docker, _) = recorded(vec![saying(0, "\n", "")]);
    assert_eq!(
        docker.container_of(&file(), "nunki-demo", "agent").unwrap(),
        None
    );

    let (docker, _) = recorded(vec![saying(0, "abc123\n", "")]);
    assert_eq!(
        docker.container_of(&file(), "nunki-demo", "agent").unwrap(),
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
    let spec = docker.compose_command(&file(), "nunki-demo", &["up"]);
    assert_eq!(spec.program, "/opt/compose-2.38.2");
    assert_eq!(
        spec.display(),
        "/opt/compose-2.38.2 -p nunki-demo -f /slots/demo/mission.yml up"
    );
}

#[test]
fn the_two_engines_spell_a_shared_namespace_differently() {
    let docker = Dialect {
        netns: Netns::Service,
        host_alias: "host.docker.internal".to_string(),
        userns: None,
    };
    assert_eq!(
        docker.netns_ref("nunki-demo", "firewall"),
        "service:firewall"
    );

    // podman-compose has no `service:` form: the generated container name has
    // to be known in advance (SPEC 4.2, engine table).
    let podman = Dialect {
        netns: Netns::Container,
        host_alias: "host.containers.internal".to_string(),
        userns: Some("keep-id".to_string()),
    };
    assert_eq!(
        podman.netns_ref("nunki-demo", "firewall"),
        "container:nunki-demo-firewall-1"
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

    engine.up(&nowhere(), "nunki-demo").unwrap();
    let container = engine
        .container_of(&nowhere(), "nunki-demo", "agent")
        .unwrap()
        .unwrap();
    assert_eq!(engine.liveness(&container).unwrap(), Liveness::Running);
    engine.stop(&nowhere(), "nunki-demo", &["agent"]).unwrap();

    assert_eq!(
        engine.calls(),
        vec![
            Call::Up("nunki-demo".to_string()),
            Call::ContainerOf("nunki-demo".to_string(), "agent".to_string()),
            Call::Liveness("c-agent".to_string()),
            Call::Stop("nunki-demo".to_string(), vec!["agent".to_string()]),
        ]
    );
}

/// The tests of a campaign frozen, or gone, between two questions rest on
/// this: the queue's last answer is kept, never used up and replaced by the
/// container's standing state.
#[test]
fn the_fake_engine_answers_liveness_in_turn_and_keeps_the_last_answer() {
    let engine = FakeEngine::default()
        .with_liveness("c-agent", Liveness::Running)
        .with_livenesses("c-agent", vec![Liveness::Running, Liveness::Paused])
        .with_livenesses("c-other", vec![Liveness::Exited(137)]);

    let asked: Vec<Liveness> = (0..4)
        .map(|_| engine.liveness("c-agent").unwrap())
        .collect();
    assert_eq!(
        asked,
        vec![
            Liveness::Running,
            Liveness::Paused,
            Liveness::Paused,
            Liveness::Paused
        ]
    );
    for _ in 0..3 {
        assert_eq!(engine.liveness("c-other").unwrap(), Liveness::Exited(137));
    }
}

#[test]
fn a_sidecar_that_never_becomes_healthy_fails_the_profile() {
    let engine = FakeEngine::default().failing_up("dependency failed to start");
    let err = engine.up(&nowhere(), "nunki-demo").unwrap_err();
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
    let project =
        nunki::compose::project_name("11111111-2222-4333-8444-555555555555", slot).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let context = dir.path().join("firewall");
    std::fs::create_dir_all(&context).unwrap();
    nunki::firewall::materialise(&context).unwrap();
    let built = std::process::Command::new("docker")
        .args(["build", "-q", "-t", "nunki/firewall:test"])
        .arg(&context)
        .output()
        .expect("docker is on the path");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let mission = write_profile(dir.path(), slot, nunki::harness::Role::Coder, "mission.yml");
    let system = write_profile(
        dir.path(),
        slot,
        nunki::harness::Role::Integrator,
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
fn write_profile(dir: &Path, slot: &str, role: nunki::harness::Role, name: &str) -> PathBuf {
    use nunki::compose::{NamedVolume, Plan, UserIds, generate};
    use nunki::perimeter::{Sources, compute};

    let tree = dir.join("tree");
    let mission_dir = dir.join("mission");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::create_dir_all(&mission_dir).unwrap();
    for f in nunki::compose::any_agent_writable() {
        let _ = std::fs::write(mission_dir.join(f), "");
    }

    let services = match role {
        nunki::harness::Role::Coder => Vec::new(),
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

    let plan = Plan {
        session: "11111111-2222-4333-8444-555555555555".into(),
        slot: slot.to_string(),
        role,
        image: "alpine:3.20".to_string(),
        firewall_image: "nunki/firewall:test".to_string(),
        user: UserIds { uid: 501, gid: 20 },
        tree,
        tree_at: PathBuf::from("/work/tree"),
        mission_dir,
        mission_dir_at: PathBuf::from("/work/mission"),
        stack_scripts: Vec::new(),
        credentials: Vec::new(),
        advisories: vec![],
        secrets: None,
        volumes: Vec::<NamedVolume>::new(),
        environment: Default::default(),
        command: vec!["sleep".to_string(), "600".to_string()],
        perimeter,
        // Re-declared identically in both profiles, which is what makes the
        // switch a no-op for it (SPEC 4.2, measured).
        project_services: Some(nunki::compose::services::Lifted::whole(
            nunki::compose::services::ServicesFile::parse(
                "services:\n  db:\n    image: nginx:alpine\n",
            )
            .unwrap(),
        )),
        prober: None,
    };

    let path = dir.join(name);
    // The engine's own dialect, not a guess: this is exactly how `nunki` will
    // ask for a file it is about to run.
    std::fs::write(&path, generate(&plan, Docker::real().dialect()).unwrap()).unwrap();
    path
}

#[test]
fn a_detached_command_puts_its_options_before_the_service_name() {
    let (docker, _) = recorded(vec![]);
    let spec = docker.detached_command(
        &file(),
        "nunki-demo",
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
    use nunki::engine::spawn::ContainerSpawner;
    use nunki::harness::spawn::{Presence, Signal, Spawner};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let context = dir.path().join("firewall");
    std::fs::create_dir_all(&context).unwrap();
    nunki::firewall::materialise(&context).unwrap();
    assert!(
        std::process::Command::new("docker")
            .args(["build", "-q", "-t", "nunki/firewall:test"])
            .arg(&context)
            .status()
            .unwrap()
            .success()
    );

    let slot = "enginerun";
    let project =
        nunki::compose::project_name("11111111-2222-4333-8444-555555555555", slot).unwrap();
    let profile = write_profile(dir.path(), slot, nunki::harness::Role::Coder, "mission.yml");

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

    // The stream is captured on the host, where nunki reads it — the container
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
                format!("{}/session-1.pid", nunki::engine::spawn::RUN_DIR),
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
    use nunki::harness::spawn::Signal;
    // SIGINT ends an agent's turn properly; SIGTERM leaves it unfinished
    // (SPEC 4.3). Going through a shell in the container, the difference is
    // one word.
    assert_eq!(Signal::Interrupt.name(), "INT");
    assert_eq!(Signal::Terminate.name(), "TERM");
}

/// A signal that fails says what failed, even when the shell says nothing.
///
/// Without this, `nunki mission stop --now` can answer `nunki: io:` — the
/// whole message — while the run goes on as if nothing had been asked, when
/// the error carries the container's stderr and nothing else, and a `kill`
/// that fails frequently writes no stderr at all, so the one case where a
/// human has least to go on was the case that said least. The status is then
/// the only thing known, which is precisely why it has to be in the sentence.
#[test]
fn a_signal_that_fails_is_never_an_empty_sentence() {
    use nunki::harness::spawn::{Signal, Spawned, Spawner};

    let spawned = || Spawned {
        pid: Some(41),
        container: "cafe1234".into(),
    };
    let spawner = |fake: &std::sync::Arc<FakeEngine>| {
        nunki::engine::spawn::ContainerSpawner::new(
            fake.clone(),
            file(),
            "nunki-demo",
            nunki::compose::AGENT_SERVICE,
        )
    };

    // The measured case: non-zero, and not a word about why.
    let mute = std::sync::Arc::new(FakeEngine::default().with_exec(ExecOutput {
        status: 1,
        stdout: String::new(),
        stderr: String::new(),
    }));
    let said = spawner(&mute)
        .signal(&spawned(), Signal::Terminate)
        .expect_err("a non-zero status is a failed kill")
        .to_string();
    assert!(
        !said.trim().is_empty(),
        "an empty error says nothing at all"
    );
    assert!(said.contains("(1)"), "the status is all there is: {said}");
    assert!(said.contains("41"), "{said}");
    assert!(said.contains("TERM"), "{said}");

    // And when the shell did say something, it is still carried.
    let noisy = std::sync::Arc::new(FakeEngine::default().with_exec(ExecOutput {
        status: 2,
        stdout: String::new(),
        stderr: "no such process\n".into(),
    }));
    let said = spawner(&noisy)
        .signal(&spawned(), Signal::Interrupt)
        .expect_err("a non-zero status is a failed kill")
        .to_string();
    assert!(said.contains("no such process"), "{said}");
    assert!(said.contains("(2)"), "{said}");
}

/// The three gestures on a real container, and the one reading that made this
/// piece necessary (SPEC 4.3, and AGENTS.md §4).
///
/// A paused container answers `Running=true` — measured on Docker 28 — so an
/// adapter reading only that field reports a frozen agent as a working one,
/// and a stall check would then read it as an agent that stopped thinking.
/// `exec` into a paused container is refused outright rather than hanging, so
/// the in-container probe cannot answer either: liveness has to be asked
/// first, and it has to have a word for this.
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_a_frozen_container_is_not_a_running_one() {
    let docker = Docker::real();
    let slot = "gesturelive";
    let project =
        nunki::compose::project_name("11111111-2222-4333-8444-555555555555", slot).unwrap();

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

    let file = write_profile(dir.path(), slot, nunki::harness::Role::Coder, "mission.yml");
    let _ = docker.down(&file, &project, true);
    docker.up(&file, &project).unwrap();

    let agent = docker
        .container_of(&file, &project, nunki::compose::AGENT_SERVICE)
        .unwrap()
        .expect("the agent has a container");
    assert_eq!(docker.liveness(&agent).unwrap(), Liveness::Running);

    docker
        .pause(&file, &project, &nunki::run::SERVICES)
        .unwrap();
    assert_eq!(
        docker.liveness(&agent).unwrap(),
        Liveness::Paused,
        "a frozen container must not read as a running one"
    );
    // And the probe that would otherwise answer cannot: the engine refuses
    // outright. Not an engine failure — a command that did not run is data,
    // and the adapter reports it as such — so what is asserted is the exit
    // status and the daemon's own words.
    let refused = docker
        .exec(
            &file,
            &project,
            nunki::compose::AGENT_SERVICE,
            &["true".to_string()],
        )
        .unwrap();
    assert!(!refused.ok(), "{refused:?}");
    assert!(
        refused.stderr.contains("paused"),
        "exec into a paused container is refused, so liveness must answer \
         first: {refused:?}"
    );

    docker
        .unpause(&file, &project, &nunki::run::SERVICES)
        .unwrap();
    assert_eq!(docker.liveness(&agent).unwrap(), Liveness::Running);
    // Unfrozen exactly there: the same container, not a new one.
    assert_eq!(
        docker
            .container_of(&file, &project, nunki::compose::AGENT_SERVICE)
            .unwrap()
            .as_deref(),
        Some(agent.as_str())
    );

    // The emergency brake: killed, not asked. `docker kill` leaves 137.
    docker.kill(&file, &project, &nunki::run::SERVICES).unwrap();
    assert!(
        matches!(docker.liveness(&agent).unwrap(), Liveness::Exited(_)),
        "the container is gone and nothing was waited for"
    );
    // And the project's own service is untouched by all three.
    let db = docker
        .container_of(&file, &project, "db")
        .unwrap()
        .expect("the project's service has a container");
    assert_eq!(docker.liveness(&db).unwrap(), Liveness::Running);

    docker.down(&file, &project, true).unwrap();
}

/// The spawner asks liveness before it probes, and a frozen container ends
/// the question there.
///
/// Two reasons, and both are measured. `exec` into a paused container is
/// refused by the engine, so the probe cannot answer at all — and its refusal
/// would arrive as `Unknown`, a silence about a run whose state is perfectly
/// known. And `Running` would be worse: a stall check reading it would count
/// a run a human froze on purpose as an agent that stopped thinking.
#[test]
fn a_frozen_container_answers_paused_and_is_never_probed() {
    use nunki::harness::spawn::{Presence, Spawned, Spawner};

    let fake =
        std::sync::Arc::new(FakeEngine::default().with_liveness("cafe1234", Liveness::Paused));
    let spawner = nunki::engine::spawn::ContainerSpawner::new(
        fake.clone(),
        file(),
        "nunki-demo",
        nunki::compose::AGENT_SERVICE,
    )
    .identified_by("s1");

    let presence = spawner
        .alive(&Spawned {
            pid: Some(41),
            container: "cafe1234".into(),
        })
        .unwrap();
    assert_eq!(presence, Presence::Paused);
    assert!(
        !fake.calls().iter().any(|c| matches!(c, Call::Exec(..))),
        "the probe was attempted on a container that refuses it: {:?}",
        fake.calls()
    );
}

/// A signal reaches the run through a **shell**, because `kill` is a builtin
/// every shell has and not a binary every image ships.
///
/// Measured on this project's own Debian agent image:
///
/// ```text
/// $ docker exec <agent> kill -0 1
/// OCI runtime exec failed: exec: "kill": executable file not found in $PATH
/// $ docker exec <agent> sh -c 'command -v kill'
/// kill
/// ```
///
/// `nunki mission stop --now` answered `kill -INT 7 in the container failed
/// (127)` and no signal was ever sent. Every gesture that signals a run went
/// through that line. The live test beside this one did not catch it and
/// could not: it lifts an Alpine container, where busybox provides
/// `/bin/kill`.
#[test]
fn a_signal_is_sent_through_a_shell_and_not_as_a_binary() {
    use nunki::engine::fake::Call;
    use nunki::harness::spawn::{Signal, Spawned, Spawner};

    let fake = std::sync::Arc::new(FakeEngine::default());
    let spawner = nunki::engine::spawn::ContainerSpawner::new(
        fake.clone(),
        file(),
        "nunki-demo",
        nunki::compose::AGENT_SERVICE,
    );
    spawner
        .signal(
            &Spawned {
                pid: Some(7),
                container: "cafe1234".into(),
            },
            Signal::Interrupt,
        )
        .unwrap();

    match fake.calls().last() {
        Some(Call::Exec(_, service, argv)) => {
            assert_eq!(service, nunki::compose::AGENT_SERVICE);
            assert_eq!(argv[0], "sh", "{argv:?}");
            assert_eq!(argv[1], "-c", "{argv:?}");
            assert_eq!(argv[2], "kill -INT 7", "{argv:?}");
        }
        other => panic!("{other:?}"),
    }
}

/// The containers of a project are asked of the engine, running or not,
/// with the label nunki puts on what it lifts — the containers this is for
/// are the ones the current file no longer names, which `compose ps` would
/// not list.
#[test]
fn the_containers_of_a_project_are_listed_with_nunkis_label() {
    let (docker, cli) = recorded(vec![saying(
        0,
        "c1\tdb\tsha256:aa\nc2\tcache\t\n\nc3\tfirewall\t\n",
        "",
    )]);
    let containers = docker.containers("nunki-demo").unwrap();
    let line = &cli.lines()[0];
    assert!(line.starts_with("docker ps -a "), "{line}");
    assert!(
        line.contains("--filter label=com.docker.compose.project=nunki-demo"),
        "{line}"
    );
    assert!(line.contains("com.docker.compose.service"), "{line}");
    assert!(line.contains(nunki::compose::services::LABEL), "{line}");
    assert_eq!(
        containers,
        vec![
            nunki::engine::Container {
                id: "c1".into(),
                service: "db".into(),
                digest: Some("sha256:aa".into()),
            },
            nunki::engine::Container {
                id: "c2".into(),
                service: "cache".into(),
                digest: None,
            },
            nunki::engine::Container {
                id: "c3".into(),
                service: "firewall".into(),
                digest: None,
            },
        ]
    );
}

/// A line nunki cannot read is said, not skipped: a container skipped here
/// is one left running.
#[test]
fn a_container_line_nunki_cannot_read_is_an_error() {
    for answer in ["c1\n", "\tdb\tsha256:aa\n", "c1\t\t\n"] {
        let (docker, _) = recorded(vec![saying(0, answer, "")]);
        assert!(
            matches!(
                docker.containers("p"),
                Err(nunki::engine::EngineError::Unreadable(_))
            ),
            "{answer:?}"
        );
    }
    let (docker, _) = recorded(vec![saying(1, "", "no engine")]);
    assert!(docker.containers("p").is_err());
}

/// Taken down by id, stopped first, and never with a volume — not a named
/// one, and not the anonymous one an image's `VOLUME` made.
#[test]
fn removing_containers_never_takes_a_volume_with_them() {
    let (docker, cli) = recorded(vec![]);
    docker.remove(&["c1".into(), "c2".into()]).unwrap();
    let lines = cli.lines();
    assert_eq!(lines, vec!["docker rm -f c1 c2".to_string()]);

    let (docker, cli) = recorded(vec![]);
    docker.remove(&[]).unwrap();
    assert!(cli.lines().is_empty(), "nothing to remove runs nothing");

    let (docker, _) = recorded(vec![saying(1, "", "busy")]);
    assert!(docker.remove(&["c1".into()]).is_err());
}

/// The removal a message names for a human (#140) takes no volume either,
/// and does not stop a container: it names one that has exited, and one
/// running after all is refused. It is data, and nothing runs it here.
#[test]
fn the_removal_named_for_a_human_takes_no_volume_and_stops_nothing() {
    let (docker, cli) = recorded(vec![]);
    assert_eq!(docker.removal("c1").display(), "docker rm c1");
    assert!(cli.lines().is_empty(), "named, not run");
}

/// A named volume is removed by name, after the engine is asked whether it
/// has one — compared whole, since `--filter name=` matches a part — and
/// never with `-f`, which would hide why one in use stayed.
#[test]
fn a_volume_is_removed_by_name_and_one_the_engine_lacks_is_not_an_error() {
    // Listed, among a longer name that contains it: removed.
    let (docker, cli) = recorded(vec![
        saying(0, "nunki-one-proof-old\nnunki-one-proof\n", ""),
        ok(),
    ]);
    assert!(docker.remove_volume("nunki-one-proof").unwrap());
    let lines = cli.lines();
    assert_eq!(
        lines,
        [
            "docker volume ls --quiet --filter name=nunki-one-proof",
            "docker volume rm nunki-one-proof"
        ]
    );

    // Only a longer name listed: none by that name, and nothing removed.
    let (docker, cli) = recorded(vec![saying(0, "nunki-one-proof-old\n", "")]);
    assert!(!docker.remove_volume("nunki-one-proof").unwrap());
    assert_eq!(cli.lines().len(), 1);

    // In use: an error carrying the engine's words.
    let (docker, _) = recorded(vec![
        saying(0, "nunki-one-proof\n", ""),
        saying(
            1,
            "",
            "Error response from daemon: remove nunki-one-proof: volume is in use",
        ),
    ]);
    let err = docker.remove_volume("nunki-one-proof").unwrap_err();
    assert!(err.to_string().contains("volume is in use"), "{err}");
}

/// Every run is started carrying `NUNKI_RUN=<its session>`, as an engine
/// option before the service: what finds what it leaves behind.
#[test]
fn a_run_is_started_tagged_with_its_session() {
    let fake = std::sync::Arc::new(nunki::engine::fake::FakeEngine::default());
    let spawner = nunki::engine::spawn::ContainerSpawner::new(
        fake,
        file(),
        "nunki-demo",
        nunki::compose::AGENT_SERVICE,
    );
    let command = spawner.command(
        &CommandSpec {
            program: "claude".into(),
            args: vec![],
            cwd: PathBuf::from("/work/tree"),
            env: Default::default(),
        },
        Path::new("/hq/runs/session-1.jsonl"),
    );
    let line = command.display();
    assert!(line.contains("-e NUNKI_RUN=session-1"), "{line}");
}

/// The token reaches the run named on the engine's command line and valued
/// only in the client's environment (#170). The command line is the
/// client's argv, which any user of the machine reads with `ps`; Compose
/// takes the value of `-e NAME` from the environment it was started with.
#[test]
fn a_secret_is_named_on_the_command_line_and_valued_only_in_the_clients_environment() {
    let fake = std::sync::Arc::new(nunki::engine::fake::FakeEngine::default());
    let spawner = nunki::engine::spawn::ContainerSpawner::new(
        fake,
        file(),
        "nunki-demo",
        nunki::compose::AGENT_SERVICE,
    )
    .passing_secret("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-secret");
    let command = spawner.command(
        &CommandSpec {
            program: "claude".into(),
            args: vec![],
            cwd: PathBuf::from("/work/tree"),
            env: Default::default(),
        },
        Path::new("/hq/runs/session-1.jsonl"),
    );

    let line = command.display();
    assert!(line.contains("-e CLAUDE_CODE_OAUTH_TOKEN "), "{line}");
    assert!(!line.contains("sk-ant-oat01-secret"), "{line}");
    assert!(
        command
            .args
            .iter()
            .all(|a| !a.contains("sk-ant-oat01-secret")),
        "{:?}",
        command.args
    );
    assert_eq!(
        command
            .env
            .get("CLAUDE_CODE_OAUTH_TOKEN")
            .map(String::as_str),
        Some("sk-ant-oat01-secret")
    );
}

/// What the reaping answers, read from the container: the pids it stopped
/// after the marker; nothing asked of a container that is not running; and
/// an answer without the marker is a check that did not run, never "none".
#[test]
fn reaping_reads_what_the_container_said_and_never_guesses() {
    use nunki::engine::fake::{Call, FakeEngine};
    use nunki::engine::{ExecOutput, Liveness};
    let spawner = |fake: &std::sync::Arc<FakeEngine>| {
        nunki::engine::spawn::ContainerSpawner::new(
            fake.clone(),
            file(),
            "nunki-demo",
            nunki::compose::AGENT_SERVICE,
        )
    };
    let log = Path::new("/hq/runs/session-1.jsonl");
    let answering = |stdout: &str| {
        std::sync::Arc::new(
            FakeEngine::default()
                .with_liveness("cafe1234", Liveness::Running)
                .with_exec(ExecOutput {
                    status: 0,
                    stdout: stdout.into(),
                    stderr: String::new(),
                }),
        )
    };

    let fake = answering("nunki-reaped 12 13\n");
    assert_eq!(
        spawner(&fake).reap("cafe1234", log).unwrap(),
        Some(vec![12, 13])
    );
    let fake = answering("nunki-reaped\n");
    assert_eq!(spawner(&fake).reap("cafe1234", log).unwrap(), Some(vec![]));
    let fake = answering("");
    assert!(spawner(&fake).reap("cafe1234", log).is_err());

    for gone in [Liveness::Exited(0), Liveness::Gone, Liveness::Paused] {
        let fake = std::sync::Arc::new(FakeEngine::default().with_liveness("cafe1234", gone));
        assert_eq!(spawner(&fake).reap("cafe1234", log).unwrap(), None);
        assert!(
            !fake.calls().iter().any(|c| matches!(c, Call::Exec(..))),
            "{:?}",
            fake.calls()
        );
    }
}

/// The reaping script itself, run by this machine's `sh` over its own
/// `/proc`: a run that left `sleep 600` behind leaves nothing once reaped,
/// and a process of another run, or of none, is left alone. Linux only —
/// there is no `/proc` on macOS, where CI runs.
#[cfg(target_os = "linux")]
#[test]
fn a_run_that_leaves_sleep_600_behind_leaves_nothing_once_it_ends() {
    use std::process::{Command, Stdio};

    let run = format!("reap-test-{}", std::process::id());
    // What a run does: start something in the background, and end.
    let leave = |tag: Option<&str>| -> u32 {
        let mut sh = Command::new("sh");
        sh.args(["-c", "sleep 600 >/dev/null 2>&1 & echo $!"])
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        if let Some(tag) = tag {
            sh.env("NUNKI_RUN", tag);
        }
        let out = sh.output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    };
    // Alive: there, and not a zombie waiting for its parent to read it.
    let alive = |pid: u32| {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|stat| {
                let state = stat.rsplit(") ").next().unwrap_or("");
                !state.starts_with('Z') && !state.starts_with('X')
            })
            .unwrap_or(false)
    };
    let left = leave(Some(&run));
    let other_run = leave(Some(&format!("{run}-other")));
    let untagged = leave(None);
    // And one that says which signal ended it: `TERM` is caught and written
    // down, which `KILL` never can be — so the marker is there only if the
    // run's leftovers were asked to end before anything killed them.
    let marker = tempfile::tempdir().unwrap();
    let asked = marker.path().join("asked");
    let out = Command::new("sh")
        .args([
            "-c",
            "sh -c 'trap \"echo term > \\\"$0\\\"; exit 0\" TERM; sleep 600 & wait' \"$1\" \
             >/dev/null 2>&1 & echo $!",
            "sh",
        ])
        .arg(&asked)
        .env("NUNKI_RUN", &run)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .unwrap();
    let polite: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    // Let it install its trap before anything is asked of it.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(alive(left) && alive(other_run) && alive(untagged) && alive(polite));

    let out = Command::new("sh")
        .args(["-c", &nunki::engine::spawn::reap_script(&run)])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);

    assert!(said.starts_with("nunki-reaped"), "{said}");
    assert!(said.contains(&left.to_string()), "{said}");
    assert!(!alive(left), "the run's `sleep 600` outlived it");
    assert!(alive(other_run), "another run's process was stopped");
    assert!(alive(untagged), "a process of no run was stopped");
    // Asked to end before anything killed it: it caught `TERM` and said so.
    assert!(!alive(polite), "the run's leftover outlived it");
    assert_eq!(
        std::fs::read_to_string(&asked).unwrap_or_default().trim(),
        "term",
        "the leftover was killed without being asked to end"
    );

    for pid in [other_run, untagged] {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

/// A leftover that will not end when asked — it ignores `TERM` — is killed
/// once the reaping has waited for it.
#[cfg(target_os = "linux")]
#[test]
fn a_leftover_that_ignores_term_is_killed() {
    use std::process::{Command, Stdio};

    let run = format!("reap-stubborn-{}", std::process::id());
    let out = Command::new("sh")
        .args([
            "-c",
            "sh -c 'trap \"\" TERM; while :; do sleep 1; done' >/dev/null 2>&1 & echo $!",
        ])
        .env("NUNKI_RUN", &run)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .unwrap();
    let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    let alive = |pid: u32| {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|stat| {
                let state = stat.rsplit(") ").next().unwrap_or("");
                !state.starts_with('Z') && !state.starts_with('X')
            })
            .unwrap_or(false)
    };
    // Let the shell install its trap before it is asked anything.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(alive(pid));

    let said = Command::new("sh")
        .args(["-c", &nunki::engine::spawn::reap_script(&run)])
        .output()
        .unwrap();

    assert!(
        String::from_utf8_lossy(&said.stdout).starts_with("nunki-reaped"),
        "{}",
        String::from_utf8_lossy(&said.stdout)
    );
    assert!(!alive(pid), "a process that ignores TERM outlived its run");
}

/// What [`ContainerSpawner::halt`] answers, read from what the engine says
/// of the container and from what the two scripts print — and never a guess.
///
/// A frozen, exited or gone container is answered from the engine alone,
/// and nothing is asked of it. A running one is stopped by the run's tag
/// (its log's stem) and then asked, apart, what still carries it. A script
/// that did not run is never read as one that found nothing.
#[test]
fn a_halt_says_what_the_container_said_and_never_guesses() {
    use nunki::engine::spawn::{ContainerSpawner, Halt, SYSTEM_SHELL, left_script, reap_script};
    use std::sync::Arc;

    let log = Path::new("/hq/runs/mutants-abc1234.log");
    let halt = |fake: FakeEngine| {
        let fake = Arc::new(fake);
        let halt = ContainerSpawner::new(
            fake.clone(),
            file(),
            "nunki-demo",
            nunki::compose::AGENT_SERVICE,
        )
        .halt("cafe1234abcdef", log);
        (halt, fake)
    };
    let says = |stdout: &str| ExecOutput {
        status: 0,
        stdout: format!("{stdout}\n"),
        stderr: String::new(),
    };
    let execs = |fake: &FakeEngine| -> Vec<Vec<String>> {
        fake.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Exec(_, _, argv) => Some(argv),
                _ => None,
            })
            .collect()
    };

    // Answered from the engine alone.
    let (paused, fake) =
        halt(FakeEngine::default().with_liveness("cafe1234abcdef", Liveness::Paused));
    assert_eq!(paused, Halt::Paused);
    assert!(
        execs(&fake).is_empty(),
        "a paused container was asked something"
    );
    let (exited, fake) =
        halt(FakeEngine::default().with_liveness("cafe1234abcdef", Liveness::Exited(137)));
    assert_eq!(
        exited,
        Halt::NotRunning("container cafe1234abcd exited (137)".into())
    );
    assert!(execs(&fake).is_empty());
    let (gone, fake) = halt(FakeEngine::default());
    assert_eq!(
        gone,
        Halt::NotRunning("the engine no longer knows container cafe1234abcd".into())
    );
    assert!(execs(&fake).is_empty());

    let running = || FakeEngine::default().with_liveness("cafe1234abcdef", Liveness::Running);

    // Stopped by the tag, asked again, and nothing left.
    let (done, fake) = halt(
        running()
            .with_exec_when("nunki-reaped", says("nunki-reaped 7 9"))
            .with_exec_when("nunki-left", says("nunki-left")),
    );
    assert_eq!(
        done,
        Halt::Done {
            signalled: vec![7, 9]
        }
    );
    let execs_done = execs(&fake);
    assert_eq!(
        execs_done,
        vec![
            vec![
                SYSTEM_SHELL.to_string(),
                "-c".into(),
                reap_script("mutants-abc1234")
            ],
            vec![
                SYSTEM_SHELL.to_string(),
                "-c".into(),
                left_script("mutants-abc1234")
            ],
        ]
    );

    // Something left after the stop: named.
    let (left, _) = halt(
        running()
            .with_exec_when("nunki-reaped", says("nunki-reaped 7 9"))
            .with_exec_when("nunki-left", says("nunki-left 9 12")),
    );
    assert_eq!(left, Halt::Left(vec![9, 12]));

    // The check prints what each of its two looks found: a process both
    // found is named once, in the order first said.
    let (twice, _) = halt(
        running()
            .with_exec_when("nunki-reaped", says("nunki-reaped 7 9"))
            .with_exec_when("nunki-left", says("nunki-left 12 9 12 15")),
    );
    assert_eq!(twice, Halt::Left(vec![12, 9, 15]));

    // The stop did not run: nothing asked after it, and said with what the
    // container said.
    let (unknown, fake) = halt(running().with_exec_when(
        "nunki-reaped",
        ExecOutput {
            status: 126,
            stdout: String::new(),
            stderr: "exec failed\n".into(),
        },
    ));
    assert_eq!(
        unknown,
        Halt::Unknown("the stop did not run in the container (126): exec failed".into())
    );
    assert_eq!(
        execs(&fake).len(),
        1,
        "checked after a stop that did not run"
    );

    // The check did not run, and said nothing: the status is all there is.
    let (unknown, _) = halt(
        running()
            .with_exec_when("nunki-reaped", says("nunki-reaped"))
            .with_exec_when(
                "nunki-left",
                ExecOutput {
                    status: 137,
                    stdout: "chatter\n".into(),
                    stderr: String::new(),
                },
            ),
    );
    assert_eq!(
        unknown,
        Halt::Unknown("the check after the stop did not run in the container (137)".into())
    );
}

/// The check after a stop, run for real: it names what carries the run's
/// tag, and nothing else — not another run's process, not an untagged one.
#[cfg(target_os = "linux")]
#[test]
fn what_is_left_of_a_run_is_named_by_its_tag() {
    use std::process::{Command, Stdio};

    let run = format!("left-{}", std::process::id());
    let leave = |tag: Option<&str>| -> u32 {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 60 >/dev/null 2>&1 & echo $!"])
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        if let Some(tag) = tag {
            command.env("NUNKI_RUN", tag);
        }
        String::from_utf8_lossy(&command.output().unwrap().stdout)
            .trim()
            .parse()
            .unwrap()
    };
    let mine = leave(Some(&run));
    let other = leave(Some(&format!("{run}-other")));
    let untagged = leave(None);

    let said = |run: &str| {
        let out = Command::new("sh")
            .args(["-c", &nunki::engine::spawn::left_script(run)])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let found = said(&run);
    for pid in [mine, other, untagged] {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }

    // Once per look: it looks twice.
    assert_eq!(found, format!("nunki-left {mine} {mine}"));
    assert_eq!(
        said(&format!("nobody-{}", std::process::id())),
        "nunki-left"
    );
}

/// A `tr` and a `grep` that find nothing, in a directory put first on the
/// `PATH`: what code a campaign runs could drop in `~/.cargo/bin`, which an
/// agent image puts first and the agent's own uid can write. Neither the
/// stop nor the check after it may take them.
#[cfg(target_os = "linux")]
#[test]
fn the_stop_and_its_check_never_take_tr_or_grep_from_the_path_they_were_given() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    let decoys = tempfile::tempdir().unwrap();
    for tool in ["tr", "grep", "sleep"] {
        let path = decoys.path().join(tool);
        std::fs::write(&path, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        decoys.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let run = format!("decoyed-{}", std::process::id());
    let pid: u32 = String::from_utf8_lossy(
        &Command::new("sh")
            .args(["-c", "sleep 60 >/dev/null 2>&1 & echo $!"])
            .env("NUNKI_RUN", &run)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .parse()
    .unwrap();

    let under_decoys = |script: String| {
        let out = Command::new(nunki::engine::spawn::SYSTEM_SHELL)
            .args(["-c", &script])
            .env("PATH", &path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let left = under_decoys(nunki::engine::spawn::left_script(&run));
    let reaped = under_decoys(nunki::engine::spawn::reap_script(&run));
    let gone = !std::path::Path::new(&format!("/proc/{pid}")).exists()
        || std::fs::read_to_string(format!("/proc/{pid}/status"))
            .map(|s| s.contains("State:\tZ"))
            .unwrap_or(true);
    let _ = Command::new("sh")
        .args(["-c", &format!("kill {pid}")])
        .status();

    assert_eq!(
        left,
        format!("nunki-left {pid} {pid}"),
        "the check was fooled"
    );
    assert_eq!(reaped, format!("nunki-reaped {pid}"), "the stop was fooled");
    assert!(gone, "the stop did not stop {pid}");
}

/// A process that appears between the check's two looks — one tagged, born
/// of an untagged parent half a second after the check starts — is found by
/// the second. One look, a single pass over `/proc`, reads "nothing left".
#[cfg(target_os = "linux")]
#[test]
fn the_check_after_a_stop_looks_twice_before_it_says_nothing_is_left() {
    use std::process::{Command, Stdio};

    let run = format!("late-{}", std::process::id());
    let check = Command::new("sh")
        .args(["-c", &nunki::engine::spawn::left_script(&run)])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // Untagged, so the first look cannot see it; it starts the tagged one
    // half a second in, which the second look, a second in, can.
    let late = Command::new("sh")
        .args([
            "-c",
            &format!("sleep 0.5; NUNKI_RUN={run} exec sleep 5 >/dev/null 2>&1"),
        ])
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let found = String::from_utf8_lossy(&check.wait_with_output().unwrap().stdout)
        .trim()
        .to_string();
    let pid = late.id();
    let _ = Command::new("sh")
        .args(["-c", &format!("kill {pid}")])
        .status();
    let mut late = late;
    let _ = late.wait();

    assert_eq!(
        found,
        format!("nunki-left {pid}"),
        "the late process was missed"
    );
}
