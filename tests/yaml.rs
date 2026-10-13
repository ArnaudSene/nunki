//! The one YAML reader, `nunki::yaml::from_str`, and its settings: each is
//! proved through a reader a caller really uses, so that flipping it in
//! `src/yaml.rs` turns a test here red.

use std::path::{Path, PathBuf};

use nunki::compose::services::{LABEL, ServicesError, ServicesFile};
use nunki::mission::dir::{MissionDirError, read_header};
use nunki::project::{Project, ProjectError};
use nunki::services::{Approval, ApprovalError, Approvals, labels_of_profile};
use nunki::yaml::{self, Mapping, Value};

mod common;

const DIGEST: &str = "sha256:aa";

fn approved() -> Approvals {
    Approvals {
        approved: vec![Approval {
            digest: DIGEST.to_string(),
            by: "a human".to_string(),
            at: "2026-10-13T00:00:00Z".to_string(),
            rendering: String::new(),
        }],
    }
}

fn profile(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("system.yml");
    std::fs::write(&path, text).unwrap();
    path
}

fn open_project(body: &str) -> Result<Project, ProjectError> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = common::project_home(&root, dir.path(), body);
    Project::open_at(root, home)
}

// --- merge keys: an ordinary key, never expanded --------------------------------

#[test]
fn a_merge_key_in_a_profile_lends_no_service_an_approved_label() {
    let dir = tempfile::tempdir().unwrap();
    // Expanded, the `<<` would give `db` the approved label it does not
    // carry itself.
    let path = profile(
        dir.path(),
        &format!("services:\n  db:\n    <<: {{labels: {{{LABEL}: \"{DIGEST}\"}}}}\n    image: x\n"),
    );
    let err = labels_of_profile(&path, &approved()).unwrap_err();
    assert!(
        matches!(&err, ApprovalError::UnapprovedProfile { service, .. } if service == "db"),
        "{err}"
    );
}

#[test]
fn a_merge_key_in_a_services_file_is_refused_by_name_and_not_expanded() {
    let err = ServicesFile::parse("services:\n  db:\n    <<: {image: x}\n").unwrap_err();
    assert!(
        matches!(&err, ServicesError::MergeKey { at } if at == "the file.services.db"),
        "{err:?}"
    );
}

// --- tags: one the reader does not know is refused, not dropped --------------------

#[test]
fn an_unknown_tag_on_a_label_refuses_the_profile_instead_of_vanishing() {
    let dir = tempfile::tempdir().unwrap();
    let path = profile(
        dir.path(),
        &format!(
            "services:\n  db:\n    image: x\n    labels:\n      {LABEL}: !custom \"{DIGEST}\"\n"
        ),
    );
    let err = labels_of_profile(&path, &approved()).unwrap_err();
    assert!(matches!(&err, ApprovalError::NotAProfile(..)), "{err}");
    assert!(err.to_string().contains("!custom"), "{err}");
}

#[test]
fn an_unknown_tag_in_nunki_yaml_refuses_the_file() {
    let err = open_project("harness: !custom claude-code\n").unwrap_err();
    assert!(matches!(err, ProjectError::Invalid(..)), "{err}");
    assert!(err.to_string().contains("nunki.yaml"), "{err}");
    assert!(err.to_string().contains("line 2, column"), "{err}");
}

// --- booleans: only `true` and `false` ----------------------------------------------

#[test]
fn a_yaml_1_1_boolean_in_a_services_file_stays_the_string_it_was_written() {
    let file = ServicesFile::parse(
        "services:\n  db:\n    image: x\n    environment:\n      A: no\n      B: yes\n      \
         C: on\n      D: off\n      E: y\n      F: n\n",
    )
    .unwrap();
    let environment = &file.services["db"].environment;
    for (name, written) in [
        ("A", "no"),
        ("B", "yes"),
        ("C", "on"),
        ("D", "off"),
        ("E", "y"),
        ("F", "n"),
    ] {
        assert_eq!(
            environment.get(name).map(String::as_str),
            Some(written),
            "{name}"
        );
    }
}

