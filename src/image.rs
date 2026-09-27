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

use crate::project::{Project, Stack};

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
    #[error(transparent)]
    Versions(#[from] crate::versions::SourcesError),
    #[error(
        "{dockerfile} declares no {args}, which `versions.txt` beside it reads from the \
         repository: the engine would drop the value and keep the default. The fragment \
         predates it — remove the Dockerfile, run `nunki init --stack {stack}` to write \
         the current one, and carry your own edits over"
    )]
    Undeclared {
        stack: String,
        args: String,
        dockerfile: PathBuf,
    },
    #[error(
        "stack {stack} is added onto {primary}'s image, and its fragment has no {at} — \
         `nunki init --stack {stack}` writes it"
    )]
    NoAddon {
        stack: String,
        primary: String,
        at: PathBuf,
    },
    #[error(
        "the image is the project's, built from {primary} with the other stacks added onto \
         it; {asked} is not its primary stack — drop `--stack`, or name {primary}"
    )]
    NotPrimary { asked: String, primary: String },
    #[error(
        "{arg} is read from the repository by both {first} and {second}: one value would \
         silently win for both. Rename it in one fragment's Dockerfile and `versions.txt`"
    )]
    SharedArgument {
        arg: String,
        first: String,
        second: String,
    },
    #[error("{engine} could not say what {image} was built with: {said}")]
    Inspect {
        engine: String,
        image: String,
        said: String,
    },
}

/// What a build produced, and the versions it read from the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub images: Images,
    pub versions: Vec<crate::versions::Pin>,
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
pub fn build(
    project: &Project,
    stack: &str,
    engine: &str,
    harness: Harness,
) -> Result<Built, ImageError> {
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

    let stacks = image_stacks(project, stack)?;
    let StackBuild {
        args,
        label,
        versions,
    } = stack_build(project, stack, uid, gid)?;
    let base = base_tag(project, stack);
    // One stack builds from its fragment, as it always has. Several build
    // from a Dockerfile composed of the primary's and the others' add-ons,
    // written where nothing else is — the primary's fragment is the project's
    // and nunki writes nothing into it after `init`.
    let composed = tempfile::tempdir().map_err(|e| ImageError::Io(PathBuf::from("."), e))?;
    let context = if stacks.len() == 1 {
        project.fragment(stack)
    } else {
        let text = compose_stacks(project, &stacks)?;
        let file = composed.path().join("Dockerfile");
        std::fs::write(&file, text).map_err(|e| ImageError::Io(file.clone(), e))?;
        composed.path().to_path_buf()
    };
    docker_build_labelled(engine, &context, &base, &args, &[label])?;

    // Then the harness, in a layer of nunki's own. The stack fragment describes
    // a stack; which harness runs on it is not the project's business, and
    // changing harness must not mean editing every project's Dockerfile.
    let provisioning = harness_provisioning(&project.config.harness);
    let layer = tempfile::tempdir().map_err(|e| ImageError::Io(PathBuf::from("."), e))?;
    std::fs::write(
        layer.path().join("Dockerfile"),
        harness_layer(
            &base,
            &provisioning,
            &stacks
                .iter()
                .flat_map(|s| project.stack_writable(&s.name))
                .collect::<Vec<_>>(),
        ),
    )
    .map_err(|e| ImageError::Io(layer.path().to_path_buf(), e))?;
    // A value that differs at every build, so the harness layer is never
    // served from cache and the install above fetches the current version.
    // The `ARG` it feeds is read by a `RUN` that does nothing else: Docker
    // invalidates from there down, which is exactly the install and what
    // follows it.
    docker_build(engine, layer.path(), &images.agent, &{
        let mut args = vec![format!(
            "NUNKI_HARNESS_BUILD={}",
            crate::state::now_rfc3339()
        )];
        if harness == Harness::KeepWhatTheImageCarries {
            args.push(format!("{HARNESS_KEEP}=1"));
        }
        args
    })?;

    Ok(Built { images, versions })
}

