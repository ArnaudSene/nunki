//! The project's services file, read into a closed model (SPEC 4.2,
//! "services et lancement").
//!
//! This reverses a doctrine on purpose. The project's blocks used to be
//! merged into the generated file verbatim and never re-typed, so that no key
//! was lost. But the file lives in the slot's tree, the coder writes the tree,
//! and the engine starts what the file says with all of its power:
//! `privileged`, `pid: host`, a bind mount of `/` or of the engine's socket,
//! an `env_file` naming a file of the human's, `${VAR}` filled from the host's
//! environment. A list of dangerous keys would rot with every Compose
//! release, so the model is **closed**: a service holds the fields below and
//! nothing else, and a key outside them refuses the whole file — it is never
//! dropped, but for `ports`, which nunki drops itself (a published port would
//! collide between slots, and the agent reaches a service by name).
//!
//! No YAML feature carries meaning past the parser: anchors, aliases, tags,
//! merge keys, directives, duplicate keys and a second document are all
//! refused, and every key is compared after decoding, so an escaped key is
//! the key it decodes to. Whether the file holds a tag, an anchor or an
//! alias is decided by the parser's own events, never by a second reading
//! of the text; what the events do not say (a directive, the document
//! markers) is read from the first columns of lines that only LF and CRLF
//! end.
//!
//! What is lifted is never the file's bytes but nunki's own rendering of the
//! model ([`ServicesFile::render`]): keys in a fixed order, every string
//! quoted and written in ASCII, every `$` doubled so Compose interpolates
//! nothing.

use std::collections::{BTreeMap, BTreeSet};

use crate::yaml::{self, Value};

use super::{AGENT_SERVICE, FIREWALL_SERVICE, PROBER_SERVICE};

/// What a volume name may never start with: nunki names the slot's own
/// volumes `nunki-<slot>-…`, and a project taking one would be handed the
/// build cache or the harness's sessions.
pub const RESERVED_VOLUME_PREFIX: &str = "nunki-";

/// The service names nunki writes itself in a generated file.
pub const RESERVED_SERVICES: [&str; 3] = [FIREWALL_SERVICE, AGENT_SERVICE, PROBER_SERVICE];

/// The label nunki puts on every container it lifts from a services file,
/// carrying the digest of the rendering it was lifted from. What tells a
/// container lifted from an approved definition from one that is not.
pub const LABEL: &str = "nunki.services";

/// The digest of a rendering, as it is shown, approved and labelled.
pub fn digest(rendering: &str) -> String {
    format!("sha256:{}", crate::init::sha256(rendering.as_bytes()))
}

/// The network every lifted service joins, and the only one: nunki's, and
/// declared `internal: true`, so the engine gives it no route out. The
/// firewall attaches to it beside the default network, which is how the
/// agent — in the firewall's namespace — reaches a service through its
/// perimeter, and how nothing else does (SPEC 4.1 bis).
pub const NETWORK: &str = "nunki-services";

/// The driver option, with its value, that leaves [`NETWORK`] without a
/// gateway address on the host.
///
/// `internal: true` takes away the route out, not the gateway: measured by
/// the HQ on Docker Engine 29.4.0, a container with every capability dropped
/// on an `--internal` network answers a ping from the bridge's gateway
/// address and, through it, reaches a port the host publishes on `0.0.0.0`.
/// With this option the bridge carries no IPv4 address: the host's published
/// port is closed from the service, and a peer on the network is still
/// reached by its name. A Docker bridge option; what Podman makes of it is
/// not measured.
///
/// It removes the IPv4 gateway only. The network is also declared with
/// `enable_ipv6: false`, so a daemon whose networks default to IPv6 gives
/// the bridge no IPv6 gateway on the host either. The HQ measured no IPv6
/// on Docker Engine 29.4.0 by default; no IPv6 daemon was at hand to
/// measure the declaration against.
pub const NO_GATEWAY: (&str, &str) = ("com.docker.network.bridge.inhibit_ipv4", "true");

