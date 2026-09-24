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
/// Measured on `qcoda-compta`, 2026-09-24: a mission pinned to a model the
/// image's harness did not know came back `API Error: 400 Claude Code
/// 2.1.278 does not support this model; version 2.1.280 or newer is
/// required`. Two patch versions, and a held mission, for an image built
/// weeks earlier.
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
/// measured 2026-09-10, a volume mounted at a path the image does not carry
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
