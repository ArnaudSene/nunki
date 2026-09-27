//! nunki's own image layer (SPEC 4.1, 4.2): the harness, and the mount points
//! nunki needs that a stack fragment has no reason to know about.
//!
//! Building it is a live matter; what it *says* is not, and what it says is
//! where the defects have been.

use nunki::harness::Provisioning;
use nunki::image;

fn provisioning() -> Provisioning {
    Provisioning {
        install: vec!["curl -fsSL https://example.invalid/install.sh | bash".into()],
        binary: "claude".into(),
        path: vec!["/home/agent/.local/bin".into()],
        config_dir: None,
        domains: vec!["api.anthropic.com".into()],
    }
}

#[test]
fn the_layer_creates_the_copy_of_head_as_the_agent_and_not_as_root() {
    let layer = image::harness_layer("nunki/demo:base", &provisioning(), &[]);
    assert!(layer.contains("FROM nunki/demo:base"), "{layer}");
    // A named volume mounted over a root-owned directory is born root-owned,
    // and nothing in the container can fix that afterwards (SPEC 4.2 bis).
    // The base image ends on `USER agent`, so this layer must not take root
    // back before creating the directory.
    assert!(
        layer.contains(&format!("RUN mkdir -p {}", nunki::exec::PROOF_AT)),
        "the copy of HEAD has nowhere to live: {layer}"
    );
    assert!(
        !layer.contains("USER root"),
        "root would own the copy, and the agent could not write in it: {layer}"
    );
}

#[test]
fn the_layer_fails_the_build_when_the_harness_is_not_on_path() {
    let layer = image::harness_layer("nunki/demo:base", &provisioning(), &[]);
    // Loudly, at build time. A layer whose PATH points at nothing once
    // shipped, and the failure surfaced much later as `claude: not found`
    // inside a run nobody was watching.
    assert!(layer.contains("command -v claude"), "{layer}");
    assert!(layer.contains("exit 1"), "{layer}");
    // The PATH is absolute: a Dockerfile's ENV does not expand $HOME, and
    // the first version of this shipped `PATH=/.local/bin`.
    assert!(
        layer.contains("ENV PATH=/home/agent/.local/bin:${PATH}"),
        "{layer}"
    );
    assert!(!layer.contains("$HOME"), "{layer}");
}

/// The harness is installed at **every** build, and the layer says so.
///
/// It used to be skipped when the image already carried the binary, on the
/// reasoning that a stack image may ship its own pinned harness. The
/// reasoning was sound; the consequence was not. The layer cached, so the
/// version in an image never moved again, and nothing said which version it
/// was or that it was frozen.
///
/// A mission pinned to a model the image's harness does not know comes back
/// `API Error: 400 Claude Code 2.1.278 does not support this model; version
/// 2.1.280 or newer is required`: two patch versions, and a held mission,
/// for an image built weeks earlier.
#[test]
fn the_harness_is_installed_at_every_build_and_never_served_from_cache() {
    let layer = image::harness_layer("nunki/demo:base", &provisioning(), &[]);

    // The install no longer asks whether the binary is already there — that
    // question froze every image that answered yes.
    assert!(
        !layer.contains("RUN command -v claude > /dev/null || (curl"),
        "the install must not be skipped because the binary happens to exist: {layer}"
    );
    // What it asks instead is whether the build **said** to keep what the
    // image carries. Declared, and defaulting to installing.
    assert!(
        layer.contains("ARG NUNKI_HARNESS_KEEP"),
        "keeping a substitute harness has to be sayable: {layer}"
    );
    assert!(
        layer.contains("RUN [ -n \"${NUNKI_HARNESS_KEEP:-}\" ] || (curl"),
        "{layer}"
    );

    // And the cache is broken above it, or "unconditional" is a word in a
    // Dockerfile that Docker never reads again.
    assert!(layer.contains("ARG NUNKI_HARNESS_BUILD"), "{layer}");
    let arg = layer.find("ARG NUNKI_HARNESS_BUILD").unwrap();
    let install = layer.find("|| (curl").unwrap();
    assert!(
        arg < install,
        "the argument has to come before the install it invalidates: {layer}"
    );

    // What feeds it is `image::build`, with `state::now_rfc3339()` -- a value
    // that differs at every build, which is what makes Docker invalidate from
    // this line down. Asserting that here would mean sleeping a second to
    // watch a clock move, and a test that waits is a test that waits for
    // ever, on every machine.

    // The verification afterwards stays conditional in shape and absolute in
    // effect: whichever way the binary got there, the build fails without it.
    assert!(
        layer.contains("RUN command -v claude > /dev/null || (echo"),
        "{layer}"
    );
}

