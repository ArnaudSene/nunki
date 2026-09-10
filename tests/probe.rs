//! `hq check --slot` (SPEC 4.1 bis, rule 7): the half of the verb that can
//! only be answered by lifting containers and trying to get out.

use std::path::Path;

use hq::image;
use hq::project::{Config, Project, ProtectedPaths};

fn project(dir: &Path) -> Project {
    Project::at(
        dir.join("repo"),
        Config {
            harness: "claude-code".to_string(),
            forge: vec!["github.com".to_string()],
            stacks: vec!["rust".to_string()],
            protected_branches: vec!["main".to_string()],
            protected_paths: ProtectedPaths::default(),
            account: None,
            bounds: Default::default(),
            credentials: None,
            run: None,
        },
        dir.join("hq"),
    )
}

#[test]
fn images_are_named_per_project_and_stack_not_per_slot() {
    let dir = tempfile::tempdir().unwrap();
    let images = image::names(&project(dir.path()), "rust");
    // Two slots of the same project share an image: rebuilding once serves
    // both, which is what makes `hq slot rebuild` bearable.
    assert_eq!(images.agent, "hq/repo-rust:latest");
    assert!(images.firewall.starts_with("hq/firewall:"));
    assert!(images.prober.starts_with("hq/prober:"));
    assert_ne!(images.firewall, images.prober);
}

#[test]
fn the_images_run_under_the_humans_own_ids() {
    let (uid, gid) = image::host_ids();
    // Not root, and not a guess: a file written under another id is
    // unreadable on the host and git refuses the tree (SPEC 4.2 bis).
    assert_ne!(uid, 0, "hq is not meant to be run as root");
    assert_eq!(uid, unsafe { libc::getuid() });
    assert_eq!(gid, unsafe { libc::getgid() });
}

#[test]
fn the_prober_carries_its_reason_with_it() {
    let dir = tempfile::tempdir().unwrap();
    hq::firewall::materialise_prober(dir.path()).unwrap();
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
    let hq_root = dir.path().join("hq");
    std::fs::create_dir_all(&root).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );

    hq::init::init(&root, &hq_root, &["rust".to_string()]).unwrap();
    std::fs::write(
        root.join(".hq/stacks/rust/Dockerfile"),
        // Debian on purpose: it carries no `nslookup`, no `nc`, no `wget`.
        // If the battery ran in the agent's container instead of the
        // prober's, every positive probe would fail here — which is exactly
        // what happened on nunki before the prober existed.
        "FROM debian:bookworm-slim\nRUN mkdir -p /work/tree /work/mission /run/hq\n",
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

    let project = Project::open(&root).map(|p| Project::at(p.root, p.config, hq_root.clone()));
    let project = project.expect("init wrote a config Project::open accepts");
    let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".to_string());

    image::build(&project, "rust", &engine_bin).expect("the images build");
    let slot = hq::slot::add(&project, "probe").expect("the slot is cloned");

    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::docker::Docker::real());
    let checks = hq::probe::mission_profile(&project, &slot, "rust", engine, &engine_bin)
        .expect("the profile lifts");

    let mut red = Vec::new();
    for check in &checks {
        let mark = match &check.verdict {
            hq::check::Verdict::Green(d) => format!("ok   {d}"),
            hq::check::Verdict::Red(d) => {
                red.push(check.what.clone());
                format!("RED  {d}")
            }
            hq::check::Verdict::NotChecked(d) => format!("--   {d}"),
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
            "name=hq-probe-check",
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

    let _ = hq::slot::rm(&project, "probe", true);
}

#[test]
fn a_probe_run_has_a_compose_project_of_its_own() {
    // A slot's services are levied once and kept between profiles; probing
    // under the slot's own name would take them down at the end of the check.
    let checking = hq::probe::compose_project("lot2").unwrap();
    let running = hq::compose::project_name("lot2").unwrap();
    assert_ne!(checking, running);
    assert!(checking.contains("check"), "{checking}");
}

#[test]
fn a_missing_image_is_something_to_do_not_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = hq::slot::Slot {
        name: "absent".to_string(),
        tree: dir.path().join("tree"),
    };
    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::fake::FakeEngine::default());

    // No image of that name was ever built, so the profile cannot be lifted.
    // `hq check` turns this into "not checked, here is what to run" rather
    // than a violation — the perimeter is not broken, it is unbuilt.
    let err =
        hq::probe::mission_profile(&project, &slot, "never-built", engine, "docker").unwrap_err();
    assert!(matches!(err, hq::probe::ProbeError::NoImages(..)), "{err}");
    assert!(err.to_string().contains("hq slot rebuild"), "{err}");
}
