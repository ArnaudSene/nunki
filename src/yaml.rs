//! YAML, read one way and written one way (SPEC 4.1, `nunki.yaml` and the
//! mission header; SPEC 4.2, the services file and the generated file).
//!
//! Every YAML file `nunki` reads goes through [`from_str`], and its settings
//! are written once, in [`options`]. They are not the library's defaults,
//! and each of them is what keeps a file `nunki` accepted yesterday reading
//! to the same thing today, or being refused — never to something else:
//!
//! - a merge key (`<<`) is an **ordinary key**, as it always was here: never
//!   expanded into its mapping, so a walk that refuses it by name still sees
//!   it, and a typed reader still ignores it as a key it does not know;
//! - **only `true` and `false` are booleans**: `restart: no`, `y`, `on` stay
//!   the strings they were (YAML 1.2), and are not read as YAML 1.1 booleans;
//! - a **duplicate key is an error**.
//!
//! And **a tag is refused, in every file**, from the parser's own events
//! ([`constructs`]): nunki writes none, and a tag is exactly where this
//! library and the one before it read the same text to different values
//! (`!!binary aGk=` was the string `aGk=` and is `hi`; `!!null ''` was an
//! empty string and is nothing). Whether a text holds a tag, an anchor or an
//! alias is the parser's to say, never a second reading of the text's.
//!
//! Every YAML file `nunki` writes — the generated Compose file, the mission
//! header — goes through [`to_string`], whose one setting is in
//! [`write_options`]: every string is quoted, so that what nunki writes is
//! read as written by any reader, Compose's included.
//!
//! A clippy rule (`clippy.toml`) refuses every other call to a
//! `serde_saphyr` reader or writer and every other use of its parser, in
//! the code and in the tests, so that these settings cannot be bypassed by
//! a call that forgot them.
//!
//! Where a reader needs a tree rather than a type, it is [`Value`], nunki's
//! own: a mapping keeps the order of the file and keeps its keys as values,
//! so a key `1` and a key `"1"` are two keys of two kinds.

use std::fmt;

use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_saphyr::granit_parser::{ErrorKind, Event, StrInput};
use serde_saphyr::{DuplicateKeyPolicy, MergeKeyPolicy};

/// The settings every read uses, and the only place they are written.
///
/// No `reject_unsupported_tags`: every tag, known or not, is refused from
/// the events before a value is returned, so the setting could change
/// nothing.
fn options() -> serde_saphyr::Options {
    serde_saphyr::options! {
        duplicate_keys: DuplicateKeyPolicy::Error,
        merge_keys: MergeKeyPolicy::AsOrdinary,
        strict_booleans: true,
        // The message carries the line and the column; a snippet would
        // copy the file's text into it, and a profile holds a token.
        with_snippet: false,
    }
}

/// Read `text` into `T`: the one function in `nunki` that parses YAML.
///
/// The reader goes first, so a text it cannot read is said in its words;
/// then the events of the same text are asked for a tag, and one refuses
/// the file, with its place.
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
    #[allow(clippy::disallowed_methods)] // the one reader, with the settings above
    let read = serde_saphyr::from_str_with_options(text, options());
    let value = read.map_err(|e| Error::from_parser(&e))?;
    if let Some(tag) = constructs(text)?
        .into_iter()
        .find(|found| found.construct == Construct::Tag)
    {
        return Err(Error {
            place: Some((tag.line, tag.column)),
            message: format!(
                "a tag ({}), which nunki reads in no file: write the value without it",
                tag.written
            ),
        });
    }
    Ok(value)
}

/// A construct the parser reports on a node, which nunki refuses: a tag in
/// every file, and an anchor and an alias too in a services file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Construct {
    Tag,
    Anchor,
    Alias,
}

impl Construct {
    /// As a refusal names it.
    pub fn described(self) -> &'static str {
        match self {
            Construct::Tag => "a tag",
            Construct::Anchor => "an anchor",
            Construct::Alias => "an alias",
        }
    }
}

/// One construct, where the parser saw the node that carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub construct: Construct,
    /// From 1, as a human counts them.
    pub line: u64,
    pub column: u64,
    /// The tag as the parser resolved it (`tag:yaml.org,2002:binary`,
    /// `!custom`); empty for an anchor and an alias.
    pub written: String,
}