#[test]
fn a_harness_that_needs_nothing_still_gets_the_mount_points() {
    let layer = image::harness_layer("nunki/demo:base", &Provisioning::default(), &[]);
    assert!(layer.contains("RUN mkdir -p"), "{layer}");
    assert!(
        !layer.contains("command -v "),
        "nothing to check for: {layer}"
    );
}

/// The directories a read-only tree still has to write are created **in the
/// image**, because that is where a named volume takes its ownership from:
/// measured, a volume mounted at a path the image does not carry
/// is born owned by root, and a container with no capability cannot repair
/// it. Depth makes no difference — `packages/web/node_modules` behaves as
/// `target` does.
#[test]
fn the_layer_creates_the_directories_a_read_only_tree_must_write() {
    let layer = image::harness_layer(
        "nunki/demo:base",
        &provisioning(),
        &[
            "target".to_string(),
            "packages/web/node_modules".to_string(),
        ],
    );
    for path in ["target", "packages/web/node_modules"] {
        assert!(
            layer.contains(&format!("RUN mkdir -p /work/tree/{path}\n")),
            "{path} has nowhere to live: {layer}"
        );
    }
    // As the agent: the base image ends on `USER agent`, and this layer must
    // not take root back before creating them.
    assert!(!layer.contains("USER root"), "{layer}");
}

/// A stack that declares none gets none: what is not declared stays closed
/// (SPEC 4.2, rule 3).
#[test]
fn a_stack_that_declares_no_writable_directory_gets_none() {
    let layer = image::harness_layer("nunki/demo:base", &provisioning(), &[]);
    assert!(!layer.contains("/work/tree/"), "{layer}");
}

/// A stack image that ends as root is one this layer cannot serve: the
/// harness keeps its state under the home of whoever installs it, so
/// installed as root it lands in root's home and the only user that runs it
/// cannot read it. Measured on a fixture that had no `agent` user at all —
/// the failure surfaced two steps later as "claude is not on PATH after the
/// install", which sends the reader to the installer instead of to the
/// missing `USER`.
#[test]
fn the_layer_refuses_a_stack_image_that_ends_as_root_and_says_why() {
    let layer = image::harness_layer("nunki/demo:base", &provisioning(), &[]);
    let check = layer
        .find("id -u")
        .unwrap_or_else(|| panic!("nothing checks the user: {layer}"));
    let install = layer
        .find("install.sh")
        .unwrap_or_else(|| panic!("{layer}"));
    assert!(
        check < install,
        "the cause is named before the symptom can happen: {layer}"
    );
    assert!(layer.contains("SPEC 4.2 bis"), "{layer}");
    assert!(
        layer.contains("USER"),
        "it names what is missing, not what broke: {layer}"
    );
}

/// A harness that installs nothing has no home to land in, so there is
/// nothing for the user to be wrong about — and a check that cannot fail is
/// a check worth removing.
#[test]
fn a_harness_that_installs_nothing_is_not_asked_about_the_user() {
    let layer = image::harness_layer("nunki/demo:base", &Provisioning::default(), &[]);
    assert!(!layer.contains("id -u"), "{layer}");
}

