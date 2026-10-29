//! The class this file proves closed: a services file in the slot's tree,
//! written by the coder, started by the engine with all of its power on the
//! human's machine (SPEC 4.2, "services et lancement").
//!
//! The model is closed: a service holds `image`, `environment`, `command`,
//! `entrypoint`, `healthcheck`, `depends_on`, `working_dir` and `volumes`, and
//! `ports`, which nunki drops. Everything else refuses the whole file. No YAML
//! feature carries meaning past the parser, and what is lifted is nunki's
//! own rendering, never the file's bytes.

use nunki::compose::services::{ServicesError, ServicesFile};
use nunki::yaml::Value;

fn parse(text: &str) -> Result<ServicesFile, ServicesError> {
    ServicesFile::parse(text)
}

/// A database service around one extra line, so a refusal is the line's
/// doing and nothing else's.
fn with_service_line(line: &str) -> String {
    format!("services:\n  db:\n    image: postgres:16\n    {line}\n")
}

fn refused_key(text: &str, key: &str) {
    match parse(text) {
        Err(ServicesError::ServiceKey { service, key: k }) => {
            assert_eq!(service, "db", "{text}");
            assert_eq!(k, key, "{text}");
        }
        other => panic!("{key} was not refused by name: {other:?}\n{text}"),
    }
}

// --- each key that carries power ----------------------------------------------

#[test]
fn a_privileged_service_is_refused() {
    refused_key(&with_service_line("privileged: true"), "privileged");
}

#[test]
fn a_service_adding_a_capability_is_refused() {
    refused_key(&with_service_line("cap_add: [SYS_ADMIN]"), "cap_add");
}

#[test]
fn a_service_mapping_a_device_is_refused() {
    refused_key(
        &with_service_line("devices: [\"/dev/kvm:/dev/kvm\"]"),
        "devices",
    );
}

#[test]
fn a_service_setting_its_security_options_is_refused() {
    refused_key(
        &with_service_line("security_opt: [\"seccomp:unconfined\"]"),
        "security_opt",
    );
}

#[test]
fn a_service_joining_the_hosts_pid_namespace_is_refused() {
    refused_key(&with_service_line("pid: host"), "pid");
}

#[test]
fn a_service_joining_the_hosts_ipc_namespace_is_refused() {
    refused_key(&with_service_line("ipc: host"), "ipc");
}

#[test]
fn a_service_choosing_its_network_mode_is_refused() {
    refused_key(&with_service_line("network_mode: host"), "network_mode");
    refused_key(
        &with_service_line("network_mode: \"service:firewall\""),
        "network_mode",
    );
}

#[test]
fn a_service_borrowing_another_containers_volumes_is_refused() {
    refused_key(&with_service_line("volumes_from: [agent]"), "volumes_from");
}

#[test]
fn a_service_reading_a_file_of_the_humans_into_its_environment_is_refused() {
    refused_key(
        &with_service_line("env_file: [/home/human/.env]"),
        "env_file",
    );
}

#[test]
fn a_service_built_from_a_context_is_refused() {
    refused_key(&with_service_line("build: ."), "build");
}

#[test]
fn a_service_extending_another_definition_is_refused() {
    refused_key(
        &with_service_line("extends: {file: other.yaml, service: x}"),
        "extends",
    );
}

#[test]
fn a_service_taking_configs_is_refused() {
    refused_key(&with_service_line("configs: [cfg]"), "configs");
}

#[test]
fn a_service_taking_secrets_is_refused() {
    refused_key(&with_service_line("secrets: [token]"), "secrets");
}

#[test]
fn a_service_choosing_its_user_is_refused() {
    refused_key(&with_service_line("user: root"), "user");
}

#[test]
fn a_service_joining_a_network_is_refused() {
    refused_key(&with_service_line("networks: [back]"), "networks");
}

#[test]
fn an_unknown_service_key_is_refused_not_dropped() {
    refused_key(&with_service_line("x-note: kept"), "x-note");
    refused_key(&with_service_line("profiles: [debug]"), "profiles");
    refused_key(&with_service_line("restart: always"), "restart");
    refused_key(
        &with_service_line("some_key_compose_adds_next_year: 1"),
        "some_key_compose_adds_next_year",
    );
}

// --- mounts -------------------------------------------------------------------

fn refused_mount(text: &str) {
    assert!(
        matches!(parse(text), Err(ServicesError::Mount { ref service, .. }) if service == "db"),
        "{text}: {:?}",
        parse(text)
    );
}

#[test]
fn a_bind_mount_in_short_syntax_is_refused() {
    refused_mount(&with_service_line("volumes: [\"/:/host\"]"));
    refused_mount(&with_service_line(
        "volumes: [\"/var/run/docker.sock:/var/run/docker.sock\"]",
    ));
    refused_mount(&with_service_line("volumes: [\"./data:/data\"]"));
    refused_mount(&with_service_line("volumes: [\"~/.ssh:/root/.ssh:ro\"]"));
}

#[test]
fn a_bind_mount_in_long_syntax_is_refused() {
    refused_mount(&with_service_line(
        "volumes: [{type: bind, source: /, target: /host}]",
    ));
    // `type: volume` with a source that is a path is a bind mount by another
    // name: the source must be a volume this file declares.
    refused_mount(&format!(
        "{}volumes:\n  data:\n",
        with_service_line("volumes: [{type: volume, source: /etc, target: /host}]")
    ));
}

#[test]
fn a_long_mount_of_another_type_is_refused_for_its_type() {
    // The human who reads the refusal has to learn that the type is what is
    // wrong, not be told the mount holds a key it may not hold.
    for kind in ["bind", "tmpfs", "npipe", "[volume]"] {
        let text = format!(
            "{}volumes:\n  data:\n",
            with_service_line(&format!(
                "volumes: [{{type: {kind}, source: data, target: /data}}]"
            ))
        );
        match parse(&text) {
            Err(ServicesError::Mount { service, why, .. }) => {
                assert_eq!(service, "db");
                assert_eq!(
                    why, "only a named volume is mounted, of type volume",
                    "{text}"
                );
            }
            other => panic!("{text}: {other:?}"),
        }
    }
}

#[test]
fn a_long_mount_carrying_options_is_refused() {
    refused_mount(&format!(
        "{}volumes:\n  data:\n",
        with_service_line(
            "volumes: [{type: volume, source: data, target: /data, volume: {nocopy: true}}]"
        )
    ));
}

#[test]
fn a_mount_of_a_volume_the_file_does_not_declare_is_refused() {
    refused_mount(&with_service_line("volumes: [\"data:/data\"]"));
}

#[test]
fn an_anonymous_volume_is_refused() {
    refused_mount(&with_service_line("volumes: [\"/data\"]"));
}

#[test]
fn a_volume_mounted_at_a_relative_path_is_refused() {
    refused_mount(&format!(
        "{}volumes:\n  data:\n",
        with_service_line("volumes: [\"data:data\"]")
    ));
}

#[test]
fn a_refused_mount_is_named_as_it_was_written() {
    match parse(&with_service_line("volumes: [\"./data:/x\"]")) {
        Err(ServicesError::Mount { mount, .. }) => assert_eq!(mount, "./data:/x"),
        other => panic!("{other:?}"),
    }
    match parse(&with_service_line(
        "volumes: [{type: bind, source: /, target: /x}]",
    )) {
        Err(ServicesError::Mount { mount, .. }) => {
            assert_eq!(mount, "type: 'bind'\nsource: '/'\ntarget: '/x'")
        }
        other => panic!("{other:?}"),
    }
}
#[test]
fn a_named_volume_of_the_file_is_mounted_in_either_syntax() {
    let file = parse(&format!(
        "{}volumes:\n  data:\n",
        with_service_line(
            "volumes: [\"data:/a\", \"data:/b:ro\", {type: volume, source: data, target: /c, read_only: true}]"
        )
    ))
    .unwrap();
    let rendering = file.render();
    for mount in ["\"data:/a\"", "\"data:/b:ro\"", "\"data:/c:ro\""] {
        assert!(rendering.contains(mount), "{mount} in {rendering}");
    }
}

// --- the top level ------------------------------------------------------------

#[test]
fn a_top_level_networks_block_is_refused() {
    let text = "services:\n  db:\n    image: postgres:16\nnetworks:\n  back: {}\n";
    assert_eq!(
        parse(text),
        Err(ServicesError::TopLevelKey("networks".to_string()))
    );
}

#[test]
fn a_top_level_volume_with_any_value_is_refused() {
    for value in [
        "{}",
        "{driver: local}",
        "{driver_opts: {type: none, o: bind, device: /}}",
        "{external: true}",
        "{name: someone-elses}",
        "local",
    ] {
        let text = format!("services: {{}}\nvolumes:\n  data: {value}\n");
        assert_eq!(
            parse(&text),
            Err(ServicesError::VolumeValue("data".to_string())),
            "{text}"
        );
    }
}

#[test]
fn every_other_top_level_key_is_refused() {
    for key in [
        "x-defaults",
        "include",
        "profiles",
        "configs",
        "secrets",
        "version",
    ] {
        let text = format!("services: {{}}\n{key}: []\n");
        assert_eq!(
            parse(&text),
            Err(ServicesError::TopLevelKey(key.to_string())),
            "{text}"
        );
    }
}

#[test]
fn a_top_level_name_is_read_and_ignored() {
    let file = parse("name: whatever\nservices:\n  db:\n    image: postgres:16\n").unwrap();
    assert!(!file.render().contains("whatever"), "{}", file.render());
    assert!(matches!(
        parse("name: [not, a, name]\nservices: {}\n"),
        Err(ServicesError::Shape { .. })
    ));
}

// --- names --------------------------------------------------------------------

#[test]
fn a_service_name_is_one_dns_label() {
    for name in ["Db", "db_1", "db.x", "-db", "db-", "\"\""] {
        let text = format!("services:\n  {name}:\n    image: postgres:16\n");
        assert!(
            matches!(parse(&text), Err(ServicesError::ServiceName(_))),
            "{name}: {:?}",
            parse(&text)
        );
    }
    let long = "a".repeat(64);
    assert!(matches!(
        parse(&format!("services:\n  {long}:\n    image: x\n")),
        Err(ServicesError::ServiceName(_))
    ));
    let longest = "a".repeat(63);
    assert!(parse(&format!("services:\n  {longest}:\n    image: x\n")).is_ok());
    assert!(parse("services:\n  db-1:\n    image: x\n").is_ok());
}

#[test]
fn a_service_may_not_take_a_name_nunki_owns() {
    for name in ["firewall", "agent", "prober"] {
        let text = format!("services:\n  {name}:\n    image: postgres:16\n");
        assert_eq!(
            parse(&text),
            Err(ServicesError::ReservedService(name.to_string()))
        );
    }
}

#[test]
fn a_volume_name_may_not_start_with_nunkis_prefix() {
    assert_eq!(
        parse("services: {}\nvolumes:\n  nunki-one-harness:\n"),
        Err(ServicesError::ReservedVolume(
            "nunki-one-harness".to_string()
        ))
    );
    assert!(matches!(
        parse("services: {}\nvolumes:\n  \"../etc\":\n"),
        Err(ServicesError::VolumeName(_))
    ));
    assert!(parse("services: {}\nvolumes:\n  nunkidata:\n").is_ok());
}

