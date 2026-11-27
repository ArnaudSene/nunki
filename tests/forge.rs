//! Opening the pull request (SPEC 4.2, `nunki push`): the real HTTP client,
//! pointed at a server this test controls.
//!
//! A fake forge would prove the fake. What a forge can get wrong silently is
//! the shape of the request — a missing `Authorization` header gets a 404
//! from GitHub, not a 401, and reads like a repository that does not exist —
//! so the request is read byte by byte, as a server receives it.
//!
//! Everything here goes through the boundary the rest of the code uses: a
//! remote names a forge and a repository, and the adapter that comes back is
//! asked through [`Forge`]. Adding another forge is then a file beside
//! `github.rs`, and these tests say what it would have to answer.

use std::net::TcpListener;

mod common;
use common::serve;

use nunki::forge::{self, Forge, ForgeError, Opened, PullRequest, Repo};

/// The adapter and the repository `git@github.com:o/r.git` names, talking to
/// the server this test lifted rather than to GitHub.
fn at(api: &str) -> (Box<dyn Forge>, Repo) {
    forge::of_remote_at("git@github.com:o/r.git", api).expect("github is the adapter nunki has")
}

fn request() -> PullRequest {
    PullRequest {
        title: "feat: the thing".into(),
        body: "Why it had to.".into(),
        head: "mission/x".into(),
        base: "dev".into(),
    }
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

#[test]
fn a_pull_request_is_opened_with_the_humans_token_and_the_missions_words() {
    let (api, server) = serve(vec![(
        201,
        r#"{"html_url":"https://github.com/o/r/pull/7"}"#,
    )]);
    let (forge, repo) = at(&api);
    let opened = forge.open("tok-123", &repo, &request()).unwrap();
    assert_eq!(
        opened,
        Opened::Created("https://github.com/o/r/pull/7".into())
    );

    let seen = server.join().unwrap();
    let sent = &seen[0];
    assert!(sent.starts_with("POST /repos/o/r/pulls HTTP/1.1"), "{sent}");
    assert_eq!(
        header(sent, "authorization"),
        Some("Bearer tok-123"),
        "without it GitHub answers 404, and that reads as a missing repository: {sent}"
    );
    assert!(
        header(sent, "user-agent").is_some_and(|v| v.starts_with("nunki/")),
        "GitHub refuses a request with no User-Agent: {sent}"
    );
    assert_eq!(
        header(sent, "content-type"),
        Some("application/json"),
        "the body is serialised by nunki, so the header is nunki's to send: {sent}"
    );
    let body: serde_json::Value = serde_json::from_str(sent.split("\r\n\r\n").nth(1).unwrap())
        .unwrap_or_else(|e| panic!("{e}: {sent}"));
    assert_eq!(body["title"], "feat: the thing");
    assert_eq!(body["body"], "Why it had to.");
    assert_eq!(body["head"], "mission/x");
    assert_eq!(body["base"], "dev");
}

/// The second push of a mission, after a volet: the pull request is already
/// there, the forge refuses a duplicate, and the one that exists is the
/// answer.
#[test]
fn a_pull_request_already_open_is_found_rather_than_reported_as_a_failure() {
    let (api, server) = serve(vec![
        (
            422,
            r#"{"message":"Validation Failed","errors":[{"message":"A pull request already exists for o:mission/x."}]}"#,
        ),
        (200, r#"[{"html_url":"https://github.com/o/r/pull/7"}]"#),
    ]);
    let (forge, repo) = at(&api);
    let opened = forge.open("tok", &repo, &request()).unwrap();
    assert_eq!(
        opened,
        Opened::AlreadyOpen("https://github.com/o/r/pull/7".into())
    );

    let seen = server.join().unwrap();
    let asked = seen[1].lines().next().unwrap();
    assert!(asked.starts_with("GET /repos/o/r/pulls?"), "{asked}");
    for part in ["head=o%3Amission%2Fx", "base=dev", "state=open"] {
        assert!(asked.contains(part), "{part} in {asked}");
    }
    assert_eq!(header(&seen[1], "authorization"), Some("Bearer tok"));
}

/// A refusal carries its reason. "403" alone would leave the human to guess
/// between a wrong token, a missing scope and a protected repository.
#[test]
fn a_refusal_is_named_with_the_forges_own_reason() {
    let (api, server) = serve(vec![(401, r#"{"message":"Bad credentials"}"#)]);
    let (forge, repo) = at(&api);
    let err = forge.open("wrong", &repo, &request()).unwrap_err();
    match err {
        ForgeError::Refused { status, message } => {
            assert_eq!(status, 401);
            assert!(message.contains("Bad credentials"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    server.join().unwrap();
}

/// Any other 422 — a base branch that does not exist, say — is a refusal,
/// not a pull request someone else already opened.
#[test]
fn a_validation_failure_that_is_not_a_duplicate_is_a_refusal() {
    let (api, server) = serve(vec![(
        422,
        r#"{"message":"Validation Failed","errors":[{"field":"base","code":"invalid"}]}"#,
    )]);
    let (forge, repo) = at(&api);
    let err = forge.open("tok", &repo, &request()).unwrap_err();
    assert!(
        matches!(err, ForgeError::Refused { status: 422, .. }),
        "{err:?}"
    );
    assert!(err.to_string().contains("base"), "{err}");
    assert_eq!(server.join().unwrap().len(), 1, "nothing was looked up");
}

#[test]
fn a_forge_nobody_answers_at_is_unreachable_not_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let api = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let (forge, repo) = at(&api);
    let err = forge.open("tok", &repo, &request()).unwrap_err();
    assert!(matches!(err, ForgeError::Unreachable(_)), "{err:?}");
}

#[test]
fn a_github_remote_names_its_forge_and_repository_and_any_other_names_none() {
    for remote in [
        "git@github.com:acme/app.git",
        "https://github.com/acme/app.git",
        "https://github.com/acme/app",
        "ssh://git@github.com/acme/app.git",
    ] {
        let (forge, repo) = forge::of_remote(remote).unwrap_or_else(|| panic!("{remote}"));
        assert_eq!(forge.name(), "GitHub", "{remote}");
        assert_eq!(
            repo,
            Repo {
                owner: "acme".into(),
                name: "app".into()
            },
            "{remote}"
        );
    }
    // A forge nunki has no adapter for is said to be one, never guessed at:
    // the credential and the request shape here are GitHub's.
    for remote in [
        "git@gitlab.com:team/thing.git",
        "/tmp/bare.git",
        "https://github.com/only-an-owner",
        "https://github.com/a/b/c",
    ] {
        assert!(forge::of_remote(remote).is_none(), "{remote}");
    }
}

/// The address a human opens the pull request at when `nunki` could not: the
/// forge's own, worked out from the repository it named.
#[test]
fn the_address_to_open_the_pull_request_by_hand_is_the_forges_own() {
    for remote in [
        "https://github.com/acme/app.git",
        "git@github.com:acme/app.git",
        "https://github.com/acme/app",
    ] {
        let (forge, repo) = forge::of_remote(remote).unwrap_or_else(|| panic!("{remote}"));
        assert_eq!(
            forge.compare(&repo, "dev", "mission/x"),
            "https://github.com/acme/app/compare/dev...mission/x?expand=1",
            "{remote}"
        );
    }
}

#[test]
fn the_title_is_the_first_line_that_says_anything_and_the_rest_is_the_body() {
    let pr = PullRequest::from_markdown(
        "\n# feat: the thing\n\nWhy it had to.\n\n## Integration\nwired\n",
        "mission/x",
        "dev",
    )
    .unwrap();
    assert_eq!(pr.title, "feat: the thing");
    assert_eq!(pr.body, "Why it had to.\n\n## Integration\nwired");
    assert_eq!((pr.head.as_str(), pr.base.as_str()), ("mission/x", "dev"));

    assert_eq!(PullRequest::from_markdown(" \n\n", "h", "b"), None);
    assert_eq!(
        PullRequest::from_markdown("#\nbody", "h", "b"),
        None,
        "a heading with no words is no title"
    );
}

/// A temporary `~/.nunki/` and one project's home inside it, `<id>/`, the
/// way [`nunki::project::Project::home`] lays them out. Never the real one.
fn homes() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("0b5e7c1a-project");
    std::fs::create_dir_all(&home).unwrap();
    (dir, home)
}

fn secret_of(credential: &forge::Credential) -> Option<&str> {
    match credential {
        forge::Credential::Token(token) => Some(token.secret()),
        forge::Credential::Absent(_) => None,
    }
}

fn absent(credential: forge::Credential) -> forge::Absent {
    match credential {
        forge::Credential::Absent(absent) => absent,
        forge::Credential::Token(_) => panic!("a token was read where none should be"),
    }
}

/// The project's own file is the one read, trimmed; an empty one is no
/// token, and says where a token goes, as a missing one does.
#[test]
fn a_token_is_read_trimmed_from_the_projects_home_and_an_empty_file_is_no_token() {
    let (_dir, home) = homes();
    let at = home.join(forge::TOKEN_FILE);

    assert_eq!(absent(forge::credential(&home)).at, at);
    std::fs::write(&at, "  \n").unwrap();
    let empty = absent(forge::credential(&home));
    assert_eq!(empty.at, at);
    assert_eq!(empty.retired, None);

    std::fs::write(&at, "tok-made-up\n").unwrap();
    assert_eq!(secret_of(&forge::credential(&home)), Some("tok-made-up"));
}

/// The machine-wide `~/.nunki/forge-token` is not read, not even as a
/// fallback: holding a token, it gives none to a project that has none — and
/// it is named, so the move is said rather than silent.
#[test]
fn the_machine_wide_file_alone_is_not_read_and_is_named() {
    let (dir, home) = homes();
    let retired = dir.path().join(forge::TOKEN_FILE);
    std::fs::write(&retired, "tok-retired\n").unwrap();

    let absent = absent(forge::credential(&home));
    assert_eq!(absent.at, home.join(forge::TOKEN_FILE));
    assert_eq!(absent.retired.as_deref(), Some(retired.as_path()));
    let note = absent.retired_note().expect("the retired file is named");
    assert!(note.contains(&retired.display().to_string()), "{note}");
    assert!(note.contains("no longer read"), "{note}");
    assert!(
        note.contains(&home.join(forge::TOKEN_FILE).display().to_string()),
        "{note}"
    );
    assert!(
        !note.contains("tok-retired"),
        "the note never says what it holds"
    );
}

/// With both files, the project's is the one used: the retired one is
/// neither read nor mixed in.
#[test]
fn the_projects_token_wins_over_a_machine_wide_file_still_there() {
    let (dir, home) = homes();
    std::fs::write(dir.path().join(forge::TOKEN_FILE), "tok-retired\n").unwrap();
    std::fs::write(home.join(forge::TOKEN_FILE), "tok-project\n").unwrap();

    assert_eq!(secret_of(&forge::credential(&home)), Some("tok-project"));
}

/// The one refusal `nunki push` and `nunki check` both give: the project's
/// path, the permissions the token needs, a file its owner alone reads and
/// the command that makes it so — and, only when the retired file is there,
/// that it is no longer read. Never what any file holds.
#[test]
fn the_refusal_names_the_path_the_permissions_and_the_owner_only_file() {
    let (dir, home) = homes();
    let at = home.join(forge::TOKEN_FILE);

    let alone = absent(forge::credential(&home)).refusal();
    assert!(
        alone.starts_with(&format!("no forge token at {}:", at.display())),
        "{alone}"
    );
    for permission in [
        "Contents: read and write",
        "Pull requests: read and write",
        "Metadata: read",
    ] {
        assert!(alone.contains(permission), "{permission}: {alone}");
    }
    assert!(alone.contains("fine-grained"), "{alone}");
    assert!(alone.contains("readable by its owner only"), "{alone}");
    assert!(
        alone.contains(&format!("chmod 600 {}", at.display())),
        "{alone}"
    );
    assert!(!alone.contains("no longer read"), "{alone}");

    let retired = dir.path().join(forge::TOKEN_FILE);
    std::fs::write(&retired, "tok-retired\n").unwrap();
    let moved = absent(forge::credential(&home)).refusal();
    assert!(
        moved.starts_with(&alone),
        "the same refusal, then the note: {moved}"
    );
    assert!(
        moved.ends_with(&format!(
            ". {}",
            absent(forge::credential(&home)).retired_note().unwrap()
        )),
        "{moved}"
    );
    assert!(!moved.contains("tok-retired"), "{moved}");
}

/// No retired file, nothing said about one: a note about a file that is not
/// there would send the human looking for it.
#[test]
fn without_a_machine_wide_file_no_move_is_announced() {
    let (_dir, home) = homes();

    let absent = absent(forge::credential(&home));
    assert_eq!(absent.retired, None);
    assert_eq!(absent.retired_note(), None);
}

/// A token carried in a value that is debugged — an assertion, a log — does
/// not reach the output.
#[test]
fn a_token_does_not_print_itself() {
    let (_dir, home) = homes();
    std::fs::write(home.join(forge::TOKEN_FILE), "tok-made-up\n").unwrap();

    let shown = format!("{:?}", forge::credential(&home));
    assert!(!shown.contains("tok-made-up"), "{shown}");
    assert!(shown.contains("Token"), "{shown}");
}

/// What a bind mount may not be: the home, a directory above it, the token
/// file, or a link to any of them. What a container does see under the home
/// — its mission folder, a stack's script — exposes neither.
#[test]
fn a_mount_exposes_the_home_from_it_or_above_it_and_never_from_below() {
    let (dir, home) = homes();
    let token = home.join(forge::TOKEN_FILE);
    std::fs::write(&token, "tok-made-up\n").unwrap();
    let mission = home.join("hq/missions/m1");
    std::fs::create_dir_all(&mission).unwrap();

    let kept = forge::kept_out(&home);
    let exposes = |source: &std::path::Path| kept.iter().any(|k| forge::exposes(source, k));

    assert!(exposes(&home));
    assert!(exposes(dir.path()), "a directory above the home holds it");
    assert!(exposes(&token));
    assert!(!exposes(&mission), "the mission folder is below the home");
    assert!(!exposes(&home.join("stacks/rust/prepush.sh")));
    assert!(!exposes(&dir.path().join("elsewhere")));

    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        let to_home = outside.path().join("to-home");
        std::os::unix::fs::symlink(&home, &to_home).unwrap();
        assert!(exposes(&to_home), "a link to the home is the home");
        let to_token = outside.path().join("to-token");
        std::os::unix::fs::symlink(&token, &to_token).unwrap();
        assert!(exposes(&to_token), "a link to the token is the token");
    }
}

/// A token file that is itself a link: the place it lands is kept out too,
/// so mounting that directory is refused like mounting the home.
#[cfg(unix)]
#[test]
fn where_a_linked_token_lands_is_kept_out_too() {
    let (_dir, home) = homes();
    let store = tempfile::tempdir().unwrap();
    let real = store.path().join("project-token");
    std::fs::write(&real, "tok-made-up\n").unwrap();
    std::os::unix::fs::symlink(&real, home.join(forge::TOKEN_FILE)).unwrap();

    let kept = forge::kept_out(&home);
    assert!(
        kept.iter().any(|k| forge::exposes(store.path(), k)),
        "{kept:?}"
    );
}

/// The one call that proves what the local server cannot: the address, the
/// certificate chain against the bundled roots, and a real credential,
/// together. Read-only — nothing is ever written to the forge from a test.
///
/// ```sh
/// HQ_LIVE_FORGE_TOKEN=$(gh auth token) cargo test --test forge -- --ignored
/// ```
#[test]
#[ignore = "talks to GitHub; needs HQ_LIVE_FORGE_TOKEN"]
fn live_the_real_forge_is_reached_over_tls_and_a_bad_token_is_refused() {
    let Ok(token) = std::env::var("HQ_LIVE_FORGE_TOKEN") else {
        eprintln!("skipped: HQ_LIVE_FORGE_TOKEN is not set");
        return;
    };
    let (forge, repo) = forge::of_remote("https://github.com/ArnaudSene/nunki").unwrap();
    forge
        .can_read(token.trim(), &repo)
        .expect("the token reads the repository");

    let err = forge.can_read("not-a-token", &repo).unwrap_err();
    assert!(
        matches!(err, ForgeError::Refused { status: 401, .. }),
        "the same call with a bad token is refused, so the green above is the token's: {err:?}"
    );
}

/// The branch question, against the real forge: this repository's own `dev`
/// answers with a protection state, and a branch that does not exist is read
/// as absent — the one 404 that is an answer. Read-only.
#[test]
#[ignore = "talks to GitHub; needs HQ_LIVE_FORGE_TOKEN"]
fn live_the_real_forge_says_whether_a_branch_is_protected() {
    let Ok(token) = std::env::var("HQ_LIVE_FORGE_TOKEN") else {
        eprintln!("skipped: HQ_LIVE_FORGE_TOKEN is not set");
        return;
    };
    let (forge, repo) = forge::of_remote("https://github.com/ArnaudSene/nunki").unwrap();
    let dev = forge
        .protection(token.trim(), &repo, "dev")
        .expect("dev is readable");
    eprintln!("dev on the forge: {dev:?}");
    assert!(matches!(
        dev,
        forge::Protection::Protected | forge::Protection::Unprotected
    ));
    assert_eq!(
        forge
            .protection(token.trim(), &repo, "no-such-branch-nunki-check")
            .unwrap(),
        forge::Protection::NoSuchBranch
    );
}
