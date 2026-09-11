//! `hq mission fetch` and `hq push` (SPEC 4.2, 4.4, 4.5): the one verb that
//! touches the forge in write, and everything it refuses first.

use std::path::{Path, PathBuf};
use std::process::Command;

use hq::harness::Role;
use hq::mission::flow::{Event, Flow, Stage};
use hq::mission::{Bounds, Header, Integration, Lot, Security, Service, Verdict};
use hq::project::{Config, Project, ProtectedPaths};
use hq::push::{self, PushError};
use hq::state::{MissionState, Store};

mod common;
use common::serve;

fn git(at: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["-c", "user.name=Push Test", "-c", "user.email=p@test"])
        .args(args)
        .output()
        .expect("git is on the path");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        at.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn header(integration: Integration, security: Security) -> Header {
    Header {
        branch: "mission/x".into(),
        base: "dev".into(),
        lots: vec![Lot {
            id: "L1".into(),
            title: "one".into(),
        }],
        integration,
        security,
        arbiter: None,
        run: None,
        account: None,
        bounds: Bounds::default(),
    }
}

fn with_wiring() -> Integration {
    Integration::Services {
        services: vec![Service {
            name: "db".into(),
            reach: vec!["db".into()],
        }],
        wiring: vec!["compose.yaml".into(), "tests/system/**".into()],
    }
}

struct World {
    _dir: tempfile::TempDir,
    project: Project,
    tree: PathBuf,
    /// A bare repository standing in for the forge.
    forge: PathBuf,
}

impl World {
    fn new(integration: Integration, security: Security) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let hq_root = dir.path().join("hq");
        for d in ["locks", "missions", "state/missions"] {
            std::fs::create_dir_all(hq_root.join(d)).unwrap();
        }

        // The forge: a bare repository, so `git push` is a real push.
        let forge = dir.path().join("forge.git");
        git(
            dir.path(),
            &["init", "--bare", "-q", "-b", "dev", "forge.git"],
        );

        // The repository, with the forge as its origin.
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "dev"]);
        std::fs::write(root.join("src.rs"), "pub fn one() -> u8 { 1 }\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "base"]);
        git(
            &root,
            &["remote", "add", "origin", &forge.display().to_string()],
        );
        git(&root, &["push", "-q", "origin", "dev"]);

        // The slot, where `hq::slot::find` looks: beside the repository.
        let slots = dir.path().join("repo-slots");
        std::fs::create_dir_all(&slots).unwrap();
        let tree = slots.join("one");
        Command::new("git")
            .args(["clone", "-q", "--no-hardlinks"])
            .arg(&root)
            .arg(&tree)
            .status()
            .unwrap();
        git(&tree, &["checkout", "-q", "-b", "mission/x"]);

        let project = Project::at(
            root,
            Config {
                harness: "claude-code".into(),
                forge: vec!["github.com".into()],
                stacks: vec!["rust".into()],
                protected_branches: vec!["main".into(), "dev".into()],
                protected_paths: ProtectedPaths::default(),
                account: None,
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: None,
                forge_protection: Default::default(),
            },
            hq_root.clone(),
        );
        let header = header(integration, security);
        hq::mission::dir::create(&hq_root, "m1", &header, "do it").unwrap();
        Store::open(&hq_root)
            .unwrap()
            .save(&MissionState {
                id: "m1".into(),
                slot: "one".into(),
                flow: Flow::new(header).unwrap(),
                run: None,
                app: None,
                verdicts: Vec::new(),
                accepted: Vec::new(),
                stopped: None,
                harness_down: None,
                spent: Default::default(),
                updated_at: String::new(),
            })
            .unwrap();

        Self {
            _dir: dir,
            project,
            tree,
            forge,
        }
    }

    fn commit(&self, path: &str, body: &str, message: &str) -> String {
        let full = self.tree.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
        git(&self.tree, &["add", "-A"]);
        git(&self.tree, &["commit", "-q", "-m", message]);
        self.head()
    }

    fn head(&self) -> String {
        git(&self.tree, &["rev-parse", "HEAD"])
    }

    fn store(&self) -> Store {
        Store::open(&self.project.hq_root).unwrap()
    }

    fn state(&self) -> MissionState {
        self.store().load("m1").unwrap()
    }

    /// Put the flow at `Verified` and record the verdicts a green run would
    /// have left, without lifting a container.
    fn verified(&self, verdicts: &[(Role, Option<Verdict>, String)]) {
        let store = self.store();
        let mut state = store.load("m1").unwrap();
        let mut events = vec![
            Event::RunEnded {
                outcome: hq::harness::Outcome::Finished(Default::default()),
                lot_done: true,
            },
            Event::GatesPassed,
        ];
        if state.flow.header().has_integration() {
            events.push(Event::Verdict {
                verdict: Verdict::Integrated,
                report: "wired".into(),
            });
        }
        if state.flow.header().has_security_agent() {
            events.push(Event::Verdict {
                verdict: Verdict::Clear,
                report: "nothing".into(),
            });
        }
        for event in events {
            store.apply(&mut state, event).unwrap();
        }
        assert!(
            matches!(state.flow.stage(), Stage::Verified),
            "{:?}",
            state.flow.stage()
        );
        for (role, verdict, head) in verdicts {
            state.conclude(*role, *verdict, head);
        }
        store.save(&state).unwrap();
    }

    fn on_forge(&self, branch: &str) -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.forge)
            .args(["rev-parse", "--verify", "--quiet", branch])
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