/// The capabilities a lifted service gets back, after all of them are
/// dropped. Fixed, nunki's, never the file's: what the official images'
/// entrypoints need to start from root and drop to their own user: change
/// a file's owner (`CHOWN`, which nginx needs), change user (`SETUID`,
/// `SETGID`), and read a data directory root does not own
/// (`DAC_OVERRIDE`, which Postgres needs to start again).
///
/// Measured by the HQ on Docker Engine 29.8.2 with
/// `live_the_capability_set_is_the_smallest_the_images_need`, each image
/// started fresh and then again over the data it wrote:
/// `postgres:16-alpine`, `postgres:16`, `questdb/questdb:9.4.3` and
/// `nginx:alpine` are healthy both times with this set; without `CHOWN`
/// nginx is not, without `SETGID` or `SETUID` none is. Without
/// `DAC_OVERRIDE` both Postgres images start once and never again: after
/// `initdb` their data directory belongs to `postgres` with mode 0700, and
/// the entrypoint's `find "$PGDATA"`, run as root, cannot read it.
///
/// `DAC_READ_SEARCH` starts Postgres again too, and is not the one given
/// back: Docker's default seccomp profile allows `open_by_handle_at` only
/// to a container that holds it, and that call opens a file by its handle,
/// past the container's own view of the filesystem. `DAC_OVERRIDE` gates no
/// call there, is in Docker's own default set, and reaches no file the
/// container does not mount: its own filesystem, the file's named volumes
/// (a services file mounts no path of the host), and the few files the
/// engine mounts in every container.
///
/// Capabilities interact, so a set is minimal only relative to itself:
/// without `FOWNER` the entrypoint's `chmod` is refused and tolerated. The
/// test removes each capability from this set, and checks exactly that.
pub const CAPABILITIES: [&str; 4] = ["CHOWN", "DAC_OVERRIDE", "SETGID", "SETUID"];

/// The memory a lifted service may use: a bound, so a service cannot take
/// the human's machine with it. Enough for a database and a JVM under test.
pub const MEMORY: &str = "2g";

/// The processes a lifted service may hold: a bound on a fork bomb. A JVM
/// counts its threads here, and QuestDB starts a few hundred.
pub const PROCESSES: u32 = 1024;

/// What a profile lifts from an approved services file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lifted {
    /// The services lifted, and the volumes they mount.
    pub services: ServicesFile,
    /// The digest of the whole approved rendering: what every lifted
    /// container is labelled with, so the next launch can tell an approved
    /// definition from one that is not.
    pub approved: String,
    /// The file's services this profile does not start.
    pub held_back: Vec<String>,
}

impl Lifted {
    /// Every service of `file`.
    pub fn whole(file: ServicesFile) -> Self {
        Self {
            approved: file.digest(),
            services: file,
            held_back: Vec::new(),
        }
    }

    /// The services of `file` a mission declares by name, and those they
    /// depend on, however deep; the others are held back, and so are the
    /// volumes only they mount. A declared name the file does not hold is a
    /// provider of another kind — a third party's test tier — and lifts
    /// nothing.
    pub fn declared(file: ServicesFile, declared: &[String]) -> Self {
        let approved = file.digest();
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        let mut queue: Vec<String> = declared
            .iter()
            .filter(|name| file.services.contains_key(*name))
            .cloned()
            .collect();
        while let Some(name) = queue.pop() {
            if wanted.insert(name.clone()) {
                queue.extend(file.services[&name].depends_on.keys().cloned());
            }
        }
        let (services, held): (BTreeMap<_, _>, BTreeMap<_, _>) = file
            .services
            .into_iter()
            .partition(|(name, _)| wanted.contains(name));
        let volumes = file
            .volumes
            .into_iter()
            .filter(|volume| {
                services
                    .values()
                    .any(|s: &ProjectService| s.volumes.iter().any(|m| m.volume == *volume))
            })
            .collect();
        Self {
            services: ServicesFile { services, volumes },
            approved,
            held_back: held.into_keys().collect(),
        }
    }
}

/// The keys a service may hold, in the order they are rendered. `ports` is
/// read and dropped, and is therefore not among them.
pub const SERVICE_KEYS: [&str; 8] = [
    "image",
    "entrypoint",
    "command",
    "working_dir",
    "environment",
    "healthcheck",
    "depends_on",
    "volumes",
];

/// The keys a healthcheck may hold, in the order they are rendered.
pub const HEALTHCHECK_KEYS: [&str; 6] = [
    "test",
    "interval",
    "timeout",
    "retries",
    "start_period",
    "start_interval",
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServicesError {
    #[error("not YAML nunki can read: {0}")]
    Yaml(String),
    #[error(
        "line {line} uses {feature}, which a services file may not: nunki lifts what the \
         file says, not what a YAML parser makes of it"
    )]
    Feature { feature: &'static str, line: usize },
    #[error("{at} uses a merge key (`<<`), which a services file may not: write the keys out")]
    MergeKey { at: String },
    #[error("{at} has a key that is not a string")]
    KeyNotString { at: String },
    #[error(
        "the file declares {0:?} at the top level; a services file holds only `services` \
         and `volumes` (and a `name`, which nunki ignores)"
    )]
    TopLevelKey(String),
    #[error(
        "service {service:?} declares {key:?}, which nunki does not lift: a service holds \
         only {} (and `ports`, which nunki drops)",
        SERVICE_KEYS.join(", ")
    )]
    ServiceKey { service: String, key: String },
    #[error(
        "service {service:?} declares healthcheck.{key}, which nunki does not lift: a \
         healthcheck holds only {}",
        HEALTHCHECK_KEYS.join(", ")
    )]
    HealthcheckKey { service: String, key: String },
    #[error("{at} must be {expected}")]
    Shape { at: String, expected: &'static str },
    #[error(
        "{0:?} is not a service name nunki lifts: one DNS label, lowercase letters, digits \
         and `-`"
    )]
    ServiceName(String),
    #[error("the file declares a service named {0:?}, which nunki reserves")]
    ReservedService(String),
    #[error(
        "{0:?} is not a volume name nunki lifts: letters, digits, `_`, `.` and `-`, \
         starting on a letter or a digit"
    )]
    VolumeName(String),
    #[error("the file declares a volume named {0:?}; names starting `nunki-` are nunki's")]
    ReservedVolume(String),
    #[error("top-level volume {0:?} has a value; a services file declares a volume by name only")]
    VolumeValue(String),
    #[error("service {service:?} mounts {mount:?}: {why}")]
    Mount {
        service: String,
        mount: String,
        why: &'static str,
    },
    #[error("service {service:?} sets environment {name:?}: {why}")]
    Environment {
        service: String,
        name: String,
        why: &'static str,
    },
    #[error("service {service:?} depends on {on:?}: {why}")]
    DependsOn {
        service: String,
        on: String,
        why: &'static str,
    },
}

