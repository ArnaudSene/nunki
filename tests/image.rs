//! hq's own image layer (SPEC 4.1, 4.2): the harness, and the mount points
//! hq needs that a stack fragment has no reason to know about.
//!
//! Building it is a live matter; what it *says* is not, and what it says is
//! where the defects have been.

use hq::harness::Provisioning;
use hq::image;

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
    let layer = image::harness_layer("hq/demo:base", &provisioning(), &[]);
    assert!(layer.contains("FROM hq/demo:base"), "{layer}");
    // A named volume mounted over a root-owned directory is born root-owned,
    // and nothing in the container can fix that afterwards (SPEC 4.2 bis).
    // The base image ends on `USER agent`, so this layer must not take root
    // back before creating the directory.
    assert!(
        layer.contains(&format!("RUN mkdir -p {}", hq::exec::PROOF_AT)),
        "the copy of HEAD has nowhere to live: {layer}"
    );
    assert!(
        !layer.contains("USER root"),
        "root would own the copy, and the agent could not write in it: {layer}"
    );
}

#[test]
fn the_layer_fails_the_build_when_the_harness_is_not_on_path() {
    let layer = image::harness_layer("hq/demo:base", &provisioning(), &[]);
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

/// A stack image may ship its own harness — pinned, or built for a base the
/// installer does not support. Reinstalling over it wastes minutes when it
/// works, and fails the build when it does not: measured on an image that
/// carried `claude` and no `curl`.
#[test]
fn the_install_is_skipped_when_the_image_already_carries_the_harness() {
    let layer = image::harness_layer("hq/demo:base", &provisioning(), &[]);
    assert!(
        layer.contains("RUN command -v claude > /dev/null || (curl"),
        "{layer}"
    );
    // And the verification afterwards is not conditional: whichever way the
    // binary got there, the build fails if it is not on PATH.
    assert!(
        layer.contains("RUN command -v claude > /dev/null || (echo"),
        "{layer}"
    );
}

#[test]
fn a_harness_that_needs_nothing_still_gets_the_mount_points() {
    let layer = image::harness_layer("hq/demo:base", &Provisioning::default(), &[]);
    assert!(layer.contains("RUN mkdir -p"), "{layer}");
    assert!(
        !layer.contains("command -v "),
        "nothing to check for: {layer}"
    );
}

/// The directories a read-only tree still has to write are created **in the
/// image**, because that is where a named volume takes its ownership from:
/// measured 2026-09-10, a volume mounted at a path the image does not carry
/// is born owned by root, and a container with no capability cannot repair
/// it. Depth makes no difference — `packages/web/node_modules` behaves as
/// `target` does.
#[test]
fn the_layer_creates_the_directories_a_read_only_tree_must_write() {
    let layer = image::harness_layer(
        "hq/demo:base",
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
    let layer = image::harness_layer("hq/demo:base", &provisioning(), &[]);
    assert!(!layer.contains("/work/tree/"), "{layer}");
}