/// Every tag, anchor and alias of `text`, in the order the parser meets
/// them, from the events of the reader's own parser.
///
/// The same parser as [`from_str`]'s, on the same text:
/// `serde_saphyr::from_str_with_options` strips one leading BOM, which the
/// parser skips by itself (measured: stripping it here changes nothing), then reads
/// with `Parser::with_options(StrInput::new(text), parser_options)`. Its
/// `Options::parser_options()` (`pub(crate)` in serde-saphyr 1.3.0) sets
/// four numbers and nothing else, from the default `Budget`: 32 buffered
/// comment events, keys looked for over 1024 characters, flow and block
/// collections nested 255 deep. The parser's own defaults, used here, are
/// the same but for the comments, buffered up to 96.
///
/// Three of the four are limits: one that is hit is an error, never another
/// reading, and none is stricter here than the reader's. The fourth is
/// grammar. A key not found within the lookahead is not an error, unless
/// the text required one there: the scanner stops taking the text for a key
/// and reads on, to other tokens. So the two passes read a text to the same
/// events only while their two lookaheads are equal, and they are equal
/// because both defaults are 1024 today, not because anything here holds
/// them together. Whoever moves serde-saphyr or its parser to another
/// version compares `serde_saphyr::Budget::default()` with
/// `granit_parser::Options::default()` first, and builds this parser with
/// the reader's numbers if they differ.
///
/// While they are equal, this pass reads every text the reader reads, to
/// the same events, so the two cannot disagree on what a text holds; and
/// what it cannot read, it refuses, as the reader does.
pub fn constructs(text: &str) -> Result<Vec<Found>, Error> {
    // The one use of the parser itself: its events, to say what the text
    // holds. Nothing here reads a value.
    #[allow(clippy::disallowed_types)]
    let parser: serde_saphyr::granit_parser::Parser<'_, StrInput<'_>> =
        serde_saphyr::granit_parser::Parser::new_from_str(text);
    let mut found = Vec::new();
    for event in parser {
        let (event, span) = match event {
            Ok(event) => event,
            // An alias to no anchor is an alias all the same: reported as
            // one, where it stands, and the reading ends there. Every caller
            // refuses the text then — a services file for the alias, the
            // others because their reader, which goes first, cannot read it.
            Err(e) if matches!(e.kind(), ErrorKind::UnknownAnchor) => {
                found.push(Found {
                    construct: Construct::Alias,
                    line: e.marker().line() as u64,
                    column: e.marker().col() as u64 + 1,
                    written: String::new(),
                });
                return Ok(found);
            }
            Err(e) => {
                return Err(Error {
                    place: Some((e.marker().line() as u64, e.marker().col() as u64 + 1)),
                    message: e.kind().to_string(),
                });
            }
        };
        let at = |construct, written: String| Found {
            construct,
            line: span.start.line() as u64,
            column: span.start.col() as u64 + 1,
            written,
        };
        let (anchor, tag) = match event {
            Event::Alias(_) => {
                found.push(at(Construct::Alias, String::new()));
                continue;
            }
            Event::Scalar(_, _, anchor, tag)
            | Event::SequenceStart(_, anchor, tag)
            | Event::MappingStart(_, anchor, tag) => (anchor, tag),
            _ => continue,
        };
        if let Some(tag) = tag {
            found.push(at(Construct::Tag, tag.to_string()));
        }
        if anchor != 0 {
            found.push(at(Construct::Anchor, String::new()));
        }
    }
    Ok(found)
}

/// The settings every write uses, and the only place they are written.
///
/// `quote_all`: every string leaves as one quoted scalar on one line, so
/// no reader has to guess its type. The writer's own judgement of which
/// strings are ambiguous is its parser's, and Compose's is another's:
/// measured on Docker Compose 5.1.2, a plain `1_000` arrived as 1000 and
/// `2026-10-13` as a timestamp. A boolean, a number and null stay bare.
fn write_options() -> serde_saphyr::SerializerOptions {
    serde_saphyr::ser_options! {
        quote_all: true,
    }
}

/// Write `value` as YAML: the one function in `nunki` that does.
pub fn to_string<T: Serialize>(value: &T) -> Result<String, Error> {
    #[allow(clippy::disallowed_methods)] // the one writer, with the settings above
    let written = serde_saphyr::to_string_with_options(value, write_options());
    written.map_err(|e| Error {
        place: None,
        message: e.to_string(),
    })
}

/// Why a text could not be read: what the parser said, and where. The
/// caller names the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Where the parser stopped, line then column, both from 1, when it
    /// said.
    pub place: Option<(u64, u64)>,
    pub message: String,
}

impl Error {
    fn from_parser(error: &serde_saphyr::Error) -> Self {
        let place = error.location().map(|l| (l.line(), l.column()));
        let mut message = error.to_string();
        // The place is said once, before the message, and the advice meant
        // for a programmer holding the library's options is not given to a
        // human who holds none.
        if let Some((line, column)) = place {
            let said = format!(" at line {line}, column {column}");
            if let Some(rest) = message.strip_suffix(&said) {
                message = rest.to_string();
            }
        }
        message = message.replace(", set DuplicateKeyPolicy in Options if acceptable", "");
        Self { place, message }
    }