/// A services file, as nunki lifts it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServicesFile {
    /// By name, so the rendering's order is the names' and not the file's.
    pub services: BTreeMap<String, ProjectService>,
    /// The file's own named volumes, by name only.
    pub volumes: BTreeSet<String>,
}

/// One service of the file. Every field is one the human reads in the
/// rendering; nothing else reaches the engine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectService {
    pub image: String,
    pub entrypoint: Option<Command>,
    pub command: Option<Command>,
    pub working_dir: Option<String>,
    /// Name to an explicit, non-empty literal: a bare entry would be filled
    /// from the host's environment.
    pub environment: BTreeMap<String, String>,
    pub healthcheck: Option<Healthcheck>,
    pub depends_on: BTreeMap<String, Condition>,
    pub volumes: Vec<Mount>,
    /// Whether the file declared `ports`, which nunki dropped.
    pub ports_dropped: bool,
}

/// A command as Compose takes it: one string for a shell, or a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Shell(String),
    Exec(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Healthcheck {
    pub test: Option<Command>,
    pub interval: Option<String>,
    pub timeout: Option<String>,
    pub retries: Option<u64>,
    pub start_period: Option<String>,
    pub start_interval: Option<String>,
}

/// What a service waits for in another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Started,
    Healthy,
    CompletedSuccessfully,
}

impl Condition {
    fn spelled(self) -> &'static str {
        match self {
            Condition::Started => "service_started",
            Condition::Healthy => "service_healthy",
            Condition::CompletedSuccessfully => "service_completed_successfully",
        }
    }
}

/// A named volume of the file, mounted at an absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub volume: String,
    pub at: String,
    pub read_only: bool,
}

impl ServicesFile {
    /// Read a services file, or refuse it whole.
    pub fn parse(text: &str) -> Result<ServicesFile, ServicesError> {
        // What the parser reports, the parser decides: a tag, an anchor or
        // an alias on any node refuses the file, with the line the parser
        // gives it. First, so that one behind a stray line break is named
        // for what it is.
        let constructs = yaml::constructs(text).map_err(|e| ServicesError::Yaml(e.to_string()))?;
        if let Some(found) = constructs.first() {
            return Err(ServicesError::Feature {
                feature: found.construct.described(),
                line: usize::try_from(found.line).unwrap_or(usize::MAX),
            });
        }
        // Then one idea of a line, before anything reads lines.
        one_kind_of_line_break(text)?;
        plain_yaml(text)?;
        let document: Value =
            yaml::from_str(text).map_err(|e| ServicesError::Yaml(e.to_string()))?;
        decoded(&document, "the file")?;
        typed(&document)
    }

    /// The names of the services whose `ports` nunki dropped.
    /// The digest of [`ServicesFile::render`]: what a human approves, and
    /// what the label on every lifted container carries.
    pub fn digest(&self) -> String {
        digest(&self.render())
    }

    pub fn dropped_ports(&self) -> Vec<&str> {
        self.services
            .iter()
            .filter(|(_, s)| s.ports_dropped)
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// nunki's own rendering of the model: the one thing that is hashed,
    /// shown, approved and lifted.
    ///
    /// Keys in a fixed order, every string double-quoted and written in
    /// ASCII — anything else is a `\u` escape, so nothing a terminal or a
    /// YAML parser could read as a line break or a control hides in it — and
    /// every `$` written `$$`, so Compose interpolates nothing.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if self.services.is_empty() {
            out.push_str("services: {}\n");
        } else {
            out.push_str("services:\n");
        }
        for (name, service) in &self.services {
            out.push_str(&format!("  {}:\n", quote(name)));
            service.render(&mut out);
        }
        if !self.volumes.is_empty() {
            out.push_str("volumes:\n");
            for name in &self.volumes {
                out.push_str(&format!("  {}: null\n", quote(name)));
            }
        }
        out
    }
}

