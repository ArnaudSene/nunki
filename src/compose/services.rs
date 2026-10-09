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
//! the key it decodes to.
//!
//! What is lifted is never the file's bytes but nunki's own rendering of the
//! model ([`ServicesFile::render`]): keys in a fixed order, every string
//! quoted and written in ASCII, every `$` doubled so Compose interpolates
//! nothing.

use std::collections::{BTreeMap, BTreeSet};

use serde_yaml_ng::Value;

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
/// dropped. Fixed, nunki's, never the file's: what the official database
/// images' entrypoints need to drop from root to their own user — read
/// their data directory whatever its mode (`DAC_OVERRIDE`), and change user
/// (`SETUID`, `SETGID`).
///
/// Measured by the HQ on Docker Engine 29.4.0 with
/// `live_the_capability_set_is_the_smallest_the_images_need`:
/// `postgres:16-alpine`, `postgres:16` and `questdb/questdb:9.4.3`, healthy
/// with the five the mission expected, then with each taken away in turn:
/// `DAC_OVERRIDE` is needed by both Postgres images, `SETGID` and `SETUID`
/// by all three, `CHOWN` and `FOWNER` by none, so those two are not given.
/// The three together are measured by the same test, which the HQ runs
/// again before the push.
pub const CAPABILITIES: [&str; 3] = ["DAC_OVERRIDE", "SETGID", "SETUID"];

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
    #[error("{at} carries a YAML tag, which a services file may not")]
    Tag { at: String },
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
        plain_yaml(text)?;
        let document: Value =
            serde_yaml_ng::from_str(text).map_err(|e| ServicesError::Yaml(e.to_string()))?;
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    Double,
    Single,
}