/// What the stack's own image is built with: the build arguments and the
/// label, before any engine is involved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackBuild {
    pub args: Vec<String>,
    /// `nunki.versions=…`, written on the image where `nunki check` and a
    /// launch read it back: the tag alone cannot say which toolchain is inside.
    pub label: String,
    pub versions: Vec<crate::versions::Pin>,
}

/// The stacks one image carries, the primary first: every stack the project
/// declares, or `stack` alone when it declares none. `stack` must be the
/// primary — the image is the project's, not a stack's (SPEC 4.2, "plusieurs
/// stacks").
pub fn image_stacks(project: &Project, stack: &str) -> Result<Vec<Stack>, ImageError> {
    let declared = &project.config.stacks;
    match declared.first() {
        None => Ok(vec![Stack::root(stack)]),
        Some(primary) if primary.name == stack => Ok(declared.clone()),
        Some(primary) => Err(ImageError::NotPrimary {
            asked: stack.to_string(),
            primary: primary.name.clone(),
        }),
    }
}

/// What a stack contributes to the image's Dockerfile: the whole of its own
/// when it is the primary, its stages and its add-on otherwise.
pub fn contribution(
    project: &Project,
    stacks: &[Stack],
    stack: &Stack,
) -> Result<String, ImageError> {
    let fragment = project.fragment(&stack.name);
    let read = |name: &str| {
        let file = fragment.join(name);
        std::fs::read_to_string(&file).map_err(|e| ImageError::Io(file.clone(), e))
    };
    let primary = &stacks[0];
    if stack == primary {
        let dockerfile = fragment.join("Dockerfile");
        if !dockerfile.is_file() {
            return Err(ImageError::NoDockerfile {
                stack: stack.name.clone(),
                at: dockerfile,
            });
        }
        return read("Dockerfile");
    }
    let addon = fragment.join(crate::project::ADDON_FILE);
    if !addon.is_file() {
        return Err(ImageError::NoAddon {
            stack: stack.name.clone(),
            primary: primary.name.clone(),
            at: addon,
        });
    }
    let stages = if fragment.join(crate::project::ADDON_STAGES_FILE).is_file() {
        read(crate::project::ADDON_STAGES_FILE)?
    } else {
        String::new()
    };
    Ok(format!("{stages}{}", read(crate::project::ADDON_FILE)?))
}

/// The Dockerfile of an image carrying several stacks.
pub fn compose_stacks(project: &Project, stacks: &[Stack]) -> Result<String, ImageError> {
    let fragment = |s: &Stack| project.fragment(&s.name);
    let primary = contribution(project, stacks, &stacks[0])?;
    let mut addons = Vec::new();
    for stack in &stacks[1..] {
        // Read through `contribution` for the refusal it makes on a missing
        // add-on, then apart, since the two halves go to two places.
        contribution(project, stacks, stack)?;
        let stages =
            std::fs::read_to_string(fragment(stack).join(crate::project::ADDON_STAGES_FILE))
                .unwrap_or_default();
        let file = fragment(stack).join(crate::project::ADDON_FILE);
        let body = std::fs::read_to_string(&file).map_err(|e| ImageError::Io(file.clone(), e))?;
        addons.push((stack.name.clone(), stages, body));
    }
    Ok(compose(&stacks[0].name, &primary, &addons))
}

/// Put an image's Dockerfile together from its primary stack's and the
/// add-ons of the others, as `(name, stages, body)`.
///
/// Every stage goes **before** the primary's first `FROM`, and every global
/// `ARG` before any stage: an `ARG` written after a `FROM` belongs to that
/// stage, and the primary's `FROM ${BASE}` would no longer see its own `BASE`.
/// The bodies go at the end, in the order the stacks are declared, each
/// starting from an image that ends as the agent and ending as the agent.
pub fn compose(primary_name: &str, primary: &str, addons: &[(String, String, String)]) -> String {
    let (head, rest) = split_at_first_from(primary);
    let mut globals = String::new();
    let mut stages = String::new();
    for (_, text, _) in addons {
        let (g, s) = split_at_first_from(text);
        globals.push_str(g);
        stages.push_str(s);
    }
    let names: Vec<&str> = addons.iter().map(|(n, _, _)| n.as_str()).collect();
    let mut out = format!(
        "# Composed by nunki from the fragments of {primary_name}, then {} (SPEC 4.2,\n\
         # \"plusieurs stacks\"). Do not edit: `nunki slot rebuild` writes it again.\n",
        names.join(", ")
    );
    out.push_str(head);
    out.push_str(&globals);
    out.push_str(&stages);
    out.push_str(rest);
    for (_, _, body) in addons {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(body);
    }
    out
}