impl ProjectService {
    fn render(&self, out: &mut String) {
        out.push_str(&format!("    image: {}\n", quote(&self.image)));
        if let Some(entrypoint) = &self.entrypoint {
            render_command(out, "    ", "entrypoint", entrypoint);
        }
        if let Some(command) = &self.command {
            render_command(out, "    ", "command", command);
        }
        if let Some(dir) = &self.working_dir {
            out.push_str(&format!("    working_dir: {}\n", quote(dir)));
        }
        if !self.environment.is_empty() {
            out.push_str("    environment:\n");
            for (name, value) in &self.environment {
                out.push_str(&format!("      {}: {}\n", quote(name), quote(value)));
            }
        }
        if let Some(check) = &self.healthcheck {
            out.push_str("    healthcheck:\n");
            if let Some(test) = &check.test {
                render_command(out, "      ", "test", test);
            }
            for (key, value) in [("interval", &check.interval), ("timeout", &check.timeout)] {
                if let Some(value) = value {
                    out.push_str(&format!("      {key}: {}\n", quote(value)));
                }
            }
            if let Some(retries) = check.retries {
                out.push_str(&format!("      retries: {retries}\n"));
            }
            for (key, value) in [
                ("start_period", &check.start_period),
                ("start_interval", &check.start_interval),
            ] {
                if let Some(value) = value {
                    out.push_str(&format!("      {key}: {}\n", quote(value)));
                }
            }
        }
        if !self.depends_on.is_empty() {
            out.push_str("    depends_on:\n");
            for (on, condition) in &self.depends_on {
                out.push_str(&format!(
                    "      {}:\n        condition: {}\n",
                    quote(on),
                    quote(condition.spelled())
                ));
            }
        }
        if !self.volumes.is_empty() {
            out.push_str("    volumes:\n");
            for mount in &self.volumes {
                let mode = if mount.read_only { ":ro" } else { "" };
                out.push_str(&format!(
                    "      - {}\n",
                    quote(&format!("{}:{}{mode}", mount.volume, mount.at))
                ));
            }
        }
    }
}

fn render_command(out: &mut String, indent: &str, key: &str, command: &Command) {
    match command {
        Command::Shell(line) => out.push_str(&format!("{indent}{key}: {}\n", quote(line))),
        Command::Exec(words) if words.is_empty() => out.push_str(&format!("{indent}{key}: []\n")),
        Command::Exec(words) => {
            out.push_str(&format!("{indent}{key}:\n"));
            for word in words {
                out.push_str(&format!("{indent}  - {}\n", quote(word)));
            }
        }
    }
}

/// A YAML double-quoted scalar, ASCII only, with every `$` doubled.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '$' => out.push_str("$$"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ' '..='~' => out.push(c),
            c if (c as u32) <= 0xffff => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push_str(&format!("\\U{:08x}", c as u32)),
        }
    }
    out.push('"');
    out
}

// --- the YAML features nunki refuses ------------------------------------------

/// The characters YAML 1.1 reads as a line break and YAML 1.2 does not:
/// NEL, LS and PS.
pub const YAML_1_1_BREAKS: [char; 3] = ['\u{85}', '\u{2028}', '\u{2029}'];

/// Refuse every line break but LF and CRLF, so that the file has one idea
/// of a line, the one [`plain_yaml`] counts by, and the parser's.
///
/// A raw NEL, LS or PS says one thing to a YAML 1.1 reader and another to
/// the YAML 1.2 reader nunki uses. Measured against the reader nunki used
/// before this one: a NEL in a quoted scalar was folded to a space and is
/// now kept, and any of the three in a comment ended the comment, so what
/// followed it on the line was read as YAML, and is now not read at all.
/// Written as an escape (`\N`, `\L`, `\P`) in a double-quoted scalar, each
/// reads the same in both, and is rendered as a `\u` escape.
///
/// A CR not followed by an LF is a line break to both readers and to no
/// text that splits lines on LF: what followed it on the "same" line was
/// read as another line, out of sight of the checks of this file. Written
/// as an escape (`\r`), it is a character of a quoted scalar like any other.
fn one_kind_of_line_break(text: &str) -> Result<(), ServicesError> {
    let lines: Vec<&str> = text.split('\n').collect();
    for (at, line) in lines.iter().enumerate() {
        // The last piece has no LF after it: a CR that ends it ends nothing
        // else either.
        let ended_by_lf = at + 1 < lines.len();
        let refuse = |feature| ServicesError::Feature {
            feature,
            line: at + 1,
        };
        if line.contains(YAML_1_1_BREAKS) {
            return Err(refuse(
                "a NEL, LS or PS character, which YAML 1.1 reads as a line break",
            ));
        }
        let line = match line.strip_suffix('\r') {
            Some(before) if ended_by_lf => before,
            _ => line,
        };
        if line.contains('\r') {
            return Err(refuse(
                "a carriage return without a line feed, which YAML reads as a line break",
            ));
        }
    }
    Ok(())
}

