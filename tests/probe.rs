//! `nunki check --slot` (SPEC 4.1 bis, rule 7): the half of the verb that can
//! only be answered by lifting containers and trying to get out.

use std::path::Path;

use nunki::image;
use nunki::project::{Config, Project, ProtectedPaths};

mod common;

fn project(dir: &Path) -> Project {
    Project::at(
        dir.join("repo"),
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
        },
        dir.join("nunki"),
    )
}

#[test]
fn images_are_named_per_project_and_stack_not_per_slot() {
    let dir = tempfile::tempdir().unwrap();
    let images = image::names(&project(dir.path()), "rust");
    // Two slots of the same project share an image: rebuilding once serves
    // both, which is what makes `nunki slot rebuild` bearable.
    assert_eq!(images.agent, "nunki/repo-rust:latest");
    assert!(images.firewall.starts_with("nunki/firewall:"));
    assert!(images.prober.starts_with("nunki/prober:"));
    assert_ne!(images.firewall, images.prober);
}

#[test]
fn the_images_run_under_the_humans_own_ids() {
    let (uid, gid) = image::host_ids();
    // Not root, and not a guess: a file written under another id is
    // unreadable on the host and git refuses the tree (SPEC 4.2 bis).
    assert_ne!(uid, 0, "nunki is not meant to be run as root");
    assert_eq!(uid, unsafe { libc::getuid() });
    assert_eq!(gid, unsafe { libc::getgid() });
}

#[test]
fn the_prober_carries_its_reason_with_it() {
    let dir = tempfile::tempdir().unwrap();
    nunki::firewall::materialise_prober(dir.path()).unwrap();
    let dockerfile = std::fs::read_to_string(dir.path().join("Dockerfile")).unwrap();
    assert!(dockerfile.starts_with("# The prober"), "{dockerfile}");
    // Alpine, because the battery needs busybox tools a stack image has no
    // reason to carry.
    assert!(dockerfile.contains("alpine"), "{dockerfile}");
}

/// Everything from an empty directory to a green perimeter, through the
/// public path: `init`, build the images, add a slot, probe it.
///
/// The stack's Dockerfile is replaced with a one-line Alpine image — a stack
/// fragment belongs to the project, and building a real Rust toolchain here
/// would cost minutes for nothing. What is being checked is the fence, which
/// belongs to the network namespace and not to the image.
///
/// ```text
/// cargo test --test probe -- --ignored --nocapture
/// ```
#[test]
#[ignore = "builds images and lifts containers; run by hand"]
fn live_a_fresh_project_ends_with_a_perimeter_that_holds() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = dir.path().join("nunki");
    std::fs::create_dir_all(&root).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );

    nunki::init::init(&root, &home, &["rust".to_string()]).unwrap();
    std::fs::write(
        home.join("stacks/rust/Dockerfile"),
        // Debian on purpose: it carries no `nslookup`, no `nc`, no `wget`.
        // If the battery ran in the agent's container instead of the
        // prober's, every positive probe would fail here — which is exactly
        // what happened on nunki before the prober existed.
        //
        // It carries the agent user all the same, and ends as that user:
        // SPEC 4.2 bis makes that a requirement of every stack image, and
        // nunki's harness layer is built on top of whatever user the stack
        // image leaves — a stack image that ends as root installs the
        // harness into root's home, where the only user that runs it cannot
        // read it. A fixture that skipped it was testing a shape no stack
        // image is allowed to have.
        "FROM debian:bookworm-slim\n\
         ARG UID=1000\n\
         ARG GID=1000\n\
         RUN apt-get -qq update \\\n\
          && apt-get -qq install --no-install-recommends -y ca-certificates curl \\\n\
          && rm -rf /var/lib/apt/lists/*\n\
         RUN groupadd -g ${GID} agent || true \\\n\
          && useradd -m -u ${UID} -g ${GID} -s /bin/bash agent\n\
         RUN mkdir -p /work/tree /work/mission /run/nunki \\\n\
          && chown -R ${UID}:${GID} /work /run/nunki\n\
         USER agent\n",
    )
    .unwrap();
    // A first commit, so the clone has something to carry.
    for args in [
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "Test"],
        vec!["add", "."],
        vec!["commit", "-qm", "first"],
    ] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(&args)
                .status()
                .unwrap()
                .success(),
            "git {args:?}"
        );
    }

    let project = Project::open_at(root.clone(), home.clone())
        .expect("init wrote a config Project::open accepts");
    let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".to_string());

    image::build(&project, "rust", &engine_bin, image::Harness::Install).expect("the images build");
    let slot = nunki::slot::add(&project, "probe").expect("the slot is cloned");

    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::docker::Docker::real());
    let checks = nunki::probe::mission_profile(&project, &slot, "rust", engine, &engine_bin)
        .expect("the profile lifts");

    let mut red = Vec::new();
    for check in &checks {
        let mark = match &check.verdict {
            nunki::check::Verdict::Green(d) => format!("ok   {d}"),
            nunki::check::Verdict::Red(d) => {
                red.push(check.what.clone());
                format!("RED  {d}")
            }
            nunki::check::Verdict::NotChecked(d) => format!("--   {d}"),
        };
        println!("{mark:<28} {}", check.what);
    }
    assert!(!checks.is_empty(), "the battery ran nothing");
    assert!(red.is_empty(), "not held: {red:?}");

    // Nothing is left running: probing a slot must not leave containers
    // behind for the next mission to trip over.
    let left = std::process::Command::new(&engine_bin)
        .args([
            "ps",
            "-a",
            "--filter",
            "name=nunki-probe-check",
            "--format",
            "{{.Names}}",
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&left.stdout).trim().is_empty(),
        "containers left behind: {}",
        String::from_utf8_lossy(&left.stdout)
    );

    let _ = nunki::slot::rm(&project, "probe", true);
}

