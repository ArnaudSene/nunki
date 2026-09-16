//! The ledger of sessions (SPEC 4.1): one identifier per project, and the
//! repository it belongs to.

use std::path::Path;

use nunki::project::Project;
use nunki::sessions::{self, SessionsError};

fn repo(at: &Path) -> std::path::PathBuf {
    std::fs::create_dir_all(at).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .arg(at)
            .status()
            .unwrap()
            .success()
    );
    std::fs::canonicalize(at).unwrap()
}

/// The reason the ledger exists: a name cannot tell two projects apart, and
/// before it the second repository called `api` was refused rather than
/// served.
#[test]
fn two_repositories_of_the_same_name_are_two_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let nunki_home = dir.path().join(".nunki");
    let first = repo(&dir.path().join("work/api"));
    let second = repo(&dir.path().join("archive/api"));

    let one = sessions::open(&nunki_home, &first).unwrap();
    let other = sessions::open(&nunki_home, &second).unwrap();
    assert_ne!(one, other, "the same name, and not the same session");

    // And each keeps its own, however many times it is asked.
    assert_eq!(sessions::open(&nunki_home, &first).unwrap(), one);
    assert_eq!(sessions::find(&nunki_home, &second).unwrap(), Some(other));
}

/// A repository nobody opened a session for is not a project, and the message
/// names both ways out.
#[test]
fn a_repository_with_no_session_is_named_with_what_to_run() {
    let dir = tempfile::tempdir().unwrap();
    let nunki_home = dir.path().join(".nunki");
    let root = repo(&dir.path().join("api"));

    assert_eq!(sessions::find(&nunki_home, &root).unwrap(), None);
    let err = Project::home_for(&root).unwrap_err();
    let said = err.to_string();
    assert!(said.contains("nunki init"), "{said}");
    assert!(said.contains("nunki adopt"), "{said}");
}

/// What the file holds, as a human reads it: the identifier, and the path.
#[test]
fn the_ledger_is_an_identifier_and_a_path() {
    let dir = tempfile::tempdir().unwrap();
    let nunki_home = dir.path().join(".nunki");
    let root = repo(&dir.path().join("api"));
    let id = sessions::open(&nunki_home, &root).unwrap();

    let text = std::fs::read_to_string(sessions::path(&nunki_home)).unwrap();
    let read: std::collections::BTreeMap<String, String> = serde_json::from_str(&text).unwrap();
    assert_eq!(read.get(&id).map(String::as_str), root.to_str());

    // Written whole and renamed into place: no half-written file is left for
    // the next verb to read.
    let leftovers: Vec<String> = std::fs::read_dir(&nunki_home)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != sessions::FILE)
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// A repository that moved is the same project: `nunki adopt` points its
/// session at the new path, and its HQ follows.
#[test]
fn a_moved_repository_is_adopted_and_keeps_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let nunki_home = dir.path().join(".nunki");
    let before = repo(&dir.path().join("old/api"));
    let id = sessions::open(&nunki_home, &before).unwrap();

    std::fs::create_dir_all(dir.path().join("new")).unwrap();
    std::fs::rename(&before, dir.path().join("new/api")).unwrap();
    let after = std::fs::canonicalize(dir.path().join("new/api")).unwrap();

    assert_eq!(sessions::adopt(&nunki_home, &id, &after).unwrap(), after);
    assert_eq!(sessions::find(&nunki_home, &after).unwrap(), Some(id));
}

/// Adopting must not take a live project's HQ: while the recorded path still
/// holds a repository, the session belongs to it.
#[test]
fn adopting_a_session_whose_repository_is_still_there_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let nunki_home = dir.path().join(".nunki");
    let live = repo(&dir.path().join("live/api"));
    let id = sessions::open(&nunki_home, &live).unwrap();
    let elsewhere = repo(&dir.path().join("elsewhere/api"));

    match sessions::adopt(&nunki_home, &id, &elsewhere).unwrap_err() {
        SessionsError::StillThere { root, .. } => assert_eq!(root, live),
        other => panic!("a live project keeps its session: {other:?}"),
    }
    assert_eq!(sessions::find(&nunki_home, &live).unwrap(), Some(id));

    // And an identifier nobody opened is said as such.
    let err = sessions::adopt(&nunki_home, "no-such-session", &elsewhere).unwrap_err();
    assert!(matches!(err, SessionsError::Unknown { .. }), "{err}");
    assert!(err.to_string().contains("nunki init"), "{err}");
}