/// Refuse a directive, a second document and a document end marker: what a
/// line says from its first column, and the parser's events do not say.
///
/// A tag, an anchor and an alias are not this function's: the parser's
/// events report them ([`crate::yaml::constructs`]), and a second reading of
/// the text would only be wrong where it disagrees. This one looks at the
/// first characters of each line and nothing else — no quote, no block
/// scalar, no comment is followed across lines — so it cannot be blinded by
/// one; and [`one_kind_of_line_break`] has made its lines the parser's. A
/// line inside a multi-line scalar that starts with `%`, `---` or `...` in
/// its first column is refused with the rest: a file refused wrongly is one
/// a human rewrites, a feature missed would be one nobody sees.
fn plain_yaml(text: &str) -> Result<(), ServicesError> {
    let mut documents = 0usize;
    let mut content = false;
    for (number, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let refuse = |feature| ServicesError::Feature {
            feature,
            line: number + 1,
        };
        if line.starts_with('%') {
            return Err(refuse("a directive"));
        }
        let marker = |m: &str| {
            line.strip_prefix(m)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
        };
        if marker("---") {
            if documents > 0 || content {
                return Err(refuse("a second document"));
            }
            documents += 1;
        } else if marker("...") {
            return Err(refuse("a document end marker"));
        } else {
            let trimmed = line.trim_start_matches([' ', '\t']);
            content |= !trimmed.is_empty() && !trimmed.starts_with('#');
        }
    }
    Ok(())
}