#[test]
fn a_probe_run_has_a_compose_project_of_its_own() {
    // A slot's services are levied once and kept between profiles; probing
    // under the slot's own name would take them down at the end of the check.
    let checking =
        nunki::probe::compose_project("11111111-2222-4333-8444-555555555555", "lot2").unwrap();
    let running =
        nunki::compose::project_name("11111111-2222-4333-8444-555555555555", "lot2").unwrap();
    assert_ne!(checking, running);
    assert!(checking.contains("check"), "{checking}");
}

#[test]
fn a_missing_image_is_something_to_do_not_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = nunki::slot::Slot {
        name: "absent".to_string(),
        tree: dir.path().join("tree"),
    };
    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::fake::FakeEngine::default());

    // No image of that name was ever built, so the profile cannot be lifted.
    // `nunki check` turns this into "not checked, here is what to run" rather
    // than a violation — the perimeter is not broken, it is unbuilt.
    let err = nunki::probe::mission_profile(&project, &slot, "never-built", engine, "docker")
        .unwrap_err();
    assert!(
        matches!(err, nunki::probe::ProbeError::NoImages(..)),
        "{err}"
    );
    assert!(err.to_string().contains("nunki slot rebuild"), "{err}");
}

// --- the system profile (SPEC 4.1 bis, rule 7, `nunki check --mission`) --------

