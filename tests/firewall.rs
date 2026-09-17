//! The firewall sidecar, checked the only way it can honestly be checked:
//! by running it and trying to get out (SPEC 4.1 bis, rule 7).
//!
//! `#[ignore]`d — it builds an image and lifts two containers. Run it by
//! hand, and after any change to `assets/firewall/`:
//!
//! ```text
//! cargo test --test firewall -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use nunki::compose::{AGENT_WRITABLE, NamedVolume, Plan, UserIds, generate, project_name};
use nunki::harness::Role;
use nunki::perimeter::{Sources, compute, probes};

const FIREWALL_IMAGE: &str = "nunki/firewall:test";
const SLOT: &str = "fwlive";
/// The one name the agent is allowed to resolve and reach.
const ALLOWED: &str = "example.com";

/// The address a container of this project got, so a probe can try to reach
/// it by number.
fn compose_version() -> String {
    let (program, leading) = compose_command();
    let out = Command::new(program)
        .args(leading)
        .arg("version")
        .output()
        .expect("the compose command is on the path");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn address_of(project: &str, service: &str) -> String {
    let (program, leading) = compose_command();
    let id = Command::new(program)
        .args(leading)
        .args(["-p", project, "ps", "-q", service])
        .output()
        .expect("the compose command is on the path");
    let id = String::from_utf8_lossy(&id.stdout).trim().to_string();
    assert!(!id.is_empty(), "{service} has no container");
    let out = Command::new("docker")
        .args([
            "inspect",
            "-f",
            "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
            &id,
        ])
        .output()
        .expect("docker is on the path");
    let address = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!address.is_empty(), "{service} has no address");
    address
}