/// Every golden profile holds `restart: no`, which a YAML 1.1 reader takes
/// for `false`.
#[test]
fn the_golden_profiles_read_back_with_restart_no_as_the_string_no() {
    for name in [
        "mission.yml",
        "system-integrator.yml",
        "system-security.yml",
    ] {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/compose")
                .join(name),
        )
        .unwrap();
        let doc: Value = yaml::from_str(&text).unwrap();
        for service in ["firewall", "agent"] {
            let service = &doc["services"][service];
            assert_eq!(service["restart"], Value::from("no"), "{name}");
            assert_eq!(service["init"], Value::Bool(true), "{name}");
        }
        assert_eq!(
            doc["services"]["firewall"]["healthcheck"]["retries"].as_u64(),
            Some(30),
            "{name}"
        );
    }
}

// --- duplicate keys: an error ----------------------------------------------------

#[test]
fn a_key_written_twice_in_nunki_yaml_refuses_the_file() {
    let err = open_project("harness: claude-code\nmutation_threshold: 90\nmutation_threshold: 1\n")
        .unwrap_err();
    assert!(matches!(err, ProjectError::Invalid(..)), "{err}");
    assert!(err.to_string().contains("line 4, column 1"), "{err}");
}

#[test]
fn a_variable_set_twice_in_a_services_file_refuses_the_file() {
    let err = ServicesFile::parse(
        "services:\n  db:\n    image: x\n    environment:\n      A: first\n      A: second\n",
    )
    .unwrap_err();
    assert!(matches!(err, ServicesError::Yaml(_)), "{err:?}");
    assert!(err.to_string().contains("duplicate"), "{err}");
}

// --- what a reading error says ----------------------------------------------------

#[test]
fn a_reading_error_says_the_place_once_and_copies_no_text_of_the_file() {
    let dir = tempfile::tempdir().unwrap();
    // A profile holds the subscription token, on the line above the one the
    // parser stops on.
    let path = profile(
        dir.path(),
        "services:\n  agent:\n    environment:\n      TOKEN: sk-secret-token\n      \
         BROKEN: [a\n",
    );
    let err = labels_of_profile(&path, &approved())
        .unwrap_err()
        .to_string();
    assert!(!err.contains("sk-secret-token"), "{err}");
    assert!(err.contains(&path.display().to_string()), "{err}");
    assert_eq!(err.matches("line ").count(), 1, "{err}");
    assert!(err.contains("line 5, column 15"), "{err}");
}

#[test]
fn a_duplicate_key_is_said_without_advice_about_the_librarys_options() {
    let err = yaml::from_str::<Value>("a: 1\na: 2\n").unwrap_err();
    assert_eq!(err.place, Some((2, 1)));
    assert_eq!(err.message, "duplicate mapping key: a");
    assert_eq!(
        err.to_string(),
        "line 2, column 1: duplicate mapping key: a"
    );
}

#[test]
fn an_error_without_a_place_is_its_message() {
    let err = yaml::Error {
        place: None,
        message: "no place".to_string(),
    };
    assert_eq!(err.to_string(), "no place");
    assert_eq!(err.clone().below(3), err);
}

#[test]
fn a_mission_header_that_cannot_be_read_names_the_file_and_its_own_line() {
    let hq = tempfile::tempdir().unwrap();
    let mission = hq.path().join("missions/m1/MISSION.md");
    std::fs::create_dir_all(mission.parent().unwrap()).unwrap();
    // Line 3 of the file, line 2 of the header.
    std::fs::write(&mission, "---\nbranch: a\nbranch: b\n---\n\n# Mission\n").unwrap();
    let err = read_header(hq.path(), "m1").unwrap_err();
    assert!(matches!(err, MissionDirError::BadHeader { .. }), "{err}");
    let said = err.to_string();
    assert!(said.contains(&mission.display().to_string()), "{said}");
    assert!(said.contains("line 3, column 1"), "{said}");
}

// --- the tree -------------------------------------------------------------------

#[test]
fn a_mapping_keeps_the_order_of_the_file_and_the_kind_of_its_keys() {
    let doc: Value = yaml::from_str("b: 1\n1: one\n\"1\": string\na: 2\ntrue: t\n~: n\n").unwrap();
    let keys: Vec<&Value> = doc.as_mapping().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            &Value::from("b"),
            &Value::Integer(1),
            &Value::from("1"),
            &Value::from("a"),
            &Value::Bool(true),
            &Value::Null,
        ]
    );
    // A lookup by string finds the string key, never the integer one.
    assert_eq!(doc["1"], Value::from("string"));
    assert_eq!(doc.get("true"), None);
}

