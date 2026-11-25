//! The versions a repository pins, read for its stack's image (SPEC 4.2,
//! "les fragments de stack", `versions.txt`).
//!
//! A stack's Dockerfile carries defaults — `RUST_VERSION=stable`, a Python
//! base tag, a pnpm release. A repository that pins something else asks its
//! toolchain for a version the image does not carry, and the toolchain goes
//! to fetch it from behind a firewall that names none of its hosts — say a
//! `rust-toolchain.toml` that pins `1.98.0` and the `wasm32-unknown-unknown`
//! target, in an image that carries `stable`.
//!
//! Reading `rust-toolchain.toml` or `package.json` here would put the stacks
//! back in the core (SPEC 3.2). So the fragment says **where** a version is,
//! one line per build argument in `versions.txt`, and this module knows file
//! formats — plain text, TOML, JSON — and nothing about any stack:
//!
//! ```text
//! # ARG           file                  key                 pattern
//! RUST_VERSION    rust-toolchain.toml   toolchain.channel
//! PNPM            package.json          packageManager      pnpm@{}
//! NODE            .nvmrc                -                   v{}
//! require: RUST_VERSION NODE
//! ```
//!
//! The `require:` line says which arguments the repository **must** pin: one
//! left to the Dockerfile's default is a toolchain nobody chose, and one that
//! changes at each rebuild. Which ones is the fragment's to say, never the
//! core's (SPEC 3.2). A colon cannot appear in an argument's name, so no
//! source line is ever read as one.
//!
//! Read at `nunki slot rebuild`, never at `nunki init`: a fragment never
//! updates itself, so a version frozen into it at init would be stale the
//! first time the repository moved. What an image was built with is written
//! on it as the [`LABEL`], so a later reading can say whether it still serves.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The image label that records what an image was built with.
pub const LABEL: &str = "nunki.versions";

/// Where a stack fragment says where its versions are.
pub const FILE: &str = "versions.txt";

/// One line of `versions.txt`: a build argument, and one place it may be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub arg: String,
    /// Relative to the repository's root.
    pub file: String,
    /// A dotted path into a TOML or JSON file; `None` for the whole of a
    /// plain-text file.
    pub key: Option<String>,
    /// What surrounds the version in the value, split at `{}`.
    pub pattern: Option<(String, String)>,
}

impl Source {
    /// Where this source reads, as a human names it.
    pub fn place(&self) -> String {
        match &self.key {
            Some(key) => format!("{} {key}", self.file),
            None => self.file.clone(),
        }
    }
}

/// A whole `versions.txt`: where each argument is read, and which arguments
/// the repository must pin.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Versions {
    pub sources: Vec<Source>,
    /// The arguments its `require:` lines name, in the order written; `None`
    /// for a file with no `require:` line at all — one written before the
    /// line existed, which says nothing about what must be pinned.
    pub required: Option<Vec<String>>,
}

impl Versions {
    /// Whether the repository must pin `arg`.
    pub fn requires(&self, arg: &str) -> bool {
        self.required
            .as_ref()
            .is_some_and(|r| r.iter().any(|a| a == arg))
    }
}

/// The directive that names the arguments a repository must pin.
pub const REQUIRE: &str = "require:";

#[derive(Debug, thiserror::Error)]
pub enum SourcesError {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{file}, line {line}: {why} — the line is `ARG file [key|-] [pattern]`")]
    Line {
        file: PathBuf,
        line: usize,
        why: String,
    },
    #[error("{file}, line {line}: {why} — the line is `require: ARG [ARG…]`")]
    Require {
        file: PathBuf,
        line: usize,
        why: String,
    },
}

fn is_name(arg: &str) -> bool {
    let mut chars = arg.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse a `versions.txt`. A line nunki cannot read is an error that names
/// it, never a line skipped: a skipped line is a version silently not read.
///
/// A `require:` line may come before the sources it names, so what it names
/// is checked once the whole file is read — and an argument no source line
/// names is an error on that `require:` line, since nothing could pin it.
pub fn parse(text: &str, file: &Path) -> Result<Versions, SourcesError> {
    let mut sources = Vec::new();
    let mut required: Option<Vec<(String, usize)>> = None;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = |why: &str| SourcesError::Line {
            file: file.to_path_buf(),
            line: n + 1,
            why: why.to_string(),
        };
        if let Some(rest) = line.strip_prefix(REQUIRE) {
            let named = required.get_or_insert_with(Vec::new);
            for arg in rest.split_whitespace() {
                if !is_name(arg) {
                    return Err(SourcesError::Require {
                        file: file.to_path_buf(),
                        line: n + 1,
                        why: format!("{arg:?} is not a build argument's name"),
                    });
                }
                if !named.iter().any(|(a, _)| a == arg) {
                    named.push((arg.to_string(), n + 1));
                }
            }
            continue;
        }
        let columns: Vec<&str> = line.split_whitespace().collect();
        let (arg, path, key, pattern) = match columns.as_slice() {
            [arg, path] => (*arg, *path, None, None),
            [arg, path, key] => (*arg, *path, Some(*key), None),
            [arg, path, key, pattern] => (*arg, *path, Some(*key), Some(*pattern)),
            _ => return Err(bad("expected two to four columns")),
        };
        if !is_name(arg) {
            return Err(bad(&format!("{arg:?} is not a build argument's name")));
        }
        // Inside the repository, or it is not the repository pinning it.
        if Path::new(path).is_absolute() || path.split('/').any(|s| s == "..") {
            return Err(bad(&format!(
                "{path:?} is not a path inside the repository"
            )));
        }
        let pattern = match pattern {
            None => None,
            Some(p) => match p.split_once("{}") {
                Some((before, after)) if !after.contains("{}") => {
                    Some((before.to_string(), after.to_string()))
                }
                _ => {
                    return Err(bad(&format!(
                        "the pattern {p:?} must hold `{{}}` exactly once"
                    )));
                }
            },
        };
        sources.push(Source {
            arg: arg.to_string(),
            file: path.to_string(),
            key: key.filter(|k| *k != "-").map(str::to_string),
            pattern,
        });
    }
    let required = match required {
        None => None,
        Some(named) => {
            for (arg, line) in &named {
                if !sources.iter().any(|s| &s.arg == arg) {
                    return Err(SourcesError::Require {
                        file: file.to_path_buf(),
                        line: *line,
                        why: format!(
                            "{arg} is required, but no line says where the repository pins it"
                        ),
                    });
                }
            }
            Some(named.into_iter().map(|(arg, _)| arg).collect())
        }
    };
    Ok(Versions { sources, required })
}