fn with_services(dir: &Path) -> (Project, nunki::slot::Slot, nunki::mission::Header) {
    let tree = dir.join("repo-slots").join("one");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(
        tree.join("compose.yaml"),
        "services:\n  db:\n    image: nginx:alpine\n    volumes: [dbdata:/data]\n  \
         cache:\n    image: nginx:alpine\nvolumes:\n  dbdata: {}\n",
    )
    .unwrap();
    let mut project = project(dir);
    project.config.services_file = Some("compose.yaml".into());
    let header = nunki::mission::Header {
        branch: "mission/x".into(),
        base: "dev".into(),
        lots: vec![nunki::mission::Lot {
            id: "L1".into(),
            title: "one".into(),
        }],
        integration: nunki::mission::Integration::Services {
            services: vec![nunki::mission::Service {
                name: "db".into(),
                reach: vec!["db".into()],
                shared: false,
            }],
            wiring: vec![],
        },
        security: nunki::mission::Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    let slot = nunki::slot::Slot {
        name: "one".into(),
        tree,
    };
    (project, slot, header)
}

/// The system profile a check lifts is the run's own plan, under a slot of
/// its own. That is what keeps the project's database from being lifted a
/// second time on the volumes of the one the slot is using: every name nunki
/// derives from the slot is the check's.
#[test]
fn a_system_profile_check_lifts_under_a_slot_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let (project, slot, header) = with_services(dir.path());
    let images = image::names(&project, "rust");
    let paths = nunki::mission::dir::Paths::of(&project.hq_root, "m1");
    let plan =
        nunki::probe::system_plan(&project, &slot, "rust", &images, &paths, &header).unwrap();

    assert_eq!(plan.slot, "one-check");
    assert_eq!(plan.role, nunki::harness::Role::Integrator);
    assert!(!plan.volumes.is_empty());
    for volume in &plan.volumes {
        assert!(
            volume.name.contains("one-check"),
            "{} would be the slot's own volume",
            volume.name
        );
    }
    let Some(serde_yaml_ng::Value::Mapping(services)) = &plan.project_services else {
        panic!("{:?}", plan.project_services);
    };
    for name in ["db", "cache", nunki::probe::PROBER_SERVICE] {
        assert!(
            services.contains_key(serde_yaml_ng::Value::from(name)),
            "{name} is lifted: {services:?}"
        );
    }
    assert_eq!(
        plan.command,
        vec!["sleep".to_string(), "600".to_string()],
        "the agent's container only sleeps: a check spends no subscription"
    );
}

/// What only a system profile has to prove: the declared service resolves,
/// and the project's other service — on the same network, merged whole —
/// does not. And the battery's "allowed host" is never a declared service,
/// whose port nobody knows.
#[test]
fn the_system_battery_resolves_what_is_declared_and_refuses_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let (project, slot, header) = with_services(dir.path());
    let images = image::names(&project, "rust");
    let paths = nunki::mission::dir::Paths::of(&project.hq_root, "m1");
    let plan =
        nunki::probe::system_plan(&project, &slot, "rust", &images, &paths, &header).unwrap();
    let probes = nunki::probe::system_probes(&plan, &header);

    let find = |start: &str| {
        probes
            .iter()
            .find(|p| p.what.starts_with(start))
            .unwrap_or_else(|| {
                panic!(
                    "no probe for {start}: {:?}",
                    probes.iter().map(|p| &p.what).collect::<Vec<_>>()
                )
            })
    };
    assert!(find("db, a service the mission declares").expected);
    assert!(!find("cache, a project service the mission does not declare").expected);
    assert!(
        !probes.iter().any(|p| p
            .what
            .starts_with(&format!("{}, ", nunki::probe::PROBER_SERVICE))),
        "the prober is nunki's, not the project's"
    );
    let allowed = find("").what.clone();
    assert!(
        !probes
            .iter()
            .any(|p| p.what.starts_with("db, an allowed host")),
        "a declared service was taken as the battery's allowed host: {allowed}"
    );
}

fn started(project: &Project, header: &nunki::mission::Header) {
    nunki::mission::dir::create(&project.hq_root, "m1", header, "probe it").unwrap();
    nunki::state::Store::open(&project.hq_root)
        .unwrap()
        .save(&nunki::state::MissionState {
            id: "m1".into(),
            slot: "one".into(),
            flow: nunki::mission::flow::Flow::new(header.clone()).unwrap(),
            run: None,
            app: None,
            verdicts: Vec::new(),
            accepted: Vec::new(),
            stopped: None,
            harness_down: None,
            spent: Default::default(),
            spared: None,
            coder_session: None,
            updated_at: String::new(),
        })
        .unwrap();
}

/// A mission that declares no service has no system profile — said, and
/// nothing lifted. The engine binary here does not exist, so reaching it
/// would fail the test.
#[test]
fn a_mission_without_services_has_no_system_profile_to_probe() {
    let dir = tempfile::tempdir().unwrap();
    let (project, _, mut header) = with_services(dir.path());
    header.integration = nunki::mission::Integration::None {
        reason: "pure domain".into(),
    };
    started(&project, &header);

    let fake = std::sync::Arc::new(nunki::engine::fake::FakeEngine::default());
    let checks =
        nunki::probe::system_profile(&project, "m1", fake.clone(), "/nonexistent/engine").unwrap();
    assert_eq!(checks.len(), 1);
    match &checks[0].verdict {
        nunki::check::Verdict::NotChecked(why) => {
            assert!(why.contains("declares no service"), "{why}")
        }
        other => panic!("{other:?}"),
    }
    assert!(
        fake.calls().is_empty(),
        "nothing was lifted: {:?}",
        fake.calls()
    );
}

