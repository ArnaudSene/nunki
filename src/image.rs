//! Building the two images a profile needs (SPEC 4.1, 4.2).
//!
//! One belongs to the project — the stack's `Dockerfile`, which `nunki init`
//! wrote and the project may edit. The other belongs to `nunki` and travels in
//! the binary: the firewall sidecar (see [`crate::firewall`]).
//!
//! Both are built with the human's uid and gid. That is not a nicety: a file
//! written under another id is unreadable on the host, and git refuses a tree
//! it does not own (SPEC 4.2 bis).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::Project;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Images {
    pub agent: String,
    pub firewall: String,
    /// The container `nunki check` probes from — never the agent's, which has no
    /// reason to carry `nslookup` (see `assets/prober/Dockerfile`).
    pub prober: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("no Dockerfile for stack {stack:?} at {at} — `nunki init --stack {stack}` writes one")]
    NoDockerfile { stack: String, at: PathBuf },
    #[error("building {image} failed:\n{stderr}")]
    Build { image: String, stderr: String },
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("the container engine is not on this machine: {0}")]
    NoEngine(String),
}

/// The uid and gid the images and containers run under: the human's own.
pub fn host_ids() -> (u32, u32) {
    // SAFETY: both are always-succeeding libc calls with no arguments.
    unsafe { (libc::getuid(), libc::getgid()) }
}

/// The image the stack alone builds, before the harness is added. Kept as
/// its own tag so a harness change rebuilds one thin layer and not a Rust
/// toolchain.
pub fn base_tag(project: &Project, stack: &str) -> String {
    format!("nunki/{}-{stack}:base", project.name().to_lowercase())
}

/// What the images are called for a given slot. Named per project and stack,
/// not per slot: two slots of the same project share an image, and rebuilding
/// one rebuilds for both — which is what makes `nunki slot rebuild` cheap.
pub fn names(project: &Project, stack: &str) -> Images {
    Images {
        agent: format!("nunki/{}-{stack}:latest", project.name().to_lowercase()),
        firewall: format!("nunki/firewall:{}", env!("CARGO_PKG_VERSION")),
        prober: format!("nunki/prober:{}", env!("CARGO_PKG_VERSION")),
    }
}

/// Build both images. `engine` is the binary that builds — `docker` unless
/// `HQ_ENGINE` says otherwise.
pub fn build(project: &Project, stack: &str, engine: &str) -> Result<Images, ImageError> {
    let images = names(project, stack);
    let (uid, gid) = host_ids();

    // The sidecar first: the agent's image is worthless without something to
    // fence it in.
    let context = tempfile::tempdir().map_err(|e| ImageError::Io(PathBuf::from("."), e))?;
    crate::firewall::materialise(context.path())
        .map_err(|e| ImageError::Io(context.path().to_path_buf(), e))?;
    docker_build(engine, context.path(), &images.firewall, &[])?;

    let prober = tempfile::tempdir().map_err(|e| ImageError::Io(PathBuf::from("."), e))?;
    crate::firewall::materialise_prober(prober.path())
        .map_err(|e| ImageError::Io(prober.path().to_path_buf(), e))?;
    docker_build(engine, prober.path(), &images.prober, &[])?;

    let fragment = project.fragment(stack);
    let dockerfile = fragment.join("Dockerfile");
    if !dockerfile.is_file() {
        return Err(ImageError::NoDockerfile {
            stack: stack.to_string(),
            at: dockerfile,
        });
    }
    let base = base_tag(project, stack);
    docker_build(
        engine,
        &fragment,
        &base,
        &[format!("UID={uid}"), format!("GID={gid}")],
    )?;

    // Then the harness, in a layer of nunki's own. The stack fragment describes
    // a stack; which harness runs on it is not the project's business, and
    // changing harness must not mean editing every project's Dockerfile.
    let provisioning = harness_provisioning(&project.config.harness);
    let layer = tempfile::tempdir().map_err(|e| ImageError::Io(PathBuf::from("."), e))?;
    std::fs::write(
        layer.path().join("Dockerfile"),
        harness_layer(&base, &provisioning, &project.stack_writable(stack)),
    )
    .map_err(|e| ImageError::Io(layer.path().to_path_buf(), e))?;
    // A value that differs at every build, so the harness layer is never
    // served from cache and the install above fetches the current version.
    // The `ARG` it feeds is read by a `RUN` that does nothing else: Docker
    // invalidates from there down, which is exactly the install and what
    // follows it.
    docker_build(
        engine,
        layer.path(),
        &images.agent,
        &[format!(
            "NUNKI_HARNESS_BUILD={}",
            crate::state::now_rfc3339()
        )],
    )?;

    Ok(images)
}

