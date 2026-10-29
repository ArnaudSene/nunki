//! The secrets a human has ruled on (SPEC 4.4, gate 8).
//!
//! The file is read by a shell script in a container and written by `nunki`,
//! so what it holds has to be unambiguous on both sides: one line per ruling,
//! an id with no space in it, and a reason that is never empty.

use nunki::secrets::{self, Accepted, SecretError};

fn file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("hq").join(secrets::FILE)
}

const ID: &str = "4566bd67adf5913662ffe852917386d9d330a480:src/store.rs:Postgres:22";

#[test]
fn a_ruling_is_written_with_its_reason_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);

    assert_eq!(
        secrets::accept(&at, ID, "a disposable test DSN").unwrap(),
        Accepted::Added
    );

    let rulings = secrets::read(&at);
    assert_eq!(rulings.len(), 1, "{rulings:?}");
    assert_eq!(rulings[0].id, ID);
    assert_eq!(rulings[0].because, "a disposable test DSN");
}

/// The first ruling creates the file, and the HQ directory under it: nothing
/// asks a human to make a folder before they may rule on a finding.
#[test]
fn the_first_ruling_makes_the_file_and_says_what_it_is_for() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);

    secrets::accept(&at, ID, "a disposable test DSN").unwrap();

    let text = std::fs::read_to_string(&at).unwrap();
    assert!(text.starts_with('#'), "no header:\n{text}");
    assert!(text.contains("nunki secret accept"), "{text}");
}

/// Ruling twice on one finding replaces the reason. Two lines for one id
/// would make which reason applies a matter of order, and the reader takes
/// the first.
#[test]
fn ruling_again_on_the_same_finding_replaces_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);
    secrets::accept(&at, ID, "the first reason").unwrap();

    let what = secrets::accept(&at, ID, "the second reason").unwrap();

    assert_eq!(what, Accepted::Replaced("the first reason".into()));
    let rulings = secrets::read(&at);
    assert_eq!(rulings.len(), 1, "{rulings:?}");
    assert_eq!(rulings[0].because, "the second reason");
}

/// A ruling is one line, `<id> <reason>`, and the script splits it on
/// whitespace. An id with a space in it would make the two indistinguishable,
/// so it is refused at the one place that writes the file.
#[test]
fn an_id_with_a_space_in_it_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);

    let e = secrets::accept(&at, "an id with spaces", "why not").unwrap_err();

    assert!(matches!(e, SecretError::SpaceInId(_)), "{e:?}");
    assert!(!at.exists(), "a refused ruling still made the file");
}

/// `mission accept` requires `--because` for this reason, and so does this: a
/// risk accepted without a reason is not accepted, it is forgotten.
#[test]
fn a_ruling_without_a_reason_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);

    let e = secrets::accept(&at, ID, "   ").unwrap_err();

    assert!(matches!(e, SecretError::EmptyReason), "{e:?}");
}

/// A reason with a newline in it would write a second line the reader would
/// take for another ruling — with the reason's first word as an id.
#[test]
fn a_reason_that_spans_two_lines_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);

    let e = secrets::accept(&at, ID, "because\nsomething-else  and this").unwrap_err();

    assert!(matches!(e, SecretError::ReasonIsNotALine), "{e:?}");
}

#[test]
fn a_ruling_can_be_dropped_and_the_others_stay() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);
    secrets::accept(&at, ID, "the first").unwrap();
    secrets::accept(&at, "another:src/x.rs:AWS:3", "the second").unwrap();

    assert!(secrets::forget(&at, ID).unwrap());

    let rulings = secrets::read(&at);
    assert_eq!(rulings.len(), 1, "{rulings:?}");
    assert_eq!(rulings[0].id, "another:src/x.rs:AWS:3");
    // And the header survives, because the file is read by a human too.
    let text = std::fs::read_to_string(&at).unwrap();
    assert!(text.starts_with('#'), "{text}");
}

/// Forgetting what was never ruled on is not an error: a credential revoked
/// twice is a human being careful.
#[test]
fn forgetting_what_was_never_ruled_on_says_so_without_failing() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);

    assert!(!secrets::forget(&at, ID).unwrap());
}

/// Nothing ruled on is the honest answer for a project that has never had a
/// secret — not an error, and not an empty file left behind by a read.
#[test]
fn a_project_that_has_ruled_on_nothing_reads_back_empty() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);

    assert!(secrets::read(&at).is_empty());
    assert!(!at.exists(), "reading made the file");
}

/// A human edits this file too — it says so in its own header. A comment is
/// not a ruling, and neither is an id somebody wrote with no reason after it:
/// that one would otherwise accept a secret silently.
#[test]
fn a_comment_and_a_bare_id_are_not_rulings() {
    let dir = tempfile::tempdir().unwrap();
    let at = file(&dir);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(
        &at,
        format!("# a comment\n\n{ID}\nanother:src/x.rs:AWS:3  a reason\n"),
    )
    .unwrap();

    let rulings = secrets::read(&at);

    assert_eq!(rulings.len(), 1, "{rulings:?}");
    assert_eq!(rulings[0].id, "another:src/x.rs:AWS:3");
}