#[test]
fn a_mission_that_never_started_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let (project, _, _) = with_services(dir.path());
    let fake = std::sync::Arc::new(nunki::engine::fake::FakeEngine::default());
    let err =
        nunki::probe::system_profile(&project, "m1", fake, "/nonexistent/engine").unwrap_err();
    assert!(
        matches!(err, nunki::probe::ProbeError::NotStarted(_)),
        "{err}"
    );
}

/// The whole of `nunki check --mission`: a project whose services file lifts a
/// database and a cache, a mission that declares only the database, and a
/// fence that lets the one through and not the other.
///
/// ```text
/// cargo test --test probe live_a_system -- --ignored --nocapture
/// ```
#[test]
#[ignore = "builds images and lifts containers; run by hand"]
fn live_a_system_profile_reaches_what_the_mission_declares_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = dir.path().join("nunki");
    std::fs::create_dir_all(&root).unwrap();
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .unwrap()
                .success(),
            "git {args:?}"
        );
    };
    git(&["init", "-q", "-b", "main"]);
    nunki::init::init(&root, &home, &["rust".to_string()]).unwrap();
    // Ends on the agent user, as every stack image must (SPEC 4.2 bis).
    std::fs::write(
        home.join("stacks/rust/Dockerfile"),
        "FROM debian:bookworm-slim\n\
         ARG UID=1000\n\
         ARG GID=1000\n\
         RUN apt-get -qq update \\\n\
          && apt-get -qq install --no-install-recommends -y ca-certificates curl \\\n\
          && rm -rf /var/lib/apt/lists/*\n\
         RUN groupadd -g ${GID} agent || true \\\n\
          && useradd -m -u ${UID} -g ${GID} -s /bin/bash agent\n\
         RUN mkdir -p /work/tree /work/mission /run/nunki \\\n\
          && chown -R ${UID}:${GID} /work /run/nunki\n\
         USER agent\n",
    )
    .unwrap();
    std::fs::write(
        root.join("compose.yaml"),
        "services:\n  db:\n    image: nginx:alpine\n  cache:\n    image: nginx:alpine\n",
    )
    .unwrap();
    let config = std::fs::read_to_string(home.join("nunki.yaml")).unwrap();
    std::fs::write(
        home.join("nunki.yaml"),
        format!("{config}\nservices_file: compose.yaml\n"),
    )
    .unwrap();
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "Test"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "first"]);

    let project = Project::open_at(root.clone(), home.clone()).unwrap();
    let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".to_string());
    image::build(&project, "rust", &engine_bin, image::Harness::Install).expect("the images build");
    let slot = nunki::slot::add(&project, "sys").expect("the slot is cloned");

    let (_, _, header) = with_services(dir.path());
    nunki::mission::dir::create(&project.hq_root, "m1", &header, "probe it").unwrap();
    nunki::state::Store::open(&project.hq_root)
        .unwrap()
        .save(&nunki::state::MissionState {
            id: "m1".into(),
            slot: slot.name.clone(),
            flow: nunki::mission::flow::Flow::new(header.clone()).unwrap(),
            run: None,
            app: None,
            verdicts: Vec::new(),
            accepted: Vec::new(),
            stopped: None,
            harness_down: None,
            spent: Default::default(),
            spared: None,
            coder_session: None,
            updated_at: String::new(),
        })
        .unwrap();

    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::docker::Docker::real());
    let checks = nunki::probe::system_profile(&project, "m1", engine, &engine_bin)
        .expect("the system profile lifts");

    let mut red = Vec::new();
    for check in &checks {
        let mark = match &check.verdict {
            nunki::check::Verdict::Green(d) => format!("ok   {d}"),
            nunki::check::Verdict::Red(d) => {
                red.push(check.what.clone());
                format!("RED  {d}")
            }
            nunki::check::Verdict::NotChecked(d) => format!("--   {d}"),
        };
        println!("{mark:<28} {}", check.what);
    }
    assert!(red.is_empty(), "not held: {red:?}");
    let green = |start: &str| {
        checks
            .iter()
            .any(|c| c.what.contains(start) && matches!(c.verdict, nunki::check::Verdict::Green(_)))
    };
    assert!(green("db, a service the mission declares, resolves"));
    assert!(green(
        "cache, a project service the mission does not declare, resolves"
    ));

    // The profile it lifted mounted a scratch mission folder, never the real
    // one — whose journal the agent's files would otherwise make writable.
    let real = nunki::mission::dir::Paths::of(&project.hq_root, "m1").dir;
    let lifted = std::fs::read_to_string(project.hq_root.join("checks/sys-system.yml")).unwrap();
    assert!(
        !lifted.contains(&real.display().to_string()),
        "the check mounted the mission's own folder:\n{lifted}"
    );

    let left = std::process::Command::new(&engine_bin)
        .args([
            "ps",
            "-a",
            "--filter",
            "name=nunki-sys-check",
            "--format",
            "{{.Names}}",
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&left.stdout).trim().is_empty(),
        "left running: {}",
        String::from_utf8_lossy(&left.stdout)
    );
}