/// Refuse what the parser decoded but the model must not read past: a merge
/// key, which the reader keeps as an ordinary key, and a key that is not a
/// string. Anywhere in the document, so that nothing below a refused key can
/// matter.
///
/// A tag is not the walk's: the parser's events refuse one before the tree
/// is read ([`crate::yaml::constructs`]), as they refuse an anchor and an
/// alias, which the tree would not show either.
fn decoded(value: &Value, at: &str) -> Result<(), ServicesError> {
    match value {
        Value::Sequence(items) => {
            for (i, item) in items.iter().enumerate() {
                decoded(item, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        Value::Mapping(map) => {
            for (key, item) in map {
                let Value::String(key) = key else {
                    return Err(ServicesError::KeyNotString { at: at.to_string() });
                };
                if key == "<<" {
                    return Err(ServicesError::MergeKey { at: at.to_string() });
                }
                decoded(item, &format!("{at}.{key}"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

// --- the closed model ----------------------------------------------------------

fn typed(document: &Value) -> Result<ServicesFile, ServicesError> {
    let top = match document {
        Value::Null => return Ok(ServicesFile::default()),
        Value::Mapping(map) => map,
        _ => {
            return Err(ServicesError::Shape {
                at: "the file".to_string(),
                expected: "a mapping",
            });
        }
    };
    let mut file = ServicesFile::default();
    for (key, value) in top {
        match key.as_str().unwrap_or_default() {
            // Compose's own project name. nunki names the project itself, so
            // this one is read and ignored: it gives the file no power.
            "name" => {
                string(value, "the top-level name")?;
            }
            "volumes" => file.volumes = top_volumes(value)?,
            "services" => {}
            other => return Err(ServicesError::TopLevelKey(other.to_string())),
        }
    }
    let services = match top.get("services") {
        None | Some(Value::Null) => return Ok(file),
        Some(Value::Mapping(map)) => map,
        Some(_) => {
            return Err(ServicesError::Shape {
                at: "services".to_string(),
                expected: "a mapping of service name to service",
            });
        }
    };
    let names: BTreeSet<String> = services
        .keys()
        .map(|k| k.as_str().unwrap_or_default().to_string())
        .collect();
    for (key, value) in services {
        let name = key.as_str().unwrap_or_default();
        if RESERVED_SERVICES.contains(&name) {
            return Err(ServicesError::ReservedService(name.to_string()));
        }
        if !dns_label(name) {
            return Err(ServicesError::ServiceName(name.to_string()));
        }
        let service = service(name, value, &file.volumes, &names)?;
        file.services.insert(name.to_string(), service);
    }
    Ok(file)
}

fn top_volumes(value: &Value) -> Result<BTreeSet<String>, ServicesError> {
    let map = match value {
        Value::Null => return Ok(BTreeSet::new()),
        Value::Mapping(map) => map,
        _ => {
            return Err(ServicesError::Shape {
                at: "volumes".to_string(),
                expected: "a mapping of volume names",
            });
        }
    };
    let mut names = BTreeSet::new();
    for (key, value) in map {
        let name = key.as_str().unwrap_or_default();
        if name.starts_with(RESERVED_VOLUME_PREFIX) {
            return Err(ServicesError::ReservedVolume(name.to_string()));
        }
        if !volume_name(name) {
            return Err(ServicesError::VolumeName(name.to_string()));
        }
        if !value.is_null() {
            return Err(ServicesError::VolumeValue(name.to_string()));
        }
        names.insert(name.to_string());
    }
    Ok(names)
}

fn service(
    name: &str,
    value: &Value,
    volumes: &BTreeSet<String>,
    services: &BTreeSet<String>,
) -> Result<ProjectService, ServicesError> {
    let Value::Mapping(map) = value else {
        return Err(ServicesError::Shape {
            at: format!("service {name:?}"),
            expected: "a mapping",
        });
    };
    let at = |key: &str| format!("service {name:?}, {key}");
    let mut out = ProjectService::default();
    let mut image = None;
    for (key, value) in map {
        let key = key.as_str().unwrap_or_default();
        match key {
            "image" => image = Some(nonempty(value, &at(key))?),
            "entrypoint" => out.entrypoint = Some(command(value, &at(key))?),
            "command" => out.command = Some(command(value, &at(key))?),
            "working_dir" => {
                let dir = string(value, &at(key))?;
                if !dir.starts_with('/') {
                    return Err(ServicesError::Shape {
                        at: at(key),
                        expected: "an absolute path",
                    });
                }
                out.working_dir = Some(dir);
            }
            "environment" => out.environment = environment(name, value)?,
            "healthcheck" => out.healthcheck = Some(healthcheck(name, value)?),
            "depends_on" => out.depends_on = depends_on(name, value, services)?,
            "volumes" => out.volumes = mounts(name, value, volumes)?,
            "ports" => out.ports_dropped = true,
            other => {
                return Err(ServicesError::ServiceKey {
                    service: name.to_string(),
                    key: other.to_string(),
                });
            }
        }
    }
    out.image = image.ok_or_else(|| ServicesError::Shape {
        at: at("image"),
        expected: "declared: nunki lifts an image, it builds nothing",
    })?;
    Ok(out)
}

fn environment(service: &str, value: &Value) -> Result<BTreeMap<String, String>, ServicesError> {
    let Value::Mapping(map) = value else {
        return Err(ServicesError::Shape {
            at: format!("service {service:?}, environment"),
            expected: "a mapping of name to value",
        });
    };
    let mut out = BTreeMap::new();
    for (key, value) in map {
        let name = key.as_str().unwrap_or_default();
        let refuse = |why| ServicesError::Environment {
            service: service.to_string(),
            name: name.to_string(),
            why,
        };
        if !variable_name(name) {
            return Err(refuse(
                "a name is letters, digits and `_`, not starting on a digit",
            ));
        }
        match value {
            Value::String(text) if !text.is_empty() => {
                out.insert(name.to_string(), text.clone());
            }
            Value::String(_) | Value::Null => {
                return Err(refuse(
                    "a value is never empty: an entry without one is filled from the host's \
                     environment",
                ));
            }
            _ => return Err(refuse("a value is a string; quote it")),
        }
    }
    Ok(out)
}

fn healthcheck(service: &str, value: &Value) -> Result<Healthcheck, ServicesError> {
    let Value::Mapping(map) = value else {
        return Err(ServicesError::Shape {
            at: format!("service {service:?}, healthcheck"),
            expected: "a mapping",
        });
    };
    let at = |key: &str| format!("service {service:?}, healthcheck.{key}");
    let mut out = Healthcheck::default();
    for (key, value) in map {
        let key = key.as_str().unwrap_or_default();
        match key {
            "test" => out.test = Some(command(value, &at(key))?),
            "interval" => out.interval = Some(nonempty(value, &at(key))?),
            "timeout" => out.timeout = Some(nonempty(value, &at(key))?),
            "start_period" => out.start_period = Some(nonempty(value, &at(key))?),
            "start_interval" => out.start_interval = Some(nonempty(value, &at(key))?),
            "retries" => {
                out.retries = Some(value.as_u64().ok_or_else(|| ServicesError::Shape {
                    at: at(key),
                    expected: "a whole number",
                })?)
            }
            other => {
                return Err(ServicesError::HealthcheckKey {
                    service: service.to_string(),
                    key: other.to_string(),
                });
            }
        }
    }
    Ok(out)
}

fn depends_on(
    service: &str,
    value: &Value,
    services: &BTreeSet<String>,
) -> Result<BTreeMap<String, Condition>, ServicesError> {
    let mut out = BTreeMap::new();
    let mut add = |on: &str, condition| {
        if !services.contains(on) {
            return Err(ServicesError::DependsOn {
                service: service.to_string(),
                on: on.to_string(),
                why: "a service depends only on another service of this file",
            });
        }
        out.insert(on.to_string(), condition);
        Ok(())
    };
    match value {
        Value::Sequence(items) => {
            for item in items {
                let on = string(item, &format!("service {service:?}, depends_on"))?;
                add(&on, Condition::Started)?;
            }
        }
        Value::Mapping(map) => {
            for (key, value) in map {
                let on = key.as_str().unwrap_or_default();
                let refuse = |why| ServicesError::DependsOn {
                    service: service.to_string(),
                    on: on.to_string(),
                    why,
                };
                let Value::Mapping(spec) = value else {
                    return Err(refuse(
                        "the dependency is a mapping holding only `condition`",
                    ));
                };
                let mut condition = Condition::Started;
                for (key, value) in spec {
                    if key.as_str() != Some("condition") {
                        return Err(refuse("the dependency holds only `condition`"));
                    }
                    condition = match value.as_str() {
                        Some("service_started") => Condition::Started,
                        Some("service_healthy") => Condition::Healthy,
                        Some("service_completed_successfully") => Condition::CompletedSuccessfully,
                        _ => {
                            return Err(refuse(
                                "the condition is service_started, service_healthy or \
                                 service_completed_successfully",
                            ));
                        }
                    };
                }
                add(on, condition)?;
            }
        }
        _ => {
            return Err(ServicesError::Shape {
                at: format!("service {service:?}, depends_on"),
                expected: "a list or a mapping of service names",
            });
        }
    }
    Ok(out)
}

fn mounts(
    service: &str,
    value: &Value,
    volumes: &BTreeSet<String>,
) -> Result<Vec<Mount>, ServicesError> {
    let Value::Sequence(items) = value else {
        return Err(ServicesError::Shape {
            at: format!("service {service:?}, volumes"),
            expected: "a list of mounts",
        });
    };
    items
        .iter()
        .map(|item| {
            let refuse = |why| ServicesError::Mount {
                service: service.to_string(),
                // A short mount as written; a long one as YAML.
                mount: match item {
                    Value::String(short) => short.clone(),
                    long => yaml::to_string(long)
                        .map(|s| s.trim_end().to_string())
                        .unwrap_or_default(),
                },
                why,
            };
            let mount = match item {
                Value::String(short) => {
                    let parts: Vec<&str> = short.split(':').collect();
                    let read_only = match parts.get(2) {
                        None => false,
                        Some(&"ro") => true,
                        Some(&"rw") => false,
                        Some(_) => return Err(refuse("the only mode is `ro` or `rw`")),
                    };
                    if parts.len() < 2 || parts.len() > 3 {
                        return Err(refuse(
                            "a mount names a volume of this file and an absolute path",
                        ));
                    }
                    Mount {
                        volume: parts[0].to_string(),
                        at: parts[1].to_string(),
                        read_only,
                    }
                }
                Value::Mapping(long) => {
                    let mut volume = None;
                    let mut at = None;
                    let mut read_only = false;
                    for (key, value) in long {
                        match (key.as_str().unwrap_or_default(), value) {
                            ("type", Value::String(kind)) if kind == "volume" => {}
                            ("type", _) => {
                                return Err(refuse(
                                    "only a named volume is mounted, of type volume",
                                ));
                            }
                            ("source", Value::String(source)) => volume = Some(source.clone()),
                            ("target", Value::String(target)) => at = Some(target.clone()),
                            ("read_only", Value::Bool(ro)) => read_only = *ro,
                            _ => {
                                return Err(refuse(
                                    "a long mount holds only type: volume, source, target and \
                                     read_only",
                                ));
                            }
                        }
                    }
                    let (Some(volume), Some(at)) = (volume, at) else {
                        return Err(refuse("a long mount names its source and its target"));
                    };
                    Mount {
                        volume,
                        at,
                        read_only,
                    }
                }
                _ => return Err(refuse("a mount is a string or a mapping")),
            };
            if !volumes.contains(&mount.volume) {
                return Err(refuse(
                    "only a named volume this file declares is mounted; a path is a bind \
                     mount of the human's machine",
                ));
            }
            if !mount.at.starts_with('/') || mount.at.contains(':') {
                return Err(refuse("a volume is mounted at an absolute path"));
            }
            Ok(mount)
        })
        .collect()
}

fn command(value: &Value, at: &str) -> Result<Command, ServicesError> {
    match value {
        Value::String(line) => Ok(Command::Shell(line.clone())),
        Value::Sequence(words) => words
            .iter()
            .map(|w| string(w, at))
            .collect::<Result<_, _>>()
            .map(Command::Exec),
        _ => Err(ServicesError::Shape {
            at: at.to_string(),
            expected: "a string or a list of strings",
        }),
    }
}

fn string(value: &Value, at: &str) -> Result<String, ServicesError> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ServicesError::Shape {
            at: at.to_string(),
            expected: "a string",
        })
}

fn nonempty(value: &Value, at: &str) -> Result<String, ServicesError> {
    let text = string(value, at)?;
    if text.is_empty() {
        return Err(ServicesError::Shape {
            at: at.to_string(),
            expected: "a non-empty string",
        });
    }
    Ok(text)
}

/// One DNS label: lowercase letters, digits and `-`, neither first nor last.
fn dns_label(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

/// Compose's own rule for a volume name.
fn volume_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

fn variable_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    //! The text checks of [`plain_yaml`] and [`one_kind_of_line_break`], and
    //! the walk of [`decoded`], branch by branch. Through
    //! [`ServicesFile::parse`] some are masked: a check that missed a line
    //! the parser then refuses for another reason reads the same. Here each
    //! case is one the check alone decides, and each says which way and on
    //! which line.

    use super::*;

    fn feature(text: &str) -> Option<(&'static str, usize)> {
        match plain_yaml(text) {
            Ok(()) => None,
            Err(ServicesError::Feature { feature, line }) => Some((feature, line)),
            Err(other) => panic!("the scan only refuses features: {other:?}"),
        }
    }

    #[test]
    fn a_document_marker_is_three_characters_then_a_blank_or_the_end() {
        assert_eq!(feature("---\n---\n"), Some(("a second document", 2)));
        assert_eq!(feature("--- a\n---\tb\n"), Some(("a second document", 2)));
        // `---y` is a plain scalar, and content: the marker after it is a
        // second document.
        assert_eq!(feature("---y: 1\n---\n"), Some(("a second document", 2)));
        assert_eq!(feature("---\nk: v\n"), None);
        assert_eq!(feature("...\n"), Some(("a document end marker", 1)));
        assert_eq!(
            feature("k: v\n... # end\n"),
            Some(("a document end marker", 2))
        );
        assert_eq!(feature("k: v\n...x: 1\n"), None);
    }

    #[test]
    fn content_before_a_first_marker_makes_it_a_second_document_and_a_comment_does_not() {
        assert_eq!(feature("k: v\n---\n"), Some(("a second document", 2)));
        assert_eq!(feature("\t- a\n---\n"), Some(("a second document", 2)));
        assert_eq!(feature("# about\n  # more\n \n---\nk: v\n"), None);
    }

    #[test]
    fn a_directive_is_a_percent_in_the_first_column_only() {
        assert_eq!(feature("%YAML 1.2\n---\n"), Some(("a directive", 1)));
        assert_eq!(feature("k: v\n%TAG ! x\n"), Some(("a directive", 2)));
        assert_eq!(feature("k: '50\n  %'\n"), None);
    }

    #[test]
    fn a_crlf_line_is_read_as_the_line_without_its_cr() {
        assert_eq!(feature("k: v\r\n---\r\n"), Some(("a second document", 2)));
        assert_eq!(feature("---\r\nk: v\r\n"), None);
    }

    fn break_in(text: &str) -> Option<(&'static str, usize)> {
        match one_kind_of_line_break(text) {
            Ok(()) => None,
            Err(ServicesError::Feature { feature, line }) => Some((feature, line)),
            Err(other) => panic!("the check only refuses features: {other:?}"),
        }
    }

    #[test]
    fn only_lf_and_crlf_end_a_line() {
        const CR: &str = "a carriage return without a line feed, which YAML reads as a line break";
        const NEL: &str = "a NEL, LS or PS character, which YAML 1.1 reads as a line break";
        assert_eq!(break_in("a: b\r\nc: d\r\n"), None);
        assert_eq!(break_in("a: b\rc: d\n"), Some((CR, 1)));
        assert_eq!(break_in("a: b\nc: d\r"), Some((CR, 2)));
        assert_eq!(break_in("a: b\n\r\nc: d\r\r\n"), Some((CR, 3)));
        assert_eq!(break_in("a: b\nc: \u{2029}\n"), Some((NEL, 2)));
    }

    #[test]
    fn the_walk_refuses_a_merge_key_inside_a_list_and_a_key_of_another_kind() {
        let listed: Value = yaml::from_str("- a\n- {\"<<\": 1}\n").unwrap();
        assert!(matches!(
            decoded(&listed, "the file"),
            Err(ServicesError::MergeKey { at }) if at == "the file[1]"
        ));
        // `1` is a key of its own kind, not the string `"1"`.
        let numbered: Value = yaml::from_str("k:\n  1: a\n").unwrap();
        assert!(matches!(
            decoded(&numbered, "the file"),
            Err(ServicesError::KeyNotString { at }) if at == "the file.k"
        ));
        let plain: Value = yaml::from_str("- a\n- {b: [c], \"1\": d}\n").unwrap();
        assert!(decoded(&plain, "the file").is_ok());
    }
}
