//! The mission folder at the HQ (SPEC 4.1): five files, and the split
//! between them is a restriction rather than a convention.

use hq::mission::dir::{MissionDirError, Paths, create, list, read_header};
use hq::mission::{Bounds, Header, Integration, Lot, Security, Service};

fn header() -> Header {
    Header {
        branch: "feat/alpha".to_string(),
        base: "dev".to_string(),
        lots: vec![
            Lot {
                id: "L1".to_string(),
                title: "Read the spec".to_string(),
            },
            Lot {
                id: "L2".to_string(),
                title: "Write the thing".to_string(),
            },
        ],
        integration: Integration::Services {
            services: vec![Service {
                name: "db".to_string(),
                reach: vec!["db".to_string(), "10.4.0.7".to_string()],
            }],
        },
        security: Security::Agent,
        arbiter: None,
        account: None,
        bounds: Bounds::default(),
    }
}

#[test]
fn a_new_mission_has_its_five_files_and_the_agents_three_are_empty() {
    let dir = tempfile::tempdir().unwrap();
    let paths = create(dir.path(), "lot-alpha", &header(), "Do the thing.").unwrap();

    assert!(paths.mission.is_file());
    assert!(paths.followup.is_file());
    // The agent writes these; they exist so a bind mount has something to
    // mount, and they say nothing yet.
    assert!(paths.pr.is_file());
    assert!(paths.verdict.is_file());
    assert_eq!(std::fs::read_to_string(&paths.pr).unwrap(), "");
    assert_eq!(
        std::fs::read_to_string(&paths.verdict).unwrap(),
        "",
        "an empty verdict is the absence of one, not an empty object"
    );

    // The journal starts with the block the run contract demands.
    let journal = std::fs::read_to_string(&paths.journal).unwrap();
    assert!(journal.contains("ÉTAT DE REPRISE"), "{journal}");
}

#[test]
fn the_folder_is_outside_the_tree_and_named_after_the_mission() {
    let paths = Paths::of(std::path::Path::new("/home/h/.hq/demo"), "m1");
    assert_eq!(
        paths.dir,
        std::path::Path::new("/home/h/.hq/demo/missions/m1")
    );
    assert!(paths.mission.starts_with(&paths.dir));
}

#[test]
fn the_header_survives_a_round_trip_through_the_file() {
    let dir = tempfile::tempdir().unwrap();
    create(dir.path(), "m1", &header(), "prose").unwrap();
    let read = read_header(dir.path(), "m1").unwrap();
    assert_eq!(read, header(), "what hq freezes must be what was written");
    assert_eq!(read.shape(), hq::mission::Shape::Full);
}

#[test]
fn the_prose_is_kept_under_the_header_where_an_agent_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let paths = create(dir.path(), "m1", &header(), "Implement the domain.").unwrap();
    let text = std::fs::read_to_string(&paths.mission).unwrap();
    assert!(text.starts_with("---\n"), "{text}");
    assert!(text.contains("\n---\n"), "the header is delimited: {text}");
    assert!(text.contains("Implement the domain."), "{text}");
    // And the header comes first, so a reader that stops early still has it.
    assert!(text.find("branch:").unwrap() < text.find("Implement").unwrap());
}

#[test]
fn an_existing_mission_is_never_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let paths = create(dir.path(), "m1", &header(), "first").unwrap();
    std::fs::write(&paths.journal, "work already done\n").unwrap();

    let err = create(dir.path(), "m1", &header(), "second").unwrap_err();
    assert!(matches!(err, MissionDirError::Exists(..)), "{err}");
    assert_eq!(
        std::fs::read_to_string(&paths.journal).unwrap(),
        "work already done\n",
        "a mission is reframed by a verb, never by writing over it"
    );
}