/// Pushing is the one thing that is not autonomous. No flag, no dialogue: an
/// argument, from somebody who has read the pull request.
#[test]
fn a_push_without_the_humans_word_does_not_happen() {
    let world = World::new(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    let head = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    world.verified(&[(Role::Coder, None, head)]);

    let err = push::push(&world.project, "m1", false).unwrap_err();
    assert!(matches!(err, PushError::NotAuthorised(_)), "{err}");
    assert!(world.on_forge("mission/x").is_none(), "nothing was pushed");
}

/// The happy path, with a real remote: the branch lands on the forge, and the
/// human is handed the address to open the pull request at.
#[test]
fn a_verified_mission_reaches_the_forge_and_the_pull_request_is_handed_over() {
    let world = World::new(with_wiring(), Security::Agent);
    let coder = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    let head = world.commit("compose.yaml", "services: {}\n", "wire it");
    world.verified(&[
        (Role::Coder, None, coder),
        (Role::Integrator, Some(Verdict::Integrated), head.clone()),
        (Role::Security, Some(Verdict::Clear), head.clone()),
    ]);

    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(pushed.branch, "mission/x");
    assert_eq!(pushed.head, head);
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
    // And the repository carries the branch too: commits leave a slot one
    // way, into the repository, and it is from there that the push happens.
    assert_eq!(git(&world.project.root, &["rev-parse", "mission/x"]), head);
}

/// A mission that is not verified is not pushed, and the message says where
/// it actually is.
#[test]
fn a_mission_that_is_not_verified_is_not_pushed() {
    let world = World::new(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(matches!(err, PushError::NotVerified { .. }), "{err}");
    assert!(err.to_string().contains("Coding"), "{err}");
    assert!(world.on_forge("mission/x").is_none());
}

/// A verdict is worth one commit. One given on another is not an opinion
/// about this one.
#[test]
fn a_verdict_on_another_commit_does_not_authorise_this_one() {
    let world = World::new(with_wiring(), Security::Gates);
    let coder = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    let head = world.commit("compose.yaml", "services: {}\n", "wire it");
    world.verified(&[
        (Role::Coder, None, coder.clone()),
        // The integrator concluded before the wiring commit.
        (Role::Integrator, Some(Verdict::Integrated), coder),
    ]);

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(
        matches!(
            err,
            PushError::VerdictElsewhere {
                role: Role::Integrator,
                ..
            }
        ),
        "{err}"
    );
    assert!(err.to_string().contains(&head[..12]), "{err}");
    assert!(world.on_forge("mission/x").is_none());
}

/// "Rien ne se pousse en rouge." A `FINDINGS` the human never lifted is a
/// red verdict, and there is no flag for it.
#[test]
fn findings_nobody_lifted_do_not_reach_the_forge() {
    let world = World::new(
        Integration::None {
            reason: "none".into(),
        },
        Security::Agent,
    );
    let head = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    world.verified(&[
        (Role::Coder, None, head.clone()),
        (Role::Security, Some(Verdict::Findings), head.clone()),
    ]);

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(matches!(err, PushError::NotLifted(_)), "{err}");
    assert!(err.to_string().contains("hq mission accept"), "{err}");

    // Lifted on this commit, by a human, with a reason — and it goes.
    let mut state = world.state();
    state.accepted.push(hq::state::Accepted {
        finding: None,
        why: "behind the VPN".into(),
        who: "Arnaud".into(),
        head: head.clone(),
        date: "2026-09-10T00:00:00Z".into(),
    });
    world.store().save(&state).unwrap();
    push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
}

/// The rule of SPEC 4.4: the coder's verdict survives the integrator's
/// commits **as long as they are wiring**. A commit after the coder's that
/// touches business code invalidates it.
#[test]
fn a_commit_after_the_coder_that_is_not_wiring_invalidates_its_verdict() {
    let world = World::new(with_wiring(), Security::Gates);
    let coder = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    let head = world.commit("src.rs", "pub fn one() -> u8 { 3 }\n", "and a bit more");
    world.verified(&[
        (Role::Coder, None, coder.clone()),
        (Role::Integrator, Some(Verdict::Integrated), head.clone()),
    ]);

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(matches!(err, PushError::NotOnlyWiring { .. }), "{err}");
    assert!(err.to_string().contains("src.rs"), "{err}");
    assert!(world.on_forge("mission/x").is_none());
}

/// And wiring that is wiring passes, including under a glob.
#[test]
fn wiring_declared_by_the_mission_keeps_the_coders_verdict_valid() {
    let world = World::new(with_wiring(), Security::Gates);
    let coder = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    world.commit("compose.yaml", "services: {}\n", "wire it");
    let head = world.commit("tests/system/smoke.rs", "// end to end\n", "a system test");
    world.verified(&[
        (Role::Coder, None, coder),
        (Role::Integrator, Some(Verdict::Integrated), head.clone()),
    ]);

    push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
}

/// A mission with no integrator gave nobody the right to commit after the
/// coder, so anything there invalidates the verdict — the allowlist is empty
/// rather than absent.
#[test]
fn without_an_integrator_nothing_may_follow_the_coders_commit() {
    let world = World::new(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    let coder = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    world.commit("compose.yaml", "services: {}\n", "who wrote this?");
    world.verified(&[(Role::Coder, None, coder)]);

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(matches!(err, PushError::NotOnlyWiring { .. }), "{err}");
    assert!(world.on_forge("mission/x").is_none());
}

/// `hq mission fetch` brings the commits over, one way, and says what moved.
#[test]
fn fetching_brings_the_slots_commits_into_the_repository() {
    let world = World::new(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    let head = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");

    let first = push::fetch(&world.project, "m1").unwrap();
    assert_eq!(first.head, head);
    assert_eq!(first.was, None, "the branch was not there before");
    assert_eq!(git(&world.project.root, &["rev-parse", "mission/x"]), head);

    let next = world.commit("src.rs", "pub fn one() -> u8 { 3 }\n", "again");
    let second = push::fetch(&world.project, "m1").unwrap();
    assert_eq!(second.was.as_deref(), Some(head.as_str()));
    assert_eq!(second.head, next);
}

/// Git refuses to fetch into the branch that is checked out, and says so in a
/// message about refs. `hq` says it about the repository instead.
#[test]
fn fetching_into_the_checked_out_branch_is_named_rather_than_left_to_git() {
    let world = World::new(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    git(&world.project.root, &["checkout", "-q", "-b", "mission/x"]);

    let err = push::fetch(&world.project, "m1").unwrap_err();
    assert!(matches!(err, PushError::FetchingIntoCurrent(_)), "{err}");
}

/// The address a human opens the pull request at, worked out from the
/// remote's own URL — both shapes git writes.
#[test]
fn the_pull_request_address_is_worked_out_from_the_remote() {
    for remote in [
        "https://github.com/ArnaudSene/nunki.git",
        "git@github.com:ArnaudSene/nunki.git",
        "https://github.com/ArnaudSene/nunki",
    ] {
        assert_eq!(
            push::pull_request_url(remote, "dev", "mission/x").as_deref(),
            Some("https://github.com/ArnaudSene/nunki/compare/dev...mission/x?expand=1"),
            "{remote}"
        );
    }
    // A forge hq does not know how to address: said as unknown rather than
    // guessed into a URL that goes nowhere.
    assert_eq!(
        push::pull_request_url("git@gitlab.com:team/thing.git", "dev", "x"),
        None
    );
}

/// A volet replays the whole chain, so a role concludes more than once. The
/// earlier answer is not a second opinion, it is a stale one — and keeping
/// both would let `hq push` find the green it wants among answers about other
/// commits.
#[test]
fn a_role_that_concludes_twice_leaves_one_answer_and_it_is_the_last() {
    let world = World::new(with_wiring(), Security::Gates);
    let first = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    world.verified(&[
        (Role::Coder, None, first.clone()),
        // Green, on the commit that was judged then.
        (Role::Integrator, Some(Verdict::Integrated), first.clone()),
    ]);

    // The coder went back out, and the integrator has not replayed yet.
    let head = world.commit("src.rs", "pub fn one() -> u8 { 3 }\n", "a volet");
    let mut state = world.state();
    state.conclude(Role::Coder, None, &head);
    world.store().save(&state).unwrap();

    assert_eq!(
        state
            .verdicts
            .iter()
            .filter(|c| c.role == Role::Integrator)
            .count(),
        1,
        "one answer per role: {:?}",
        state.verdicts
    );
    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(
        matches!(
            err,
            PushError::VerdictElsewhere {
                role: Role::Integrator,
                ..
            }
        ),
        "the stale INTEGRATED must not authorise the new commit: {err}"
    );

    // And when it does replay, the new answer replaces the old one entirely.
    let mut state = world.state();
    state.conclude(Role::Integrator, Some(Verdict::Integrated), &head);
    world.store().save(&state).unwrap();
    assert_eq!(
        world
            .state()
            .verdicts
            .iter()
            .filter(|c| c.role == Role::Integrator)
            .count(),
        1
    );
    push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
}

// --- the pull request, opened on the forge (SPEC 4.2) -----------------------

impl World {
    /// The shape of a real project: `origin` reads as a GitHub repository, and
    /// pushes go to the bare repository standing in for it — so `hq` works
    /// out the forge from the remote exactly as it would, and the push is
    /// still a real push.
    fn on_github(&self) {
        git(
            &self.project.root,
            &["remote", "set-url", "origin", "https://github.com/o/r.git"],
        );
        git(
            &self.project.root,
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                &self.forge.display().to_string(),
            ],
        );
    }

    fn with_token(&self) {
        std::fs::write(
            self.project.hq_root.join(hq::forge::TOKEN_FILE),
            "tok-human\n",
        )
        .unwrap();
    }

    fn pr_says(&self, text: &str) {
        std::fs::write(self.project.hq_root.join("missions/m1/PR.md"), text).unwrap();
    }

    fn ready(&self) -> String {
        let coder = self.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
        let head = self.commit("compose.yaml", "services: {}\n", "wire it");
        self.verified(&[
            (Role::Coder, None, coder),
            (Role::Integrator, Some(Verdict::Integrated), head.clone()),
            (Role::Security, Some(Verdict::Clear), head.clone()),
        ]);
        head
    }
}

/// The whole verb: pushed, then the pull request opened with the human's
/// token and titled with what the mission wrote in `PR.md`.
#[test]
fn a_verified_mission_is_pushed_and_its_pull_request_opened() {
    let world = World::new(with_wiring(), Security::Agent);
    world.on_github();
    world.with_token();
    world.pr_says("# feat: one returns two\n\nBecause it had to.\n");
    let head = world.ready();

    let (api, server) = serve(vec![(
        201,
        r#"{"html_url":"https://github.com/o/r/pull/9"}"#,
    )]);
    let pushed = push::push_to(&world.project, "m1", true, &api).unwrap();

    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
    assert_eq!(
        pushed.pull_request,
        push::PullRequestState::Opened(hq::forge::Opened::Created(
            "https://github.com/o/r/pull/9".into()
        ))
    );
    let sent = &server.join().unwrap()[0];
    assert!(sent.starts_with("POST /repos/o/r/pulls "), "{sent}");
    assert!(sent.contains("Bearer tok-human"), "{sent}");
    // Read as JSON, not as a string: how the client lays the body out is
    // its business, what it says is the test's.
    let body: serde_json::Value = serde_json::from_str(sent.split("\r\n\r\n").nth(1).unwrap())
        .unwrap_or_else(|e| panic!("{e}: {sent}"));
    assert_eq!(body["title"], "feat: one returns two", "{sent}");
    assert_eq!(body["body"], "Because it had to.", "{sent}");
    assert_eq!(body["head"], "mission/x");
    assert_eq!(body["base"], "dev");
}

/// No credential at the HQ: the push still happens — it is what `--yes`
/// authorised — and the human is handed the exact address, and told where
/// the credential would go.
#[test]
fn without_a_credential_the_push_happens_and_the_address_is_handed_over() {
    let world = World::new(with_wiring(), Security::Agent);
    world.on_github();
    world.pr_says("# feat: one returns two\n");
    let head = world.ready();

    let pushed = push::push_to(&world.project, "m1", true, "http://127.0.0.1:9").unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
    match pushed.pull_request {
        push::PullRequestState::ByHand { compare, why } => {
            assert_eq!(
                compare.as_deref(),
                Some("https://github.com/o/r/compare/dev...mission/x?expand=1")
            );
            assert!(why.contains(hq::forge::TOKEN_FILE), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

/// The forge refusing the pull request does not undo the push, and must not
/// read as if it had: a push reported red is retried, and the second push is
/// a no-op that hides the first.
#[test]
fn a_forge_that_refuses_the_pull_request_leaves_the_push_standing() {
    let world = World::new(with_wiring(), Security::Agent);
    world.on_github();
    world.with_token();
    world.pr_says("# feat: one returns two\n");
    let head = world.ready();

    let (api, server) = serve(vec![(
        403,
        r#"{"message":"Resource not accessible by personal access token"}"#,
    )]);
    let pushed = push::push_to(&world.project, "m1", true, &api).unwrap();
    server.join().unwrap();

    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
    match pushed.pull_request {
        push::PullRequestState::ByHand { compare, why } => {
            assert!(compare.is_some());
            assert!(why.contains("403"), "{why}");
            assert!(
                why.contains("Resource not accessible"),
                "the forge's own reason: {why}"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// A remote that is not on GitHub is said to be elsewhere. The forge is
/// never asked — the server here would fail the test if it were.
#[test]
fn a_remote_elsewhere_is_named_and_no_forge_is_asked() {
    let world = World::new(with_wiring(), Security::Agent);
    world.with_token();
    world.pr_says("# feat: one returns two\n");
    world.ready();

    let pushed = push::push_to(&world.project, "m1", true, "http://127.0.0.1:9").unwrap();
    match pushed.pull_request {
        push::PullRequestState::ByHand { compare, why } => {
            assert_eq!(compare, None, "no GitHub address to work out");
            assert!(why.contains("not on GitHub"), "{why}");
        }
        other => panic!("{other:?}"),
    }
}