#[test]
fn every_kind_of_scalar_reads_to_its_own_kind() {
    let doc: Value = yaml::from_str(
        "nothing: null\ntilde: ~\nempty:\nint: -5\nbig: 18446744073709551615\nfloat: 1.5\n\
         string: 'x'\nlist: [a, 1]\n",
    )
    .unwrap();
    assert!(doc.get("nothing").unwrap().is_null());
    assert!(doc.get("tilde").unwrap().is_null());
    assert!(doc.get("empty").unwrap().is_null());
    assert_eq!(doc["int"], Value::Integer(-5));
    assert_eq!(doc["int"].as_u64(), None);
    assert_eq!(doc["big"].as_u64(), Some(u64::MAX));
    assert_eq!(doc["float"], Value::Float(1.5));
    assert_eq!(doc["float"].as_u64(), None);
    assert_eq!(doc["string"].as_str(), Some("x"));
    assert_eq!(
        doc["list"],
        Value::Sequence(vec![Value::from("a"), Value::Integer(1)])
    );
    assert_eq!(doc["list"][1], Value::Integer(1));
    // What is not there reads as null, and is not an error.
    assert!(doc["list"][2].is_null());
    assert!(doc["missing"]["deeper"].is_null());
    assert!(doc["string"][0].is_null());
}

#[test]
fn a_mapping_is_changed_in_place_and_keeps_its_order() {
    let mut map = Mapping::new();
    assert!(map.is_empty());
    assert_eq!(map.insert(Value::from("a"), Value::from("1")), None);
    assert_eq!(map.insert(Value::Integer(1), Value::from("int")), None);
    assert_eq!(map.insert(Value::from("b"), Value::from("2")), None);
    // Set again, a key keeps its place and gives back what it held.
    assert_eq!(
        map.insert(Value::from("a"), Value::from("3")),
        Some(Value::from("1"))
    );
    assert_eq!(map.len(), 3);
    assert!(map.contains_key(&Value::Integer(1)));
    assert!(!map.contains_key(&Value::from("1")));
    let mut doc = Value::from(map);
    *doc.get_mut("b").unwrap() = Value::from(true);
    for (_, value) in doc.as_mapping_mut().unwrap().iter_mut() {
        if value.as_bool() == Some(true) {
            *value = Value::from(7u32);
        }
    }
    let map = doc.as_mapping_mut().unwrap();
    assert_eq!(map.remove("a"), Some(Value::from("3")));
    assert_eq!(map.remove("a"), None);
    let entries: Vec<(&Value, &Value)> = map.iter().collect();
    assert_eq!(
        entries,
        [
            (&Value::Integer(1), &Value::from("int")),
            (&Value::from("b"), &Value::Integer(7)),
        ]
    );
    let mut list = Value::from(vec![Value::from("x")]);
    list.as_sequence_mut().unwrap().push(Value::Null);
    assert_eq!(list.as_sequence().map(Vec::len), Some(2));
    assert!(Value::Null.as_sequence_mut().is_none());
    assert!(Value::Null.as_mapping_mut().is_none());
    assert!(Value::Null.get_mut("a").is_none());
}

#[test]
fn a_tree_writes_back_what_it_holds() {
    let mut map = Mapping::new();
    map.insert(Value::from("n"), Value::Null);
    map.insert(Value::from("b"), Value::Bool(false));
    map.insert(Value::from("i"), Value::Integer(-1));
    map.insert(Value::from("u"), Value::Integer(u64::MAX.into()));
    map.insert(Value::from("w"), Value::Integer(1 << 70));
    map.insert(Value::from("f"), Value::Float(0.5));
    map.insert(Value::from("s"), Value::Sequence(vec![Value::from("x")]));
    assert_eq!(
        serde_json::to_string(&Value::from(map)).unwrap(),
        format!(
            "{{\"n\":null,\"b\":false,\"i\":-1,\"u\":{},\"w\":{},\"f\":0.5,\"s\":[\"x\"]}}",
            u64::MAX,
            1i128 << 70
        )
    );
}

#[test]
fn a_node_of_no_kind_the_tree_holds_is_refused_saying_what_was_expected() {
    use serde::Deserialize;
    let bytes = serde::de::value::BytesDeserializer::<serde::de::value::Error>::new(b"x");
    let err = Value::deserialize(bytes).unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid type: byte array, expected a YAML node"
    );
}

