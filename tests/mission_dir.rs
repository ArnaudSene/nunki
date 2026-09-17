//! The mission folder at the HQ (SPEC 4.1): five files, and the split
//! between them is a restriction rather than a convention.

use nunki::mission::dir::{MissionDirError, Paths, create, list, read_header};
use nunki::mission::{Bounds, Header, Integration, Lot, Security, Service};

mod common;

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
            wiring: Vec::new(),
            services: vec![Service {
                name: "db".to_string(),
                reach: vec!["db".to_string(), "10.4.0.7".to_string()],
                shared: false,
            }],
        },
        security: Security::Agent,
        arbiter: None,
        run: None,
        account: None,
        model: None,
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
    let paths = Paths::of(std::path::Path::new("/home/h/.nunki/demo"), "m1");
    assert_eq!(
        paths.dir,
        std::path::Path::new("/home/h/.nunki/demo/missions/m1")
    );
    assert!(paths.mission.starts_with(&paths.dir));
}

#[test]
fn the_header_survives_a_round_trip_through_the_file() {
    let dir = tempfile::tempdir().unwrap();
    create(dir.path(), "m1", &header(), "prose").unwrap();
    let read = read_header(dir.path(), "m1").unwrap();
    assert_eq!(
        read,
        header(),
        "what nunki freezes must be what was written"
    );
    assert_eq!(read.shape(), nunki::mission::Shape::Full);
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
    assert_eq!(read.shape(), nunki::mission::Shape::CodeOnly);
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
    common::project_home(&root, &home, "harness: claude-code\n");

    let nunki = env!("CARGO_BIN_EXE_nunki");
    let run = |args: &[&str]| {
        std::process::Command::new(nunki)
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
    assert!(!home.join(".nunki/repo/hq/missions/beta").exists());
}

/// `FOLLOWUP_HQ.md` is the file an agent is told to read before anything
/// else. It was being written with runs of leading spaces — a lost line
/// continuation — which Markdown renders as a code block: the two sentences
/// that say what the file is for arrived as monospaced source. Prose, and
/// nothing that indents into a code block.
#[test]
fn the_follow_up_file_is_prose_and_not_a_code_block() {
    let dir = tempfile::tempdir().unwrap();
    let paths = nunki::mission::dir::create(dir.path(), "m1", &header(), "do it").unwrap();
    let text = std::fs::read_to_string(&paths.followup).unwrap();

    for line in text.lines() {
        assert!(
            !line.starts_with("    "),
            "this line renders as a code block: {line:?}\n{text}"
        );
        assert!(
            !line.contains("  "),
            "a run of spaces inside a line: {line:?}\n{text}"
        );
    }
    // And it still says who it is for, and that the agent does not write it.
    assert!(text.contains("never writes it"), "{text}");
}

/// The four files the agent writes are bind-mounted one at a time, so the
/// rest of the folder can stay read-only. An engine handed a source that does
/// not exist creates a **directory** there, and the mission then fails on a
/// message naming the symptom: `MUTANTS.triage.json: Is a directory (os error
/// 21)`, measured on 2026-09-17 after that file was removed by hand between
/// two runs. `create` writes the four once; this is the guard that holds at
/// every lift, because the folder is the human's and removing a file from it
/// — clearing a `VERDICT.json` that froze wrong — is a repair, not a mistake.
#[test]
fn every_file_the_agent_writes_is_put_back_before_a_lift() {
    let dir = tempfile::tempdir().unwrap();
    let paths = create(dir.path(), "m1", &header(), "do it").unwrap();

    // Driven by the list compose mounts file by file, and not by a copy of
    // it: a fifth writable file added there is covered here without anyone
    // remembering to come back.
    assert!(!nunki::compose::AGENT_WRITABLE.is_empty());
    for file in nunki::compose::AGENT_WRITABLE {
        let path = paths.dir.join(file);
        std::fs::remove_file(&path).unwrap();

        nunki::mission::dir::ensure_writable(&paths.dir).unwrap();

        assert!(path.is_file(), "{file} is not a file after the guard");
    }
}

/// The placeholder the engine leaves is an **empty** directory, and putting a
/// file back means removing it. `remove_dir` is the call that does it:
/// it succeeds on an empty directory and on nothing else, so the guard can
/// undo the engine and can never destroy what someone put there.
#[test]
fn the_empty_directory_an_engine_leaves_becomes_a_file_again() {
    let dir = tempfile::tempdir().unwrap();
    let paths = create(dir.path(), "m1", &header(), "do it").unwrap();

    // Exactly what the engine does with a bind mount whose source is gone.
    std::fs::remove_file(&paths.triage).unwrap();
    std::fs::create_dir(&paths.triage).unwrap();

    nunki::mission::dir::ensure_writable(&paths.dir).unwrap();

    assert!(paths.triage.is_file(), "the directory was not replaced");
    assert_eq!(std::fs::read_to_string(&paths.triage).unwrap(), "");
}

/// A directory with something in it is not the engine's placeholder, and the
/// guard says so rather than taking it. Nothing in the mission folder is
/// deleted on a guess.
#[test]
fn a_directory_holding_something_is_named_and_not_removed() {
    let dir = tempfile::tempdir().unwrap();
    let paths = create(dir.path(), "m1", &header(), "do it").unwrap();

    std::fs::remove_file(&paths.triage).unwrap();
    std::fs::create_dir(&paths.triage).unwrap();
    std::fs::write(paths.triage.join("kept.txt"), "someone's").unwrap();

    let error = nunki::mission::dir::ensure_writable(&paths.dir).unwrap_err();

    assert!(
        matches!(error, MissionDirError::EngineLeftADirectory { .. }),
        "{error:?}"
    );
    let said = error.to_string();
    assert!(said.contains("is a directory"), "{said}");
    assert!(said.contains("nothing"), "{said}");
    assert_eq!(
        std::fs::read_to_string(paths.triage.join("kept.txt")).unwrap(),
        "someone's",
        "the guard took what it found"
    );
}

/// The guard is idempotent: a file that is there keeps its content. A run
/// whose predecessor wrote a verdict must not find it emptied.
#[test]
fn a_file_that_is_there_keeps_what_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let paths = create(dir.path(), "m1", &header(), "do it").unwrap();
    std::fs::write(&paths.verdict, r#"{"verdict":"CLEAR"}"#).unwrap();

    nunki::mission::dir::ensure_writable(&paths.dir).unwrap();

    assert_eq!(
        std::fs::read_to_string(&paths.verdict).unwrap(),
        r#"{"verdict":"CLEAR"}"#
    );
}