#[test]
fn a_mission_id_that_could_not_be_a_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    for bad in ["../escape", "with space", "-leading", ""] {
        assert!(
            matches!(
                create(dir.path(), bad, &header(), "").unwrap_err(),
                MissionDirError::BadId(_)
            ),
            "{bad:?} should be refused"
        );
    }
    assert!(create(dir.path(), "lot_2-b", &header(), "").is_ok());
}

#[test]
fn a_mission_folder_without_a_header_says_so_rather_than_guessing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = create(dir.path(), "m1", &header(), "prose").unwrap();
    std::fs::write(&paths.mission, "# just prose, no header\n").unwrap();
    assert!(matches!(
        read_header(dir.path(), "m1").unwrap_err(),
        MissionDirError::NoHeader(_)
    ));

    std::fs::write(&paths.mission, "---\nbranch: [not, a, string]\n---\n").unwrap();
    assert!(matches!(
        read_header(dir.path(), "m1").unwrap_err(),
        MissionDirError::BadHeader { .. }
    ));
}

#[test]
fn missions_are_listed_in_order_and_an_unknown_one_is_named() {
    let dir = tempfile::tempdir().unwrap();
    create(dir.path(), "beta", &header(), "").unwrap();
    create(dir.path(), "alpha", &header(), "").unwrap();
    assert_eq!(list(dir.path()), vec!["alpha", "beta"]);

    assert!(matches!(
        read_header(dir.path(), "gamma").unwrap_err(),
        MissionDirError::Unknown(..)
    ));
}

#[test]
fn a_mission_with_no_service_states_why_rather_than_leaving_it_open() {
    // The shape is declared, never inferred from silence: an integration
    // mission that forgot to say so would run with an empty perimeter
    // (SPEC 2).
    let dir = tempfile::tempdir().unwrap();
    let mut h = header();
    h.integration = Integration::None {
        reason: "pure domain logic, nothing external".to_string(),
    };
    h.security = Security::Gates;
    create(dir.path(), "m1", &h, "").unwrap();

    let read = read_header(dir.path(), "m1").unwrap();
    assert_eq!(read.shape(), hq::mission::Shape::CodeOnly);
    match read.integration {
        Integration::None { reason } => assert!(reason.contains("pure domain")),
        other => panic!("{other:?}"),
    }
}

/// The verb, through the binary: a mission is framed from the command line
/// and read back by `status`.
#[test]
fn the_verbs_write_and_read_the_same_mission() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(root.join("hq.yaml"), "harness: claude-code\n").unwrap();

    let hq = env!("CARGO_BIN_EXE_hq");
    let run = |args: &[&str]| {
        std::process::Command::new(hq)
            .env("HOME", &home)
            .args(["-C"])
            .arg(&root)
            .args(args)
            .output()
            .unwrap()
    };

    let new = run(&[
        "mission",
        "new",
        "alpha",
        "--branch",
        "feat/alpha",
        "--lot",
        "L1:Read the spec",
        "--service",
        "db=db,10.4.0.7",
        "--security-agent",
    ]);
    assert!(
        new.status.success(),
        "{}",
        String::from_utf8_lossy(&new.stderr)
    );

    let status = run(&["mission", "status", "alpha"]);
    let text = String::from_utf8_lossy(&status.stdout);
    assert!(text.contains("feat/alpha (from dev)"), "{text}");
    assert!(text.contains("L1 — Read the spec"), "{text}");
    assert!(text.contains("db → db, 10.4.0.7"), "{text}");
    assert!(text.contains("Full"), "{text}");
    // It exists and has never run, and both halves are said.
    assert!(text.contains("not started"), "{text}");

    let list = run(&["mission", "list"]);
    assert!(String::from_utf8_lossy(&list.stdout).contains("alpha"));

    // A lot that is not `id:title` is refused before anything is written.
    let bad = run(&[
        "mission", "new", "beta", "--branch", "b", "--lot", "no-colon",
    ]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("id:title"),
        "{:?}",
        String::from_utf8_lossy(&bad.stderr)
    );
    assert!(!home.join(".hq/repo/missions/beta").exists());
}
