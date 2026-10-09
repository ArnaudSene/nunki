//! Compose generation (SPEC 4.2) and the container doctrine of 4.1 bis.
//!
//! Golden files under `tests/fixtures/compose/`: generation happens at every
//! launch, so the bytes must not move unless a decision moves. Refresh them
//! with `HQ_BLESS=1 cargo test --test compose` and read the diff.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nunki::compose::{
    AGENT_SERVICE, AGENT_WRITABLE, ComposeError, FIREWALL_SERVICE, NamedVolume, PROBER_SERVICE,
    Plan, UserIds, generate, project_name,
};
use nunki::harness::Role;
use nunki::mission::Service;
use nunki::perimeter::{Sources, compute};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn services() -> Vec<Service> {
    vec![Service {
        name: "db".to_string(),
        reach: strings(&["db", "10.4.0.7"]),
        shared: false,
    }]
}

const SESSION: &str = "11111111-2222-4333-8444-555555555555";

fn plan(role: Role) -> Plan {
    let mission_services = match role {
        Role::Coder => Vec::new(),
        Role::Integrator | Role::Security => services(),
    };
    let perimeter = compute(
        role,
        &Sources {
            stack: &strings(&["index.crates.io", "static.crates.io"]),
            harness: &strings(&["api.anthropic.com", "statsig.anthropic.com"]),
            services: &mission_services,
            forge: &strings(&["github.com"]),
        },
    )
    .unwrap();

    let credentials = match role {
        Role::Coder => Vec::new(),
        _ => vec![(
            PathBuf::from("/Users/h/.nunki/demo/credentials/db.env"),
            PathBuf::from("/run/nunki/credentials/db.env"),
        )],
    };

    let mut volumes = vec![NamedVolume {
        name: "nunki-demo-1-harness".to_string(),
        at: PathBuf::from("/home/agent/.harness"),
    }];
    if role == Role::Security {
        // What a read-only tree still needs to write, from `writable.txt`.
        volumes.push(NamedVolume {
            name: "nunki-demo-1-target".to_string(),
            at: PathBuf::from("/work/tree/target"),
        });
    }

    let mut environment = BTreeMap::new();
    environment.insert("HQ_ROLE".to_string(), format!("{role:?}").to_lowercase());

    Plan {
        session: "11111111-2222-4333-8444-555555555555".into(),
        slot: "demo-1".to_string(),
        role,
        image: "nunki/rust:1".to_string(),
        firewall_image: "nunki/firewall:1".to_string(),
        user: UserIds { uid: 501, gid: 20 },
        tree: PathBuf::from("/Users/h/code/demo-slot-1"),
        tree_at: PathBuf::from("/work/tree"),
        mission_dir: PathBuf::from("/Users/h/.nunki/demo/hq/missions/m1"),
        mission_dir_at: PathBuf::from("/work/mission"),
        stack_scripts: vec![(
            PathBuf::from("/Users/h/.nunki/demo/stacks/rust/prepush.sh"),
            PathBuf::from("/work/stack/prepush.sh"),
        )],
        credentials,
        // The golden files freeze a profile without one: `nunki check` is
        // what proves a real database reaches the container.
        advisories: vec![],
        secrets: None,
        volumes,
        environment,
        command: strings(&["sleep", "infinity"]),
        perimeter,
        project_services: None,
        prober: None,
    }
}

fn golden(name: &str, produced: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/compose")
        .join(name);
    if std::env::var_os("HQ_BLESS").is_some() {
        std::fs::write(&path, produced).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} — bless with HQ_BLESS=1", path.display()));
    assert_eq!(produced, expected, "{} moved", path.display());
}

#[test]
fn the_mission_profile_is_stable() {
    golden(
        "mission.yml",
        &generate(&plan(Role::Coder), &dialect()).unwrap(),
    );
}

#[test]
fn the_system_profile_of_the_integrator_is_stable() {
    golden(
        "system-integrator.yml",
        &generate(&plan(Role::Integrator), &dialect()).unwrap(),
    );
}

#[test]
fn the_system_profile_of_the_security_agent_is_stable() {
    golden(
        "system-security.yml",
        &generate(&plan(Role::Security), &dialect()).unwrap(),
    );
}

#[test]
fn generating_twice_yields_the_same_bytes() {
    let plan = plan(Role::Integrator);
    assert_eq!(
        generate(&plan, &dialect()).unwrap(),
        generate(&plan, &dialect()).unwrap()
    );
}