/// A project whose rust fragment reads its toolchain from the repository.
fn pinning(dir: &std::path::Path, dockerfile: &str) -> nunki::project::Project {
    let root = dir.join("repo");
    let home = dir.join("home");
    let fragment = home.join("stacks/rust");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&fragment).unwrap();
    std::fs::write(
        fragment.join(nunki::versions::FILE),
        "RUST_VERSION rust-toolchain.toml toolchain.channel\n\
         RUST_TARGETS rust-toolchain.toml toolchain.targets\n",
    )
    .unwrap();
    std::fs::write(fragment.join("Dockerfile"), dockerfile).unwrap();
    std::fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.98.0\"\ntargets = [\"wasm32-unknown-unknown\"]\n",
    )
    .unwrap();
    let config = serde_yaml_ng::from_str("harness: claude-code\nstacks: [rust]\n").unwrap();
    nunki::project::Project::at(root, config, home)
}

const DECLARING: &str = "ARG RUST_VERSION=stable\nARG RUST_TARGETS=\nFROM debian\n";

/// What reaches the engine: the repository's toolchain as build arguments,
/// and the same values on the label a launch reads back.
#[test]
fn the_image_is_built_with_what_the_repository_pins_and_says_so_on_its_label() {
    let dir = tempfile::tempdir().unwrap();
    let project = pinning(dir.path(), DECLARING);
    let build = image::stack_build(&project, "rust", 501, 20).unwrap();
    assert_eq!(
        build.args,
        [
            "UID=501",
            "GID=20",
            "RUST_TARGETS=wasm32-unknown-unknown",
            "RUST_VERSION=1.98.0"
        ]
    );
    assert_eq!(
        build.label,
        "nunki.versions=RUST_TARGETS=wasm32-unknown-unknown;RUST_VERSION=1.98.0"
    );
}

/// An argument the Dockerfile does not declare would be dropped by the
/// engine, and the image built on its default as if nothing were wrong.
#[test]
fn a_build_whose_dockerfile_would_drop_a_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let project = pinning(dir.path(), "ARG RUST_VERSION=stable\nFROM debian\n");
    let err = image::stack_build(&project, "rust", 501, 20).unwrap_err();
    assert!(matches!(err, image::ImageError::Undeclared { .. }), "{err}");
    assert!(err.to_string().contains("RUST_TARGETS"), "{err}");
}

/// A stand-in engine whose image carries `label`.
fn engine_with(dir: &std::path::Path, label: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("engine");
    std::fs::write(
        &bin,
        format!("#!/bin/sh\necho '{{\"nunki.versions\":\"{label}\"}}'\n"),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin.to_string_lossy().into_owned()
}

/// A launch reads what the **slot's** tree pins, on the mission's branch —
/// not the repository as it stood at the last rebuild. The image was built
/// for 1.98.0; the branch moved to 1.99.0, and no agent may start on it.
#[test]
fn a_launch_refuses_an_image_the_branch_has_moved_away_from() {
    let dir = tempfile::tempdir().unwrap();
    let project = pinning(dir.path(), DECLARING);
    let engine = engine_with(
        dir.path(),
        "RUST_TARGETS=wasm32-unknown-unknown;RUST_VERSION=1.98.0",
    );
    let tree = dir.path().join("slot");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::copy(
        project.root.join("rust-toolchain.toml"),
        tree.join("rust-toolchain.toml"),
    )
    .unwrap();

    nunki::run::image_serves(&project, &tree, "rust", &engine, "nunki/x-rust:latest")
        .expect("the same toolchain serves");

    std::fs::write(
        tree.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.99.0\"\ntargets = [\"wasm32-unknown-unknown\"]\n",
    )
    .unwrap();
    let err = nunki::run::image_serves(&project, &tree, "rust", &engine, "nunki/x-rust:latest")
        .unwrap_err();
    match &err {
        nunki::run::RunError::StaleImage { drift, .. } => {
            assert_eq!(drift.len(), 1, "{drift:?}");
            assert!(drift[0].contains("\"1.98.0\"") && drift[0].contains("\"1.99.0\""));
        }
        other => panic!("{other}"),
    }
    assert!(err.to_string().contains("nunki slot rebuild"), "{err}");
}