#[test]
fn a_value_that_cannot_be_written_is_an_error_saying_why() {
    struct Unwritable;
    impl serde::Serialize for Unwritable {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("no YAML for this"))
        }
    }
    let err = yaml::to_string(&Unwritable).unwrap_err();
    assert_eq!(err.place, None);
    assert!(err.message.contains("no YAML for this"), "{err}");
}

/// A serializer that answers with the name of the method it was given, and
/// refuses everything that is not a scalar.
struct Spy;

macro_rules! spied {
    ($($method:ident($ty:ty) => $name:literal,)*) => {
        $(fn $method(self, _: $ty) -> Result<&'static str, Self::Error> { Ok($name) })*
    };
}

impl serde::Serializer for Spy {
    type Ok = &'static str;
    type Error = serde::de::value::Error;
    type SerializeSeq = serde::ser::Impossible<&'static str, Self::Error>;
    type SerializeTuple = serde::ser::Impossible<&'static str, Self::Error>;
    type SerializeTupleStruct = serde::ser::Impossible<&'static str, Self::Error>;
    type SerializeTupleVariant = serde::ser::Impossible<&'static str, Self::Error>;
    type SerializeMap = serde::ser::Impossible<&'static str, Self::Error>;
    type SerializeStruct = serde::ser::Impossible<&'static str, Self::Error>;
    type SerializeStructVariant = serde::ser::Impossible<&'static str, Self::Error>;

    spied! {
        serialize_bool(bool) => "bool",
        serialize_i8(i8) => "i8",
        serialize_i16(i16) => "i16",
        serialize_i32(i32) => "i32",
        serialize_i64(i64) => "i64",
        serialize_i128(i128) => "i128",
        serialize_u8(u8) => "u8",
        serialize_u16(u16) => "u16",
        serialize_u32(u32) => "u32",
        serialize_u64(u64) => "u64",
        serialize_u128(u128) => "u128",
        serialize_f32(f32) => "f32",
        serialize_f64(f64) => "f64",
        serialize_char(char) => "char",
        serialize_str(&str) => "str",
        serialize_bytes(&[u8]) => "bytes",
        serialize_unit_struct(&'static str) => "unit_struct",
    }

    fn serialize_none(self) -> Result<&'static str, Self::Error> {
        Ok("none")
    }
    fn serialize_some<T: ?Sized + serde::Serialize>(
        self,
        _: &T,
    ) -> Result<&'static str, Self::Error> {
        Ok("some")
    }
    fn serialize_unit(self) -> Result<&'static str, Self::Error> {
        Ok("unit")
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<&'static str, Self::Error> {
        Ok("unit_variant")
    }
    fn serialize_newtype_struct<T: ?Sized + serde::Serialize>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<&'static str, Self::Error> {
        Ok("newtype_struct")
    }
    fn serialize_newtype_variant<T: ?Sized + serde::Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<&'static str, Self::Error> {
        Ok("newtype_variant")
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(serde::ser::Error::custom("a scalar only"))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(serde::ser::Error::custom("a scalar only"))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(serde::ser::Error::custom("a scalar only"))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(serde::ser::Error::custom("a scalar only"))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(serde::ser::Error::custom("a scalar only"))
    }
    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(serde::ser::Error::custom("a scalar only"))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(serde::ser::Error::custom("a scalar only"))
    }
}

/// An integer is handed to a serializer as the narrowest of `i64`, `u64`
/// and `i128` it fits. The writers nunki uses print the three alike, but
/// not every serializer takes an `i128` (serde's own default refuses it), so
/// a `-1` or a `30` written as one would be refused by those.
#[test]
fn an_integer_is_written_as_the_narrowest_kind_it_fits() {
    use serde::Serialize;
    let kind = |n: i128| Value::Integer(n).serialize(Spy).unwrap();
    assert_eq!(kind(-1), "i64");
    assert_eq!(kind(30), "i64");
    assert_eq!(kind(i64::MAX.into()), "i64");
    assert_eq!(kind(i64::MIN.into()), "i64");
    assert_eq!(kind(i128::from(i64::MAX) + 1), "u64");
    assert_eq!(kind(u64::MAX.into()), "u64");
    assert_eq!(kind(i128::from(u64::MAX) + 1), "i128");
    assert_eq!(kind(i128::from(i64::MIN) - 1), "i128");
}
