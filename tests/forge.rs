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
        forge::Credential::Absent(_) | forge::Credential::Refused(_) => None,
    }
}

fn absent(credential: forge::Credential) -> forge::Absent {
    match credential {
        forge::Credential::Absent(absent) => absent,
        other => panic!("not absent: {other:?}"),
    }
}

fn refused(credential: forge::Credential) -> forge::Refused {
    match credential {
        forge::Credential::Refused(refused) => refused,
        other => panic!("not refused: {other:?}"),
    }
}

/// The project's own file is the one read, trimmed; an empty one is no
/// token, and says where a token goes, as a missing one does.
#[test]
fn a_token_is_read_trimmed_from_the_projects_home_and_an_empty_file_is_no_token() {
    let (_dir, home) = homes();
    let at = home.join(forge::TOKEN_FILE);

    assert_eq!(absent(forge::credential(&home)).at, at);
    common::write_token(&at, "  \n");
    let empty = absent(forge::credential(&home));
    assert_eq!(empty.at, at);
    assert_eq!(empty.retired, None);

    common::write_token(&at, "tok-made-up\n");
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
    common::write_token(&home.join(forge::TOKEN_FILE), "tok-project\n");

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
    common::write_token(&home.join(forge::TOKEN_FILE), "tok-made-up\n");

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

// --- a token file others can read is refused (SPEC 4.2, `nunki push`) -------

#[cfg(unix)]
fn token_with_mode(home: &std::path::Path, mode: u32) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let at = home.join(forge::TOKEN_FILE);
    std::fs::write(&at, "tok-made-up\n").unwrap();
    std::fs::set_permissions(&at, std::fs::Permissions::from_mode(mode)).unwrap();
    at
}

/// Readable by its owner only — 0600, or 0400 — the token is used.
#[cfg(unix)]
#[test]
fn a_token_its_owner_alone_can_read_is_used() {
    for mode in [0o600, 0o400] {
        let (_dir, home) = homes();
        token_with_mode(&home, mode);
        assert_eq!(
            secret_of(&forge::credential(&home)),
            Some("tok-made-up"),
            "{mode:o}"
        );
    }
}

/// Any permission to the group or to others is refused, as ssh refuses such
/// a key: the reason names the file, its mode, that others can reach it and
/// the command that fixes it — never what it holds.
#[cfg(unix)]
#[test]
fn a_token_open_to_its_group_or_to_others_is_refused_with_the_fix() {
    for mode in [0o640, 0o604, 0o644, 0o620, 0o602, 0o610, 0o601, 0o666] {
        let (_dir, home) = homes();
        let at = token_with_mode(&home, mode);

        let refused = refused(forge::credential(&home));
        assert_eq!(refused.at, at, "{mode:o}");
        assert_eq!(refused.why, forge::Why::OpenToOthers(mode), "{mode:o}");
        let why = refused.refusal();
        assert!(
            why.contains(&format!("{} has mode {mode:04o}", at.display())),
            "{why}"
        );
        assert!(why.contains("others can read"), "{why}");
        assert!(
            why.contains(&format!("`chmod 600 {}`", at.display())),
            "{why}"
        );
        assert!(why.contains("nothing is pushed"), "{why}");
        assert!(!why.contains("tok-made-up"), "{why}");
    }
}

/// The mode judged is that of the file the read would open: a link's own
/// bits say nothing, its target's decide.
#[cfg(unix)]
#[test]
fn a_linked_token_is_judged_by_the_file_it_lands_on() {
    use std::os::unix::fs::PermissionsExt;
    for (mode, used) in [(0o600, true), (0o644, false)] {
        let (_dir, home) = homes();
        let store = tempfile::tempdir().unwrap();
        let real = store.path().join("token");
        std::fs::write(&real, "tok-made-up\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(mode)).unwrap();
        std::os::unix::fs::symlink(&real, home.join(forge::TOKEN_FILE)).unwrap();

        match forge::credential(&home) {
            forge::Credential::Token(t) if used => assert_eq!(t.secret(), "tok-made-up"),
            forge::Credential::Refused(r) if !used => {
                assert_eq!(r.why, forge::Why::OpenToOthers(mode))
            }
            other => panic!("{mode:o}: {other:?}"),
        }
    }
}

/// A file that is there and cannot be opened or read is refused with the
/// reason — never taken for no file, which would send the human to create
/// one that exists.
#[cfg(unix)]
#[test]
fn a_token_file_that_cannot_be_read_is_refused_and_not_taken_for_none() {
    use std::os::unix::fs::PermissionsExt;
    // Its owner cannot open it.
    let (_dir, home) = homes();
    let at = token_with_mode(&home, 0o000);
    if std::fs::File::open(&at).is_ok() {
        // Running as root: the mode does not stop the open, nothing to prove.
        return;
    }
    let refused = refused(forge::credential(&home));
    assert!(
        matches!(refused.why, forge::Why::Unopenable(_)),
        "{:?}",
        refused.why
    );
    let why = refused.refusal();
    assert!(why.contains("could not be read"), "{why}");
    assert!(
        why.contains(&format!("chmod 600 {}", at.display())),
        "{why}"
    );

    // A directory its owner alone may enter: opened, judged, unreadable.
    let (_dir, home) = homes();
    let at = home.join(forge::TOKEN_FILE);
    std::fs::create_dir(&at).unwrap();
    std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        matches!(refused_why(&home), forge::Why::Unopenable(_)),
        "{:?}",
        refused_why(&home)
    );
}