#[test]
fn the_agent_joins_the_firewall_and_waits_for_it() {
    // Measured against Docker Compose v5.1.2: in the other direction the
    // agent starts first and has a network before any rule is laid.
    let yaml = generate(&plan(Role::Coder), &dialect()).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let agent = &doc["services"][AGENT_SERVICE];
    let firewall = &doc["services"][FIREWALL_SERVICE];

    assert_eq!(
        agent["network_mode"].as_str(),
        Some("service:firewall"),
        "the agent must join the firewall's namespace"
    );
    assert!(
        firewall["network_mode"].is_null(),
        "the firewall owns the namespace, it joins nothing"
    );
    assert_eq!(
        agent["depends_on"]["firewall"]["condition"].as_str(),
        Some("service_healthy"),
        "a sidecar that fails is an agent that does not start"
    );
    assert!(
        !firewall["healthcheck"].is_null(),
        "without a healthcheck, service_healthy can never be met"
    );
}

#[test]
fn the_agent_has_no_power_and_the_firewall_has_it_all() {
    let yaml = generate(&plan(Role::Coder), &dialect()).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let agent = &doc["services"][AGENT_SERVICE];
    let firewall = &doc["services"][FIREWALL_SERVICE];

    assert_eq!(agent["cap_drop"][0].as_str(), Some("ALL"));
    assert!(agent["cap_add"].is_null(), "the agent gains no capability");
    assert_eq!(
        agent["security_opt"][0].as_str(),
        Some("no-new-privileges:true")
    );
    assert_eq!(agent["user"].as_str(), Some("501:20"));

    assert_eq!(firewall["cap_drop"][0].as_str(), Some("ALL"));
    let caps: Vec<_> = firewall["cap_add"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(caps, vec!["NET_ADMIN", "NET_RAW", "SETUID", "SETGID"]);
}

#[test]
fn the_agent_declares_no_network_because_compose_refuses_both() {
    // "mutually exclusive `network_mode` and `networks`: invalid compose
    // project" — measured. The firewall is what attaches to the services.
    let yaml = generate(&plan(Role::Integrator), &dialect()).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    assert!(doc["services"][AGENT_SERVICE]["networks"].is_null());
    assert!(doc["services"][AGENT_SERVICE]["ports"].is_null());
}

#[test]
fn the_allowlist_reaches_the_sidecar_and_only_the_sidecar() {
    let coder = generate(&plan(Role::Coder), &dialect()).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&coder).unwrap();
    let env = &doc["services"][FIREWALL_SERVICE]["environment"];
    assert_eq!(
        env["HQ_ALLOW_DOMAINS"].as_str(),
        Some("api.anthropic.com,index.crates.io,static.crates.io,statsig.anthropic.com")
    );
    assert_eq!(env["HQ_ALLOW_ADDRESSES"].as_str(), Some(""));
    assert!(
        doc["services"][AGENT_SERVICE]["environment"]["HQ_ALLOW_DOMAINS"].is_null(),
        "the agent is not told its own allowlist by the generator"
    );

    let integrator = generate(&plan(Role::Integrator), &dialect()).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&integrator).unwrap();
    let env = &doc["services"][FIREWALL_SERVICE]["environment"];
    assert!(env["HQ_ALLOW_DOMAINS"].as_str().unwrap().contains(",db,"));
    assert_eq!(env["HQ_ALLOW_ADDRESSES"].as_str(), Some("10.4.0.7"));
}

