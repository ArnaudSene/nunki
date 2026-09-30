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
//! ```
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
}

/// Parse a `versions.txt`. A line nunki cannot read is an error that names
/// it, never a line skipped: a skipped line is a version silently not read.
pub fn parse(text: &str, file: &Path) -> Result<Vec<Source>, SourcesError> {
    let mut sources = Vec::new();
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
        let columns: Vec<&str> = line.split_whitespace().collect();
        let (arg, path, key, pattern) = match columns.as_slice() {
            [arg, path] => (*arg, *path, None, None),
            [arg, path, key] => (*arg, *path, Some(*key), None),
            [arg, path, key, pattern] => (*arg, *path, Some(*key), Some(*pattern)),
            _ => return Err(bad("expected two to four columns")),
        };
        let mut chars = arg.chars();
        let named = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !named {
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
    Ok(sources)
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
    /// No source gave an exact version, so the Dockerfile's default stands.
    /// Each source tried, and why it gave nothing.
    Unpinned { arg: String, tried: Vec<String> },
}

impl Pin {
    pub fn arg(&self) -> &str {
        match self {
            Pin::Found { arg, .. } | Pin::Unpinned { arg, .. } => arg,
        }
    }
}

/// Read every build argument the sources name, from the repository at
/// `root`. Sources for one argument are tried in the order they are written,
/// and the first exact version wins — so `.nvmrc` can come before
/// `engines.node`.
pub fn resolve(root: &Path, sources: &[Source]) -> Vec<Pin> {
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
            for source in sources.iter().filter(|s| s.arg == arg) {
                match read(root, source) {
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
fn read(root: &Path, source: &Source) -> Result<String, String> {
    let path = root.join(&source.file);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("absent".to_string()),
        Err(e) => return Err(format!("could not be read ({e})")),
    };
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
/// the next source is tried, and failing all of them the Dockerfile's default
/// stands and the build says so.
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

    fn sources(text: &str) -> Vec<Source> {
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
        let pins = resolve(dir.path(), &sources("NODE .nvmrc\nNODE .node-version\n"));
        assert_eq!(pinned(&pins)["NODE"], "22");
    }

    #[test]
    fn nothing_exact_leaves_the_argument_unpinned_and_says_why_for_each_source() {
        let dir = repo(&[("package.json", r#"{"engines": {"node": "^20"}}"#)]);
        let pins = resolve(
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
            &sources("PNPM package.json packageManager pnpm@{}\n"),
        );
        assert_eq!(pinned(&pins)["PNPM"], "11.15.0");
    }

    #[test]
    fn another_package_manager_is_not_read_as_pnpm() {
        let dir = repo(&[("package.json", r#"{"packageManager": "yarn@4.1.0"}"#)]);
        let pins = resolve(
            dir.path(),
            &sources("PNPM package.json packageManager pnpm@{}\n"),
        );
        assert!(pinned(&pins).is_empty(), "{pins:?}");
    }

    #[test]
    fn a_plain_file_gives_its_first_line_and_skips_comments() {
        let dir = repo(&[(".python-version", "# pinned for the lab\n3.13\n")]);
        let pins = resolve(dir.path(), &sources("PYTHON .python-version\n"));
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
        let pins = resolve(dir.path(), &sources("PYTHON .python-version - {}\n"));
        assert_eq!(pinned(&pins)["PYTHON"], "3.13", "{pins:?}");
    }

    #[test]
    fn a_key_into_a_plain_file_is_refused_rather_than_ignored() {
        let dir = repo(&[(".python-version", "3.13\n")]);
        let pins = resolve(dir.path(), &sources("PYTHON .python-version version\n"));
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
                &sources("PYTHON .python-version\nNODE .nvmrc\n"),
                "ARG PYTHON\n"
            ),
            vec!["NODE".to_string()]
        );
    }
}