    /// The same error, for a text read from `lines` lines into its file: the
    /// line it names is then the file's.
    pub fn below(self, lines: u64) -> Self {
        Self {
            place: self.place.map(|(line, column)| (line + lines, column)),
            ..self
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.place {
            Some((line, column)) => write!(f, "line {line}, column {column}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for Error {}

/// A YAML tree, as `nunki` reads it.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    /// Every integer the parser hands over, signed or not: it hands over
    /// what fits an `i64` or a `u64`, and reads a larger one as a float.
    Integer(i128),
    Float(f64),
    String(String),
    Sequence(Vec<Value>),
    Mapping(Mapping),
}

/// A mapping in the order of the file, keyed by values rather than by
/// strings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mapping {
    entries: Vec<(Value, Value)>,
}

/// What an index that finds nothing points at, as a missing key reads.
static NULL: Value = Value::Null;

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// An integer that fits a `u64`, and nothing else: not a float, not a
    /// string of digits.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Integer(n) => u64::try_from(*n).ok(),
            _ => None,
        }
    }

    pub fn as_sequence(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Sequence(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_sequence_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Sequence(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_mapping(&self) -> Option<&Mapping> {
        match self {
            Value::Mapping(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_mapping_mut(&mut self) -> Option<&mut Mapping> {
        match self {
            Value::Mapping(map) => Some(map),
            _ => None,
        }
    }

    /// The value under the string key `key`, when this is a mapping.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_mapping()?.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.as_mapping_mut()?.get_mut(key)
    }
}

impl Mapping {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Set `key` to `value`: in place when the key is there, at the end
    /// when it is not. The previous value, if any.
    pub fn insert(&mut self, key: Value, value: Value) -> Option<Value> {
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some((_, slot)) => Some(std::mem::replace(slot, value)),
            None => {
                self.entries.push((key, value));
                None
            }
        }
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// The value under the string key `key`. A key `1` is not the key
    /// `"1"`, and this finds only the second.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k.as_str() == Some(key))
            .map(|(_, v)| v)
    }

    /// Take the string key `key` out, keeping the order of the others.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let at = self
            .entries
            .iter()
            .position(|(k, _)| k.as_str() == Some(key))?;
        Some(self.entries.remove(at).1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Value, &mut Value)> {
        self.entries.iter_mut().map(|(k, v)| (&*k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &Value> {
        self.entries.iter().map(|(k, _)| k)
    }
}

impl<'a> IntoIterator for &'a Mapping {
    type Item = (&'a Value, &'a Value);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (Value, Value)>,
        fn(&'a (Value, Value)) -> (&'a Value, &'a Value),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter().map(|(k, v)| (k, v))
    }
}

impl std::ops::Index<&str> for Value {
    type Output = Value;

    /// The value under `key`, or null: a missing key and a key that holds
    /// null read the same, as they do in YAML.
    fn index(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&NULL)
    }
}

impl std::ops::Index<usize> for Value {
    type Output = Value;

    fn index(&self, at: usize) -> &Value {
        self.as_sequence()
            .and_then(|items| items.get(at))
            .unwrap_or(&NULL)
    }
}

impl From<&str> for Value {
    fn from(text: &str) -> Self {
        Value::String(text.to_string())
    }
}

impl From<String> for Value {
    fn from(text: String) -> Self {
        Value::String(text)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<u32> for Value {
    fn from(n: u32) -> Self {
        Value::Integer(n.into())
    }
}

impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Self {
        Value::Sequence(items)
    }
}

impl From<Mapping> for Value {
    fn from(map: Mapping) -> Self {
        Value::Mapping(map)
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Integer(n) => match (i64::try_from(*n), u64::try_from(*n)) {
                (Ok(n), _) => serializer.serialize_i64(n),
                (_, Ok(n)) => serializer.serialize_u64(n),
                _ => serializer.serialize_i128(*n),
            },
            Value::Float(x) => serializer.serialize_f64(*x),
            Value::String(text) => serializer.serialize_str(text),
            Value::Sequence(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Value::Mapping(map) => map.serialize(serializer),
        }
    }
}

impl Serialize for Mapping {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut out = serializer.serialize_map(Some(self.entries.len()))?;
        for (key, value) in &self.entries {
            out.serialize_entry(key, value)?;
        }
        out.end()
    }
}

impl<'de> serde::Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a YAML node")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E: de::Error>(self, b: bool) -> Result<Value, E> {
        Ok(Value::Bool(b))
    }

    fn visit_i64<E: de::Error>(self, n: i64) -> Result<Value, E> {
        Ok(Value::Integer(n.into()))
    }

    fn visit_u64<E: de::Error>(self, n: u64) -> Result<Value, E> {
        Ok(Value::Integer(n.into()))
    }

    fn visit_f64<E: de::Error>(self, x: f64) -> Result<Value, E> {
        Ok(Value::Float(x))
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Value, E> {
        Ok(Value::String(text.to_string()))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Value::Sequence(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut entries = Vec::new();
        while let Some((key, value)) = map.next_entry()? {
            entries.push((key, value));
        }
        Ok(Value::Mapping(Mapping { entries }))
    }
}