#[test]
fn every_character_of_a_volume_name_is_checked_not_only_the_first() {
    for name in ["data/../etc", "data:/host", "data x", "data$x"] {
        assert_eq!(
            parse(&format!("services: {{}}\nvolumes:\n  \"{name}\":\n")),
            Err(ServicesError::VolumeName(name.to_string())),
            "{name}"
        );
    }
    assert!(parse("services: {}\nvolumes:\n  Data_1.v-2:\n").is_ok());
}

// --- the environment ------------------------------------------------------------

#[test]
fn a_bare_environment_entry_is_refused() {
    // Compose fills it from the host's environment.
    for line in [
        "environment:\n      HOME:",
        "environment:\n      HOME: null",
        "environment:\n      HOME: \"\"",
        "environment: [HOME]",
        "environment: [\"HOME=x\"]",
    ] {
        assert!(
            matches!(
                parse(&with_service_line(line)),
                Err(ServicesError::Environment { .. }) | Err(ServicesError::Shape { .. })
            ),
            "{line}: {:?}",
            parse(&with_service_line(line))
        );
    }
}

#[test]
fn an_environment_value_is_a_literal_string() {
    assert!(matches!(
        parse(&with_service_line("environment:\n      PORT: 5432")),
        Err(ServicesError::Environment { .. })
    ));
    assert!(matches!(
        parse(&with_service_line("environment:\n      \"A=B\": x")),
        Err(ServicesError::Environment { .. })
    ));
}