/// Refuse anchors, aliases, tags, directives and a second document, which
/// the parser would otherwise resolve silently: an alias expands, `!!str`
/// vanishes, and nobody reading the file sees what the engine was given.
///
/// A scan of the text rather than of the parser's events, which the parser
/// does not expose. Those indicators mean something only where a node
/// starts — at the start of a line, after `- `, `? `, `:`, and inside `[…]`
/// and `{…}` after `[`, `{` and `,` — and quoted scalars, block scalars and
/// comments are skipped as the parser skips them. It errs towards refusing:
/// every `:` is taken as a node start, and so is the start of every line,
/// even one continuing a plain scalar — a file it refuses wrongly is one a
/// human rewrites, a feature it missed would be one nobody sees.
fn plain_yaml(text: &str) -> Result<(), ServicesError> {
    let mut quote: Option<Quote> = None;
    let mut flow = 0usize;
    // Lines more indented than this belong to a block scalar.
    let mut block: Option<usize> = None;
    let mut documents = 0usize;
    let mut content = false;

    for (number, raw) in text.split('\n').enumerate() {
        let line: Vec<char> = raw.strip_suffix('\r').unwrap_or(raw).chars().collect();
        let refuse = |feature| ServicesError::Feature {
            feature,
            line: number + 1,
        };
        let indent = line.iter().take_while(|c| **c == ' ').count();
        let blank = line.iter().all(|c| c.is_whitespace());

        let mut i = 0;
        let mut at_node = true;
        if quote.is_none() {
            if let Some(parent) = block {
                if blank || indent > parent {
                    continue;
                }
                block = None;
            }
            if line.first() == Some(&'%') {
                return Err(refuse("a directive"));
            }
            let marker = |m: &str| {
                let m: Vec<char> = m.chars().collect();
                line.starts_with(&m) && line.get(3).is_none_or(|c| *c == ' ' || *c == '\t')
            };
            if marker("---") {
                if documents > 0 || content {
                    return Err(refuse("a second document"));
                }
                documents += 1;
                i = 3;
            } else if marker("...") {
                return Err(refuse("a document end marker"));
            }
        } else {
            at_node = false;
        }

        // Where the node that owns this line's block scalar starts: the
        // first thing on the line that is not `- ` or `? `. Never less than
        // the parent's indentation, so a line skipped as the scalar's is
        // never one the parser reads as structure.
        let mut entry = None;
        let mut space_before = true;
        while i < line.len() {
            let c = line[i];
            match quote {
                Some(Quote::Double) => {
                    if c == '\\' {
                        i += 2;
                        continue;
                    }
                    if c == '"' {
                        quote = None;
                    }
                    i += 1;
                    continue;
                }
                Some(Quote::Single) => {
                    if c == '\'' {
                        if line.get(i + 1) == Some(&'\'') {
                            i += 2;
                            continue;
                        }
                        quote = None;
                    }
                    i += 1;
                    continue;
                }
                None => {}
            }
            if c == ' ' || c == '\t' {
                space_before = true;
                i += 1;
                continue;
            }
            if c == '#' && space_before {
                break;
            }
            content = true;
            let spaced = line.get(i + 1).is_none_or(|n| *n == ' ' || *n == '\t');
            let opens_entry = (c == '-' || c == '?') && spaced && flow == 0;
            if entry.is_none() && !opens_entry {
                entry = Some(i);
            }
            if at_node {
                match c {
                    '&' => return Err(refuse("an anchor")),
                    '*' => return Err(refuse("an alias")),
                    '!' => return Err(refuse("a tag")),
                    '|' | '>' if flow == 0 => {
                        block = Some(entry.unwrap_or(indent));
                        break;
                    }
                    '"' => {
                        quote = Some(Quote::Double);
                        at_node = false;
                    }
                    '\'' => {
                        quote = Some(Quote::Single);
                        at_node = false;
                    }
                    '-' | '?' if spaced => {}
                    ':' => {}
                    '[' | '{' => flow += 1,
                    ']' | '}' => {
                        flow = flow.saturating_sub(1);
                        at_node = false;
                    }
                    // An empty entry of a flow mapping, `{a: , b: c}`: what
                    // follows still starts a node. Outside brackets a `,`
                    // cannot start a plain scalar, so the parser refuses the
                    // line whatever this says.
                    ',' => {}
                    _ => at_node = false,
                }
            } else {
                match c {
                    ':' => at_node = true,
                    ',' if flow > 0 => at_node = true,
                    // No `[` or `{` here: inside brackets one can only open
                    // a node, which the branch above counts, and outside them
                    // it is a plain scalar's text.
                    ']' | '}' if flow > 0 => flow -= 1,
                    _ => {}
                }
            }
            space_before = false;
            i += 1;
        }
    }
    Ok(())
}