#[test]
fn the_security_agent_reads_the_tree_and_the_others_write_it() {
    for (role, mode) in [
        (Role::Coder, "rw"),
        (Role::Integrator, "rw"),
        (Role::Security, "ro"),
    ] {
        let yaml = generate(&plan(role), &dialect()).unwrap();
        let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
        let mounts: Vec<_> = doc["services"][AGENT_SERVICE]["volumes"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let tree = mounts
            .iter()
            .find(|m| m.contains("demo-slot-1:/work/tree:"))
            .unwrap_or_else(|| panic!("{role:?} has no tree mount in {mounts:?}"));
        assert!(tree.ends_with(&format!(":{mode}")), "{role:?}: {tree}");
    }
}

/// The scripts that judge the agent reach it from the project's home, one
/// file at a time and read-only — never the stack's directory, whose
/// Dockerfile and allowlist are read on the host only.
#[test]
fn the_stacks_scripts_are_mounted_read_only_one_file_at_a_time() {
    for role in [Role::Coder, Role::Integrator, Role::Security] {
        let yaml = generate(&plan(role), &dialect()).unwrap();
        let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
        let mounts: Vec<_> = doc["services"][AGENT_SERVICE]["volumes"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(
            mounts.contains(
                &"/Users/h/.nunki/demo/stacks/rust/prepush.sh:/work/stack/prepush.sh:ro"
                    .to_string()
            ),
            "{role:?}: {mounts:?}"
        );
        assert!(
            !mounts
                .iter()
                .any(|m| m.contains("/stacks/rust:") || m.contains("/stacks/rust/:")),
            "{role:?} mounts the whole stack directory: {mounts:?}"
        );
    }
}

#[test]
fn the_mission_folder_is_read_only_but_for_the_agents_own_files() {
    let yaml = generate(&plan(Role::Coder), &dialect()).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let mounts: Vec<_> = doc["services"][AGENT_SERVICE]["volumes"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    assert!(
        mounts.contains(&"/Users/h/.nunki/demo/hq/missions/m1:/work/mission:ro".to_string()),
        "{mounts:?}"
    );
    for file in AGENT_WRITABLE {
        let expected =
            format!("/Users/h/.nunki/demo/hq/missions/m1/{file}:/work/mission/{file}:rw");
        assert!(
            mounts.contains(&expected),
            "missing {expected} in {mounts:?}"
        );
    }
    // MISSION.md is inside the read-only mount and nowhere else: the header
    // the run is framed by cannot be rewritten by the run (SPEC 4.1).
    assert!(
        !mounts.iter().any(|m| m.contains("MISSION.md")),
        "{mounts:?}"
    );
}

#[test]
fn credentials_are_read_only_and_never_on_the_mission_profile() {
    let yaml = generate(&plan(Role::Integrator), &dialect()).unwrap();
    assert!(yaml.contains("/run/nunki/credentials/db.env:ro"), "{yaml}");

    let mut coder = plan(Role::Coder);
    coder.credentials = vec![(
        PathBuf::from("/Users/h/.nunki/demo/credentials/db.env"),
        PathBuf::from("/run/nunki/credentials/db.env"),
    )];
    assert!(matches!(
        generate(&coder, &dialect()).unwrap_err(),
        ComposeError::CredentialsOnMissionProfile(1)
    ));
}

fn services_file(text: &str) -> nunki::compose::services::ServicesFile {
    nunki::compose::services::ServicesFile::parse(text).expect("the services file parses")
}

/// What lands in the generated file is nunki's rendering of the project's
/// services, read back — not the file's bytes.
#[test]
fn the_projects_services_are_lifted_as_nunkis_rendering() {
    let mut plan = plan(Role::Integrator);
    let project = services_file(
        "services:\n  db:\n    image: postgres:16\n    environment:\n      PGHOST: ${HOME}\n    \
         healthcheck:\n      test: [\"CMD\", \"pg_isready\"]\n",
    );
    let rendered: serde_yaml_ng::Value = serde_yaml_ng::from_str(&project.render()).unwrap();
    let digest = project.digest();
    plan.project_services = Some(nunki::compose::services::Lifted::whole(project));
    let yaml = generate(&plan, &dialect()).unwrap();
    let mut doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    // nunki's own label says which rendering the container came from …
    assert_eq!(
        doc["services"]["db"]["labels"][nunki::compose::services::LABEL].as_str(),
        Some(digest.as_str()),
        "{yaml}"
    );
    // … and, but for nunki's own keys, the service is the rendering read
    // back.
    let db = doc["services"]["db"].as_mapping_mut().unwrap();
    for key in FENCE_KEYS {
        db.remove(key);
    }
    assert_eq!(doc["services"]["db"], rendered["services"]["db"], "{yaml}");
    assert_eq!(doc["services"]["db"]["image"].as_str(), Some("postgres:16"));
    assert_eq!(
        doc["services"]["db"]["environment"]["PGHOST"].as_str(),
        Some("$${HOME}"),
        "Compose interpolates nothing: {yaml}"
    );
    assert_eq!(
        doc["services"]["db"]["healthcheck"]["test"][1].as_str(),
        Some("pg_isready")
    );
}

/// The closed model refuses these names; the generator refuses them again
/// on the document, so a service nunki writes is never overwritten by one a
/// model built some other way carries.
#[test]
fn a_project_service_may_not_take_a_name_hq_reserves() {
    for reserved in [FIREWALL_SERVICE, AGENT_SERVICE, PROBER_SERVICE] {
        let mut plan = plan(Role::Integrator);
        plan.prober = Some(Default::default());
        let mut project = nunki::compose::services::ServicesFile::default();
        project.services.insert(
            reserved.to_string(),
            nunki::compose::services::ProjectService {
                image: "nginx".to_string(),
                ..Default::default()
            },
        );
        plan.project_services = Some(nunki::compose::services::Lifted::whole(project));
        assert!(
            matches!(generate(&plan, &dialect()).unwrap_err(), ComposeError::ReservedService(name) if name == reserved),
            "{reserved} should be refused"
        );
    }
}

/// nunki's prober has a field of its own and lands beside the firewall and
/// the agent, with what it holds and the project's model would refuse.
#[test]
fn the_prober_travels_apart_from_the_projects_services() {
    let mut plan = plan(Role::Coder);
    plan.prober = Some(nunki::compose::model::Service {
        image: "img/probe".to_string(),
        network_mode: Some("service:firewall".to_string()),
        ..Default::default()
    });
    let yaml = generate(&plan, &dialect()).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(
        doc["services"][PROBER_SERVICE]["network_mode"].as_str(),
        Some("service:firewall"),
        "{yaml}"
    );
    assert!(
        generate(&self::plan(Role::Coder), &dialect())
            .map(|y| !y.contains(PROBER_SERVICE))
            .unwrap()
    );
}

#[test]
fn the_project_name_is_stable_across_profiles_and_legal_for_compose() {
    // Same slot, three profiles, one project name: this is what keeps the
    // project's services up between profiles (measured).
    let names: Vec<_> = [Role::Coder, Role::Integrator, Role::Security]
        .iter()
        .map(|role| {
            let yaml = generate(&plan(*role), &dialect()).unwrap();
            let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
            doc["name"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(names, vec!["nunki-11111111-demo-1"; 3]);

    // Compose refuses anything else: "must consist only of lowercase
    // alphanumeric characters, hyphens, and underscores as well as start
    // with a letter or number" — measured.
    assert_eq!(
        project_name(SESSION, "Feat/Mission Resume").unwrap(),
        "nunki-11111111-feat-mission-resume"
    );
    assert_eq!(project_name(SESSION, "_1").unwrap(), "nunki-11111111-1");
    assert!(matches!(
        project_name(SESSION, "///"),
        Err(ComposeError::SlotName(_))
    ));
}

/// Two projects, two slots of the same name, two Compose projects.
///
/// If two projects each have a slot called `one` and both become the Compose
/// project `nunki-one`, they share one set of containers, one network, and
/// one harness volume holding both projects' sessions. Starting a mission on
/// either recreates the other's containers under a running agent, and each
/// HQ's own `locks/one` says the slot is free.
#[test]
fn a_slot_of_the_same_name_in_another_session_is_another_compose_project() {
    let one = project_name("b885dda8-689d-4f98-a5f0-093d68f58f6d", "one").unwrap();
    let other = project_name("127b7deb-e2c6-42be-90eb-968793abaade", "one").unwrap();
    assert_ne!(one, other);
    assert_eq!(one, "nunki-b885dda8-one");
    assert_eq!(other, "nunki-127b7deb-one");

    // The session comes first, so `docker ps` groups a project's containers
    // together — the reading that was impossible before.
    assert!(one.starts_with("nunki-b885dda8-"), "{one}");

    // And the same session with two slots is still two projects.
    assert_ne!(
        project_name("b885dda8-689d-4f98-a5f0-093d68f58f6d", "two").unwrap(),
        one
    );
}

#[test]
fn a_relative_host_path_is_refused_before_the_engine_sees_it() {
    let mut plan = plan(Role::Coder);
    plan.tree = PathBuf::from("relative/tree");
    assert!(matches!(
        generate(&plan, &dialect()).unwrap_err(),
        ComposeError::RelativePath { .. }
    ));
}

#[test]
fn an_empty_allowlist_is_refused() {
    let mut plan = plan(Role::Coder);
    plan.perimeter = Default::default();
    assert!(matches!(
        generate(&plan, &dialect()).unwrap_err(),
        ComposeError::EmptyPerimeter
    ));
}

/// Everything above proves what `nunki` writes; this one proves the engine
/// accepts it. Skipped, not failed, on a machine without a container engine
/// (SPEC 4.2 bis, CI on two platforms).
#[test]
fn a_real_compose_accepts_every_generated_profile() {
    let Some(engine) = engine() else {
        eprintln!("skipped: no `docker compose` on this machine");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for role in [Role::Coder, Role::Integrator, Role::Security] {
        let mut plan = plan(role);
        if role != Role::Coder {
            plan.project_services = Some(nunki::compose::services::Lifted::whole(services_file(
                "services:\n  db:\n    image: postgres:16\n",
            )));
        }
        let file = dir.path().join(format!("{role:?}.yml"));
        std::fs::write(&file, generate(&plan, &dialect()).unwrap()).unwrap();

        let out = engine.config(&file);
        assert!(
            out.status.success(),
            "{role:?} rejected by the engine:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let normalised = String::from_utf8_lossy(&out.stdout);
        // `no` must survive as a string, not become the boolean it is in
        // YAML 1.1.
        assert!(normalised.contains(r#"restart: "no""#), "{normalised}");
    }
}

/// And the shape the generator refuses to write is one the engine refuses
/// too — the reason the agent carries no `networks:`.
#[test]
fn a_real_compose_refuses_an_agent_that_declares_both() {
    let Some(engine) = engine() else {
        eprintln!("skipped: no `docker compose` on this machine");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("both.yml");
    std::fs::write(
        &file,
        "services:\n  firewall:\n    image: alpine\n  agent:\n    image: alpine\n    network_mode: \"service:firewall\"\n    networks: [db]\nnetworks:\n  db:\n",
    )
    .unwrap();
    let out = engine.config(&file);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("mutually exclusive"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Engine;

impl Engine {
    fn config(&self, file: &Path) -> std::process::Output {
        let (program, leading) = compose_command();
        std::process::Command::new(program)
            .args(leading)
            .args(["-f"])
            .arg(file)
            .arg("config")
            .output()
            .expect("the compose command is on the path")
    }
}

/// `docker compose` by default; `HQ_COMPOSE` overrides it, so the same
/// checks can be run against another version of the engine.
fn compose_command() -> (String, Vec<String>) {
    let raw = std::env::var("HQ_COMPOSE").unwrap_or_else(|_| "docker compose".to_string());
    let mut words = raw.split_whitespace().map(str::to_string);
    let program = words.next().expect("HQ_COMPOSE must name a program");
    (program, words.collect())
}

fn engine() -> Option<Engine> {
    let (program, leading) = compose_command();
    let ok = std::process::Command::new(program)
        .args(leading)
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    ok.then_some(Engine)
}

#[test]
fn a_project_declaring_no_network_gets_no_networks_block() {
    // Measured: services and firewall both land on the generated default
    // network and reach each other there, so nunki invents nothing.
    let yaml = generate(&plan(Role::Integrator), &dialect()).unwrap();
    assert!(!yaml.contains("\nnetworks:"), "{yaml}");
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    assert!(doc["services"][FIREWALL_SERVICE]["networks"].is_null());
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

/// A project's own volumes are declared beside nunki's. Measured on
/// Compose v5.1.2: without them, a service naming a volume the
/// document does not declare makes the whole project invalid — `service "db"
/// refers to undefined volume dbdata` — and they are exactly where a
/// project keeps the state a profile switch must not take with it.
#[test]
fn the_projects_volumes_are_declared_beside_hqs() {
    let mut plan = plan(Role::Integrator);
    plan.project_services = Some(nunki::compose::services::Lifted::whole(services_file(
        "services:\n  db:\n    image: postgres:16\n    volumes:\n      - dbdata:/data\n\
         volumes:\n  dbdata:\n",
    )));

    let yaml = generate(&plan, &dialect()).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let volumes = doc["volumes"].as_mapping().expect("volumes are declared");
    assert!(
        volumes.contains_key(serde_yaml_ng::Value::from("dbdata")),
        "{yaml}"
    );
    assert_eq!(
        doc["services"]["db"]["volumes"][0].as_str(),
        Some("dbdata:/data")
    );
    // And nunki's own are still there: the two sets are merged, not replaced.
    assert!(
        volumes.contains_key(serde_yaml_ng::Value::from("nunki-demo-1-harness")),
        "{yaml}"
    );
}

/// A project that names one of the slot's own volumes would be handed the
/// build cache or the harness's sessions. The closed model refuses the
/// prefix; the generator refuses the collision again on the document, so a
/// model built some other way cannot overwrite one either.
#[test]
fn a_project_volume_may_not_take_a_slots_own_name() {
    let mut mounted = plan(Role::Integrator);
    let mut project = nunki::compose::services::ServicesFile::default();
    project.services.insert(
        "db".to_string(),
        nunki::compose::services::ProjectService {
            image: "postgres:16".to_string(),
            ..Default::default()
        },
    );
    project.volumes.insert("nunki-demo-1-harness".to_string());
    mounted.project_services = Some(nunki::compose::services::Lifted::whole(project));
    assert!(matches!(
        generate(&mounted, &dialect()),
        Err(ComposeError::ReservedVolume(name)) if name == "nunki-demo-1-harness"
    ));
}

/// The keys nunki writes on every lifted service, after the rendering.
const FENCE_KEYS: [&str; 7] = [
    "labels",
    "networks",
    "cap_drop",
    "cap_add",
    "security_opt",
    "mem_limit",
    "pids_limit",
];

/// Every lifted service is fenced by nunki, whatever the file says: its
/// own internal network, with no gateway on the host, and no other, every
/// capability dropped and a fixed few given back, no new privilege, a memory
/// and a process bound — and no published port. The firewall joins that
/// network beside the default one, which is how the agent, in its namespace,
/// still reaches the service.
#[test]
fn the_lifted_services_are_fenced_by_nunki() {
    use nunki::compose::services::{Lifted, MEMORY, NETWORK, NO_GATEWAY, PROCESSES};
    let mut plan = plan(Role::Integrator);
    plan.project_services = Some(Lifted::whole(services_file(
        "services:\n  db:\n    image: postgres:16\n    ports: [\"5432:5432\"]\n  \
         cache:\n    image: redis:7\n    ports: [\"127.0.0.1::6379\"]\n",
    )));
    let yaml = generate(&plan, &dialect()).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let strings = |v: &serde_yaml_ng::Value| -> Vec<String> {
        v.as_sequence()
            .unwrap_or_else(|| panic!("a list: {v:?}\n{yaml}"))
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    };
    for name in ["db", "cache"] {
        let service = &doc["services"][name];
        assert_eq!(strings(&service["networks"]), vec![NETWORK], "{yaml}");
        assert_eq!(strings(&service["cap_drop"]), vec!["ALL"], "{yaml}");
        assert_eq!(
            strings(&service["cap_add"]),
            // The set the HQ measured (Docker Engine 29.4.0): each one needed
            // by at least one of the images in real use. Named here and not
            // read from the constant, so a change to the set is seen.
            vec!["DAC_OVERRIDE", "SETGID", "SETUID"],
            "{yaml}"
        );
        assert_eq!(
            strings(&service["security_opt"]),
            vec!["no-new-privileges:true"],
            "{yaml}"
        );
        assert_eq!(service["mem_limit"].as_str(), Some(MEMORY), "{yaml}");
        assert_eq!(
            service["pids_limit"].as_u64(),
            Some(u64::from(PROCESSES)),
            "{yaml}"
        );
        assert!(service["ports"].is_null(), "no port is published: {yaml}");
        assert!(service["network_mode"].is_null(), "{yaml}");
    }
    assert!(!yaml.contains("5432") && !yaml.contains("6379"), "{yaml}");
    assert_eq!(
        doc["networks"][NETWORK]["internal"].as_bool(),
        Some(true),
        "{yaml}"
    );
    assert_eq!(
        doc["networks"][NETWORK]["driver_opts"][NO_GATEWAY.0].as_str(),
        Some(NO_GATEWAY.1),
        "the host has no address on the services' network: {yaml}"
    );
    assert_eq!(
        doc["networks"][NETWORK]["enable_ipv6"].as_bool(),
        Some(false),
        "no IPv6 on the services' network, whatever the daemon's default: {yaml}"
    );
    assert_eq!(doc["networks"].as_mapping().unwrap().len(), 1, "{yaml}");
    assert_eq!(
        strings(&doc["services"][FIREWALL_SERVICE]["networks"]),
        vec!["default", NETWORK],
        "the firewall keeps its way out and joins the services: {yaml}"
    );
    assert!(
        doc["services"][AGENT_SERVICE]["networks"].is_null(),
        "the agent has the firewall's namespace and no network of its own"
    );
}

/// No service lifted, no network: a profile without the project's services
/// is the profile it always was.
#[test]
fn a_profile_lifting_no_service_has_no_services_network() {
    use nunki::compose::services::{Lifted, ServicesFile};
    for project in [None, Some(Lifted::whole(ServicesFile::default()))] {
        let mut plan = plan(Role::Integrator);
        plan.project_services = project;
        let yaml = generate(&plan, &dialect()).unwrap();
        let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
        assert!(doc["networks"].is_null(), "{yaml}");
        assert!(
            doc["services"][FIREWALL_SERVICE]["networks"].is_null(),
            "{yaml}"
        );
    }
}