#[cfg(unix)]
fn refused_why(home: &std::path::Path) -> forge::Why {
    refused(forge::credential(home)).why
}

/// The rule itself, apart from any file: a mode that could not be read is
/// refused with its own reason, never read as allowed; the file-type bits are
/// not permissions; and a platform with no such bits has nothing to judge.
#[test]
fn a_mode_that_cannot_be_read_is_refused_and_only_group_and_other_bits_count() {
    assert_eq!(
        forge::judge(Err("stat failed".into())),
        Err(forge::Why::ModeUnknown("stat failed".into()))
    );
    assert_eq!(forge::judge(Ok(Some(0o100600))), Ok(()));
    assert_eq!(forge::judge(Ok(Some(0o100400))), Ok(()));
    assert_eq!(forge::judge(Ok(Some(0o100700))), Ok(()));
    assert_eq!(
        forge::judge(Ok(Some(0o100640))),
        Err(forge::Why::OpenToOthers(0o640))
    );
    assert_eq!(
        forge::judge(Ok(Some(0o104601))),
        Err(forge::Why::OpenToOthers(0o601)),
        "only the permission bits are said"
    );
    assert_eq!(forge::judge(Ok(None)), Ok(()));

    let unknown = forge::Refused {
        at: "/h/.nunki/p/forge-token".into(),
        why: forge::Why::ModeUnknown("stat failed".into()),
    }
    .refusal();
    assert!(
        unknown.contains("could not be read (stat failed)"),
        "{unknown}"
    );
    assert!(
        unknown.contains("chmod 600 /h/.nunki/p/forge-token"),
        "{unknown}"
    );
    assert!(unknown.contains("nothing is pushed"), "{unknown}");
}

// --- `git push` with the project's token alone (SPEC 4.2, `nunki push`) -----

/// The configuration the push is given, exactly: every helper reset, then
/// one scoped to the forge's address that reads the token from the
/// environment — its name, never its value — no askpass, no redirect, and the
/// forge's protocol alone. Each line is a door the owner's credentials or
/// the token could go through; the push's own tests prove the whole against
/// real git, and this pins each door.
#[test]
fn the_push_is_configured_with_one_helper_scoped_to_the_forge_and_nothing_else() {
    let args = forge::git::config_args("https://github.com/");
    let pairs: Vec<&str> = args
        .chunks(2)
        .map(|c| {
            assert_eq!(c[0], "-c");
            c[1].as_str()
        })
        .collect();
    assert_eq!(
        pairs,
        [
            "credential.helper=",
            "credential.https://github.com/.helper=!f() { test \"$1\" = get || exit 0; \
             printf 'username=x-access-token\\npassword=%s\\n' \"$NUNKI_FORGE_TOKEN\"; }; f",
            "core.askPass=",
            "http.followRedirects=false",
            "protocol.allow=never",
            "protocol.https.allow=always",
        ]
    );
    let local = forge::git::config_args("http://127.0.0.1:4242/");
    assert!(
        local.contains(&"protocol.http.allow=always".to_string()),
        "{local:?}"
    );
    assert!(
        local
            .iter()
            .any(|a| a.starts_with("credential.http://127.0.0.1:4242/.helper=")),
        "{local:?}"
    );
}