/// What the repository says about one build argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pin {
    /// Pinned, by the first source that gave an exact version.
    Found {
        arg: String,
        value: String,
        from: String,
    },
    /// No source gave an exact version. Each source tried, and why it gave
    /// nothing. For an optional argument the Dockerfile's default stands; a
    /// required one stops every launch ([`unmet`]).
    Unpinned { arg: String, tried: Vec<String> },
}

impl Pin {
    pub fn arg(&self) -> &str {
        match self {
            Pin::Found { arg, .. } | Pin::Unpinned { arg, .. } => arg,
        }
    }
}

/// Read every build argument the sources name, from the stack's directory
/// `root` inside the repository's `tree` — the project's root, or a slot's
/// clone of it. Sources for one argument are tried in the order they are written,
/// and the first exact version wins — so `.nvmrc` can come before
/// `engines.node`.
///
/// For a required argument a version must also not name a channel that
/// moves ([`moving_channel`]): `stable`, `lts` or `latest`, with or without a
/// host triple after them, are refused the way a range is, so the next
/// source is tried.
///
/// A source that leads outside `tree` gives nothing (SPEC 4.2): the tree is
/// one the agents write, and a link in it must not make the host read, or
/// print, a file of its own ([`inside`]).
pub fn resolve(tree: &Path, root: &Path, versions: &Versions) -> Vec<Pin> {
    // Resolved once, the way each source is: a tree reached through a link
    // (`/tmp` on macOS) must still hold the files resolved inside it.
    let tree = std::fs::canonicalize(tree)
        .map_err(|e| format!("the tree {} could not be resolved ({e})", tree.display()));
    let sources = &versions.sources;
    let mut order: Vec<&str> = Vec::new();
    for s in sources {
        if !order.contains(&s.arg.as_str()) {
            order.push(&s.arg);
        }
    }
    order
        .into_iter()
        .map(|arg| {
            let mut tried = Vec::new();
            let required = versions.requires(arg);
            for source in sources.iter().filter(|s| s.arg == arg) {
                match read(&tree, root, source) {
                    Ok(value) if required && moving_channel(&value) => tried.push(format!(
                        "{}: {value:?} names a channel that moves, not a version",
                        source.place()
                    )),
                    Ok(value) => {
                        return Pin::Found {
                            arg: arg.to_string(),
                            value,
                            from: source.place(),
                        };
                    }
                    Err(why) => tried.push(format!("{}: {why}", source.place())),
                }
            }
            Pin::Unpinned {
                arg: arg.to_string(),
                tried,
            }
        })
        .collect()
}

/// Whether a value names a channel that moves rather than a version (SPEC
/// 4.2, "Une version requise contient un chiffre").
///
/// The value is cut at its first `-`. A first part that holds a digit is a
/// version — `22`, `1.89.0`, `1.89.0-x86_64-unknown-linux-gnu`. One that
/// holds none is a channel, and a channel is pinned only when a date written
/// `YYYY-MM-DD` follows it, alone or before a further `-`. Anything else after
/// it — a host triple, a codename — still follows the channel wherever it
/// moves, which is what a required argument exists to refuse.
///
/// The rule is about the shape of a value: it names no channel and no host
/// triple, and knows no stack (SPEC 3.2). Whether the date's day exists is
/// not checked.
fn moving_channel(value: &str) -> bool {
    let (head, rest) = match value.split_once('-') {
        Some((head, rest)) => (head, Some(rest)),
        None => (value, None),
    };
    if head.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    !rest.is_some_and(dated)
}

/// Whether `rest` opens on a date written `YYYY-MM-DD`, followed by nothing
/// or by a `-` and whatever comes after it.
fn dated(rest: &str) -> bool {
    const SHAPE: &[u8] = b"dddd-dd-dd";
    let bytes = rest.as_bytes();
    bytes.len() >= SHAPE.len()
        && SHAPE.iter().zip(bytes).all(|(want, got)| match want {
            b'd' => got.is_ascii_digit(),
            other => got == other,
        })
        && matches!(bytes.get(SHAPE.len()), None | Some(b'-'))
}

/// Every required argument the repository left without an exact version, one
/// line each: every source tried and why it gave nothing, then what to add,
/// taken from the fragment's own source lines. Empty when every required
/// argument is pinned.
pub fn unmet(versions: &Versions, pins: &[Pin]) -> Vec<String> {
    pins.iter()
        .filter_map(|p| match p {
            Pin::Unpinned { arg, tried } if versions.requires(arg) => {
                let mut places: Vec<String> = Vec::new();
                for s in versions.sources.iter().filter(|s| &s.arg == arg) {
                    let place = match &s.key {
                        Some(key) => format!("{} ({key})", s.file),
                        None => s.file.clone(),
                    };
                    if !places.contains(&place) {
                        places.push(place);
                    }
                }
                Some(format!(
                    "{arg}: nothing exact ({}); pin an exact version in one of {}",
                    tried.join("; "),
                    places.join(", ")
                ))
            }
            _ => None,
        })
        .collect()
}

/// The pinned arguments alone, as the image label records them.
pub fn pinned(pins: &[Pin]) -> BTreeMap<String, String> {
    pins.iter()
        .filter_map(|p| match p {
            Pin::Found { arg, value, .. } => Some((arg.clone(), value.clone())),
            Pin::Unpinned { .. } => None,
        })
        .collect()
}