/// A Dockerfile cut before its first `FROM`: what comes before is global.
fn split_at_first_from(text: &str) -> (&str, &str) {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let word = line.split_whitespace().next().unwrap_or("");
        if word.eq_ignore_ascii_case("FROM") {
            return text.split_at(at);
        }
        at += line.len();
    }
    (text, "")
}

/// The build arguments for the image: the human's ids, and the versions the
/// repository pins for every stack it carries, each read in that stack's
/// directory — read now and not at `init`, since the fragment never updates
/// itself and the repository does (SPEC 4.2).
///
/// Refused when `versions.txt` names an argument its stack's contribution does
/// not declare, since the engine would drop it and the image keep its default;
/// and when two stacks read one argument, since one value would win for both.
pub fn stack_build(
    project: &Project,
    stack: &str,
    uid: u32,
    gid: u32,
) -> Result<StackBuild, ImageError> {
    let (versions, pinned) = read_versions(project, &project.root, stack)?;
    let mut args = vec![format!("UID={uid}"), format!("GID={gid}")];
    args.extend(pinned.iter().map(|(arg, value)| format!("{arg}={value}")));
    Ok(StackBuild {
        args,
        label: format!(
            "{}={}",
            crate::versions::LABEL,
            crate::versions::label(&pinned)
        ),
        versions,
    })
}

/// What the repository at `tree` pins for the image `stack` is the primary
/// of — every stack it carries, each read in its own directory — as the label
/// records it. What `nunki check` and a launch compare an image against.
pub fn pinned_now(
    project: &Project,
    tree: &Path,
    stack: &str,
) -> Result<std::collections::BTreeMap<String, String>, ImageError> {
    Ok(read_versions(project, tree, stack)?.1)
}

type Read = (
    Vec<crate::versions::Pin>,
    std::collections::BTreeMap<String, String>,
);

fn read_versions(project: &Project, tree: &Path, stack: &str) -> Result<Read, ImageError> {
    let stacks = image_stacks(project, stack)?;
    let mut owner: std::collections::BTreeMap<String, String> = Default::default();
    let mut versions = Vec::new();
    for s in &stacks {
        let sources = project.stack_versions(&s.name)?;
        if sources.is_empty() {
            continue;
        }
        let text = contribution(project, &stacks, s)?;
        let missing = crate::versions::undeclared(&sources, &text);
        if !missing.is_empty() {
            return Err(ImageError::Undeclared {
                stack: s.name.clone(),
                args: missing.join(", "),
                dockerfile: project.fragment(&s.name).join(if s == &stacks[0] {
                    "Dockerfile"
                } else {
                    crate::project::ADDON_FILE
                }),
            });
        }
        for source in &sources {
            match owner.get(&source.arg) {
                Some(first) if *first != s.name => {
                    return Err(ImageError::SharedArgument {
                        arg: source.arg.clone(),
                        first: first.clone(),
                        second: s.name.clone(),
                    });
                }
                _ => {
                    owner.insert(source.arg.clone(), s.name.clone());
                }
            }
        }
        versions.extend(crate::versions::resolve(
            &project.stack_tree(tree, &s.name),
            &sources,
        ));
    }
    let pinned = crate::versions::pinned(&versions);
    Ok((versions, pinned))
}

/// What an image records about the versions it was built with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// No such image on this machine.
    Absent,
    /// Built before nunki recorded versions, so what it carries is unknown.
    Unrecorded,
    /// Built with these pinned arguments.
    Pinned(std::collections::BTreeMap<String, String>),
}