/// A declared service that sorts first among the allowed names must still
/// not be taken as the battery's "allowed host": its port is its own, and
/// the battery's TCP probe on 443 would fail on it and read as a wall.
#[test]
fn the_battery_never_takes_a_declared_service_as_its_allowed_host() {
    let dir = tempfile::tempdir().unwrap();
    let (project, slot, mut header) = with_services(dir.path());
    header.integration = nunki::mission::Integration::Services {
        services: vec![nunki::mission::Service {
            name: "aaa-store".into(),
            reach: vec!["aaa-store".into()],
            shared: false,
        }],
        wiring: vec![],
    };
    let images = image::names(&project, "rust");
    let paths = nunki::mission::dir::Paths::of(&project.hq_root, "m1");
    let plan =
        nunki::probe::system_plan(&project, &slot, "rust", &images, &paths, &header).unwrap();
    let probes = nunki::probe::system_probes(&plan, &header);
    assert!(
        !probes
            .iter()
            .any(|p| p.what.starts_with("aaa-store, an allowed")),
        "{:?}",
        probes.iter().map(|p| &p.what).collect::<Vec<_>>()
    );
}

/// The verb, not only the library: `nunki check --mission` on a mission that
/// declares no service says so — which it can only do by calling the probe.
#[test]
fn the_verb_probes_the_system_profile_of_the_mission_it_is_given() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("repo");
    let project_home =
        common::project_home(&root, home.path(), "harness: claude-code\nstacks: [rust]\n");
    let (project, _, mut header) = with_services(home.path());
    header.integration = nunki::mission::Integration::None {
        reason: "pure domain".into(),
    };
    let project = Project::at(root.clone(), project.config, project_home);
    std::fs::create_dir_all(&project.hq_root).unwrap();
    started(&project, &header);

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_nunki"))
        .env("HOME", home.path())
        .env("HQ_ENGINE", "/nonexistent/engine")
        .args(["-C"])
        .arg(&root)
        .args(["check", "--mission", "m1"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("system profile of m1"), "{text}");
    assert!(text.contains("declares no service"), "{text}");
}

/// The mission folder a system-profile check mounts is a scratch one under
/// `checks/`, with the agent's files present — never the mission's own.
#[test]
fn a_system_profile_check_mounts_a_scratch_mission_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (project, slot, _) = with_services(dir.path());
    let paths = nunki::probe::scratch_paths(&project, &slot, "m1").unwrap();
    let real = nunki::mission::dir::Paths::of(&project.hq_root, "m1");

    assert_ne!(paths.dir, real.dir);
    assert!(
        paths.dir.starts_with(project.hq_root.join("checks")),
        "{}",
        paths.dir.display()
    );
    for file in nunki::compose::AGENT_WRITABLE {
        assert!(
            paths.dir.join(file).is_file(),
            "{file} is there to be mounted"
        );
    }
}