/// The label's value: `ARG=value` pairs, `;`-separated. A version never holds
/// either character — [`exact`] refuses them.
pub fn label(pinned: &BTreeMap<String, String>) -> String {
    pinned
        .iter()
        .map(|(arg, value)| format!("{arg}={value}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// The label read back.
pub fn parse_label(text: &str) -> BTreeMap<String, String> {
    text.split(';')
        .filter_map(|pair| pair.split_once('='))
        .map(|(arg, value)| (arg.to_string(), value.to_string()))
        .collect()
}

/// Where what an image was built with differs from what the repository pins
/// now, one line per argument; empty when the image serves. An argument the
/// repository no longer pins counts too: the image carries a version the
/// Dockerfile's default would not have given.
pub fn drift(built: &BTreeMap<String, String>, wanted: &BTreeMap<String, String>) -> Vec<String> {
    let args: BTreeSet<&String> = built.keys().chain(wanted.keys()).collect();
    args.into_iter()
        .filter_map(|arg| match (built.get(arg), wanted.get(arg)) {
            (b, w) if b == w => None,
            (b, w) => Some(format!(
                "{arg}: the image has {}, the repository pins {}",
                b.map_or("the Dockerfile's default".to_string(), |v| format!("{v:?}")),
                w.map_or("nothing".to_string(), |v| format!("{v:?}")),
            )),
        })
        .collect()
}

/// The build arguments a Dockerfile declares, in any stage.
pub fn declared(dockerfile: &str) -> BTreeSet<String> {
    dockerfile
        .lines()
        .map(str::trim)
        .filter_map(|l| {
            let (word, rest) = l.split_once(char::is_whitespace)?;
            word.eq_ignore_ascii_case("ARG").then_some(rest)
        })
        .flat_map(str::split_whitespace)
        .map(|a| a.split('=').next().unwrap_or(a).to_string())
        .collect()
}

/// The arguments `versions.txt` names that the Dockerfile does not declare. A
/// build argument nobody declares is dropped by the engine with a warning
/// nobody reads, and the image quietly keeps its default — which is the
/// failure this whole module exists to prevent.
pub fn undeclared(sources: &[Source], dockerfile: &str) -> Vec<String> {
    let declared = declared(dockerfile);
    let mut missing: Vec<String> = sources
        .iter()
        .map(|s| s.arg.clone())
        .filter(|a| !declared.contains(a))
        .collect();
    missing.dedup();
    missing
}

/// One source's value, or why it gave none.
fn read(tree: &Result<PathBuf, String>, root: &Path, source: &Source) -> Result<String, String> {
    let path = inside(tree, &root.join(&source.file))?;
    let text = regular_file(&path, SOURCE_LIMIT)?;
    let items = match (format(&source.file), &source.key) {
        (Format::Plain, None) => {
            // The version is on the first line that opens as the pattern
            // does. A lockfile names every package it resolved, one per line
            // (`'@playwright/test@1.61.1':` in a `pnpm-lock.yaml`), and the
            // one wanted is rarely first. A pattern that opens on the version
            // itself, or no pattern, opens as every line does: the first line
            // is then the whole value, as in `.nvmrc`.
            let before = source
                .pattern
                .as_ref()
                .map_or("", |(before, _)| before.as_str());
            let line = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .find(|l| l.starts_with(before))
                .ok_or_else(|| match before {
                    "" => "empty".to_string(),
                    before => format!("no line starts with {before:?}"),
                })?;
            vec![line.to_string()]
        }
        (Format::Plain, Some(_)) => {
            return Err("a key was given, but only a .toml or .json file has keys".to_string());
        }
        (_, None) => return Err("a .toml or .json file needs a key".to_string()),
        (Format::Toml, Some(key)) => {
            let table: toml::Table = text.parse().map_err(|e| format!("not TOML ({e})"))?;
            let value = walk_toml(&toml::Value::Table(table), key)?;
            toml_items(&value)?
        }
        (Format::Json, Some(key)) => {
            let doc: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| format!("not JSON ({e})"))?;
            let value = walk_json(&doc, key)?;
            json_items(value)?
        }
    };
    let versions = items
        .iter()
        .map(|item| extract(item, source.pattern.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(versions.join(","))
}

/// Where `path` really is, every link along it followed — or why it is not
/// read. A link at any component counts, not only the last, and the tree is
/// resolved the same way.
///
/// A path that lands outside `tree` gives nothing: the reason names neither
/// where it leads nor anything found there, since the source's own place is
/// all the caller prefixes it with. A link that stays inside the tree is
/// followed — a `frontend/.nvmrc` that links to `../.nvmrc` is a choice a
/// repository may make.
///
/// The file then read is the resolved path, so no link is followed a second
/// time between this check and the read.
fn inside(tree: &Result<PathBuf, String>, path: &Path) -> Result<PathBuf, String> {
    let tree = tree.as_ref().map_err(Clone::clone)?;
    let real = match std::fs::canonicalize(path) {
        Ok(real) => real,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("absent".to_string()),
        Err(e) => return Err(format!("could not be resolved ({e})")),
    };
    if real.starts_with(tree) {
        Ok(real)
    } else {
        Err("leads outside the tree, and is not read".to_string())
    }
}

/// The most a version source may hold. A `pnpm-lock.yaml` is a source, and
/// the largest real one measured held 183,116 bytes for 625 packages (about
/// 300 bytes each): this is room for some 200,000 packages, and still a
/// read the host survives.
pub const SOURCE_LIMIT: u64 = 64 << 20;

// Far above the largest real lockfile measured: a limit lowered under a
// hundred of them does not build.
const _: () = assert!(SOURCE_LIMIT >= 100 * 183_116);

/// The text of the source at `path`, already resolved by [`inside`], or why
/// it gives none (SPEC 4.2).
///
/// Only a regular file is read. A FIFO, a socket, a device or a directory
/// gives nothing, and this is asked before the file is opened: opening a
/// FIFO blocks until something writes into it, so a check after the open
/// would come too late. The look does not follow a link, so a link set
/// down on the resolved path since [`inside`] gives nothing either.
///
/// Never more than `limit` and one byte is read, however large the file is
/// or grows to; a larger one gives nothing.
fn regular_file(path: &Path, limit: u64) -> Result<String, String> {
    use std::io::Read as _;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("absent".to_string()),
        Err(e) => return Err(format!("could not be read ({e})")),
    };
    if !meta.is_file() {
        return Err(format!(
            "is {}, not a regular file, and is not read",
            kind(&meta.file_type())
        ));
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(limit + 1).read_to_string(&mut text))
        .map_err(|e| format!("could not be read ({e})"))?;
    if text.len() as u64 > limit {
        return Err(format!(
            "holds more than {limit} bytes, more than any version source, and is not read"
        ));
    }
    Ok(text)
}

/// What a file that is not a regular file is, in words.
fn kind(file: &std::fs::FileType) -> &'static str {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt as _;
        if file.is_fifo() {
            return "a FIFO";
        }
        if file.is_socket() {
            return "a socket";
        }
        if file.is_block_device() || file.is_char_device() {
            return "a device";
        }
    }
    if file.is_dir() {
        "a directory"
    } else if file.is_symlink() {
        "a link"
    } else {
        "something else"
    }
}

enum Format {
    Plain,
    Toml,
    Json,
}

fn format(file: &str) -> Format {
    if file.ends_with(".toml") {
        Format::Toml
    } else if file.ends_with(".json") {
        Format::Json
    } else {
        Format::Plain
    }
}

fn walk_toml(root: &toml::Value, key: &str) -> Result<toml::Value, String> {
    let mut at = root;
    for part in key.split('.') {
        at = at.get(part).ok_or_else(|| format!("no {key}"))?;
    }
    Ok(at.clone())
}

fn toml_items(value: &toml::Value) -> Result<Vec<String>, String> {
    match value {
        toml::Value::String(s) => Ok(vec![s.clone()]),
        toml::Value::Integer(i) => Ok(vec![i.to_string()]),
        toml::Value::Array(items) => items
            .iter()
            .map(|i| match i {
                toml::Value::String(s) => Ok(s.clone()),
                other => Err(format!("{other} in a list is not a version")),
            })
            .collect(),
        other => Err(format!("{other} is not a version")),
    }
}

fn walk_json<'a>(root: &'a serde_json::Value, key: &str) -> Result<&'a serde_json::Value, String> {
    let mut at = root;
    for part in key.split('.') {
        at = at.get(part).ok_or_else(|| format!("no {key}"))?;
    }
    Ok(at)
}