/// Read back what [`build`] wrote on an image. An engine that cannot answer is
/// an error, never an absent image: the two send a human to different places.
pub fn recorded(engine: &str, image: &str) -> Result<Recorded, ImageError> {
    let out = Command::new(engine)
        .args([
            "image",
            "inspect",
            "--format",
            "{{json .Config.Labels}}",
            image,
        ])
        .output()
        .map_err(|e| ImageError::NoEngine(e.to_string()))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        // Docker says "No such image", Podman "image not known".
        let lower = stderr.to_lowercase();
        if lower.contains("no such image") || lower.contains("not known") {
            return Ok(Recorded::Absent);
        }
        return Err(ImageError::Inspect {
            engine: engine.to_string(),
            image: image.to_string(),
            said: stderr.trim().to_string(),
        });
    }
    let labels: Option<std::collections::BTreeMap<String, String>> =
        serde_json::from_slice(&out.stdout).map_err(|e| ImageError::Inspect {
            engine: engine.to_string(),
            image: image.to_string(),
            said: format!("labels that are not JSON ({e})"),
        })?;
    Ok(
        match labels.as_ref().and_then(|l| l.get(crate::versions::LABEL)) {
            Some(value) => Recorded::Pinned(crate::versions::parse_label(value)),
            None => Recorded::Unrecorded,
        },
    )
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

/// The build argument that keeps the harness an image already carries.
const HARNESS_KEEP: &str = "NUNKI_HARNESS_KEEP";

/// Whether a build installs the harness or keeps the one the image carries.
///
/// A parameter rather than something inferred from the image, because the
/// previous version inferred it — it skipped the install whenever the binary
/// happened to be present — and that had two effects where only one was
/// wanted. It let an image carry a substitute harness, and it froze the real
/// one for ever: an image built once kept its version until somebody deleted
/// it, and nothing said which version that was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    /// Install it, at every build. What an ordinary build does, so that an
    /// image is never older than the day it was built.
    Install,
    /// Keep whatever the image already has.
    ///
    /// For an image carrying a **substitute**: nunki's own live tests write a
    /// `claude` that answers with canned JSON, so a mission can be started
    /// without spending an agent. Installing the real one over it would
    /// defeat the test, and that image carries no `curl` to install it with
    /// — measured, exit 127.
    KeepWhatTheImageCarries,
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
             ARG NUNKI_HARNESS_KEEP\n\
             RUN echo \"harness build ${NUNKI_HARNESS_BUILD:-unset}\" > /dev/null\n",
        );
    }
    for step in &provisioning.install {
        // As the agent, not as root: the harness keeps its state under the
        // agent's home, and installing it as root would leave it unreadable
        // by the only user that runs it.
        //
        // `NUNKI_HARNESS_KEEP` is the one way out, and it is **declared**
        // rather than inferred. The previous version skipped the install
        // whenever the binary happened to be present, which had two effects
        // and only one of them was wanted: it let an image carry a
        // substitute harness, and it froze the real one for ever. Saying so
        // with an argument keeps the first and drops the second — a build
        // that wants the harness it already has has to ask, and a build that
        // says nothing gets the current one.
        //
        // What asks for it: nunki's own live tests, whose fixture image
        // writes a `claude` that answers with canned JSON so a mission can
        // be started without spending an agent. Installing the real one over
        // it would defeat the test, and that image carries no `curl` to
        // install it with — measured, exit 127.
        out.push_str(&format!(
            "RUN [ -n \"${{NUNKI_HARNESS_KEEP:-}}\" ] || ({step})\n"
        ));
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
    docker_build_labelled(engine, context, tag, build_args, &[])
}

fn docker_build_labelled(
    engine: &str,
    context: &Path,
    tag: &str,
    build_args: &[String],
    labels: &[String],
) -> Result<(), ImageError> {
    let mut command = Command::new(engine);
    command.args(["build", "-q", "-t", tag]);
    for arg in build_args {
        command.args(["--build-arg", arg]);
    }
    for label in labels {
        command.args(["--label", label]);
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
