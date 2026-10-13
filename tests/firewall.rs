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

use nunki::compose::{NamedVolume, Plan, UserIds, generate, project_name};
use nunki::harness::Role;
use nunki::perimeter::{Sources, compute, probes, resolves};

const FIREWALL_IMAGE: &str = "nunki/firewall:test";
const SLOT: &str = "fwlive";
/// The one name the agent is allowed to resolve and reach.
const ALLOWED: &str = "example.com";
/// The search domain every container gets here, whatever the host has.
const SEARCH_DOMAIN: &str = "search.internal.example";

/// The generated file, with a search list on the sidecar — which owns the
/// namespace, so the agent reads the same `resolv.conf` (measured: a
/// container joining another's namespace gets its `search` line).
///
/// The engine copies the host's `search` domains into every container. A
/// Mac has none; GitHub's ubuntu runner carries Azure's, and a Linux desktop
/// on a DHCP-provided domain carries that one. It is what turned the
/// declared service "refused" on the runner while every Mac said "reached"
/// (see `nunki::perimeter::resolves`), so the battery meets it on every
/// platform: a probe a search list can fool goes red here too.
fn with_search_list(yaml: String) -> String {
    let mut doc: nunki::yaml::Value = nunki::yaml::from_str(&yaml).unwrap();
    let firewall = doc
        .get_mut("services")
        .and_then(|s| s.get_mut("firewall"))
        .and_then(|f| f.as_mapping_mut())
        .expect("the generated file has a firewall service");
    firewall.insert(
        "dns_search".into(),
        nunki::yaml::Value::Sequence(vec![SEARCH_DOMAIN.into()]),
    );
    nunki::yaml::to_string(&doc).unwrap()
}

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
    for file in nunki::compose::any_agent_writable() {
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
        advisories: vec![],
        secrets: None,
        volumes: Vec::<NamedVolume>::new(),
        environment: BTreeMap::new(),
        command: vec!["sleep".to_string(), "600".to_string()],
        perimeter,
        // A neighbour on the slot's own network, declared the way a project
        // declares its services. Nothing allows it: reaching it must fail.
        project_services: Some(nunki::compose::services::Lifted::whole(
            nunki::compose::services::ServicesFile::parse(
                "services:\n  neighbour:\n    image: nginx:alpine\n",
            )
            .unwrap(),
        )),
        prober: None,
    };

    let file = dir.path().join("mission.yml");
    std::fs::write(
        &file,
        with_search_list(generate(&plan, &dialect()).unwrap()),
    )
    .unwrap();

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
    for file in nunki::compose::any_agent_writable() {
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
        advisories: vec![],
        secrets: None,
        volumes: Vec::<NamedVolume>::new(),
        environment: BTreeMap::new(),
        command: vec!["sleep".to_string(), "600".to_string()],
        perimeter,
        project_services: Some(nunki::compose::services::Lifted::whole(
            nunki::compose::services::ServicesFile::parse(
                "services:\n  db:\n    image: nginx:alpine\n",
            )
            .unwrap(),
        )),
        prober: None,
    };

    let file = dir.path().join("system.yml");
    std::fs::write(
        &file,
        with_search_list(generate(&plan, &dialect()).unwrap()),
    )
    .unwrap();
    let project = project_name("11111111-2222-4333-8444-555555555555", "fwlivesys").unwrap();
    down(&file, &project);
    let up = compose(&file, &project, &["up", "-d", "--wait"]);
    assert!(
        up.status.success(),
        "the profile did not come up:\n{}",
        String::from_utf8_lossy(&up.stderr)
    );

    // The resolve probes are `nunki check`'s own, not a copy of them: what
    // this proves is what the verb will measure (SPEC 4.1 bis, rule 7).
    let checks: Vec<(&str, bool, String)> = vec![
        (
            "the declared service resolves by its compose name",
            true,
            resolves("db"),
        ),
        (
            "the declared service is reachable",
            true,
            "nc -z -w3 db 80".to_string(),
        ),
        (
            "an off-list name still refuses to resolve",
            false,
            resolves("github.com"),
        ),
        (
            "an undeclared address is still refused",
            false,
            "nc -z -w3 1.1.1.1 443".to_string(),
        ),
    ];

    let mut failures = Vec::new();
    for (what, expected, script) in &checks {
        let got = compose(
            &file,
            &project,
            &["exec", "-T", "agent", "sh", "-c", script.as_str()],
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

/// Whether this machine has an engine to lift a pair with: the compose
/// command `HQ_COMPOSE` names (or `docker compose`), and `docker` itself,
/// which builds the sidecar. Without them a live test skips, it does not
/// fail.
fn an_engine_is_there() -> bool {
    let (program, leading) = compose_command();
    let answers = |mut command: Command| {
        command
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    let mut compose = Command::new(program);
    compose.args(leading).arg("version");
    let mut docker = Command::new("docker");
    docker.arg("info");
    answers(compose) && answers(docker)
}

/// What the agent tries on the mission folder: append to `PR.md`, replace
/// it through the folder (`mv` over it), remove it, append to
/// `VERDICT.json`. Each attempt prints what happened, so one exec answers
/// them all and a failure shows every line.
const PR_ATTEMPTS: &str = "\
    (echo agent >> /work/mission/PR.md) 2>/dev/null \
        && echo PR-WRITTEN || echo PR-REFUSED; \
    (echo agent > /work/mission/PR.new && mv -f /work/mission/PR.new /work/mission/PR.md) \
        2>/dev/null && echo PR-REPLACED || echo PR-NOT-REPLACED; \
    (rm -f /work/mission/PR.md && test ! -e /work/mission/PR.md) 2>/dev/null \
        && echo PR-REMOVED || echo PR-NOT-REMOVED; \
    (echo agent >> /work/mission/VERDICT.json) 2>/dev/null \
        && echo VERDICT-WRITTEN || echo VERDICT-REFUSED";

/// What the security agent's container must answer to [`PR_ATTEMPTS`].
const SECURITY_ANSWERS: [&str; 4] = [
    "PR-REFUSED",
    "PR-NOT-REPLACED",
    "PR-NOT-REMOVED",
    "VERDICT-WRITTEN",
];

/// A slot's tree and a mission folder under `dir`, every file an agent
/// writes present, and `PR.md` saying what the coder wrote.
fn mission_folder(dir: &Path) -> (PathBuf, PathBuf) {
    let tree = dir.join("tree");
    let mission = dir.join("mission");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::create_dir_all(&mission).unwrap();
    for file in nunki::compose::any_agent_writable() {
        std::fs::write(mission.join(file), "").unwrap();
    }
    std::fs::write(mission.join("PR.md"), "the coder's\n").unwrap();
    (tree, mission)
}

/// The profile `role` runs in, on `slot`, over `tree` and `mission`, under
/// the host's uid: what compose generates for it, nothing added.
fn role_plan(role: Role, slot: &str, tree: &Path, mission: &Path) -> Plan {
    let (uid, gid) = nunki::image::host_ids();
    let perimeter = compute(
        role,
        &Sources {
            stack: &[ALLOWED.to_string()],
            harness: &[],
            services: &[],
            forge: &[],
        },
    )
    .unwrap();
    Plan {
        session: "11111111-2222-4333-8444-555555555555".into(),
        slot: slot.to_string(),
        role,
        image: "alpine:3.20".to_string(),
        firewall_image: FIREWALL_IMAGE.to_string(),
        user: UserIds { uid, gid },
        tree: tree.to_path_buf(),
        tree_at: PathBuf::from("/work/tree"),
        mission_dir: mission.to_path_buf(),
        mission_dir_at: PathBuf::from("/work/mission"),
        stack_scripts: Vec::new(),
        credentials: Vec::new(),
        advisories: vec![],
        secrets: None,
        volumes: Vec::<NamedVolume>::new(),
        environment: BTreeMap::new(),
        command: vec!["sleep".to_string(), "600".to_string()],
        perimeter,
        project_services: None,
        prober: None,
    }
}

/// SPEC 4.5, "Un constat accepté n'est pas un constat corrigé": in the
/// profile the security agent runs in, `PR.md` is read-only — not
/// writable, not replaceable through the folder it sits in — while its
/// verdict is; in the coder's, both are writable. What the generated file
/// says is checked in `tests/compose.rs`; this is what the engine makes of
/// it, measured, each role lifted in a fresh project.
#[test]
#[ignore = "builds an image and lifts containers; run by hand"]
fn live_the_security_agent_cannot_write_pr_md_and_the_coder_can() {
    if !an_engine_is_there() {
        eprintln!("skipped: no container engine on this machine");
        return;
    }
    let _context = build_image();

    for role in [Role::Security, Role::Coder] {
        let dir = tempfile::tempdir().unwrap();
        let (tree, mission) = mission_folder(dir.path());
        let plan = role_plan(role, SLOT, &tree, &mission);
        let file = dir.path().join("profile.yml");
        std::fs::write(&file, generate(&plan, &dialect()).unwrap()).unwrap();

        let project = project_name("11111111-2222-4333-8444-555555555555", SLOT).unwrap();
        down(&file, &project);
        let up = compose(&file, &project, &["up", "-d", "--wait"]);
        assert!(
            up.status.success(),
            "{role:?}: the pair did not come up:\n{}\n{}",
            String::from_utf8_lossy(&up.stdout),
            String::from_utf8_lossy(&up.stderr)
        );
        let said = compose(
            &file,
            &project,
            &["exec", "-T", "agent", "sh", "-c", PR_ATTEMPTS],
        );
        down(&file, &project);
        let said = String::from_utf8_lossy(&said.stdout).to_string();
        println!("{role:?}:\n{said}");

        let pr = std::fs::read_to_string(mission.join("PR.md")).unwrap_or_default();
        match role {
            Role::Security => {
                for line in SECURITY_ANSWERS {
                    assert!(said.contains(line), "security: expected {line}:\n{said}");
                }
                assert_eq!(pr, "the coder's\n", "PR.md moved under the security agent");
            }
            _ => {
                for line in ["PR-WRITTEN", "VERDICT-WRITTEN"] {
                    assert!(said.contains(line), "{role:?}: expected {line}:\n{said}");
                }
                assert!(
                    pr.contains("agent"),
                    "the coder's write did not land: {pr:?}"
                );
            }
        }
    }
}

/// Takes a Compose project down, volumes included, when dropped: a test that
/// fails half-way leaves no container behind.
struct TakenDown<'a> {
    engine: &'a dyn nunki::engine::Engine,
    file: &'a Path,
    project: &'a str,
}

impl Drop for TakenDown<'_> {
    fn drop(&mut self) {
        if self.file.is_file() {
            let _ = self.engine.down(self.file, self.project, true);
        }
    }
}

/// The guard of SPEC 4.5 as a mission meets it: the security agent is
/// lifted on the very Compose project the coder's or the integrator's
/// container was just running in, and it is that switch that must leave it
/// a container without `PR.md` read-write. Played as `run::launch` plays it
/// — the real engine's own `stop` of `run::SERVICES` from the file that is
/// up, the security agent's profile written to that same file, then the
/// engine's `up` — and never its command lines retyped, so the test moves
/// with them. `Docker::real()` honours `HQ_COMPOSE`.
///
/// In the container up after the switch, `PR.md` cannot be appended to,
/// replaced or removed, the host's file is byte for byte what it was before
/// the switch, and `VERDICT.json` is written. The previous role's write to
/// `PR.md` succeeding first is what shows the switch is from a container
/// that could write it.
#[test]
#[ignore = "builds an image and lifts containers; run by hand"]
fn live_the_security_agent_cannot_write_pr_md_after_a_role_switch_on_one_project() {
    use nunki::engine::Engine;

    if !an_engine_is_there() {
        eprintln!("skipped: no container engine on this machine");
        return;
    }
    let _context = build_image();
    let docker = nunki::engine::docker::Docker::real();
    let engine: &dyn Engine = &docker;
    let slot = "fwswitch";
    let project = project_name("11111111-2222-4333-8444-555555555555", slot).unwrap();
    let agent = nunki::compose::AGENT_SERVICE;
    let attempts = |file: &Path| {
        engine
            .exec(
                file,
                &project,
                agent,
                &["sh".to_string(), "-c".to_string(), PR_ATTEMPTS.to_string()],
            )
            .unwrap()
            .stdout
    };

    for before in [Role::Coder, Role::Integrator] {
        // One mission folder, one profile file, one Compose project.
        let dir = tempfile::tempdir().unwrap();
        let (tree, mission) = mission_folder(dir.path());
        let file = dir.path().join("profile.yml");
        let _down = TakenDown {
            engine,
            file: &file,
            project: &project,
        };

        // The previous role's container, up, writing PR.md.
        let profile = |role: Role| {
            generate(&role_plan(role, slot, &tree, &mission), engine.dialect()).unwrap()
        };
        std::fs::write(&file, profile(before)).unwrap();
        let _ = engine.down(&file, &project, true);
        engine.up(&file, &project).unwrap();
        let first = engine.container_of(&file, &project, agent).unwrap();
        let said = attempts(&file);
        println!("{before:?}:\n{said}");
        assert!(
            said.contains("PR-WRITTEN"),
            "{before:?} could not write PR.md:\n{said}"
        );
        let written = std::fs::read_to_string(mission.join("PR.md")).unwrap();
        assert!(written.contains("agent"), "{before:?}: {written:?}");

        // The switch, as `run::launch` plays it.
        engine.stop(&file, &project, &nunki::run::SERVICES).unwrap();
        std::fs::write(&file, profile(Role::Security)).unwrap();
        // The previous role wrote VERDICT.json too: emptied, so what is in
        // it after is the security agent's.
        std::fs::write(mission.join("VERDICT.json"), "").unwrap();
        engine.up(&file, &project).unwrap();
        let second = engine.container_of(&file, &project, agent).unwrap();
        assert!(
            second.is_some(),
            "no agent after the switch from {before:?}"
        );
        assert_ne!(first, second, "the agent container survived the switch");

        let said = attempts(&file);
        println!("Security after {before:?}:\n{said}");
        for line in SECURITY_ANSWERS {
            assert!(
                said.contains(line),
                "security after {before:?}: expected {line}:\n{said}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(mission.join("PR.md")).unwrap(),
            written,
            "PR.md moved under the security agent, after {before:?}"
        );
        let verdict = std::fs::read_to_string(mission.join("VERDICT.json")).unwrap();
        assert!(verdict.contains("agent"), "after {before:?}: {verdict:?}");
    }
}