fn json_items(value: &serde_json::Value) -> Result<Vec<String>, String> {
    use serde_json::Value;
    match value {
        Value::String(s) => Ok(vec![s.clone()]),
        Value::Number(n) => Ok(vec![n.to_string()]),
        Value::Array(items) => items
            .iter()
            .map(|i| match i {
                Value::String(s) => Ok(s.clone()),
                other => Err(format!("{other} in a list is not a version")),
            })
            .collect(),
        other => Err(format!("{other} is not a version")),
    }
}

/// The version inside one value, through the pattern when there is one.
///
/// Only an **exact** version is taken: `>=20`, `^3.12` or `lts/iron` name a
/// range or an alias, and turning one into an image tag is a guess. Refused,
/// the next source is tried. Failing all of them, an optional argument keeps
/// the Dockerfile's default and the build says so; a required one stops every
/// launch ([`unmet`]).
///
/// Behind the pattern's `{}`, what follows the version must be the pattern's
/// own suffix — or, when it has none, nothing but `+` build metadata, the way
/// `packageManager` writes `pnpm@11.15.0+sha512.…`.
fn extract(value: &str, pattern: Option<&(String, String)>) -> Result<String, String> {
    let value = value.trim();
    let Some((before, after)) = pattern else {
        return exact(value)
            .then(|| value.to_string())
            .ok_or_else(|| format!("{value:?} is not an exact version"));
    };
    let rest = value
        .strip_prefix(before.as_str())
        .ok_or_else(|| format!("{value:?} does not start with {before:?}"))?;
    let end = rest.find(|c: char| !token(c)).unwrap_or(rest.len());
    let (version, tail) = rest.split_at(end);
    let tail_fits = if after.is_empty() {
        tail.is_empty() || tail.starts_with('+')
    } else {
        tail == after
    };
    if version.is_empty() || !tail_fits {
        return Err(format!("{value:?} is not an exact version"));
    }
    Ok(version.to_string())
}