/// Refuse what the parser decoded but the model must not read past: a tag
/// it kept, a merge key, a key that is not a string. Anywhere in the
/// document, so that nothing below a refused key can matter.
fn decoded(value: &Value, at: &str) -> Result<(), ServicesError> {
    match value {
        Value::Tagged(_) => Err(ServicesError::Tag { at: at.to_string() }),
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
                mount: serde_yaml_ng::to_string(item)
                    .map(|s| s.trim_end().to_string())
                    .unwrap_or_default(),
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
    //! The scan of [`plain_yaml`] and the walk of [`decoded`], branch by
    //! branch. Through [`ServicesFile::parse`] most of them are masked: a
    //! scan that missed an indicator on a line the parser then refuses for
    //! another reason reads the same. Here each case is one the scan alone
    //! decides, and each says which way.

    use super::*;

    fn feature(text: &str) -> Option<&'static str> {
        match plain_yaml(text) {
            Ok(()) => None,
            Err(ServicesError::Feature { feature, .. }) => Some(feature),
            Err(other) => panic!("the scan only refuses features: {other:?}"),
        }
    }

    #[test]
    fn a_document_marker_is_three_characters_then_a_blank() {
        // `--- ` and `---<tab>` open a document, and a node starts after them.
        assert_eq!(feature("--- &a x\n"), Some("an anchor"));
        assert_eq!(feature("---\t&a x\n"), Some("an anchor"));
        // `---y` is a plain scalar, not a marker: what refuses this line is
        // the anchor after its `:`, not a second document.
        assert_eq!(feature("x: 1\n---y: &a 2\n"), Some("an anchor"));
    }

    #[test]
    fn a_second_marker_is_a_second_document_even_with_nothing_between() {
        assert_eq!(feature("---\n---\nk: v\n"), Some("a second document"));
        assert_eq!(feature("---\nk: v\n"), None);
    }

    #[test]
    fn an_escape_in_double_quotes_skips_one_character_and_no_more() {
        // `"\\"` closes after the escaped backslash; the anchor after it is
        // at a node.
        assert_eq!(feature("k: [\"\\\\\", &a x]\n"), Some("an anchor"));
        // And what is inside the quotes is text: `: &b` there starts nothing.
        assert_eq!(feature("k: \"a: &b\"\n"), None);
    }

    #[test]
    fn single_quotes_close_on_a_lone_quote_and_not_on_a_doubled_one() {
        assert_eq!(feature("k: 'a: &b'\n"), None);
        assert_eq!(feature("k: ['a', &b x]\n"), Some("an anchor"));
        // `''` is a quote inside the string, so `: &c` is still text.
        assert_eq!(feature("k: 'b'': &c'\n"), None);
        // `'b'''` is the string `b'`, closed after the doubled quote.
        assert_eq!(feature("k: ['b''', &c x]\n"), Some("an anchor"));
    }

    #[test]
    fn a_dash_starts_an_entry_only_before_a_blank() {
        assert_eq!(feature("k: -&a\n"), None);
        assert_eq!(feature("k:\n  - &a x\n"), Some("an anchor"));
        assert_eq!(feature("k:\n  -\t&a x\n"), Some("an anchor"));
    }

    #[test]
    fn a_block_scalars_lines_end_where_its_owner_starts() {
        // The owner of `|` is the key `a b`, at column 4 — not `b`, which a
        // blank after `a` does not make an entry. Its text, at column 6, is
        // text.
        assert_eq!(feature("k:\n  - a b: |\n      &x\n"), None);
    }

    #[test]
    fn a_bar_inside_brackets_is_not_a_block_scalar() {
        assert_eq!(feature("k: [>, &a x]\n"), Some("an anchor"));
    }

    #[test]
    fn a_colon_at_the_start_of_a_node_starts_another() {
        assert_eq!(feature("? a\n: &b c\n"), Some("an anchor"));
    }

    #[test]
    fn brackets_are_counted_open_and_closed() {
        // Inside brackets, a `,` starts a node.
        assert_eq!(feature("k: [a, &b c]\n"), Some("an anchor"));
        assert_eq!(feature("k: {a: ,&c d}\n"), Some("an anchor"));
        // Closed, at a node or after one, they count for nothing: in block
        // context `a, &b` is one plain scalar.
        assert_eq!(feature("k: []\nm: a, &b\n"), None);
        assert_eq!(feature("k: [a]\nm: a, &b\n"), None);
        assert_eq!(feature("k: [[a]]\nm: a, &b\n"), None);
        // And text never opens one.
        assert_eq!(feature("m: a[1], &b\n"), None);
    }

    #[test]
    fn the_walk_refuses_a_tag_the_parser_kept_and_walks_into_lists() {
        let tagged: Value = serde_yaml_ng::from_str("!custom 5").unwrap();
        assert!(matches!(
            decoded(&tagged, "here"),
            Err(ServicesError::Tag { at }) if at == "here"
        ));
        let listed: Value = serde_yaml_ng::from_str("- a\n- {\"<<\": 1}\n").unwrap();
        assert!(matches!(
            decoded(&listed, "the file"),
            Err(ServicesError::MergeKey { at }) if at == "the file[1]"
        ));
        let plain: Value = serde_yaml_ng::from_str("- a\n- {b: [c]}\n").unwrap();
        assert!(decoded(&plain, "the file").is_ok());
    }
}