/// The push's environment keeps `PATH` and nothing else of what nunki
/// inherited — no `HOME`, no `GIT_*`, no askpass, no SSH, no proxy — then
/// closes the system and global configurations, the prompt, and puts `HOME`
/// in the scratch directory, where curl finds no `.netrc`.
#[test]
fn the_push_environment_keeps_path_alone_and_closes_every_configuration() {
    let inherited: Vec<(std::ffi::OsString, std::ffi::OsString)> = [
        ("PATH", "/usr/bin"),
        ("HOME", "/Users/owner"),
        ("XDG_CONFIG_HOME", "/Users/owner/.config"),
        ("GIT_CONFIG_GLOBAL", "/Users/owner/.gitconfig"),
        ("GIT_CONFIG_SYSTEM", "/opt/homebrew/etc/gitconfig"),
        ("GIT_CONFIG_PARAMETERS", "'credential.helper'='osxkeychain'"),
        ("GIT_ASKPASS", "/bin/askpass"),
        ("SSH_ASKPASS", "/bin/askpass"),
        ("GIT_SSH_COMMAND", "ssh -i key"),
        ("GIT_TERMINAL_PROMPT", "1"),
        ("https_proxy", "http://u:p@proxy"),
        ("NUNKI_FORGE_TOKEN", "tok-inherited"),
    ]
    .iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let home = std::path::Path::new("/tmp/nunki-push-x/home");

    let env = forge::git::environment(&inherited, home);

    let env: Vec<(String, String)> = env
        .into_iter()
        .map(|(k, v)| (k.into_string().unwrap(), v.into_string().unwrap()))
        .collect();
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(
        pairs,
        [
            ("PATH", "/usr/bin"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("HOME", "/tmp/nunki-push-x/home"),
            ("LC_ALL", "C"),
        ]
    );
}

/// The address pushed to is the forge's base and the repository the remote
/// names; a name GitHub would not accept is refused before it reaches a URL,
/// where it could change what the address means.
#[test]
fn the_push_address_is_the_forges_and_a_strange_name_is_refused() {
    let repo = |owner: &str, name: &str| forge::Repo {
        owner: owner.into(),
        name: name.into(),
    };
    assert_eq!(
        forge::git::url("https://github.com/", &repo("o-1", "r_2.x")).unwrap(),
        "https://github.com/o-1/r_2.x.git"
    );
    for (owner, name) in [
        ("o", ""),
        ("", "r"),
        ("o", ".."),
        (".", "r"),
        ("o", "r?x"),
        ("o@evil.example", "r"),
        ("o", "r/s"),
        ("o", "r#x"),
        ("o", "r x"),
    ] {
        let err = forge::git::url("https://github.com/", &repo(owner, name)).unwrap_err();
        assert!(
            err.to_string().contains("is not a GitHub repository name"),
            "{owner}/{name}: {err}"
        );
    }
}

/// What git said is given back on one line, and never with the token in it.
#[test]
fn what_git_said_is_given_back_without_the_token() {
    assert_eq!(
        forge::git::redact("fatal: tok-x refused\n  for tok-x\n", "tok-x"),
        "fatal: [the token] refused for [the token]"
    );
    assert_eq!(forge::git::redact("a\nb", ""), "a b");
}

/// The push command after its configuration: the scratch repository, no
/// hook, and the judged commit by id onto the full ref.
#[test]
fn the_push_command_pushes_the_commit_by_id_to_the_full_ref_without_hooks() {
    let args = forge::git::push_args(
        "https://github.com/",
        std::path::Path::new("/tmp/s/repo.git"),
        "https://github.com/o/r.git",
        "0123abcd",
        "mission/x",
    );
    let config = forge::git::config_args("https://github.com/");
    assert_eq!(&args[..config.len()], &config[..]);
    assert_eq!(
        &args[config.len()..],
        [
            "--git-dir=/tmp/s/repo.git",
            "push",
            "--no-verify",
            "--quiet",
            "https://github.com/o/r.git",
            "0123abcd:refs/heads/mission/x",
        ]
    );
}

fn git_in(at: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(at)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The push borrows the project's objects whatever their format: a sha256
/// repository is pushed from a sha256 scratch, and the commit lands.
#[test]
fn a_sha256_project_is_pushed_from_a_scratch_of_its_own_format() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    git_in(&project, &["init", "-q", "--object-format=sha256"]);
    std::fs::write(project.join("f"), "x\n").unwrap();
    git_in(&project, &["add", "f"]);
    git_in(
        &project,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "one",
        ],
    );
    let commit = git_in(&project, &["rev-parse", "HEAD"]);
    let root = dir.path().join("git");
    std::fs::create_dir_all(root.join("o")).unwrap();
    git_in(
        &root.join("o"),
        &["init", "-q", "--bare", "--object-format=sha256", "r.git"],
    );
    let server = common::git_http::GitServer::start(&root, "tok-made-up");
    let (_home_dir, home) = homes();
    common::write_token(&home.join(forge::TOKEN_FILE), "tok-made-up\n");
    let forge::Credential::Token(token) = forge::credential(&home) else {
        panic!("no token");
    };

    forge::git::push(&forge::git::Push {
        objects_of: &project,
        base: &server.base,
        repo: &forge::Repo {
            owner: "o".into(),
            name: "r".into(),
        },
        commit: &commit,
        branch: "mission/x",
        token: &token,
        inherited: &[("PATH".into(), std::env::var_os("PATH").unwrap_or_default())],
    })
    .unwrap();

    assert_eq!(
        git_in(
            &root.join("o/r.git"),
            &["rev-parse", "refs/heads/mission/x"]
        ),
        commit
    );
}