/// Whether an image is already on this machine, so a caller can say what is
/// missing instead of building for minutes.
pub fn present(engine: &str, image: &str) -> bool {
    Command::new(engine)
        .args(["image", "inspect", image])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The Dockerfile of nunki's own layer, on top of a stack's image: the harness,
/// and the mount points nunki needs that a stack fragment has no reason to know
/// about.
pub fn harness_layer(
    base: &str,
    provisioning: &crate::harness::Provisioning,
    writable: &[String],
) -> String {
    let mut out = format!(
        "# Generated by nunki: the harness, on top of the stack's own image.\n\
         # Do not edit — `nunki slot rebuild` writes it again.\n\
         FROM {base}\n"
    );
    // nunki's own mount point, created as the agent so that the named volume
    // Compose puts here is born owned by the only user that writes in it: a
    // volume mounted over a root-owned directory is root-owned, and nothing
    // in the container can fix that afterwards (SPEC 4.2 bis). Measured on
    // this image.
    out.push_str(&format!("RUN mkdir -p {}\n", crate::exec::PROOF_AT));
    // The directories a read-only tree still has to write, from the stack's
    // `writable.txt` (SPEC 4.2, rule 3). They are created **here**, in the
    // image, because that is where a named volume takes its ownership from:
    // measured on 2026-09-10, a volume mounted at a path the image does not
    // carry is born owned by root, and nothing in a container with no
    // capability can repair that afterwards. Depth makes no difference —
    // `packages/web/node_modules` behaves as `target` does — and the bind
    // that hides these directories at run time does not hide them from the
    // volume's initialisation.
    for path in writable {
        out.push_str(&format!("RUN mkdir -p {}/{path}\n", crate::run::TREE_AT));
    }
    if !provisioning.path.is_empty() {
        out.push_str(&format!(
            "ENV PATH={}:${{PATH}}\n",
            provisioning.path.join(":")
        ));
    }
    // Before anything is installed: a stack image that ends as root is a
    // stack image the rest of this layer cannot serve. The harness keeps its
    // state under the home of whoever installs it, so installed as root it
    // lands in root's home and the only user that runs it cannot read it —
    // and the failure surfaced two steps later as "claude is not on PATH
    // after the install", which sends the reader to the installer instead of
    // to the missing `USER`. A check that names the wrong cause is a check
    // that lies (SPEC 4.2 bis).
    if !provisioning.install.is_empty() {
        out.push_str(
            "RUN [ \"$(id -u)\" != 0 ] || (echo \"nunki: this stack's image ends as root; \
             its Dockerfile must end with USER set to the agent, so that what the agent \
             writes belongs to the human on the host (SPEC 4.2 bis)\" >&2; exit 1)\n",
        );
    }
    if !provisioning.install.is_empty() {
        // The harness is installed at **every** build, and the argument above
        // it is what makes Docker believe that.
        //
        // It used to be `command -v <binary> || (install)`, on the reasoning
        // that a stack image may ship its own harness and reinstalling over
        // it wastes minutes. The reasoning was sound and the consequence was
        // not: the layer cached, so the version in an image never moved
        // again, and nothing said which version it was or that it was frozen.
        //
        // Measured on `qcoda-compta`, 2026-09-24. A mission was pinned to a
        // model the harness in its image did not know, and the run came back
        // `API Error: 400 Claude Code 2.1.278 does not support this model;
        // version 2.1.280 or newer is required`. Two patch versions, and a
        // held mission, for an image built weeks earlier.
        //
        // Installing at build time rather than at launch is deliberate: a
        // build has ordinary network, while a run sits behind the sidecar
        // whose allowlist is the stack's needs plus the harness's and
        // nothing else (SPEC 4.1 bis, rule 6). Updating from inside a run
        // would mean opening the installer's host for the whole of every
        // run, to serve a need that lasts a second.
        out.push_str(
            "ARG NUNKI_HARNESS_BUILD\n\
             RUN echo \"harness build ${NUNKI_HARNESS_BUILD:-unset}\" > /dev/null\n",
        );
    }
    for step in &provisioning.install {
        // As the agent, not as root: the harness keeps its state under the
        // agent's home, and installing it as root would leave it unreadable
        // by the only user that runs it.
        out.push_str(&format!("RUN {step}\n"));
    }
    if !provisioning.binary.is_empty() {
        // Loudly, at build time. The first version of this layer produced an
        // image whose PATH pointed at nothing, and the failure surfaced much
        // later as `claude: not found` inside a run nobody was watching.
        out.push_str(&format!(
            "RUN command -v {0} > /dev/null || (echo \"nunki: {0} is not on PATH after \
             the install\" >&2; exit 1)\n",
            provisioning.binary
        ));
    }
    out
}

/// What a named harness needs on the container side. The one place a harness
/// name becomes an adapter; everything else in `nunki` only knows the name.
pub fn harness_provisioning(harness: &str) -> crate::harness::Provisioning {
    use crate::harness::Harness;
    match harness {
        "claude-code" => crate::harness::claude_code::ClaudeCode::new(
            Default::default(),
            Box::new(crate::harness::spawn::LocalSpawner),
        )
        .provision(),
        _ => crate::harness::Provisioning::default(),
    }
}

fn docker_build(
    engine: &str,
    context: &Path,
    tag: &str,
    build_args: &[String],
) -> Result<(), ImageError> {
    let mut command = Command::new(engine);
    command.args(["build", "-q", "-t", tag]);
    for arg in build_args {
        command.args(["--build-arg", arg]);
    }
    command.arg(context);
    let out = command
        .output()
        .map_err(|e| ImageError::NoEngine(e.to_string()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(ImageError::Build {
            image: tag.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}
