//! Who `hq` is talking to (SPEC 4.5): a mission ends by handing something
//! back to a person, and it has to be able to name them.

use std::path::Path;

use hq::human::{Human, ME_FILE, Source, me};

fn git(at: &Path, args: &[&str]) {
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(at)
            .args(args)
            .status()
            .unwrap()
            .success(),
        "git {args:?}"
    );
}

#[test]
fn a_declared_name_wins_over_everything_else() {
    let dir = tempfile::tempdir().unwrap();
    let hq_home = dir.path().join(".hq");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&hq_home).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Some Committer"]);

    std::fs::write(
        hq_home.join(ME_FILE),
        "name: Arnaud\nemail: arnaud@example.com\n",
    )
    .unwrap();

    let who = me(&hq_home, Some(&repo));
    assert_eq!(who.name.as_deref(), Some("Arnaud"));
    assert_eq!(who.email.as_deref(), Some("arnaud@example.com"));
    // Where the name came from is kept, because a name from a file is a
    // statement and a name from the environment is a guess.
    assert!(matches!(who.source, Source::Declared(_)));
}

#[test]
fn git_answers_when_nothing_was_declared() {
    let dir = tempfile::tempdir().unwrap();
    let hq_home = dir.path().join(".hq");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&hq_home).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Igor"]);
    git(&repo, &["config", "user.email", "igor@example.com"]);

    let who = me(&hq_home, Some(&repo));
    assert_eq!(who.name.as_deref(), Some("Igor"));
    assert_eq!(who.email.as_deref(), Some("igor@example.com"));
    assert_eq!(who.source, Source::Git);
}

#[test]
fn an_empty_declaration_is_not_a_name() {
    let dir = tempfile::tempdir().unwrap();
    let hq_home = dir.path().join(".hq");
    std::fs::create_dir_all(&hq_home).unwrap();
    std::fs::write(hq_home.join(ME_FILE), "name: \"   \"\n").unwrap();

    // It falls through rather than addressing somebody as blank.
    let who = me(&hq_home, None);
    assert_ne!(who.source, Source::Declared(hq_home.join(ME_FILE)));
}

#[test]
fn a_name_nobody_gave_is_never_invented() {
    let unknown = Human {
        name: None,
        email: None,
        source: Source::Unknown,
    };
    assert!(!unknown.is_known());
    // It says "the human" rather than making one up.
    assert_eq!(unknown.addressed(), "the human");
    assert!(
        unknown
            .source
            .describe()
            .contains("nothing says who you are")
    );
}

#[test]
fn a_mission_says_who_arbitrates_and_the_follow_up_is_addressed_to_them() {
    let dir = tempfile::tempdir().unwrap();
    let header = hq::mission::Header {
        branch: "feat/x".to_string(),
        base: "dev".to_string(),
        lots: vec![hq::mission::Lot {
            id: "L1".to_string(),
            title: "one".to_string(),
        }],
        integration: hq::mission::Integration::None {
            reason: "none".to_string(),
        },
        security: hq::mission::Security::Gates,
        arbiter: Some("Igor".to_string()),
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    let paths = hq::mission::dir::create(dir.path(), "m1", &header, "").unwrap();

    let followup = std::fs::read_to_string(&paths.followup).unwrap();
    // The exact line, not a substring of it: "for Igorsomebody" would
    // contain "for Igor" and address nobody.
    assert_eq!(
        followup.lines().next(),
        Some("# Follow-up — for Igor"),
        "{followup}"
    );
    // An arbitration for Arnaud is not an arbitration for Igor.
    assert!(!followup.contains("Arnaud"), "{followup}");

    // And it survives the round trip through the file, frozen with the rest.
    let read = hq::mission::dir::read_header(dir.path(), "m1").unwrap();
    assert_eq!(read.arbiter.as_deref(), Some("Igor"));
}

#[test]
fn a_mission_with_nobody_named_still_addresses_somebody() {
    let dir = tempfile::tempdir().unwrap();
    let header = hq::mission::Header {
        branch: "feat/x".to_string(),
        base: "dev".to_string(),
        lots: Vec::new(),
        integration: hq::mission::Integration::None {
            reason: "none".to_string(),
        },
        security: hq::mission::Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    let paths = hq::mission::dir::create(dir.path(), "m1", &header, "").unwrap();
    let followup = std::fs::read_to_string(&paths.followup).unwrap();
    assert_eq!(
        followup.lines().next(),
        Some("# Follow-up — for the human"),
        "{followup}"
    );
}

/// The verb, through the binary.
#[test]
fn whoami_says_where_the_name_came_from() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(home.join(".hq")).unwrap();
    std::fs::write(root.join("hq.yaml"), "harness: claude-code\n").unwrap();
    std::fs::write(home.join(".hq").join(ME_FILE), "name: Arnaud\n").unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hq"))
        .env("HOME", &home)
        .args(["-C"])
        .arg(&root)
        .arg("whoami")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Arnaud"), "{text}");
    assert!(text.contains("me.yaml"), "it says where it got it: {text}");
}

#[test]
fn with_nothing_to_go_on_hq_says_so_instead_of_inventing_a_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(home.join(".hq")).unwrap();
    std::fs::write(root.join("hq.yaml"), "harness: claude-code\n").unwrap();

    // No declaration, no git repository, and no account name: a mission that
    // came back would have nobody to come back to, and saying "the human"
    // as if it were a name would be worse than saying nothing.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hq"))
        .env("HOME", &home)
        .env_remove("USER")
        .args(["-C"])
        .arg(&root)
        .arg("whoami")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("does not know who you are"), "{text}");
    assert!(
        text.contains("me.yaml"),
        "and it says what to write: {text}"
    );
}