#[test]
#[ignore = "builds an image and lifts containers; run by hand"]
fn live_the_firewall_holds() {
    let _context = build_image();

    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::create_dir_all(&mission).unwrap();
    for file in AGENT_WRITABLE {
        std::fs::write(mission.join(file), "").unwrap();
    }

    let perimeter = compute(
        Role::Coder,
        &Sources {
            stack: &[ALLOWED.to_string()],
            harness: &[],
            services: &[],
            forge: &["github.com".to_string()],
        },
    )
    .unwrap();

    let plan = Plan {
        session: "11111111-2222-4333-8444-555555555555".into(),
        slot: SLOT.to_string(),
        role: Role::Coder,
        // Busybox tools are all the probes need.
        image: "alpine:3.20".to_string(),
        firewall_image: FIREWALL_IMAGE.to_string(),
        user: UserIds { uid: 501, gid: 20 },
        tree: tree.clone(),
        tree_at: PathBuf::from("/work/tree"),
        mission_dir: mission.clone(),
        mission_dir_at: PathBuf::from("/work/mission"),
        stack_scripts: Vec::new(),
        credentials: Vec::new(),
        advisories: None,
        volumes: Vec::<NamedVolume>::new(),
        environment: BTreeMap::new(),
        command: vec!["sleep".to_string(), "600".to_string()],
        perimeter,
        // A neighbour on the slot's own network, declared the way a project
        // declares its services. Nothing allows it: reaching it must fail.
        project_services: Some(
            serde_yaml_ng::from_str("neighbour:\n  image: nginx:alpine\n").unwrap(),
        ),
        project_networks: None,
        project_volumes: None,
    };

    let file = dir.path().join("mission.yml");
    std::fs::write(&file, generate(&plan, &dialect()).unwrap()).unwrap();

    let project = project_name("11111111-2222-4333-8444-555555555555", SLOT).unwrap();
    down(&file, &project);
    let up = compose(&file, &project, &["up", "-d", "--wait"]);
    assert!(
        up.status.success(),
        // A sidecar that fails to pose its rules is an agent that never
        // starts (rule 2) — so this failing is itself the guard working.
        "the pair did not come up:\n{}\n{}",
        String::from_utf8_lossy(&up.stdout),
        String::from_utf8_lossy(&up.stderr)
    );

    let neighbour = address_of(&project, "neighbour");
    println!("the neighbour sits at {neighbour}");

    // The battery lives in `nunki::perimeter` so that `nunki check` runs this very
    // list and not a copy of it (SPEC 4.1 bis, rule 7). Of the two forbidden
    // addresses, only the neighbour is hermetic: 1.1.1.1 needs the machine to
    // have a way out at all, or it goes green for the wrong reason.
    let checks = probes(
        ALLOWED,
        &[("1.1.1.1".to_string(), 443), (neighbour.clone(), 80)],
    );

    let mut failures = Vec::new();
    for probe in &checks {
        let got = compose(
            &file,
            &project,
            &["exec", "-T", "agent", "sh", "-c", &probe.script],
        )
        .status
        .success();
        println!("{:<52} {}", probe.what, verb(got));
        if got != probe.expected {
            failures.push(format!(
                "{}: expected {}, got {}",
                probe.what,
                verb(probe.expected),
                verb(got)
            ));
        }
    }

    let id = compose(&file, &project, &["exec", "-T", "agent", "id"]);
    let id = String::from_utf8_lossy(&id.stdout).trim().to_string();
    down(&file, &project);

    assert!(
        id.starts_with("uid=501"),
        "the agent must run under the host's uid, got {id:?}"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn verb(reached: bool) -> &'static str {
    if reached { "reached" } else { "refused" }
}

/// Built from what the binary carries, not from the repository's layout —
/// which is how a slot will get it too (`nunki::firewall`).
fn build_image() -> tempfile::TempDir {
    let context = tempfile::tempdir().unwrap();
    nunki::firewall::materialise(context.path()).unwrap();
    let out = Command::new("docker")
        .args(["build", "-q", "-t", FIREWALL_IMAGE])
        .arg(context.path())
        .output()
        .expect("docker is on the path");
    println!("built {FIREWALL_IMAGE} for {}", compose_version());
    assert!(
        out.status.success(),
        "building the sidecar failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    context
}

fn compose(file: &Path, project: &str, args: &[&str]) -> std::process::Output {
    let (program, leading) = compose_command();
    Command::new(program)
        .args(leading)
        .args(["-p", project, "-f"])
        .arg(file)
        .args(args)
        .output()
        .expect("the compose command is on the path")
}

/// `docker compose` by default; `HQ_COMPOSE` overrides it with a whole
/// command line. That is how the same battery gets run against another
/// version of the engine — the development machine and CI are three major
/// versions apart, and SPEC 4.2 bis promises portability, not one machine.
fn compose_command() -> (String, Vec<String>) {
    let raw = std::env::var("HQ_COMPOSE").unwrap_or_else(|_| "docker compose".to_string());
    let mut words = raw.split_whitespace().map(str::to_string);
    let program = words.next().expect("HQ_COMPOSE must name a program");
    (program, words.collect())
}

fn down(file: &Path, project: &str) {
    compose(file, project, &["down", "-v", "--timeout", "1"]);
}

/// The system profile is the one where the sidecar must both fence the agent
/// in and let it reach what the mission declared. Measured, because a
/// generated file that lifts is not a generated file that works.
#[test]
#[ignore = "builds an image and lifts containers; run by hand"]
fn live_a_declared_service_is_reachable_and_nothing_else_is() {
    let _context = build_image();

    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::create_dir_all(&mission).unwrap();
    for file in AGENT_WRITABLE {
        std::fs::write(mission.join(file), "").unwrap();
    }

    let declared = vec![nunki::mission::Service {
        name: "db".to_string(),
        reach: vec!["db".to_string()],
        shared: false,
    }];
    let perimeter = compute(
        Role::Integrator,
        &Sources {
            stack: &[ALLOWED.to_string()],
            harness: &[],
            services: &declared,
            forge: &["github.com".to_string()],
        },
    )
    .unwrap();

    let plan = Plan {
        session: "11111111-2222-4333-8444-555555555555".into(),
        slot: "fwlivesys".to_string(),
        role: Role::Integrator,
        image: "alpine:3.20".to_string(),
        firewall_image: FIREWALL_IMAGE.to_string(),
        user: UserIds { uid: 501, gid: 20 },
        tree,
        tree_at: PathBuf::from("/work/tree"),
        mission_dir: mission,
        mission_dir_at: PathBuf::from("/work/mission"),
        stack_scripts: Vec::new(),
        credentials: Vec::new(),
        advisories: None,
        volumes: Vec::<NamedVolume>::new(),
        environment: BTreeMap::new(),
        command: vec!["sleep".to_string(), "600".to_string()],
        perimeter,
        project_services: Some(serde_yaml_ng::from_str("db:\n  image: nginx:alpine\n").unwrap()),
        project_networks: None,
        project_volumes: None,
    };

    let file = dir.path().join("system.yml");
    std::fs::write(&file, generate(&plan, &dialect()).unwrap()).unwrap();
    let project = project_name("11111111-2222-4333-8444-555555555555", "fwlivesys").unwrap();
    down(&file, &project);
    let up = compose(&file, &project, &["up", "-d", "--wait"]);
    assert!(
        up.status.success(),
        "the profile did not come up:\n{}",
        String::from_utf8_lossy(&up.stderr)
    );

    let checks: &[(&str, bool, &str)] = &[
        (
            "the declared service resolves by its compose name",
            true,
            "nslookup db 2>&1 | tail -5 | grep -q 'Address: [0-9]'",
        ),
        ("the declared service is reachable", true, "nc -z -w3 db 80"),
        (
            "an off-list name still refuses to resolve",
            false,
            "nslookup github.com 2>&1 | tail -5 | grep -q 'Address: [0-9]'",
        ),
        (
            "an undeclared address is still refused",
            false,
            "nc -z -w3 1.1.1.1 443",
        ),
    ];

    let mut failures = Vec::new();
    for (what, expected, script) in checks {
        let got = compose(
            &file,
            &project,
            &["exec", "-T", "agent", "sh", "-c", script],
        )
        .status
        .success();
        println!("{:<52} {}", what, verb(got));
        if got != *expected {
            failures.push(format!(
                "{what}: expected {}, got {}",
                verb(*expected),
                verb(got)
            ));
        }
    }
    down(&file, &project);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The binary carries its own sidecar: nothing at run time reads the
/// repository's layout, and nothing is written into an orchestrated project
/// to get the image built (SPEC 3.3).
#[test]
fn the_build_context_travels_in_the_binary() {
    let dir = tempfile::tempdir().unwrap();
    nunki::firewall::materialise(dir.path()).unwrap();

    let dockerfile = std::fs::read_to_string(dir.path().join("Dockerfile")).unwrap();
    assert!(dockerfile.contains("dnsmasq"), "{dockerfile}");
    assert!(
        dockerfile.contains("nftables"),
        "the rules need it: {dockerfile}"
    );

    let entrypoint = std::fs::read_to_string(dir.path().join("entrypoint.sh")).unwrap();
    assert!(
        entrypoint.contains("local=/#/"),
        "without it the resolver relays what it does not know"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for script in ["entrypoint.sh", "fw-ready"] {
            let mode = std::fs::metadata(dir.path().join(script))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o111,
                0o111,
                "{script} must come out executable, got {mode:o}"
            );
        }
    }
}

/// The engine's dialect, which the generator needs and must not spell
/// itself. Docker's, since Docker is the first version's only target.
fn dialect() -> nunki::engine::Dialect {
    nunki::engine::Dialect {
        netns: nunki::engine::Netns::Service,
        host_alias: "host.docker.internal".to_string(),
        userns: None,
    }
}
