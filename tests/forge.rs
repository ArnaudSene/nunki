//! Opening the pull request (SPEC 4.2, `hq push`): the real HTTP client,
//! pointed at a server this test controls.
//!
//! A fake forge would prove the fake. What a forge can get wrong silently is
//! the shape of the request — a missing `Authorization` header gets a 404
//! from GitHub, not a 401, and reads like a repository that does not exist —
//! so the request is read byte by byte, as a server receives it.

use std::net::TcpListener;

mod common;
use common::serve;

use hq::forge::{self, ForgeError, Opened, PullRequest, Repo};

fn repo() -> Repo {
    Repo {
        owner: "o".into(),
        name: "r".into(),
    }
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
    let opened = forge::open(&api, "tok-123", &repo(), &request()).unwrap();
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
        header(sent, "user-agent").is_some_and(|v| v.starts_with("hq/")),
        "GitHub refuses a request with no User-Agent: {sent}"
    );
    assert_eq!(
        header(sent, "content-type"),
        Some("application/json"),
        "the body is serialised by hq, so the header is hq's to send: {sent}"
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
    let opened = forge::open(&api, "tok", &repo(), &request()).unwrap();
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
    let err = forge::open(&api, "wrong", &repo(), &request()).unwrap_err();
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
    let err = forge::open(&api, "tok", &repo(), &request()).unwrap_err();
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
    let err = forge::open(&api, "tok", &repo(), &request()).unwrap_err();
    assert!(matches!(err, ForgeError::Unreachable(_)), "{err:?}");
}

#[test]
fn a_github_remote_names_its_repository_and_any_other_names_none() {
    for remote in [
        "git@github.com:ArnaudSene/nunki.git",
        "https://github.com/ArnaudSene/nunki.git",
        "https://github.com/ArnaudSene/nunki",
        "ssh://git@github.com/ArnaudSene/nunki.git",
    ] {
        assert_eq!(
            Repo::of_remote(remote),
            Some(Repo {
                owner: "ArnaudSene".into(),
                name: "nunki".into()
            }),
            "{remote}"
        );
    }
    for remote in [
        "git@gitlab.com:team/thing.git",
        "/tmp/bare.git",
        "https://github.com/only-an-owner",
        "https://github.com/a/b/c",
    ] {
        assert_eq!(Repo::of_remote(remote), None, "{remote}");
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

#[test]
fn a_token_is_read_trimmed_and_an_empty_file_is_no_token() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(forge::token(dir.path()), None);
    std::fs::write(dir.path().join(forge::TOKEN_FILE), "  \n").unwrap();
    assert_eq!(forge::token(dir.path()), None);
    std::fs::write(dir.path().join(forge::TOKEN_FILE), "ghp_abc\n").unwrap();
    assert_eq!(forge::token(dir.path()).as_deref(), Some("ghp_abc"));
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
    let repo = Repo::of_remote("https://github.com/ArnaudSene/nunki").unwrap();
    forge::can_read(forge::API, token.trim(), &repo).expect("the token reads the repository");

    let err = forge::can_read(forge::API, "not-a-token", &repo).unwrap_err();
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
    let repo = Repo::of_remote("https://github.com/ArnaudSene/nunki").unwrap();
    let dev = forge::protection(forge::API, token.trim(), &repo, "dev").expect("dev is readable");
    eprintln!("dev on the forge: {dev:?}");
    assert!(matches!(
        dev,
        forge::Protection::Protected | forge::Protection::Unprotected
    ));
    assert_eq!(
        forge::protection(forge::API, token.trim(), &repo, "no-such-branch-hq-check").unwrap(),
        forge::Protection::NoSuchBranch
    );
}
