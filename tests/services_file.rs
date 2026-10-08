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
    refused_feature("\"services\":&a {}\n", "an anchor");
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
    let back: serde_yaml_ng::Value = serde_yaml_ng::from_str(&rendering).unwrap();
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