#[test]
fn an_environment_name_may_start_with_an_underscore_but_not_a_digit() {
    let names = |file: ServicesFile| -> Vec<String> {
        file.services["db"].environment.keys().cloned().collect()
    };
    let file = parse(&with_service_line(
        "environment:\n      _PRIVATE: x\n      PG_1: y",
    ))
    .unwrap();
    assert_eq!(names(file), ["PG_1", "_PRIVATE"]);
    match parse(&with_service_line("environment:\n      \"1PORT\": x")) {
        Err(ServicesError::Environment { name, .. }) => assert_eq!(name, "1PORT"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn interpolation_is_rendered_as_a_literal() {
    let file = parse(&with_service_line(
        "environment:\n      PGHOST: ${HOME}\n      B: $$X\n    command: echo $PATH",
    ))
    .unwrap();
    let rendering = file.render();
    assert!(rendering.contains(r#""PGHOST": "$${HOME}""#), "{rendering}");
    assert!(rendering.contains(r#""B": "$$$$X""#), "{rendering}");
    assert!(
        rendering.contains(r#"command: "echo $$PATH""#),
        "{rendering}"
    );
    assert_eq!(rendering.matches('$').count() % 2, 0, "{rendering}");
}

// --- the YAML features --------------------------------------------------------

fn refused_feature(text: &str, feature: &str) {
    match parse(text) {
        Err(ServicesError::Feature { feature: f, .. }) => assert_eq!(f, feature, "{text}"),
        other => panic!("{feature} not refused: {other:?}\n{text}"),
    }
}

#[test]
fn a_merge_key_is_refused() {
    let text = "services:\n  db:\n    <<: {image: postgres:16, privileged: true}\n";
    assert!(
        matches!(parse(text), Err(ServicesError::MergeKey { .. })),
        "{:?}",
        parse(text)
    );
    // Even quoted, and even where a key could be anything.
    let quoted = with_service_line("environment:\n      \"<<\": x");
    assert!(matches!(
        parse(&quoted),
        Err(ServicesError::MergeKey { .. })
    ));
}

#[test]
fn a_tag_is_refused() {
    refused_feature(&with_service_line("command: !!str echo"), "a tag");
    refused_feature(&with_service_line("command: !custom echo"), "a tag");
    refused_feature("!!map\nservices: {}\n", "a tag");
    refused_feature("services:\n  !!str db:\n    image: x\n", "a tag");
    refused_feature(
        "services: {db: !<tag:yaml.org,2002:map> {image: x}}\n",
        "a tag",
    );
}

#[test]
fn an_anchor_and_an_alias_are_refused() {
    refused_feature(
        "services:\n  db:\n    image: &img postgres:16\n  other:\n    image: *img\n",
        "an anchor",
    );
    refused_feature("services:\n  db:\n    image: *img\n", "an alias");
    refused_feature("services:\n  - &x a\n", "an anchor");
    refused_feature("services: {db: {image: &a x}}\n", "an anchor");
    refused_feature("services: [&a x]\n", "an anchor");
    // Not YAML to the parser at all (a quoted key, `:` and an anchor with
    // no space between): refused as unreadable. The previous scan named an
    // anchor here; the parser, which decides now, sees none.
    assert!(matches!(
        parse("\"services\":&a {}\n"),
        Err(ServicesError::Yaml(_))
    ));
    refused_feature("\"services\": &a {}\n", "an anchor");
    refused_feature("services:\n  db:\n    image:\n      &a x\n", "an anchor");
}

#[test]
fn an_anchor_after_a_block_scalar_is_still_seen() {
    refused_feature(
        "services:\n  db:\n    image: x\n    command: |\n      echo *not-an-alias\n    \
         entrypoint: &a [sh]\n",
        "an anchor",
    );
    refused_feature(
        "services:\n  - key: |\n      text\n    other: &a x\n",
        "an anchor",
    );
}

#[test]
fn indicators_inside_quotes_block_scalars_and_comments_are_text() {
    let file = parse(
        "# a comment with &anchor and *alias and !tag\nservices:\n  db:\n    image: \"&x\"\n    \
         command: 'a && b * c !d'\n    entrypoint: >\n      &literal *text !here\n    \
         working_dir: /a&b # trailing &comment\n",
    )
    .unwrap();
    let rendering = file.render();
    assert!(rendering.contains(r#"image: "&x""#), "{rendering}");
    assert!(
        rendering.contains(r#"command: "a && b * c !d""#),
        "{rendering}"
    );
    assert!(
        rendering.contains(r#"entrypoint: "&literal *text !here\n""#),
        "{rendering}"
    );
    assert!(rendering.contains(r#"working_dir: "/a&b""#), "{rendering}");
}

#[test]
fn duplicate_keys_are_refused() {
    let text = "services:\n  db:\n    image: x\n    image: y\n";
    assert!(
        matches!(parse(text), Err(ServicesError::Yaml(_))),
        "{:?}",
        parse(text)
    );
}

#[test]
fn an_escaped_key_is_the_key_it_decodes_to() {
    // `\x65` is `e`: the key is `privileged`, and it is refused as such.
    refused_key(&with_service_line(r#""privil\x65ged": true"#), "privileged");
    refused_key(&with_service_line(r#""pid": host"#), "pid");
    // And a key spelled twice, once escaped, is a duplicate.
    let twice = "services:\n  db:\n    image: x\n    \"im\\x61ge\": y\n";
    assert!(matches!(parse(twice), Err(ServicesError::Yaml(_))));
}

#[test]
fn more_than_one_document_is_refused() {
    refused_feature(
        "services: {}\n---\nservices:\n  db:\n    image: x\n",
        "a second document",
    );
    refused_feature("---\nservices: {}\n---\n", "a second document");
    refused_feature("services: {}\n...\n", "a document end marker");
    assert!(parse("---\nservices: {}\n").is_ok());
}

#[test]
fn a_directive_is_refused() {
    refused_feature("%YAML 1.1\n---\nservices: {}\n", "a directive");
    refused_feature("%TAG ! tag:x,2000:\n---\nservices: {}\n", "a directive");
}

#[test]
fn a_feature_is_named_with_its_line() {
    assert_eq!(
        parse("services:\n  db:\n    image: *img\n"),
        Err(ServicesError::Feature {
            feature: "an alias",
            line: 3
        })
    );
}

// --- what the YAML 1.2 reader reads otherwise ---------------------------------
//
// The reader nunki uses now (YAML 1.2) and the one it used before (YAML 1.1)
// disagree on a few spellings. A file accepted before is read to the same
// model, or it is refused; these tests hold each disagreement measured to
// one side or the other.

#[test]
fn text_after_a_document_end_marker_is_refused() {
    refused_feature("services: {}\n...\nvolumes: {}\n", "a document end marker");
    refused_feature("services: {}\n... # end\n", "a document end marker");
}

/// A file refused for `feature`, on `line`.
fn refused_at(text: &str, feature: &str, line: usize) {
    match parse(text) {
        Err(ServicesError::Feature {
            feature: f,
            line: l,
        }) => {
            assert_eq!((f, l), (feature, line), "{text:?}")
        }
        other => panic!("{feature} on line {line} not refused: {other:?}\n{text:?}"),
    }
}

/// Every tag is refused, from the parser's own events, with its line: the
/// ones the reader would resolve without a trace (`!!binary aGk=` to `hi`,
/// `!!str 5` and `! 5` to `5`, `!!null x` to null, `!!str db:` to the key
/// `db`) as much as one it does not know. And the reader, read alone, refuses
/// each of them too, naming the tag: the services file is not the only
/// file a tag cannot pass.
#[test]
fn a_tag_is_refused_in_every_form_by_the_parsers_events_and_by_the_reader_alone() {
    for (text, line, tag) in [
        (with_service_line("command: !custom echo"), 4, "!custom"),
        (
            "services:\n  db:\n    image: !!binary aGk=\n".to_string(),
            3,
            "tag:yaml.org,2002:binary",
        ),
        (
            with_service_line("environment:\n      A: !!str 5"),
            5,
            "tag:yaml.org,2002:str",
        ),
        (with_service_line("environment:\n      A: ! 5"), 5, "!"),
        (
            "services:\n  db:\n    image: !<tag:yaml.org,2002:str> x\n".to_string(),
            3,
            "tag:yaml.org,2002:str",
        ),
        (
            "services: {}\nvolumes:\n  data: !!null x\n".to_string(),
            3,
            "tag:yaml.org,2002:null",
        ),
        (
            "services:\n  !!str db:\n    image: x\n".to_string(),
            2,
            "tag:yaml.org,2002:str",
        ),
    ] {
        refused_at(&text, "a tag", line);
        let alone = nunki::yaml::from_str::<Value>(&text).unwrap_err();
        assert_eq!(alone.place.map(|(l, _)| l), Some(line as u64), "{text:?}");
        assert!(
            alone.message.starts_with(&format!("a tag ({tag}), ")),
            "{text:?}: {alone}"
        );
    }
}

#[test]
fn a_merge_key_is_refused_in_every_spelling() {
    for spelled in ["<<", "\"<<\"", "'<<'"] {
        let text = format!("services:\n  db:\n    {spelled}: {{image: x}}\n");
        assert!(
            matches!(
                parse(&text),
                Err(ServicesError::MergeKey { at }) if at == "the file.services.db"
            ),
            "{text}: {:?}",
            parse(&text)
        );
    }
    // Tagged, it is a tag, refused as one, by the services file and by the
    // reader alone.
    let tagged = "services:\n  db:\n    !!merge <<: {image: x}\n";
    refused_at(tagged, "a tag", 3);
    assert!(nunki::yaml::from_str::<Value>(tagged).is_err());
}

#[test]
fn a_key_written_twice_is_refused_even_when_one_is_escaped() {
    for text in [
        with_service_line("environment:\n      a: x\n      \"\\x61\": y"),
        "services: {}\n\"s\\x65rvices\": {}\n".to_string(),
    ] {
        let err = parse(&text).unwrap_err();
        assert!(
            matches!(&err, ServicesError::Yaml(said) if said.contains("duplicate")),
            "{text}: {err:?}"
        );
    }
}

#[test]
fn a_key_of_any_kind_but_a_string_is_refused() {
    // The last four were strings to the reader nunki used before, and are
    // numbers to this one.
    for key in [
        "1", "true", "null", "~", "1.5", "[a]", "{a: b}", "", "012", "00", "1_000", "-1_0",
    ] {
        let text = format!("services:\n  db:\n    image: x\n    {key}: y\n");
        assert!(
            matches!(
                parse(&text),
                Err(ServicesError::KeyNotString { at }) if at == "the file.services.db"
            ),
            "{key:?}: {:?}",
            parse(&text)
        );
    }
    // Quoted, they are names like any other, and refused as names.
    refused_key(&with_service_line("\"1_000\": y"), "1_000");
}

/// `012`, `00` and `09` read as floats, `1_000` and `-1_0` as integers:
/// where the model takes a string, the file is refused, as a number always
/// was there, and never lifted as the number's text.
#[test]
fn a_plain_scalar_read_as_a_number_is_refused_where_a_string_is_expected() {
    for value in ["012", "00", "09", "1_000", "-1_0"] {
        for (line, expected) in [
            (format!("image: {value}"), "a string"),
            (
                format!("entrypoint: {value}"),
                "a string or a list of strings",
            ),
            (format!("command: [run, {value}]"), "a string"),
            (format!("working_dir: {value}"), "a string"),
            (format!("healthcheck:\n      interval: {value}"), "a string"),
        ] {
            let text = format!("services:\n  db:\n    {line}\n    image: x\n").replacen(
                "    image: x\n",
                "",
                usize::from(line.starts_with("image")),
            );
            assert!(
                matches!(
                    parse(&text),
                    Err(ServicesError::Shape { expected: e, .. }) if e == expected
                ),
                "{text}: {:?}",
                parse(&text)
            );
        }
        let environment = with_service_line(&format!("environment:\n      A: {value}"));
        assert!(
            matches!(
                parse(&environment),
                Err(ServicesError::Environment { why, .. }) if why.contains("quote it")
            ),
            "{environment}: {:?}",
            parse(&environment)
        );
        // A volume, and a service, named so.
        for text in [
            format!("services: {{}}\nvolumes:\n  {value}:\n"),
            format!("services:\n  {value}:\n    image: x\n"),
        ] {
            assert!(
                matches!(parse(&text), Err(ServicesError::KeyNotString { .. })),
                "{text}: {:?}",
                parse(&text)
            );
        }
        // Quoted, each is the string written (and a volume name starts on a
        // letter or a digit, so `-1_0` is no volume's).
        let quoted = parse(&format!("services:\n  db:\n    image: \"{value}\"\n")).unwrap();
        assert_eq!(quoted.services["db"].image, value);
        if !value.starts_with('-') {
            let named = parse(&format!(
                "services:\n  db:\n    image: x\n    volumes: [\"{value}:/data\"]\n\
                 volumes:\n  \"{value}\":\n"
            ))
            .unwrap();
            assert!(named.volumes.contains(value));
        }
    }
}

/// The one plain scalar this reader takes as a number where the one before
/// refused it: `1_000` was a string, which a count refuses; it is now the
/// integer 1000, which is what Compose reads too, and what the rendering
/// shows the human who approves it.
#[test]
fn a_count_of_retries_spelled_with_an_underscore_is_the_number_it_spells() {
    let file = parse(&with_service_line("healthcheck:\n      retries: 1_000")).unwrap();
    assert_eq!(
        file.services["db"]
            .healthcheck
            .as_ref()
            .and_then(|h| h.retries),
        Some(1000)
    );
    assert!(file.render().contains("      retries: 1000\n"));
    // A negative or a float is no count, as before.
    for retries in ["-1_0", "012", "1.5"] {
        assert!(matches!(
            parse(&with_service_line(&format!(
                "healthcheck:\n      retries: {retries}"
            ))),
            Err(ServicesError::Shape {
                expected: "a whole number",
                ..
            })
        ));
    }
}

/// A raw NEL, LS or PS is a line break to a YAML 1.1 reader and text to
/// this one. Measured before this reader: in a quoted scalar a NEL was
/// folded to a space, and in a comment each of the three ended the comment
/// and made the rest of the line YAML. Refused, wherever it stands.
#[test]
fn a_character_yaml_1_1_reads_as_a_line_break_is_refused_anywhere() {
    const FEATURE: &str = "a NEL, LS or PS character, which YAML 1.1 reads as a line break";
    for c in ['\u{85}', '\u{2028}', '\u{2029}'] {
        for line in [
            format!("A: \"a{c}b\""),
            format!("A: 'a{c}b'"),
            format!("A: a{c}b"),
            format!("A: a # c{c}privileged: true"),
        ] {
            let text = with_service_line(&format!("environment:\n      {line}"));
            assert_eq!(
                parse(&text),
                Err(ServicesError::Feature {
                    feature: FEATURE,
                    line: 5
                }),
                "{text:?}"
            );
        }
    }
    // Written as an escape, each reads the same to both readers, and is kept.
    let file = parse(&with_service_line(
        "environment:\n      A: \"a\\Nb\\Lc\\Pd\"",
    ))
    .unwrap();
    assert_eq!(
        file.services["db"].environment["A"],
        "a\u{85}b\u{2028}c\u{2029}d"
    );
    assert!(
        file.render().contains(r#""A": "a\u0085b\u2028c\u2029d""#),
        "{}",
        file.render()
    );
}

// --- the other fields -----------------------------------------------------------

#[test]
fn a_healthcheck_holds_only_its_own_keys() {
    let ok = parse(&with_service_line(
        "healthcheck:\n      test: pg_isready\n      interval: 5s\n      timeout: 3s\n      \
         retries: 5\n      start_period: 10s\n      start_interval: 1s",
    ))
    .unwrap();
    let rendering = ok.render();
    assert!(
        rendering.contains(
            "    healthcheck:\n      test: \"pg_isready\"\n      interval: \"5s\"\n      \
             timeout: \"3s\"\n      retries: 5\n      start_period: \"10s\"\n      \
             start_interval: \"1s\"\n"
        ),
        "{rendering}"
    );
    assert_eq!(
        parse(&with_service_line("healthcheck:\n      disable: true")),
        Err(ServicesError::HealthcheckKey {
            service: "db".to_string(),
            key: "disable".to_string()
        })
    );
    assert!(matches!(
        parse(&with_service_line("healthcheck:\n      retries: many")),
        Err(ServicesError::Shape { .. })
    ));
}

#[test]
fn a_service_depends_only_on_a_service_of_the_file() {
    let file = parse(
        "services:\n  app:\n    image: x\n    depends_on: [db]\n  db:\n    image: y\n  \
         web:\n    image: z\n    depends_on:\n      db: {condition: service_healthy}\n",
    )
    .unwrap();
    let rendering = file.render();
    assert!(
        rendering
            .contains("    depends_on:\n      \"db\":\n        condition: \"service_started\"\n"),
        "{rendering}"
    );
    assert!(
        rendering.contains("condition: \"service_healthy\""),
        "{rendering}"
    );

    for (line, on) in [
        ("depends_on: [firewall]", "firewall"),
        ("depends_on: [elsewhere]", "elsewhere"),
        (
            "depends_on:\n      db: {condition: service_healthy, restart: true}",
            "db",
        ),
        ("depends_on:\n      db: {condition: whenever}", "db"),
    ] {
        let text = format!("services:\n  app:\n    image: x\n    {line}\n  db:\n    image: y\n");
        assert!(
            matches!(parse(&text), Err(ServicesError::DependsOn { on: ref o, .. }) if o == on),
            "{line}: {:?}",
            parse(&text)
        );
    }
}

#[test]
fn a_service_without_an_image_is_refused() {
    assert!(matches!(
        parse("services:\n  db:\n    command: x\n"),
        Err(ServicesError::Shape { .. })
    ));
}

#[test]
fn a_working_dir_is_an_absolute_path() {
    assert!(matches!(
        parse(&with_service_line("working_dir: here")),
        Err(ServicesError::Shape { .. })
    ));
}

#[test]
fn ports_are_dropped_from_the_rendering_and_said() {
    let file = parse(&with_service_line("ports: [\"5432:5432\"]")).unwrap();
    assert!(!file.render().contains("5432"), "{}", file.render());
    assert_eq!(file.dropped_ports(), vec!["db"]);
    assert!(
        parse(&with_service_line("environment: {A: b}"))
            .unwrap()
            .dropped_ports()
            .is_empty()
    );
}

// --- the rendering --------------------------------------------------------------

#[test]
fn the_rendering_is_canonical() {
    // The same model written two ways renders the same bytes.
    let one = parse(
        "services:\n  web:\n    image: nginx\n    command: [a, b]\n  db:\n    \
         environment: {B: \"2\", A: \"1\"}\n    image: postgres:16\n",
    )
    .unwrap();
    let two = parse(
        "services:\n  db:\n    image: 'postgres:16'\n    environment:\n      A: '1'\n      \
         B: \"2\"\n  web:\n    command:\n      - a\n      - b\n    image: \"nginx\"\n",
    )
    .unwrap();
    assert_eq!(one.render(), two.render());
    assert_eq!(
        one.render(),
        "services:\n  \"db\":\n    image: \"postgres:16\"\n    environment:\n      \
         \"A\": \"1\"\n      \"B\": \"2\"\n  \"web\":\n    image: \"nginx\"\n    command:\n      \
         - \"a\"\n      - \"b\"\n"
    );
}

#[test]
fn the_rendering_is_ascii_and_reads_back_as_the_model() {
    let file = parse(&with_service_line(
        "environment:\n      A: \"caf\\u00e9 \\x1b[2J \\u2028 \\U0001F600 \\\" \\\\\"",
    ))
    .unwrap();
    let rendering = file.render();
    assert!(rendering.is_ascii(), "{rendering}");
    assert!(
        rendering.contains(&format!(
            r#""A": "caf{b}u00e9 {b}u001b[2J {b}u2028 {b}U0001f600 {b}" {b}{b}""#,
            b = '\x5c'
        )),
        "{rendering}"
    );
    let back: nunki::yaml::Value = nunki::yaml::from_str(&rendering).unwrap();
    assert_eq!(
        back["services"]["db"]["environment"]["A"].as_str(),
        Some("café \u{1b}[2J \u{2028} 😀 \" \\")
    );
}

#[test]
fn an_empty_file_lifts_nothing() {
    assert_eq!(parse("").unwrap(), ServicesFile::default());
    assert_eq!(parse("services:\n").unwrap().render(), "services: {}\n");
}

#[test]
fn a_file_that_is_not_a_mapping_is_refused() {
    assert!(matches!(parse("- a\n"), Err(ServicesError::Shape { .. })));
    assert!(matches!(
        parse("services: [a]\n"),
        Err(ServicesError::Shape { .. })
    ));
    assert!(matches!(
        parse("services:\n  db: x\n"),
        Err(ServicesError::Shape { .. })
    ));
    assert!(matches!(
        parse("services: {}\nvolumes: [a]\n"),
        Err(ServicesError::Shape { .. })
    ));
    assert!(matches!(
        parse("services:\n  [a]: x\n"),
        Err(ServicesError::KeyNotString { .. })
    ));
    assert!(matches!(parse("a: [\n"), Err(ServicesError::Yaml(_))));
}

// --- the files in real use --------------------------------------------------------

/// A Postgres service with a healthcheck, without ports.
const POSTGRES: &str = "\
name: demo
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: demo
      POSTGRES_PASSWORD: demo
      POSTGRES_DB: demo
    healthcheck:
      test: [\"CMD-SHELL\", \"pg_isready -U demo -d demo\"]
      interval: 2s
      timeout: 5s
      retries: 30
";

/// The same, publishing its port.
const POSTGRES_WITH_PORTS: &str = "\
name: demo
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: demo
      POSTGRES_PASSWORD: demo
      POSTGRES_DB: demo
    ports:
      - \"5432:5432\"
    healthcheck:
      test: [\"CMD-SHELL\", \"pg_isready -U demo -d demo\"]
      interval: 2s
      timeout: 5s
      retries: 30
";

/// A Postgres and a QuestDB service, each on a loopback port.
const POSTGRES_AND_QUESTDB: &str = "\
name: market
services:
  postgres:
    image: postgres:16
    environment:
      POSTGRES_USER: market
      POSTGRES_PASSWORD: market
      POSTGRES_DB: market
    ports: [\"127.0.0.1::5432\"]
    healthcheck:
      test: [\"CMD-SHELL\", \"pg_isready -U market\"]
      interval: 5s
      timeout: 5s
      retries: 10
  questdb:
    image: questdb/questdb:8.2.1
    environment:
      QDB_TELEMETRY_ENABLED: \"false\"
    ports: [\"127.0.0.1::9000\", \"127.0.0.1::8812\"]
    healthcheck:
      test: [\"CMD-SHELL\", \"curl -fs http://localhost:9003/status || exit 1\"]
      interval: 5s
      timeout: 5s
      retries: 20
";

#[test]
fn the_three_files_in_real_use_parse() {
    for (text, services) in [
        (POSTGRES, vec!["postgres"]),
        (POSTGRES_WITH_PORTS, vec!["postgres"]),
        (POSTGRES_AND_QUESTDB, vec!["postgres", "questdb"]),
    ] {
        let file = parse(text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(file.services.keys().collect::<Vec<_>>(), services);
        let rendering = file.render();
        assert!(!rendering.contains("ports"), "{rendering}");
        assert!(rendering.contains("pg_isready"), "{rendering}");
    }
    // With ports or without, the definition a human approves is the same.
    assert_eq!(
        parse(POSTGRES).unwrap().render(),
        parse(POSTGRES_WITH_PORTS).unwrap().render()
    );
    assert_eq!(
        parse(POSTGRES_AND_QUESTDB).unwrap().dropped_ports(),
        vec!["postgres", "questdb"]
    );
}

// --- trust is a digest a human approved (L2) ------------------------------------

use std::path::{Path, PathBuf};

use nunki::services::{ApprovalError, Approvals, Approved, At};

fn git(at: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(at)
        .args(args)
        .output()
        .expect("git is on the path");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository and one slot of it, both holding `compose.yaml` in their
/// one commit, and a project that declares it as its services file.
struct World {
    _dir: tempfile::TempDir,
    project: nunki::project::Project,
    slot: PathBuf,
}

const FIRST: &str = "services:\n  db:\n    image: postgres:16\n";
const SECOND: &str = "services:\n  db:\n    image: postgres:17\n";

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let slot = dir.path().join("repo-slots").join("one");
        for tree in [&root, &slot] {
            std::fs::create_dir_all(tree).unwrap();
            git(tree, &["init", "-q", "-b", "dev"]);
            git(tree, &["config", "user.name", "Alex Martin"]);
            git(tree, &["config", "user.email", "alex@example.com"]);
            std::fs::write(tree.join("compose.yaml"), FIRST).unwrap();
            git(tree, &["add", "-A"]);
            git(tree, &["commit", "-q", "-m", "services"]);
        }
        let project = nunki::project::Project::at(
            root,
            nunki::project::Config {
                root: None,
                harness: "claude-code".into(),
                forge: vec!["github.com".into()],
                stacks: vec!["rust".into()],
                protected_branches: vec!["main".into()],
                protected_paths: Default::default(),
                account: None,
                model: None,
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: Some("compose.yaml".into()),
                permission_mode: "auto".to_string(),
                rigor: None,
                mutation_threshold: 80,
                mutation_jobs: None,
                forge_protection: Default::default(),
                security_findings: Default::default(),
            },
            dir.path().join("nunki"),
        );
        Self {
            _dir: dir,
            project,
            slot,
        }
    }

    /// A commit that changes the file, read where the slot's HEAD is: the
    /// reading of a commit, whoever made it. On a mission, only the base's
    /// is ever lifted, and gate 4 refuses an agent's.
    fn commits(&self, body: &str) {
        std::fs::write(self.slot.join("compose.yaml"), body).unwrap();
        git(&self.slot, &["commit", "-q", "-am", "the file changes"]);
    }

    /// The slot's HEAD, read and rendered: what `--show` prints.
    fn shown(&self) -> nunki::services::Current {
        nunki::services::read(&self.project, At::Slot(&self.slot))
            .unwrap()
            .unwrap()
    }

    fn approve(&self, digest: &str) -> Result<Approved, ApprovalError> {
        let current = vec![self.shown()];
        nunki::services::approve(&self.project, digest, &current)
    }

    fn approvals(&self) -> Approvals {
        Approvals::load(&nunki::services::file(&self.project)).unwrap()
    }

    fn lifted(&self) -> Result<Option<ServicesFile>, ApprovalError> {
        nunki::services::approved(&self.project, At::Slot(&self.slot))
    }
}

#[test]
fn a_rendering_nobody_approved_lifts_nothing_and_names_its_digest() {
    let world = World::new();
    let shown = world.shown();
    assert_eq!(
        shown.digest,
        nunki::compose::services::digest(&shown.rendering)
    );
    assert!(shown.digest.starts_with("sha256:"), "{}", shown.digest);
    match world.lifted() {
        Err(ApprovalError::NotApproved { path, digest }) => {
            assert_eq!(path, "compose.yaml");
            assert_eq!(digest, shown.digest);
        }
        other => panic!("an unapproved rendering was let through: {other:?}"),
    }
    let said = world.lifted().unwrap_err().to_string();
    assert!(
        said.contains(&format!("nunki services --approve {}", shown.digest)),
        "{said}"
    );
}

#[test]
fn an_approved_rendering_is_lifted() {
    let world = World::new();
    let digest = world.shown().digest;
    assert!(matches!(world.approve(&digest), Ok(Approved::Now(_))));
    let lifted = world
        .lifted()
        .unwrap()
        .expect("a services file is declared");
    assert_eq!(lifted.digest(), digest);
    assert!(lifted.services.contains_key("db"));
}

#[test]
fn approving_a_stale_digest_is_refused_and_records_nothing() {
    let world = World::new();
    let shown = world.shown().digest;
    // Between `--show` and `--approve`, the file changes.
    world.commits(SECOND);
    match world.approve(&shown) {
        Err(ApprovalError::Stale { digest, current }) => {
            assert_eq!(digest, shown);
            assert!(current.contains(&world.shown().digest), "{current}");
        }
        other => panic!("a digest of a file that changed was approved: {other:?}"),
    }
    assert!(world.approvals().approved.is_empty());
    assert!(
        !nunki::services::file(&world.project).exists(),
        "a refused approval writes nothing"
    );
    let said = world.approve(&shown).unwrap_err().to_string();
    assert!(said.contains("changed since it was shown"), "{said}");
}

#[test]
fn a_later_change_needs_a_new_approval() {
    let world = World::new();
    let first = world.shown().digest;
    world.approve(&first).unwrap();
    assert!(world.lifted().is_ok());

    world.commits(SECOND);
    let second = world.shown().digest;
    assert_ne!(first, second);
    assert!(
        matches!(world.lifted(), Err(ApprovalError::NotApproved { ref digest, .. }) if *digest == second),
        "{:?}",
        world.lifted()
    );
    world.approve(&second).unwrap();
    assert_eq!(world.lifted().unwrap().unwrap().digest(), second);
    // The first stays approved: a definition approved once serves every
    // mission until it changes, and changing back needs nothing new.
    assert_eq!(world.approvals().approved.len(), 2);
}

#[test]
fn a_change_in_the_tree_that_is_not_committed_changes_nothing() {
    let world = World::new();
    let digest = world.shown().digest;
    world.approve(&digest).unwrap();
    std::fs::write(
        world.slot.join("compose.yaml"),
        "services:\n  db:\n    image: postgres:16\n    privileged: true\n",
    )
    .unwrap();
    assert_eq!(world.shown().digest, digest);
    assert_eq!(world.lifted().unwrap().unwrap().digest(), digest);
}

#[test]
fn a_file_that_is_only_in_the_tree_is_not_on_the_commit() {
    let world = World::new();
    git(&world.slot, &["rm", "-q", "compose.yaml"]);
    git(&world.slot, &["commit", "-q", "-m", "gone"]);
    std::fs::write(world.slot.join("compose.yaml"), FIRST).unwrap();
    assert!(matches!(
        world.lifted(),
        Err(ApprovalError::NotOnCommit { .. })
    ));
}

#[test]
fn a_symbolic_link_in_place_of_the_file_is_refused() {
    let world = World::new();
    git(&world.slot, &["rm", "-q", "compose.yaml"]);
    std::os::unix::fs::symlink("/etc/passwd", world.slot.join("compose.yaml")).unwrap();
    git(&world.slot, &["add", "-A"]);
    git(&world.slot, &["commit", "-q", "-m", "a link"]);
    assert!(
        matches!(world.lifted(), Err(ApprovalError::NotAFile { ref mode, .. }) if mode == "120000"),
        "{:?}",
        world.lifted()
    );
}

#[test]
fn a_services_file_outside_the_repository_is_refused() {
    let mut world = World::new();
    for path in ["../compose.yaml", "/etc/compose.yaml", ""] {
        world.project.config.services_file = Some(path.into());
        assert!(
            matches!(world.lifted(), Err(ApprovalError::Path(_))),
            "{path}: {:?}",
            world.lifted()
        );
    }
    world.project.config.services_file = Some("./compose.yaml".into());
    assert!(matches!(
        world.lifted(),
        Err(ApprovalError::NotApproved { .. })
    ));
}

#[test]
fn a_project_without_a_services_file_lifts_nothing_and_needs_no_approval() {
    let mut world = World::new();
    world.project.config.services_file = None;
    assert!(world.lifted().unwrap().is_none());
    assert!(matches!(
        nunki::services::readings(&world.project, None, true),
        Err(ApprovalError::NoServicesFile)
    ));
}

#[test]
fn an_approval_records_who_when_and_the_rendering() {
    let world = World::new();
    let shown = world.shown();
    world.approve(&shown.digest).unwrap();
    let approvals = world.approvals();
    let approval = &approvals.approved[0];
    assert_eq!(approval.digest, shown.digest);
    assert_eq!(approval.rendering, shown.rendering);
    assert_eq!(approval.by, "Alex Martin <alex@example.com>");
    assert!(!approval.at.is_empty());
    // Approving it again adds nothing.
    assert!(matches!(
        world.approve(&shown.digest),
        Ok(Approved::Already(_))
    ));
    assert_eq!(world.approvals().approved.len(), 1);
}

#[test]
fn an_approval_by_a_human_whose_declaration_cannot_be_read_says_so() {
    let world = World::new();
    let me_yaml = world.project.nunki_home().join(nunki::human::ME_FILE);
    std::fs::create_dir_all(me_yaml.parent().unwrap()).unwrap();
    std::fs::write(&me_yaml, "name: [Alex\n").unwrap();
    let shown = world.shown();
    world.approve(&shown.digest).unwrap();
    let by = &world.approvals().approved[0].by;
    assert!(by.starts_with("a human nunki has no name for ("), "{by}");
    assert!(by.contains("me.yaml, which cannot be read: line 1"), "{by}");
    // Not git's name in its place.
    assert!(!by.contains("Alex Martin"), "{by}");
}
#[test]
fn a_list_of_approvals_nunki_cannot_read_approves_nothing() {
    let world = World::new();
    let file = nunki::services::file(&world.project);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "not json").unwrap();
    assert!(matches!(world.lifted(), Err(ApprovalError::Corrupt(..))));
}

/// An existing project — a services file, and no approval yet — needs one
/// command: the digest `--show` printed, approved. `--approve` without a
/// mission looks at the repository and every slot.
#[test]
fn an_existing_project_gets_its_first_approval_with_one_command() {
    let world = World::new();
    assert!(!nunki::services::file(&world.project).exists());
    let readings = nunki::services::readings(&world.project, None, true).unwrap();
    let froms: Vec<&str> = readings.iter().map(|r| r.from.as_str()).collect();
    assert_eq!(
        froms,
        vec!["the HEAD of the repository", "the HEAD of slot one"]
    );
    let current: Vec<_> = readings
        .into_iter()
        .filter_map(|r| r.current.ok())
        .collect();
    let digest = current[0].digest.clone();
    assert!(matches!(
        nunki::services::approve(&world.project, &digest, &current),
        Ok(Approved::Now(_))
    ));
    assert!(world.lifted().unwrap().is_some());
}

#[test]
fn show_prints_the_rendering_its_digest_and_whether_it_is_approved() {
    let world = World::new();
    std::fs::write(
        world.slot.join("compose.yaml"),
        "services:\n  db:\n    image: postgres:16\n    ports: [\"5432:5432\"]\n",
    )
    .unwrap();
    git(&world.slot, &["commit", "-q", "-am", "ports"]);
    let shown = world.shown();
    let text = nunki::services::show(&shown, "slot \u{1b}[2Jone", &world.approvals());
    assert!(
        text.starts_with("compose.yaml on slot \\u{1b}[2Jone\n"),
        "{text}"
    );
    assert!(
        text.contains(&format!("digest {}\n", shown.digest)),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "not approved: `nunki services --approve {}`",
            shown.digest
        )),
        "{text}"
    );
    assert!(text.contains("ports dropped from db"), "{text}");
    assert!(
        text.ends_with(&format!("---\n{}", shown.rendering)),
        "{text}"
    );

    world.approve(&shown.digest).unwrap();
    let text = nunki::services::show(&shown, "slot one", &world.approvals());
    assert!(
        text.contains("approved by Alex Martin <alex@example.com> at "),
        "{text}"
    );
}

#[test]
fn check_reports_an_unapproved_file_as_a_red_line() {
    let world = World::new();
    let digest = world.shown().digest;
    let checks = nunki::services::checks(&world.project, Some("one"), None);
    assert_eq!(checks.len(), 1);
    match &checks[0].verdict {
        nunki::check::Verdict::Red(why) => assert!(why.contains(&digest), "{why}"),
        other => panic!("{other:?}"),
    }
    world.approve(&digest).unwrap();
    let checks = nunki::services::checks(&world.project, Some("one"), None);
    assert!(
        matches!(&checks[0].verdict, nunki::check::Verdict::Green(why) if why.contains(&digest)),
        "{checks:?}"
    );
    // A mission that has not started has no slot to read: said, not passed.
    let checks = nunki::services::checks(&world.project, None, Some("m9"));
    assert!(
        matches!(&checks[0].verdict, nunki::check::Verdict::NotChecked(_)),
        "{checks:?}"
    );
    assert!(nunki::services::checks(&world.project, None, None).is_empty());
}

// --- what was lifted before is taken down ---------------------------------------

fn container(id: &str, service: &str, digest: Option<&str>) -> nunki::engine::Container {
    nunki::engine::Container {
        id: id.into(),
        service: service.into(),
        digest: digest.map(str::to_string),
    }
}

#[test]
fn a_system_profile_takes_down_every_definition_but_the_one_it_lifts() {
    let mut approvals = Approvals::default();
    for digest in ["sha256:old", "sha256:new"] {
        approvals.approved.push(nunki::services::Approval {
            digest: digest.into(),
            by: "A".into(),
            at: "t".into(),
            rendering: String::new(),
        });
    }
    let file =
        parse("services:\n  db:\n    image: postgres:16\n  cache:\n    image: redis\n").unwrap();
    let mut lifting = nunki::compose::services::Lifted::declared(file, &["db".to_string()]);
    lifting.approved = "sha256:new".into();
    let containers = [
        container("c1", "db", Some("sha256:new")),
        container("c2", "db", Some("sha256:old")),
        container("c3", "cache", None),
        container("c4", "firewall", None),
        container("c5", "agent", None),
        container("c6", "prober", None),
        container("c7", "gone", Some("sha256:unknown")),
        // Approved, but not what this mission declares: not lifted, so not
        // left up beside it either.
        container("c8", "cache", Some("sha256:new")),
    ];
    assert_eq!(
        nunki::services::stale(&containers, Some(&lifting), &approvals),
        vec!["c2", "c3", "c7", "c8"]
    );
    // A mission profile lifts nothing and keeps what a human approved.
    assert_eq!(
        nunki::services::stale(&containers, None, &approvals),
        vec!["c3", "c7"]
    );
}

// --- only what the mission declares (L3) ----------------------------------------

const THREE: &str = "services:\n  app:\n    image: app\n    depends_on: [db]\n    \
                     volumes: [\"appdata:/a\"]\n  db:\n    image: postgres:16\n    \
                     depends_on:\n      cache: {condition: service_healthy}\n    \
                     volumes: [\"dbdata:/d\"]\n  cache:\n    image: redis\n  \
                     miner:\n    image: coin\n    volumes: [\"loot:/l\"]\n\
                     volumes:\n  appdata:\n  dbdata:\n  loot:\n";

#[test]
fn a_profile_lifts_what_the_mission_declares_and_what_it_depends_on() {
    use nunki::compose::services::Lifted;
    let file = parse(THREE).unwrap();
    let whole = file.digest();
    let lifted = Lifted::declared(file, &["app".to_string()]);
    assert_eq!(
        lifted.services.services.keys().collect::<Vec<_>>(),
        vec!["app", "cache", "db"],
        "app, what it depends on, and what that depends on"
    );
    assert_eq!(lifted.held_back, vec!["miner".to_string()]);
    assert_eq!(
        lifted.services.volumes.iter().collect::<Vec<_>>(),
        vec!["appdata", "dbdata"],
        "a volume only a held-back service mounts is not made"
    );
    // The label is the approved digest — the whole file's — and not the
    // digest of the part lifted, which nobody approved as such.
    assert_eq!(lifted.approved, whole);
    assert_ne!(lifted.services.digest(), whole);
}

#[test]
fn a_declared_name_the_file_does_not_hold_lifts_nothing() {
    use nunki::compose::services::Lifted;
    // A provider of another kind — a third party's test tier — is declared
    // by name too, and is nobody's container.
    let lifted = Lifted::declared(parse(THREE).unwrap(), &["stripe".to_string()]);
    assert!(lifted.services.services.is_empty());
    assert!(lifted.services.volumes.is_empty());
    assert_eq!(lifted.held_back.len(), 4);
    let none = Lifted::declared(parse(THREE).unwrap(), &[]);
    assert!(none.services.services.is_empty());
}

#[test]
fn the_whole_file_is_lifted_when_asked_for_whole() {
    use nunki::compose::services::Lifted;
    let file = parse(THREE).unwrap();
    let lifted = Lifted::whole(file.clone());
    assert_eq!(lifted.services, file);
    assert_eq!(lifted.approved, file.digest());
    assert!(lifted.held_back.is_empty());
}

#[test]
fn the_ports_line_names_the_services_and_says_why() {
    let line = nunki::services::ports_dropped(&["db", "questdb"]);
    assert_eq!(
        line,
        "ports dropped from db, questdb: the agent reaches a service by its name, and a \
         published port would collide between slots"
    );
}

// --- the fences, measured (L3) ----------------------------------------------------
//
// `#[ignore]`d: they build the firewall's image and lift containers, and
// need a network. The HQ runs them by hand, after touching what they cover:
//
//     cargo test --test services_file -- --ignored --nocapture
//
// Each negative probe has a control beside it — the same attempt from a
// container nunki did not fence — and a control that fails makes the test
// fail as inconclusive rather than pass: a probe is worth nothing if it
// would fail without the guard.

mod live {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use nunki::compose::services::{CAPABILITIES, Lifted, NETWORK, NO_GATEWAY, ServicesFile};
    use nunki::compose::{NamedVolume, Plan, UserIds, generate, project_name};
    use nunki::harness::Role;
    use nunki::perimeter::{Sources, compute};

    const FIREWALL_IMAGE: &str = "nunki/firewall:test";
    const SESSION: &str = "22222222-3333-4444-8555-666666666666";
    /// Where a TCP connection out is attempted: a public resolver's HTTPS.
    const OUT: &str = "1.1.1.1 443";

    fn docker(args: &[&str]) -> Output {
        let engine = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".to_string());
        Command::new(engine)
            .args(args)
            .output()
            .expect("the engine is on the path")
    }

    fn compose(file: &Path, project: &str, args: &[&str]) -> Output {
        let raw = std::env::var("HQ_COMPOSE").unwrap_or_else(|_| "docker compose".to_string());
        let mut words = raw.split_whitespace();
        Command::new(words.next().expect("HQ_COMPOSE names a program"))
            .args(words)
            .args(["-p", project, "-f"])
            .arg(file)
            .args(args)
            .output()
            .expect("the compose command is on the path")
    }

    fn said(out: &Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    fn build_firewall() -> tempfile::TempDir {
        let context = tempfile::tempdir().unwrap();
        nunki::firewall::materialise(context.path()).unwrap();
        let out = docker(&[
            "build",
            "-q",
            "-t",
            FIREWALL_IMAGE,
            context.path().to_str().unwrap(),
        ]);
        assert!(
            out.status.success(),
            "the sidecar did not build: {}",
            said(&out)
        );
        context
    }

    /// A lifted profile: the integrator's, declaring `declared`, lifting
    /// `lifted`, with an agent that only sleeps.
    struct Profile {
        /// Held so the generated file outlives the profile.
        _dir: tempfile::TempDir,
        file: PathBuf,
        project: String,
    }

    impl Profile {
        fn generate(slot: &str, declared: &[&str], lifted: Lifted) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let tree = dir.path().join("tree");
            let mission = dir.path().join("mission");
            std::fs::create_dir_all(&tree).unwrap();
            std::fs::create_dir_all(&mission).unwrap();
            for file in nunki::compose::any_agent_writable() {
                std::fs::write(mission.join(file), "").unwrap();
            }
            let services: Vec<nunki::mission::Service> = declared
                .iter()
                .map(|name| nunki::mission::Service {
                    name: name.to_string(),
                    reach: vec![name.to_string()],
                    shared: false,
                })
                .collect();
            let perimeter = compute(
                Role::Integrator,
                &Sources {
                    stack: &["example.com".to_string()],
                    harness: &[],
                    services: &services,
                    forge: &["github.com".to_string()],
                },
            )
            .unwrap();
            let plan = Plan {
                session: SESSION.into(),
                slot: slot.to_string(),
                role: Role::Integrator,
                image: "alpine:3.20".to_string(),
                firewall_image: FIREWALL_IMAGE.to_string(),
                user: UserIds { uid: 501, gid: 20 },
                tree,
                tree_at: PathBuf::from("/work/tree"),
                mission_dir: mission,
                mission_dir_at: PathBuf::from("/work/mission"),
                stack_scripts: Vec::new(),
                credentials: Vec::new(),
                advisories: vec![],
                secrets: None,
                volumes: Vec::<NamedVolume>::new(),
                environment: BTreeMap::new(),
                command: vec!["sleep".to_string(), "600".to_string()],
                perimeter,
                project_services: Some(lifted),
                prober: None,
            };
            let docker = nunki::engine::docker::Docker::real();
            let yaml = generate(&plan, nunki::engine::Engine::dialect(&docker)).unwrap();
            let file = dir.path().join("system.yml");
            std::fs::write(&file, yaml).unwrap();
            let project = project_name(SESSION, slot).unwrap();
            let profile = Self {
                _dir: dir,
                file,
                project,
            };
            profile.down();
            profile
        }

        /// The generated file with `edit` applied to it first: how the
        /// capability measurement takes one capability away.
        fn edit(&self, edit: impl Fn(&mut nunki::yaml::Value)) {
            let mut doc: nunki::yaml::Value =
                nunki::yaml::from_str(&std::fs::read_to_string(&self.file).unwrap()).unwrap();
            edit(&mut doc);
            std::fs::write(&self.file, nunki::yaml::to_string(&doc).unwrap()).unwrap();
        }

        fn up(&self) -> Output {
            compose(&self.file, &self.project, &["up", "-d", "--wait"])
        }

        fn exec(&self, service: &str, script: &str) -> Output {
            compose(
                &self.file,
                &self.project,
                &["exec", "-T", service, "sh", "-c", script],
            )
        }

        fn down(&self) {
            compose(&self.file, &self.project, &["down", "-v", "--timeout", "1"]);
        }

        /// Up, with every anonymous volume made anew rather than taken
        /// over from a previous container of the same service.
        fn up_fresh(&self) -> Output {
            compose(
                &self.file,
                &self.project,
                &["up", "-d", "--wait", "--renew-anon-volumes"],
            )
        }

        /// Its containers stopped and kept, with their volumes: what a stack
        /// that stopped leaves for the slot's next `up` to start again.
        fn stop(&self) -> Output {
            compose(&self.file, &self.project, &["stop", "--timeout", "30"])
        }

        /// Down, and then every volume its containers mounted removed by
        /// name — the image's anonymous ones included — so that nothing it
        /// wrote is there for whatever is lifted next.
        fn down_with_its_volumes(&self) {
            let ids = docker(&[
                "ps",
                "-aq",
                "--filter",
                &format!("label=com.docker.compose.project={}", self.project),
            ]);
            let ids: Vec<String> = String::from_utf8_lossy(&ids.stdout)
                .split_whitespace()
                .map(String::from)
                .collect();
            let mut volumes = Vec::new();
            if !ids.is_empty() {
                let mut args = vec![
                    "inspect",
                    "-f",
                    "{{range .Mounts}}{{if eq .Type \"volume\"}}{{.Name}} {{end}}{{end}}",
                ];
                args.extend(ids.iter().map(String::as_str));
                let out = docker(&args);
                volumes.extend(
                    String::from_utf8_lossy(&out.stdout)
                        .split_whitespace()
                        .map(String::from),
                );
            }
            self.down();
            for volume in &volumes {
                let _ = docker(&["volume", "rm", "-f", volume]);
                assert!(
                    !docker(&["volume", "inspect", volume]).status.success(),
                    "the volume {volume} outlived its iteration"
                );
            }
        }
    }

    impl Drop for Profile {
        fn drop(&mut self) {
            self.down();
        }
    }

    fn file(text: &str) -> ServicesFile {
        ServicesFile::parse(text).expect("the services file parses")
    }

    /// The same attempt from a container nunki did not fence: it must
    /// succeed, or the negative result beside it proves nothing.
    fn control(image: &str, script: &str) {
        let out = docker(&["run", "--rm", image, "sh", "-c", script]);
        assert!(
            out.status.success(),
            "inconclusive: `{script}` fails from an unfenced {image} too, so a refusal from \
             the fenced one proves nothing on this machine: {}",
            said(&out)
        );
    }

    #[test]
    #[ignore = "builds an image and lifts containers; the HQ runs it by hand"]
    fn live_a_lifted_service_cannot_open_a_connection_to_a_public_address() {
        let _context = build_firewall();
        let script = format!("nc -z -w3 {OUT}");
        control("alpine:3.20", &script);

        let profile = Profile::generate(
            "svcout",
            &["probe"],
            Lifted::whole(file(
                "services:\n  probe:\n    image: alpine:3.20\n    command: [sleep, \"600\"]\n",
            )),
        );
        let up = profile.up();
        assert!(
            up.status.success(),
            "the profile did not come up: {}",
            said(&up)
        );
        let out = profile.exec("probe", &script);
        assert!(
            !out.status.success(),
            "a lifted service reached {OUT}: {}",
            said(&out)
        );
    }

    #[test]
    #[ignore = "builds an image and lifts containers; the HQ runs it by hand"]
    fn live_a_program_run_through_the_database_cannot_reach_out() {
        let _context = build_firewall();
        let copy = "psql -U postgres -v ON_ERROR_STOP=1 -c \
                    \"COPY (SELECT 1) TO PROGRAM 'nc -z -w3 1.1.1.1 443'\"";

        // The control: the same superuser, the same COPY, from a Postgres
        // nobody fenced.
        let name = "nunki-live-unfenced-postgres";
        let _ = docker(&["rm", "-f", name]);
        let started = docker(&[
            "run",
            "-d",
            "--name",
            name,
            "-e",
            "POSTGRES_PASSWORD=live",
            "postgres:16-alpine",
        ]);
        assert!(started.status.success(), "{}", said(&started));
        // The **final** server, and not the first one to answer. The image
        // runs a temporary server for `initdb`'s scripts, stops it and starts
        // the real one; `pg_isready` on the socket answers for the temporary
        // one, and a COPY played then fails for a reason that is not the
        // fence (measured on CI's Linux runner, #118). The temporary server
        // listens on no TCP address (`listen_addresses=''`), so asking over
        // TCP waits for the final one.
        let ready = (0..60).any(|_| {
            std::thread::sleep(std::time::Duration::from_secs(1));
            docker(&[
                "exec",
                name,
                "pg_isready",
                "-h",
                "127.0.0.1",
                "-U",
                "postgres",
            ])
            .status
            .success()
        });
        let unfenced = docker(&["exec", name, "sh", "-c", copy]);
        let _ = docker(&["rm", "-f", name]);
        assert!(
            ready,
            "inconclusive: the unfenced Postgres never became ready"
        );
        assert!(
            unfenced.status.success(),
            "inconclusive: COPY TO PROGRAM fails from an unfenced Postgres too: {}",
            said(&unfenced)
        );

        let profile = Profile::generate(
            "svcpg",
            &["db"],
            Lifted::whole(file(
                "services:\n  db:\n    image: postgres:16-alpine\n    environment:\n      \
                 POSTGRES_PASSWORD: live\n    healthcheck:\n      \
                 test: [\"CMD-SHELL\", \"pg_isready -h 127.0.0.1 -U postgres\"]\n      \
                 interval: 2s\n      retries: 30\n",
            )),
        );
        // Healthy by the same TCP question as the control: healthy on the
        // temporary server, the COPY below could fail on the restart and
        // read as the fence holding.
        let up = profile.up();
        assert!(
            up.status.success(),
            "Postgres did not come up healthy with nunki's fences: {}",
            said(&up)
        );
        let fenced = profile.exec("db", copy);
        assert!(
            !fenced.status.success(),
            "a program run through the fenced database reached out: {}",
            said(&fenced)
        );
    }

    #[test]
    #[ignore = "builds an image and lifts containers; the HQ runs it by hand"]
    fn live_the_agent_still_reaches_the_declared_service_by_name() {
        let _context = build_firewall();
        let profile = Profile::generate(
            "svcname",
            &["db"],
            Lifted::whole(file("services:\n  db:\n    image: nginx:alpine\n")),
        );
        let up = profile.up();
        assert!(
            up.status.success(),
            "the profile did not come up: {}",
            said(&up)
        );
        let out = profile.exec("agent", "nc -z -w3 db 80");
        assert!(
            out.status.success(),
            "the agent no longer reaches the declared service: {}",
            said(&out)
        );
        // And the way out the agent had is still the perimeter's: an
        // off-list address stays refused.
        let out = profile.exec("agent", &format!("nc -z -w3 {OUT}"));
        assert!(!out.status.success(), "{}", said(&out));
    }

    /// The host is not on the services' network: neither a port a container
    /// publishes on the host's `0.0.0.0`, nor a process of the host's own
    /// listening on the network's gateway address, is reachable from a lifted
    /// service — through the services network's gateway or through the
    /// default bridge's.
    ///
    /// The control is the same profile without [`NO_GATEWAY`] — `internal:
    /// true` alone — reaching **the host's own process** through the gateway.
    /// Not the published port: Linux filters a port Docker publishes from an
    /// internal network even without the fence (measured on CI's runner,
    /// #118), so a control on it ended "inconclusive" there, and proved the
    /// fence on the HQ's engine only. A process listening on the host is
    /// reached over the gateway address itself, which the host owns, and
    /// which is what [`NO_GATEWAY`] takes away.
    #[test]
    #[ignore = "builds an image and lifts containers; the HQ runs it by hand"]
    fn live_a_lifted_service_cannot_reach_a_port_the_host_publishes() {
        let _context = build_firewall();
        const PORT: &str = "18080";
        let name = "nunki-live-host-port";
        let _ = docker(&["rm", "-f", name]);
        let published = docker(&[
            "run",
            "-d",
            "--name",
            name,
            "-p",
            &format!("0.0.0.0:{PORT}:80"),
            "nginx:alpine",
        ]);
        assert!(published.status.success(), "{}", said(&published));
        struct Remove(&'static str);
        impl Drop for Remove {
            fn drop(&mut self) {
                let _ = docker(&["rm", "-f", self.0]);
            }
        }
        let _published = Remove(name);
        // The host's own process: a listener of this test's, on every address
        // the host has — the gateway of each network below among them, once
        // that network is up. Listening is enough for `nc -z`: the kernel
        // completes the connection for a listening socket before anything
        // accepts it.
        let host = std::net::TcpListener::bind(("0.0.0.0", 0)).expect("the host can listen");
        let listened = host.local_addr().unwrap().port().to_string();

        let gateway = |network: &str| {
            let out = docker(&[
                "network",
                "inspect",
                network,
                "-f",
                "{{range .IPAM.Config}}{{.Gateway}} {{end}}",
            ]);
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .filter(|a| a.contains('.'))
                .map(String::from)
                .collect::<Vec<_>>()
        };
        let text = "services:\n  web:\n    image: nginx:alpine\n  \
                    probe:\n    image: alpine:3.20\n    command: [sleep, \"600\"]\n";
        let reaches = |profile: &Profile, addresses: &[String], port: &str| {
            addresses.iter().any(|address| {
                profile
                    .exec("probe", &format!("nc -z -w3 {address} {port}"))
                    .status
                    .success()
            })
        };

        // The control: internal, with its gateway.
        let open = Profile::generate("svchostc", &["web", "probe"], Lifted::whole(file(text)));
        open.edit(|doc| {
            let network = doc
                .get_mut("networks")
                .and_then(|n| n.get_mut(NETWORK))
                .and_then(nunki::yaml::Value::as_mapping_mut)
                .unwrap();
            network.remove("driver_opts");
        });
        let up = open.up();
        assert!(up.status.success(), "{}", said(&up));
        let through = gateway(&format!("{}_{NETWORK}", open.project));
        assert!(
            // Either way in: the host's own process where the test runs on the
            // engine's host (Linux), the published port where the engine runs
            // in a machine of its own and this test's listener is not behind
            // the gateway — the case the HQ measured. The fence below must
            // refuse both.
            reaches(&open, &through, &listened) || reaches(&open, &through, PORT),
            "inconclusive: neither the host's own port {listened} nor its published port \
             {PORT} is open through {through:?} even without {NO_GATEWAY:?}, so their being \
             closed with it proves nothing here"
        );
        drop(open);

        let fenced = Profile::generate("svchost", &["web", "probe"], Lifted::whole(file(text)));
        let up = fenced.up();
        assert!(
            up.status.success(),
            "the profile did not come up: {}",
            said(&up)
        );
        let mut tried = gateway(&format!("{}_{NETWORK}", fenced.project));
        tried.extend(gateway("bridge"));
        tried.extend(through);
        for port in [PORT, listened.as_str()] {
            assert!(
                !reaches(&fenced, &tried, port),
                "a lifted service reached the host's port {port} through one of {tried:?}"
            );
        }
        drop(host);
        // And the network still carries what it is for.
        let out = fenced.exec("agent", "nc -z -w3 web 80");
        assert!(
            out.status.success(),
            "the agent no longer reaches the declared service by name: {}",
            said(&out)
        );
    }

    /// No name outside resolves from a lifted service: the engine's resolver
    /// answers for the services' network and forwards nothing. Measured by
    /// the HQ on Docker 29.4.0 as a SERVFAIL; this keeps measuring it.
    #[test]
    #[ignore = "builds an image and lifts containers; the HQ runs it by hand"]
    fn live_a_lifted_service_cannot_resolve_a_name_outside() {
        let _context = build_firewall();
        // An answer, and not merely an exit code: busybox prints `Name:`
        // only for a name it resolved.
        let script = "nslookup example.com 2>&1 | grep -q '^Name:'";
        control("alpine:3.20", script);

        let profile = Profile::generate(
            "svcdns",
            &["probe"],
            Lifted::whole(file(
                "services:\n  probe:\n    image: alpine:3.20\n    command: [sleep, \"600\"]\n",
            )),
        );
        let up = profile.up();
        assert!(
            up.status.success(),
            "the profile did not come up: {}",
            said(&up)
        );
        let out = profile.exec("probe", "nslookup example.com 2>&1");
        assert!(
            !profile.exec("probe", script).status.success(),
            "a lifted service resolved a name outside: {}",
            said(&out)
        );
    }

    #[test]
    #[ignore = "builds an image and lifts containers; the HQ runs it by hand"]
    fn live_an_undeclared_service_in_the_file_does_not_start() {
        let _context = build_firewall();
        let text = "services:\n  db:\n    image: nginx:alpine\n  \
                    miner:\n    image: alpine:3.20\n    command: [sleep, \"600\"]\n";
        let running = |profile: &Profile, service: &str| {
            let out = docker(&[
                "ps",
                "-q",
                "--filter",
                &format!("label=com.docker.compose.project={}", profile.project),
                "--filter",
                &format!("label=com.docker.compose.service={service}"),
            ]);
            !String::from_utf8_lossy(&out.stdout).trim().is_empty()
        };

        // The control: lifted whole, the same file starts it.
        let whole = Profile::generate("svcwhole", &["db"], Lifted::whole(file(text)));
        assert!(whole.up().status.success());
        assert!(
            running(&whole, "miner"),
            "inconclusive: the undeclared service does not start even when lifted"
        );
        drop(whole);

        let declared = Profile::generate(
            "svcdecl",
            &["db"],
            Lifted::declared(file(text), &["db".to_string()]),
        );
        let up = declared.up();
        assert!(
            up.status.success(),
            "the profile did not come up: {}",
            said(&up)
        );
        assert!(running(&declared, "db"));
        assert!(
            !running(&declared, "miner"),
            "a service the mission does not declare was started"
        );
    }

    /// A lifted service whose container does not start again over what it
    /// kept is found by [`nunki::run::not_started`] on the real engine, and
    /// the removal it names is the way out: the next `up` makes it anew
    /// (#140). The state that stops it is in the container's own
    /// filesystem, as the anonymous volume was in #140, so removing the
    /// container takes it away.
    #[test]
    #[ignore = "builds an image and lifts containers; the HQ runs it by hand"]
    fn live_a_service_that_does_not_start_again_is_found_and_removed() {
        use nunki::engine::Engine;
        let _context = build_firewall();
        let engine = nunki::engine::docker::Docker::real();
        let lifted = |profile: &Profile| -> Vec<nunki::engine::Container> {
            engine
                .containers(&profile.project)
                .unwrap()
                .into_iter()
                .filter(|c| c.digest.is_some())
                .collect()
        };

        // The control: a service that fails its first start had no
        // container before, and keeps the engine's own words.
        let first = Profile::generate(
            "svcfirst",
            &["db"],
            Lifted::whole(file(
                "services:\n  db:\n    image: alpine:3.20\n    command: [sh, -c, \"exit 3\"]\n",
            )),
        );
        let before = lifted(&first);
        assert!(before.is_empty(), "{before:?}");
        assert!(engine.up(&first.file, &first.project).is_err());
        assert_eq!(
            nunki::run::not_started(&engine, &first.project, &before),
            None
        );
        drop(first);

        let once = Profile::generate(
            "svconce",
            &["db"],
            Lifted::whole(file(
                "services:\n  db:\n    image: alpine:3.20\n    \
                 command: [sh, -c, \"test ! -e /once && touch /once && exec sleep 600\"]\n",
            )),
        );
        let up = once.up();
        assert!(up.status.success(), "the first start: {}", said(&up));
        assert!(once.stop().status.success());
        let before = lifted(&once);
        assert_eq!(before.len(), 1, "{before:?}");
        let failed = engine
            .up(&once.file, &once.project)
            .expect_err("inconclusive: it started again");
        // What the engine said, which the launch's line ends with.
        println!("{failed}");

        let stuck = nunki::run::not_started(&engine, &once.project, &before)
            .expect("the service that did not start again");
        assert_eq!(stuck.len(), 1, "{stuck:?}");
        assert_eq!(
            (stuck[0].service.as_str(), stuck[0].code),
            ("db", 1),
            "{stuck:?}"
        );
        let removal = engine.removal(&stuck[0].container);
        let removed = Command::new(&removal.program)
            .args(&removal.args)
            .output()
            .unwrap();
        assert!(removed.status.success(), "{}", said(&removed));
        let again = once.up();
        assert!(
            again.status.success(),
            "the way out did not bring it back: {}",
            said(&again)
        );
    }

    /// The images in real use start, and pass their healthcheck, with
    /// nunki's fixed set, and start again over the data they wrote — and
    /// each capability of the set is one at least one of them cannot do
    /// without. What it prints is the measurement PR.md asks for.
    ///
    /// Every iteration measures a fresh start: its own Compose project,
    /// anonymous volumes renewed on `up`, and taken down with every volume
    /// its containers mounted before the next one. The first version lifted
    /// every iteration under one slot, so the Postgres image's anonymous
    /// data volume, initialised by the first, fully capable iteration, was
    /// found ready by the later ones: it measured that state, not what an
    /// entrypoint needs on a fresh data directory.
    ///
    /// Every iteration then stops its stack and lifts it again with a plain
    /// `up`, which starts the same container over the same volume, as a
    /// slot's next role does once its stack has stopped. The second version
    /// measured the fresh start only, and the set it found started Postgres
    /// once and never again (#140).
    ///
    /// Capabilities interact, so a set is minimal only relative to itself:
    /// each one is removed from nunki's own set, never from another.
    #[test]
    #[ignore = "builds an image and lifts containers; the HQ runs it by hand"]
    fn live_the_capability_set_is_the_smallest_the_images_need() {
        let _context = build_firewall();
        let images = [
            (
                "postgres:16-alpine",
                "services:\n  db:\n    image: postgres:16-alpine\n    environment:\n      \
                 POSTGRES_PASSWORD: live\n    healthcheck:\n      \
                 test: [\"CMD-SHELL\", \"pg_isready -U postgres\"]\n      interval: 2s\n      \
                 retries: 30\n",
            ),
            (
                "postgres:16",
                "services:\n  db:\n    image: postgres:16\n    environment:\n      \
                 POSTGRES_PASSWORD: live\n    healthcheck:\n      \
                 test: [\"CMD-SHELL\", \"pg_isready -U postgres\"]\n      interval: 2s\n      \
                 retries: 30\n",
            ),
            (
                // The version in real use, with the real file's healthcheck:
                // the image carries no curl, so a check through it is never
                // healthy (measured by the HQ on 8.2.1 and 9.4.3).
                "questdb/questdb:9.4.3",
                "services:\n  db:\n    image: questdb/questdb:9.4.3\n    healthcheck:\n      \
                 test: [\"CMD\", \"bash\", \"-c\", \"</dev/tcp/127.0.0.1/9000\"]\n      \
                 interval: 2s\n      retries: 60\n",
            ),
            (
                // The image nunki's own live tests lift as a declared
                // service, so the set must serve it too.
                "nginx:alpine",
                "services:\n  db:\n    image: nginx:alpine\n    healthcheck:\n      \
                 test: [\"CMD\", \"wget\", \"-q\", \"-O\", \"/dev/null\", \"http://127.0.0.1/\"]\n      \
                 interval: 2s\n      retries: 30\n",
            ),
        ];
        let iteration = std::cell::Cell::new(0);
        let healthy = |text: &str, without: Option<&str>| {
            iteration.set(iteration.get() + 1);
            let slot = format!("svccaps{}", iteration.get());
            let profile = Profile::generate(&slot, &["db"], Lifted::whole(file(text)));
            if let Some(cap) = without {
                profile.edit(|doc| {
                    let caps = doc
                        .get_mut("services")
                        .and_then(|s| s.get_mut("db"))
                        .and_then(|d| d.get_mut("cap_add"))
                        .and_then(nunki::yaml::Value::as_sequence_mut)
                        .unwrap();
                    caps.retain(|c| c.as_str() != Some(cap));
                });
            }
            let fresh = profile.up_fresh().status.success();
            let again = fresh && profile.stop().status.success() && profile.up().status.success();
            profile.down_with_its_volumes();
            (fresh, again)
        };
        let said_of = |(fresh, again): (bool, bool)| match (fresh, again) {
            (true, true) => "healthy, and again after a stop",
            (true, false) => "healthy, and NOT again after a stop",
            _ => "NOT healthy",
        };

        let mut failures = Vec::new();
        for (image, text) in &images {
            let measured = healthy(text, None);
            println!(
                "{image:<24} with the whole set          {}",
                said_of(measured)
            );
            if measured != (true, true) {
                failures.push(format!(
                    "{image} is {} with {CAPABILITIES:?}",
                    said_of(measured)
                ));
            }
        }
        for cap in CAPABILITIES {
            let needed_by: Vec<String> = images
                .iter()
                .filter_map(|(image, text)| {
                    let measured = healthy(text, Some(cap));
                    (measured != (true, true)).then(|| format!("{image}: {}", said_of(measured)))
                })
                .collect();
            println!("without {cap:<14} {needed_by:?}");
            if needed_by.is_empty() {
                failures.push(format!("{cap} is needed by none of the images: drop it"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

// --- what the first campaign found untested ---------------------------------------

#[test]
fn an_empty_volumes_block_declares_no_volume() {
    let file = parse("services: {}\nvolumes:\n").unwrap();
    assert!(file.volumes.is_empty());
}

#[test]
fn a_bare_environment_entry_is_refused_for_what_it_would_do() {
    for line in [
        "environment:\n      HOME:",
        "environment:\n      HOME: \"\"",
    ] {
        match parse(&with_service_line(line)) {
            Err(ServicesError::Environment { why, .. }) => {
                assert!(why.contains("filled from the host's environment"), "{why}")
            }
            other => panic!("{line}: {other:?}"),
        }
    }
    match parse(&with_service_line("environment:\n      PORT: 5432")) {
        Err(ServicesError::Environment { why, .. }) => assert!(why.contains("quote it"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_condition_compose_knows_is_rendered_as_written() {
    for condition in [
        "service_started",
        "service_healthy",
        "service_completed_successfully",
    ] {
        let text = format!(
            "services:\n  app:\n    image: x\n    depends_on:\n      db: {{condition: {condition}}}\n  \
             db:\n    image: y\n"
        );
        let rendering = parse(&text).unwrap().render();
        assert!(
            rendering.contains(&format!("condition: \"{condition}\"")),
            "{rendering}"
        );
    }
}

#[test]
fn an_empty_command_is_rendered_as_an_empty_list() {
    let file = parse(&with_service_line("entrypoint: []")).unwrap();
    let rendering = file.render();
    assert!(rendering.contains("    entrypoint: []\n"), "{rendering}");
    let back: nunki::yaml::Value = nunki::yaml::from_str(&rendering).unwrap();
    assert_eq!(
        back["services"]["db"]["entrypoint"],
        nunki::yaml::Value::Sequence(vec![]),
        "an empty list, not a null: an entrypoint reset, not an absent one"
    );
}

/// The repository's HEAD is read as such: what `--show` without a mission
/// prints, and what `--approve` looks at first.
#[test]
fn the_repositorys_head_is_read_from_its_commit() {
    let world = World::new();
    let current = nunki::services::read(&world.project, At::Repository(&world.project.root))
        .unwrap()
        .expect("a services file is declared");
    assert_eq!(current.path, "compose.yaml");
    assert_eq!(
        current.digest,
        world.shown().digest,
        "the same file as the slot's"
    );
    // From the commit, not the tree.
    std::fs::write(world.project.root.join("compose.yaml"), SECOND).unwrap();
    let again = nunki::services::read(&world.project, At::Repository(&world.project.root))
        .unwrap()
        .unwrap();
    assert_eq!(again.digest, current.digest);
}

/// A list of approvals that is there and cannot be read approves nothing,
/// and says why: it is not "no approval yet".
#[test]
fn a_list_of_approvals_that_cannot_be_read_is_an_error_and_not_an_empty_list() {
    let world = World::new();
    let file = nunki::services::file(&world.project);
    std::fs::create_dir_all(&file).unwrap();
    assert!(
        matches!(world.lifted(), Err(ApprovalError::Unreadable(..))),
        "{:?}",
        world.lifted()
    );
}

// --- what the parser reports, the parser decides (volet 2) ----------------------
//
// Each form the security agent and the HQ measured accepted on d1c19a3: a
// tag, an anchor or an alias that the text scan of the time did not see,
// behind a bare CR or a false quote. Each is refused now as what the parser
// reports, on the line the parser gives it (a bare CR ends a line for the
// parser).

/// A services file: one service, `db`, whose lines after `image` are `body`
/// (each already indented under the service).
fn db(body: &str) -> String {
    format!("services:\n  db:\n    image: x\n{body}")
}

#[test]
fn a_binary_tag_behind_a_bare_cr_is_refused_as_a_tag() {
    refused_at(
        "services:\n  db:\n    image:\r      !!binary cG9zdGdyZXM6MTY=\n",
        "a tag",
        4,
    );
}

#[test]
fn a_non_specific_tag_behind_a_bare_cr_is_refused_as_a_tag() {
    refused_at(&db("    environment:\n      A:\r        ! 5\n"), "a tag", 6);
}

#[test]
fn an_anchor_and_its_alias_behind_bare_crs_are_refused_as_an_anchor() {
    refused_at(
        &format!(
            "{}  other:\n    image: x\n    environment:\r      *m\n",
            db("    environment:\r      &m {A: b}\n")
        ),
        "an anchor",
        5,
    );
}

#[test]
fn a_tag_after_a_false_quote_in_a_flow_mapping_is_refused() {
    refused_at(
        &db("    environment: {K: a:'b, L: !!binary aGk=, M: c'd}\n"),
        "a tag",
        4,
    );
}

#[test]
fn a_tag_after_a_false_quote_in_a_flow_list_is_refused() {
    refused_at(&db("    command: [a:'b, !!binary aGk=, c'd]\n"), "a tag", 4);
}

#[test]
fn a_tag_between_false_single_quotes_in_block_lines_is_refused() {
    refused_at(
        &db("    environment:\n      K: a:'b\n      L: !!binary aGk=\n      M: c'd\n"),
        "a tag",
        6,
    );
}

#[test]
fn a_tag_between_false_double_quotes_in_block_lines_is_refused() {
    refused_at(
        &db("    environment:\n      K: a:\"b\n      L: !!binary aGk=\n      M: c\"d\n"),
        "a tag",
        6,
    );
}

#[test]
fn a_tag_after_a_quote_that_starts_a_continuation_line_is_refused() {
    refused_at(
        &db("    environment:\n      K: a\n        'b\n      L: !!binary aGk=\n      M: c'd\n"),
        "a tag",
        7,
    );
}

#[test]
fn a_tag_after_a_false_quote_on_another_key_is_refused() {
    refused_at(
        &db("    working_dir: /a:'b\n    command: [!!binary aGk=, \"c'd\"]\n"),
        "a tag",
        5,
    );
}

#[test]
fn an_anchor_and_an_alias_after_a_false_quote_are_refused_as_an_anchor() {
    refused_at(
        &db("    environment: {A: a:'b, K: &x v, L: *x, M: c'd}\n"),
        "an anchor",
        4,
    );
}

#[test]
fn a_tag_after_a_comment_ended_by_a_bare_cr_is_refused() {
    refused_at(
        &db("    environment:\n      K: v # c\r      L: !!binary aGk=\n"),
        "a tag",
        6,
    );
}

#[test]
fn a_tag_on_a_key_behind_a_bare_cr_is_refused() {
    refused_at(
        &db("    environment:\n      K: v\r      !!str L: w\n"),
        "a tag",
        6,
    );
}

#[test]
fn a_tag_behind_lf_cr_before_the_value_is_refused() {
    refused_at(
        &db("    environment:\n      K:\n\r        !!binary aGk=\n"),
        "a tag",
        7,
    );
}

#[test]
fn a_tag_behind_cr_cr_before_the_value_is_refused() {
    refused_at(
        &db("    environment:\n      K:\r\r        !!binary aGk=\n"),
        "a tag",
        7,
    );
}

#[test]
fn a_tag_behind_a_bare_cr_inside_braces_is_refused() {
    refused_at(&db("    environment: {K:\r !!binary aGk=}\n"), "a tag", 5);
}

#[test]
fn a_tag_behind_a_bare_cr_inside_brackets_is_refused() {
    refused_at(&db("    command: [a,\r !!binary aGk=]\n"), "a tag", 5);
}

/// A directive behind a comment ended by a bare CR carries no tag, anchor
/// or alias for the parser to report: it is the bare CR that refuses it,
/// before any line is read for a directive.
#[test]
fn a_directive_behind_a_comment_ended_by_a_bare_cr_is_refused_for_the_cr() {
    refused_at(
        "# c\r%YAML 1.1\n---\nservices: {}\n",
        "a carriage return without a line feed, which YAML reads as a line break",
        1,
    );
}

/// What the previous scan refused and the parser reads as text: an
/// indicator that starts a continuation line of a plain value is part of
/// the value, not an alias, an anchor or a tag.
#[test]
fn an_indicator_starting_a_continuation_line_is_the_text_it_is() {
    let file = parse(&db(
        "    command: echo\n      *not-an-alias &nor-anchor !nor-tag\n",
    ))
    .unwrap();
    assert_eq!(
        file.services["db"].command,
        Some(nunki::compose::services::Command::Shell(
            "echo *not-an-alias &nor-anchor !nor-tag".to_string()
        ))
    );
}

/// A place to put a separator and a payload, and the file around it.
type Frame = (&'static str, fn(&str) -> String);

/// Frames × separators × payloads, as the HQ asked: no generated text is
/// accepted while the parser reports a tag, an anchor or an alias in it.
#[test]
fn no_generated_text_is_accepted_while_the_parser_reports_a_tag_an_anchor_or_an_alias() {
    let frames: [Frame; 9] = [
        ("an environment value", |v| {
            db(&format!("    environment:\n      K:{v}\n"))
        }),
        ("a command item", |v| {
            db(&format!("    command:\n      -{v}\n"))
        }),
        ("a flow mapping", |v| {
            db(&format!("    environment: {{K:{v}}}\n"))
        }),
        ("a flow list", |v| db(&format!("    command: [a,{v}]\n"))),
        ("the image", |v| {
            format!("services:\n  db:\n    image:{v}\n")
        }),
        ("after a quoted value", |v| {
            db(&format!("    environment:\n      J: \"v\"\n      K:{v}\n"))
        }),
        ("after a block scalar", |v| {
            db(&format!("    entrypoint: |\n      run\n    command:{v}\n"))
        }),
        ("after a false quote in block", |v| {
            db(&format!(
                "    environment:\n      J: a:'b\n      K:{v}\n      M: c'd\n"
            ))
        }),
        ("after a false quote in flow", |v| {
            db(&format!("    environment: {{J: a:'b, K:{v}, M: c'd}}\n"))
        }),
    ];
    let separators = [" ", "\t", "\r", "\r ", "\n\r", "\r\r", " # c\r"];
    let payloads = [
        "!!binary aGk=",
        "! 5",
        "!!str x",
        "!!null x",
        "!custom x",
        "!<tag:yaml.org,2002:str> x",
        "&a x",
        "*a",
    ];
    let mut reported = 0;
    for (frame, build) in frames {
        for separator in separators {
            for payload in payloads {
                let text = build(&format!("{separator}{payload}"));
                let found = nunki::yaml::constructs(&text);
                if found.as_ref().is_ok_and(|f| !f.is_empty()) {
                    reported += 1;
                    assert!(
                        parse(&text).is_err(),
                        "{frame}, {separator:?}, {payload}: accepted while the parser \
                         reports {found:?}\n{text:?}"
                    );
                }
            }
        }
    }
    // Not vacuous: the parser reported something in most of the texts.
    assert!(
        reported > 9 * 7 * 8 / 2,
        "only {reported} texts carried one"
    );
}