fn token(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

fn exact(value: &str) -> bool {
    !value.is_empty() && value.chars().all(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(text: &str) -> Versions {
        parse(text, Path::new("versions.txt")).expect("the fixture parses")
    }

    fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("the fixture is written");
        }
        dir
    }

    #[test]
    fn a_toolchain_file_pins_the_channel_the_components_and_the_targets() {
        let dir = repo(&[(
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"1.98.0\"\ncomponents = [\"rustfmt\", \"clippy\"]\n\
             targets = [\"wasm32-unknown-unknown\"]\n",
        )]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources(
                "RUST_VERSION rust-toolchain.toml toolchain.channel\n\
                 RUST_COMPONENTS rust-toolchain.toml toolchain.components\n\
                 RUST_TARGETS rust-toolchain.toml toolchain.targets\n",
            ),
        );
        let got = pinned(&pins);
        assert_eq!(got["RUST_VERSION"], "1.98.0");
        assert_eq!(got["RUST_COMPONENTS"], "rustfmt,clippy");
        assert_eq!(got["RUST_TARGETS"], "wasm32-unknown-unknown");
    }

    #[test]
    fn a_range_is_not_a_version_and_the_next_source_is_tried() {
        let dir = repo(&[
            ("package.json", r#"{"engines": {"node": ">=20"}}"#),
            (".nvmrc", "v22.3.0\n"),
        ]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources("NODE package.json engines.node\nNODE .nvmrc - v{}\n"),
        );
        assert_eq!(
            pins,
            vec![Pin::Found {
                arg: "NODE".into(),
                value: "22.3.0".into(),
                from: ".nvmrc".into()
            }]
        );
    }

    #[test]
    fn the_first_exact_source_wins_even_when_a_later_one_also_pins() {
        let dir = repo(&[(".nvmrc", "22\n"), (".node-version", "24\n")]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources("NODE .nvmrc\nNODE .node-version\n"),
        );
        assert_eq!(pinned(&pins)["NODE"], "22");
    }

    #[test]
    fn nothing_exact_leaves_the_argument_unpinned_and_says_why_for_each_source() {
        let dir = repo(&[("package.json", r#"{"engines": {"node": "^20"}}"#)]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources("NODE .nvmrc\nNODE package.json engines.node\n"),
        );
        let Pin::Unpinned { tried, .. } = &pins[0] else {
            panic!("expected nothing pinned, got {pins:?}");
        };
        assert_eq!(tried.len(), 2);
        assert!(tried[0].contains("absent"), "{tried:?}");
        assert!(tried[1].contains("not an exact version"), "{tried:?}");
        assert!(pinned(&pins).is_empty());
    }

    #[test]
    fn a_package_manager_field_gives_its_version_without_the_hash() {
        let dir = repo(&[(
            "package.json",
            r#"{"packageManager": "pnpm@11.15.0+sha512.abcdef"}"#,
        )]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources("PNPM package.json packageManager pnpm@{}\n"),
        );
        assert_eq!(pinned(&pins)["PNPM"], "11.15.0");
    }

    #[test]
    fn another_package_manager_is_not_read_as_pnpm() {
        let dir = repo(&[("package.json", r#"{"packageManager": "yarn@4.1.0"}"#)]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources("PNPM package.json packageManager pnpm@{}\n"),
        );
        assert!(pinned(&pins).is_empty(), "{pins:?}");
    }

    #[test]
    fn a_plain_file_gives_its_first_line_and_skips_comments() {
        let dir = repo(&[(".python-version", "# pinned for the lab\n3.13\n")]);
        let pins = resolve(dir.path(), dir.path(), &sources("PYTHON .python-version\n"));
        assert_eq!(pinned(&pins)["PYTHON"], "3.13");
    }

    /// A lockfile names every package it resolved, one per line, and the
    /// one wanted is rarely first: with a pattern that opens on text of its
    /// own, the version is on the first line that opens the same way.
    #[test]
    fn a_pattern_finds_its_line_in_a_lockfile() {
        let lock = "lockfileVersion: '9.0'\n\
                    importers:\n  .:\n    devDependencies:\n      '@playwright/test':\n\
                    \x20       specifier: ^1.51.1\n        version: 1.61.1\n\
                    packages:\n  '@babel/core@7.29.0':\n    resolution: {}\n\
                    \x20 '@playwright/test@1.61.1':\n    resolution: {}\n\
                    \x20 playwright@1.61.1:\n";
        let dir = repo(&[("pnpm-lock.yaml", lock)]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources("PLAYWRIGHT pnpm-lock.yaml - '@playwright/test@{}':\n"),
        );
        assert_eq!(pinned(&pins)["PLAYWRIGHT"], "1.61.1", "{pins:?}");
    }

    /// And a lockfile that resolved no such package pins nothing: the
    /// Dockerfile's default stands, and the reason names the pattern.
    #[test]
    fn a_pattern_no_line_opens_with_pins_nothing_and_says_so() {
        let dir = repo(&[(
            "pnpm-lock.yaml",
            "lockfileVersion: '9.0'\npackages:\n  next@16.3.3:\n",
        )]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources("PLAYWRIGHT pnpm-lock.yaml - '@playwright/test@{}':\n"),
        );
        assert!(pinned(&pins).is_empty(), "{pins:?}");
        match &pins[0] {
            Pin::Unpinned { tried, .. } => assert!(
                tried.iter().any(|t| t.contains("no line starts with")),
                "{tried:?}"
            ),
            other => panic!("pinned: {other:?}"),
        }
    }

    /// A pattern that opens on the version itself still reads the first
    /// line, as for a `.nvmrc`: searching would take any line at all.
    #[test]
    fn a_pattern_that_opens_on_the_version_reads_the_first_line() {
        let dir = repo(&[(".python-version", "3.13\n3.12\n")]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources("PYTHON .python-version - {}\n"),
        );
        assert_eq!(pinned(&pins)["PYTHON"], "3.13", "{pins:?}");
    }

    #[test]
    fn a_key_into_a_plain_file_is_refused_rather_than_ignored() {
        let dir = repo(&[(".python-version", "3.13\n")]);
        let pins = resolve(
            dir.path(),
            dir.path(),
            &sources("PYTHON .python-version version\n"),
        );
        assert!(pinned(&pins).is_empty());
    }

    #[test]
    fn a_line_nunki_cannot_read_is_an_error_naming_it() {
        for bad in [
            "RUST_VERSION\n",
            "1ARG file\n",
            "ARG /etc/passwd\n",
            "ARG ../outside\n",
            "ARG f.json key no-placeholder\n",
            "ARG f.json key {}{}\n",
            "ARG f.json key pattern extra\n",
        ] {
            let err = parse(bad, Path::new("versions.txt")).expect_err(bad);
            assert!(err.to_string().contains("line 1"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn the_label_reads_back_as_it_was_written() {
        let map: BTreeMap<String, String> = [
            ("RUST_VERSION".to_string(), "1.98.0".to_string()),
            (
                "RUST_TARGETS".to_string(),
                "wasm32-unknown-unknown,x86_64".to_string(),
            ),
        ]
        .into();
        assert_eq!(parse_label(&label(&map)), map);
        assert!(parse_label("").is_empty());
    }

    #[test]
    fn drift_names_every_argument_that_moved_and_nothing_else() {
        let built = parse_label("RUST_VERSION=1.97.0;RUST_TARGETS=wasm32-unknown-unknown");
        let wanted = parse_label(
            "RUST_VERSION=1.98.0;RUST_COMPONENTS=rust-src;RUST_TARGETS=wasm32-unknown-unknown",
        );
        let d = drift(&built, &wanted);
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d[0].starts_with("RUST_COMPONENTS: the image has the Dockerfile's default"));
        assert!(d[1].contains("\"1.97.0\"") && d[1].contains("\"1.98.0\""));
        assert!(drift(&wanted, &wanted).is_empty());
    }

    #[test]
    fn a_dockerfile_declares_its_arguments_in_every_stage_and_form() {
        let d = declared(
            "ARG PYTHON=3.12\nARG BASE=python:${PYTHON}-slim\nFROM ${BASE}\n  arg UID GID=1000\n\
             RUN echo ARG NOT_ONE\n",
        );
        let want: BTreeSet<String> = ["PYTHON", "BASE", "UID", "GID"].map(String::from).into();
        assert_eq!(d, want);
        assert_eq!(
            undeclared(
                &sources("PYTHON .python-version\nNODE .nvmrc\n").sources,
                "ARG PYTHON\n"
            ),
            vec!["NODE".to_string()]
        );
    }

    /// Every line written before `require:` existed reads as it did, and says
    /// nothing about what must be pinned — which is not the same as saying
    /// nothing must be.
    #[test]
    fn a_file_without_require_reads_its_sources_and_requires_nothing_it_knows_of() {
        let v = sources("RUST_VERSION rust-toolchain.toml toolchain.channel\n");
        assert_eq!(v.sources.len(), 1);
        assert_eq!(v.required, None);
        assert!(!v.requires("RUST_VERSION"));
    }

    /// A `require:` line names arguments, wherever it sits and however many
    /// there are; an argument named twice is required once.
    #[test]
    fn require_lines_name_the_arguments_that_must_be_pinned() {
        let v = sources(
            "require: NODE\nNODE .nvmrc\nPNPM package.json packageManager pnpm@{}\n\
             PLAYWRIGHT pnpm-lock.yaml\nrequire:PNPM NODE\n",
        );
        assert_eq!(v.sources.len(), 3, "{v:?}");
        assert_eq!(
            v.required,
            Some(vec!["NODE".to_string(), "PNPM".to_string()])
        );
        assert!(v.requires("NODE") && v.requires("PNPM"));
        assert!(!v.requires("PLAYWRIGHT"));
        // Written, and empty: a fragment that requires nothing, and says so.
        assert_eq!(sources("require:\nA f\n").required, Some(vec![]));
    }

    /// A required argument no line reads could never be pinned: an error on
    /// the `require:` line, naming the file and the line.
    #[test]
    fn a_required_argument_no_line_reads_is_an_error_naming_the_line() {
        let err = parse(
            "# header\nNODE .nvmrc\nrequire: NODE PNPM\n",
            Path::new("/frag/versions.txt"),
        )
        .unwrap_err();
        let said = err.to_string();
        assert!(
            said.starts_with("/frag/versions.txt, line 3: PNPM is required"),
            "{said}"
        );
        assert!(said.contains("`require: ARG [ARG…]`"), "{said}");
        let err = parse("require: 1NODE\nNODE .nvmrc\n", Path::new("v.txt")).unwrap_err();
        assert!(
            err.to_string().starts_with("v.txt, line 1: \"1NODE\""),
            "{err}"
        );
    }

    /// A required argument needs a digit: a channel name moves under the
    /// image. Refused the way a range is, so the next source is tried.
    #[test]
    fn a_required_argument_refuses_a_channel_and_accepts_a_version() {
        for channel in ["stable", "beta", "nightly", "lts", "latest"] {
            let dir = repo(&[("v", &format!("{channel}\n"))]);
            let pins = resolve(dir.path(), dir.path(), &sources("require: A\nA v\n"));
            match &pins[0] {
                Pin::Unpinned { tried, .. } => assert_eq!(
                    tried,
                    &[format!(
                        "v: \"{channel}\" names a channel that moves, not a version"
                    )]
                ),
                other => panic!("{channel}: {other:?}"),
            }
        }
        for version in ["1.99", "22", "3.12", "nightly-2026-11-01"] {
            let dir = repo(&[("v", &format!("{version}\n"))]);
            let pins = resolve(dir.path(), dir.path(), &sources("require: A\nA v\n"));
            assert_eq!(pinned(&pins)["A"], version, "{pins:?}");
        }
        // The channel refused, the next source is read.
        let dir = repo(&[("v", "stable\n"), ("w", "1.98.0\n")]);
        let pins = resolve(dir.path(), dir.path(), &sources("require: A\nA v\nA w\n"));
        assert_eq!(pinned(&pins)["A"], "1.98.0", "{pins:?}");
    }

    /// What one required value resolves to, alone in its source: the value
    /// pinned, or the reasons it was not.
    fn required(value: &str) -> Result<String, Vec<String>> {
        let dir = repo(&[("v", &format!("{value}\n"))]);
        match resolve(dir.path(), dir.path(), &sources("require: A\nA v\n")).remove(0) {
            Pin::Found { value, .. } => Ok(value),
            Pin::Unpinned { tried, .. } => Err(tried),
        }
    }

    /// The value is cut at its first `-`: a first part with a digit is a
    /// version, whatever follows; one without is a channel, pinned only by a
    /// date after it. A host triple or a codename after a channel still
    /// follows the channel, and is refused the way the bare channel is.
    #[test]
    fn a_channel_followed_by_anything_but_a_date_is_refused() {
        for channel in [
            "stable",
            "nightly",
            "lts",
            "latest",
            "stable-x86_64-unknown-linux-gnu",
            "beta-aarch64-apple-darwin",
            "lts-iron",
        ] {
            assert_eq!(
                required(channel),
                Err(vec![format!(
                    "v: \"{channel}\" names a channel that moves, not a version"
                )]),
                "{channel}"
            );
        }
        for version in [
            "nightly-2026-11-01",
            "nightly-2026-11-01-x86_64-unknown-linux-gnu",
            "1.89.0",
            "1.89.0-x86_64-unknown-linux-gnu",
            "22",
            "3.12",
        ] {
            assert_eq!(required(version), Ok(version.to_string()), "{version}");
        }
    }

    /// A date is four digits, two and two, with a `-` between each, and ends
    /// the value or a `-` follows it. Any other shape is no date, and the
    /// channel before it is refused. The day itself is not checked.
    #[test]
    fn a_date_after_a_channel_has_exactly_the_shape_yyyy_mm_dd() {
        for value in [
            "nightly-2026-11-0",
            "nightly-2026-11-1x",
            "nightly-2026-1-01",
            "nightly-26-11-01",
            "nightly-2026_11-01",
            "nightly-2026-11_01",
            "nightly-x026-11-01",
            "nightly-2026-11-012",
            "nightly-2026-11-01x86_64",
            "nightly-",
        ] {
            assert!(required(value).is_err(), "{value}: {:?}", required(value));
        }
        assert_eq!(
            required("nightly-2026-13-45"),
            Ok("nightly-2026-13-45".to_string())
        );
    }

    /// An optional argument keeps today's rule: an exact token is taken,
    /// digit or not.
    #[test]
    fn an_optional_argument_still_takes_a_channel() {
        let dir = repo(&[("v", "stable\n")]);
        let pins = resolve(dir.path(), dir.path(), &sources("require: B\nA v\nB v\n"));
        assert_eq!(pinned(&pins).get("A").map(String::as_str), Some("stable"));
        assert_eq!(pinned(&pins).get("B"), None);
    }

    /// What is unmet: every required argument left unpinned, with each
    /// source tried and why, then each place to pin it, once.
    #[test]
    fn unmet_names_what_was_tried_and_where_to_pin_it() {
        let v = sources(
            "require: NODE PNPM\nNODE .nvmrc - v{}\nNODE .nvmrc\nNODE package.json engines.node\n\
             PNPM package.json packageManager pnpm@{}\nPLAYWRIGHT pnpm-lock.yaml\n",
        );
        let dir = repo(&[("package.json", r#"{"engines": {"node": ">=20"}}"#)]);
        let pins = resolve(dir.path(), dir.path(), &v);
        assert_eq!(
            unmet(&v, &pins),
            [
                "NODE: nothing exact (.nvmrc: absent; .nvmrc: absent; package.json engines.node: \
                 \">=20\" is not an exact version); pin an exact version in one of .nvmrc, \
                 package.json (engines.node)",
                "PNPM: nothing exact (package.json packageManager: no packageManager); pin an \
                 exact version in one of package.json (packageManager)",
            ]
        );
        std::fs::write(dir.path().join(".nvmrc"), "22\n").unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"packageManager": "pnpm@10.4.1"}"#,
        )
        .unwrap();
        assert!(unmet(&v, &resolve(dir.path(), dir.path(), &v)).is_empty());
    }

    /// A repository's tree, and beside it a directory outside it whose name
    /// opens like the tree's: a bound checked on the text of a path would
    /// take one for the other.
    struct Fenced {
        _base: tempfile::TempDir,
        tree: PathBuf,
        outside: PathBuf,
    }

    const MARKER: &str = "SECRET_TOKEN=hunter2-marker";

    fn fenced() -> Fenced {
        let base = tempfile::tempdir().expect("a temp dir");
        let tree = base.path().join("tree");
        let outside = base.path().join("tree-outside");
        std::fs::create_dir_all(&tree).expect("the tree");
        std::fs::create_dir_all(&outside).expect("the outside");
        std::fs::write(outside.join("secret"), format!("{MARKER}\n")).expect("the secret");
        Fenced {
            _base: base,
            tree,
            outside,
        }
    }

    /// A link at `at` to `target`, or `false` where links cannot be made — the
    /// test that asked then has nothing to prove, and skips.
    fn link(target: &Path, at: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, at);
        #[cfg(not(unix))]
        let made: std::io::Result<()> = Err(std::io::Error::other("no links"));
        match made {
            Ok(()) => true,
            Err(e) => {
                eprintln!("skipped: no link can be made here ({e})");
                false
            }
        }
    }

    fn reasons(pins: &[Pin]) -> Vec<String> {
        match &pins[0] {
            Pin::Unpinned { tried, .. } => tried.clone(),
            other => panic!("pinned: {other:?}"),
        }
    }

    /// A `rust-toolchain.toml` that links to a host file: the source gives
    /// nothing, its reason names the source and neither the target nor what
    /// it holds, and the next source is tried.
    #[test]
    fn a_source_that_links_out_of_the_tree_gives_nothing_and_never_says_what_it_holds() {
        let f = fenced();
        if !link(
            &f.outside.join("secret"),
            &f.tree.join("rust-toolchain.toml"),
        ) {
            return;
        }
        let v = sources("require: A\nA rust-toolchain.toml toolchain.channel\n");
        let pins = resolve(&f.tree, &f.tree, &v);
        assert_eq!(
            reasons(&pins),
            ["rust-toolchain.toml toolchain.channel: leads outside the tree, and is not read"]
        );
        let said = unmet(&v, &pins).join("\n");
        assert!(!said.contains("hunter2"), "{said}");
        assert!(!said.contains("tree-outside"), "{said}");

        // Refused, the next source is read.
        std::fs::write(f.tree.join("v"), "1.91.0\n").unwrap();
        let v = sources("require: A\nA rust-toolchain.toml toolchain.channel\nA v\n");
        assert_eq!(pinned(&resolve(&f.tree, &f.tree, &v))["A"], "1.91.0");
    }

    /// A link at any component counts, not only the last: a directory that
    /// links out leads every file under it out too.
    #[test]
    fn a_directory_that_links_out_of_the_tree_leads_its_files_out_too() {
        let f = fenced();
        if !link(&f.outside, &f.tree.join("sub")) {
            return;
        }
        let pins = resolve(&f.tree, &f.tree, &sources("A sub/secret\n"));
        assert_eq!(
            reasons(&pins),
            ["sub/secret: leads outside the tree, and is not read"]
        );
        // And a stack that lives in that directory reads nothing through it.
        let pins = resolve(&f.tree, &f.tree.join("sub"), &sources("A secret\n"));
        assert_eq!(
            reasons(&pins),
            ["secret: leads outside the tree, and is not read"]
        );
    }

    /// A link that stays inside the tree is followed, even out of the stack's
    /// own directory: the tree is the bound, not the stack.
    #[test]
    fn a_link_that_stays_inside_the_tree_is_followed() {
        let f = fenced();
        std::fs::write(f.tree.join(".nvmrc"), "22\n").unwrap();
        std::fs::create_dir_all(f.tree.join("frontend")).unwrap();
        if !link(Path::new("../.nvmrc"), &f.tree.join("frontend/.nvmrc")) {
            return;
        }
        let pins = resolve(
            &f.tree,
            &f.tree.join("frontend"),
            &sources("require: NODE\nNODE .nvmrc\n"),
        );
        assert_eq!(pinned(&pins)["NODE"], "22", "{pins:?}");
    }

    /// A tree reached through a link — `/tmp` on macOS is one — still holds
    /// its own files: the tree is resolved the way each source is.
    #[test]
    fn a_tree_reached_through_a_link_still_holds_its_files() {
        let f = fenced();
        std::fs::write(f.tree.join("v"), "1.91.0\n").unwrap();
        let via = f.outside.join("via");
        if !link(&f.tree, &via) {
            return;
        }
        let pins = resolve(&via, &via, &sources("require: A\nA v\n"));
        assert_eq!(pinned(&pins)["A"], "1.91.0", "{pins:?}");
    }

    /// A link to nothing is a file that is not there, inside the tree or out.
    #[test]
    fn a_link_to_nothing_is_absent() {
        let f = fenced();
        if !link(&f.outside.join("nothing"), &f.tree.join("v")) {
            return;
        }
        let pins = resolve(&f.tree, &f.tree, &sources("A v\n"));
        assert_eq!(reasons(&pins), ["v: absent"]);
    }

    /// A path that cannot be resolved for another reason than a missing file
    /// is not called absent: "absent" tells a human to add the file, and the
    /// file may be there. A path through a regular file fails as not a
    /// directory, and a link to itself as a loop; each says it could not be
    /// resolved, and why.
    #[test]
    fn a_path_that_cannot_be_resolved_is_not_called_absent() {
        let f = fenced();
        std::fs::write(f.tree.join("v"), "1.91.0\n").unwrap();
        let pins = resolve(&f.tree, &f.tree, &sources("A v/x\n"));
        let tried = reasons(&pins);
        assert_eq!(tried.len(), 1, "{tried:?}");
        assert!(
            tried[0].starts_with("v/x: could not be resolved ("),
            "{tried:?}"
        );

        if !link(&f.tree.join("loop"), &f.tree.join("loop")) {
            return;
        }
        let tried = reasons(&resolve(&f.tree, &f.tree, &sources("A loop\n")));
        assert!(
            tried[0].starts_with("loop: could not be resolved ("),
            "{tried:?}"
        );
    }

    /// A tree that cannot be resolved reads nothing, and every source says
    /// why.
    #[test]
    fn a_tree_that_cannot_be_resolved_reads_nothing_and_says_why() {
        let f = fenced();
        let gone = f.tree.join("gone");
        let pins = resolve(&gone, &gone, &sources("A v\nA w\n"));
        let tried = reasons(&pins);
        assert_eq!(tried.len(), 2, "{tried:?}");
        for t in &tried {
            assert!(
                t.contains(&format!(
                    "the tree {} could not be resolved",
                    gone.display()
                )),
                "{t}"
            );
        }
    }

    /// A FIFO at `at`, or `false` where one cannot be made — the test that
    /// asked then has nothing to prove, and skips.
    fn fifo(at: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            let Ok(c) = std::ffi::CString::new(at.as_os_str().as_bytes()) else {
                return false;
            };
            // SAFETY: `c` is a valid NUL-terminated path for the call's span.
            if unsafe { libc::mkfifo(c.as_ptr(), 0o644) } == 0 {
                return true;
            }
            eprintln!(
                "skipped: no FIFO can be made here ({})",
                std::io::Error::last_os_error()
            );
        }
        false
    }

    /// `resolve` on another thread, waited on for `bound` at most. A source
    /// that blocks fails the test rather than hanging it, and the FIFO is
    /// then opened for writing and closed, so the blocked read ends too.
    fn resolved_within(tree: &Path, v: Versions, unblock: &Path) -> Vec<Pin> {
        let (tx, rx) = std::sync::mpsc::channel();
        let at = tree.to_path_buf();
        std::thread::spawn(move || {
            let _ = tx.send(resolve(&at, &at, &v));
        });
        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(pins) => pins,
            Err(_) => {
                let _ = std::fs::OpenOptions::new().write(true).open(unblock);
                panic!("reading the sources blocked for 10 seconds");
            }
        }
    }

    /// A FIFO as `rust-toolchain.toml`: opening it would block until
    /// something writes into it. It gives nothing, says what it is, never
    /// blocks, and the next source is read.
    #[test]
    fn a_fifo_as_a_source_gives_nothing_and_never_blocks() {
        let f = fenced();
        let at = f.tree.join("rust-toolchain.toml");
        if !fifo(&at) {
            return;
        }
        std::fs::write(f.tree.join("v"), "1.91.0\n").unwrap();
        let v = sources("require: A\nA rust-toolchain.toml toolchain.channel\nA v\n");
        let pins = resolved_within(&f.tree, v, &at);
        assert_eq!(
            pins,
            [Pin::Found {
                arg: "A".into(),
                value: "1.91.0".into(),
                from: "v".into(),
            }]
        );
        let v = sources("A rust-toolchain.toml toolchain.channel\n");
        assert_eq!(
            reasons(&resolved_within(&f.tree, v, &at)),
            [
                "rust-toolchain.toml toolchain.channel: is a FIFO, not a regular file, \
                 and is not read"
            ]
        );
    }

    /// A directory gives nothing, and says it is one.
    #[test]
    fn a_directory_as_a_source_gives_nothing() {
        let f = fenced();
        std::fs::create_dir_all(f.tree.join(".nvmrc")).unwrap();
        let pins = resolve(&f.tree, &f.tree, &sources("A .nvmrc\n"));
        assert_eq!(
            reasons(&pins),
            [".nvmrc: is a directory, not a regular file, and is not read"]
        );
    }

    /// A socket gives nothing either, and says it is one.
    #[cfg(unix)]
    #[test]
    fn a_socket_as_a_source_gives_nothing() {
        let f = fenced();
        let Ok(_listening) = std::os::unix::net::UnixListener::bind(f.tree.join(".nvmrc")) else {
            eprintln!("skipped: no socket can be bound here");
            return;
        };
        let pins = resolve(&f.tree, &f.tree, &sources("A .nvmrc\n"));
        assert_eq!(
            reasons(&pins),
            [".nvmrc: is a socket, not a regular file, and is not read"]
        );
    }

    /// A device cannot be made in a tree without privileges, so what one is
    /// called is proved on the host's own `/dev/null`; and a link, which the
    /// look at a resolved path does not follow, is called a link.
    #[cfg(unix)]
    #[test]
    fn a_device_and_a_link_are_called_what_they_are() {
        let null = std::fs::symlink_metadata("/dev/null").unwrap();
        assert_eq!(kind(&null.file_type()), "a device");
        let f = fenced();
        if !link(&f.outside.join("secret"), &f.tree.join("v")) {
            return;
        }
        let looked = std::fs::symlink_metadata(f.tree.join("v")).unwrap();
        assert_eq!(kind(&looked.file_type()), "a link");
        // A link set down where `inside` resolved a file is not followed.
        assert_eq!(
            regular_file(&f.tree.join("v"), SOURCE_LIMIT),
            Err("is a link, not a regular file, and is not read".to_string())
        );
    }

    /// Never more than the limit and one byte: a file of exactly the limit is
    /// read, one byte more gives nothing, with the reason.
    #[test]
    fn a_source_is_read_up_to_its_limit_and_not_a_byte_past_it() {
        let f = fenced();
        let at = f.tree.join("v");
        std::fs::write(&at, "1.91.0\n").unwrap();
        assert_eq!(regular_file(&at, 7), Ok("1.91.0\n".to_string()));
        assert_eq!(
            regular_file(&at, 6),
            Err("holds more than 6 bytes, more than any version source, and is not read".into())
        );
    }

    /// The same at the real limit, with sparse files so the test stays fast:
    /// a file of `SOURCE_LIMIT` bytes pins, one byte more gives nothing.
    #[test]
    fn a_source_past_the_limit_gives_nothing_and_one_at_it_is_read() {
        let f = fenced();
        let at = f.tree.join("v");
        std::fs::write(&at, "1.91.0\n").unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&at).unwrap();
        file.set_len(SOURCE_LIMIT).unwrap();
        let pins = resolve(&f.tree, &f.tree, &sources("A v\n"));
        assert_eq!(pinned(&pins)["A"], "1.91.0", "{pins:?}");

        file.set_len(SOURCE_LIMIT + 1).unwrap();
        let pins = resolve(&f.tree, &f.tree, &sources("A v\n"));
        assert_eq!(
            reasons(&pins),
            [format!(
                "v: holds more than {SOURCE_LIMIT} bytes, more than any version source, \
                 and is not read"
            )]
        );
    }
}
