//! `nunki mission fetch` and `nunki push` (SPEC 4.2, 4.4, 4.5): the one verb that
//! touches the forge in write, and everything it refuses first.

use std::path::{Path, PathBuf};
use std::process::Command;

use nunki::harness::Role;
use nunki::mission::flow::{Event, Flow, Stage};
use nunki::mission::{Bounds, Header, Integration, Lot, Security, Service, Verdict};
use nunki::project::{Config, Project, ProtectedPaths};
use nunki::push::{self, PushError};
use nunki::state::{MissionState, Store};

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
        rigor: Default::default(),
        mutation_threshold: None,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Bounds::default(),
    }
}

fn with_wiring() -> Integration {
    Integration::Services {
        services: vec![Service {
            name: "db".into(),
            reach: vec!["db".into()],
            shared: false,
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
    /// The user home the real binary is run with, for a world [`World::opened`].
    home: PathBuf,
}

impl World {
    fn new(integration: Integration, security: Security) -> Self {
        Self::built(integration, security, false)
    }

    /// The same world, in a project home the real binary opens with `HOME`
    /// set to [`World::home`]: for what `nunki push` prints, which nothing
    /// below the binary can see.
    fn opened(integration: Integration, security: Security) -> Self {
        Self::built(integration, security, true)
    }

    fn built(integration: Integration, security: Security, opened: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let nunki_home = if opened {
            common::project_home(&root, &dir.path().join("home"), "harness: claude-code\n")
        } else {
            std::fs::create_dir_all(&root).unwrap();
            git(&root, &["init", "-q", "-b", "dev"]);
            dir.path().join("nunki")
        };
        let root = std::fs::canonicalize(&root).unwrap();
        let hq_root = nunki_home.join(nunki::project::HQ_DIR);
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
        std::fs::write(root.join("src.rs"), "pub fn one() -> u8 { 1 }\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "base"]);
        git(
            &root,
            &["remote", "add", "origin", &forge.display().to_string()],
        );
        git(&root, &["push", "-q", "origin", "HEAD"]);

        // The slot, where `nunki::slot::find` looks: beside the repository.
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
                root: None,
                harness: "claude-code".into(),
                forge: vec!["github.com".into()],
                stacks: vec!["rust".into()],
                protected_branches: vec!["main".into(), "dev".into()],
                protected_paths: ProtectedPaths::default(),
                account: None,
                model: None,
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: None,
                permission_mode: "auto".to_string(),
                rigor: None,
                mutation_threshold: 80,
                forge_protection: Default::default(),
            },
            hq_root.parent().unwrap().to_path_buf(),
        );
        let header = header(integration, security);
        nunki::mission::dir::create(&hq_root, "m1", &header, "do it").unwrap();
        // The campaign a green gate 7 leaves behind, with nothing surviving:
        // push fails closed on a mission whose rigor owes one and that has
        // none, so every world starts where a verified mission stands.
        nunki::mutants::write(
            &nunki::mission::dir::Paths::of(&hq_root, "m1").dir,
            &nunki::mutants::Campaign {
                fingerprint: "f".into(),
                head: String::new(),
                date: "2026-10-06T12:00:00Z".into(),
                survivors: Vec::new(),
                tried: Some(0),
            },
        )
        .unwrap();
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
                spared: None,
                coder_session: None,
                pushed: None,
                updated_at: String::new(),
                revision: 0,
            })
            .unwrap();

        Self {
            home: dir.path().join("home"),
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
                outcome: nunki::harness::Outcome::Finished(Default::default()),
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
    assert!(err.to_string().contains("nunki mission accept"), "{err}");

    // Lifted on this commit, by a human, with a reason — and it goes.
    let mut state = world.state();
    state.accepted.push(nunki::state::Accepted {
        finding: None,
        why: "behind the VPN".into(),
        who: "Alex Martin".into(),
        head: head.clone(),
        date: "2026-09-10T00:00:00Z".into(),
        by_nunki: false,
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

/// `nunki mission fetch` brings the commits over, one way, and says what moved.
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
/// message about refs. `nunki` says it about the repository instead.
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

/// A volet replays the whole chain, so a role concludes more than once. The
/// earlier answer is not a second opinion, it is a stale one — and keeping
/// both would let `nunki push` find the green it wants among answers about other
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
    /// pushes go to the bare repository standing in for it — so `nunki` works
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
        let at = self.project.nunki_home();
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(at.join(nunki::forge::TOKEN_FILE), "tok-human\n").unwrap();
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
    // The state says so, on the commit pushed: `mission wait` reads it there.
    let state = world.store().load("m1").unwrap();
    assert_eq!(
        state.pushed.as_ref().map(|p| p.head.as_str()),
        Some(head.as_str())
    );
    assert_eq!(
        pushed.pull_request,
        push::PullRequestState::Opened(nunki::forge::Opened::Created(
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
            assert!(why.contains(nunki::forge::TOKEN_FILE), "{why}");
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

/// A remote on a forge nunki has no adapter for is said to be one, and named.
/// No forge is asked — the server here would fail the test if one were.
#[test]
fn a_remote_elsewhere_is_named_and_no_forge_is_asked() {
    let world = World::new(with_wiring(), Security::Agent);
    world.with_token();
    world.pr_says("# feat: one returns two\n");
    world.ready();

    let pushed = push::push_to(&world.project, "m1", true, "http://127.0.0.1:9").unwrap();
    match pushed.pull_request {
        push::PullRequestState::ByHand { compare, why } => {
            assert_eq!(compare, None, "no adapter, so no address to work out");
            assert!(why.contains("no adapter for"), "{why}");
            assert!(
                why.contains("forge.git"),
                "the remote itself is named: {why}"
            );
        }
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Push once the security rounds are spent (SPEC 4.5).
//
// The flow is driven the way `nunki verify` drives it — an event, and the
// verdict recorded on the commit it was given on — without lifting a
// container. Dates are written by hand, so the order of a verdict and a lift
// is the test's and not the clock's.
// ---------------------------------------------------------------------------

const T1: &str = "2026-10-01T10:00:00Z";
const T2: &str = "2026-10-01T11:00:00Z";
const T3: &str = "2026-10-01T12:00:00Z";

fn no_integration() -> Integration {
    Integration::None {
        reason: "none".into(),
    }
}

impl World {
    /// A mission with a security agent and no integrator, at `rigor`.
    fn at(rigor: nunki::mission::Rigor) -> Self {
        Self::rigged(World::new(no_integration(), Security::Agent), rigor)
    }

    /// [`World::at`], in a project home the real binary opens.
    fn opened_at(rigor: nunki::mission::Rigor) -> Self {
        Self::rigged(World::opened(no_integration(), Security::Agent), rigor)
    }

    fn rigged(world: World, rigor: nunki::mission::Rigor) -> Self {
        let mut h = header(no_integration(), Security::Agent);
        h.rigor = rigor;
        let mut state = world.state();
        state.flow = Flow::new(h).unwrap();
        world.store().save(&state).unwrap();
        world
    }

    /// `nunki push m1 --yes`, run by the real binary: what it printed, once
    /// it succeeded.
    fn pushed_by_the_binary(&self) -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_nunki"))
            .arg("-C")
            .arg(&self.project.root)
            .args(["push", "m1", "--yes"])
            .env("HOME", &self.home)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        stdout
    }

    fn event(&self, event: Event) {
        let mut state = self.state();
        self.store().apply(&mut state, event).unwrap();
    }

    fn stage(&self) -> Stage {
        self.state().flow.stage().clone()
    }

    /// The coder's run ends on a commit, and its gates pass on it.
    fn coded(&self, body: &str, message: &str) -> String {
        let head = self.commit("src.rs", body, message);
        self.event(Event::RunEnded {
            outcome: nunki::harness::Outcome::Finished(Default::default()),
            lot_done: true,
        });
        let mut state = self.state();
        state.conclude(Role::Coder, None, &head);
        self.store().save(&state).unwrap();
        self.event(Event::GatesPassed);
        head
    }

    /// The security agent concludes `verdict` on `HEAD`, at `date`.
    fn security(&self, verdict: Verdict, date: &str) {
        let mut state = self.state();
        assert!(
            matches!(state.flow.stage(), Stage::SecurityAgent { .. }),
            "a round is played: {:?}",
            state.flow.stage()
        );
        state.conclude(Role::Security, Some(verdict), &self.head());
        state.verdicts.last_mut().unwrap().date = date.into();
        self.store().save(&state).unwrap();
        self.event(Event::Verdict {
            verdict,
            report: format!("{verdict:?}"),
        });
    }

    /// A human's lift of the verdict as a whole, on `head`, at `date`, as
    /// `nunki mission accept` records it; the flow moves when it is on the
    /// findings.
    fn accepted(&self, head: &str, date: &str) {
        self.lifted(None, head, date);
    }

    fn lifted(&self, finding: Option<&str>, head: &str, date: &str) {
        let mut state = self.state();
        state.accepted.push(nunki::state::Accepted {
            finding: finding.map(str::to_string),
            why: "behind the VPN".into(),
            who: "Alex Martin".into(),
            head: head.to_string(),
            date: date.to_string(),
            by_nunki: false,
        });
        self.store().save(&state).unwrap();
        if matches!(self.stage(), Stage::Findings { .. }) && finding.is_none() {
            self.event(Event::HumanAccepted);
        }
    }

    fn review(&self) {
        self.event(Event::Reviewed {
            because: "rename it".into(),
        });
    }
}

/// How `nunki push` names a commit it did not see attacked.
fn named(head: &str, subject: &str) -> String {
    format!("{} {subject}", &head[..12])
}

/// At `standard`, a CLEAR spends the one round. The HQ reads the branch and
/// sends it back; the volet's gates are green and the mission is verified
/// without a second round. Push takes the CLEAR for the commits after it —
/// no verdict could ever come on them — and names them as not attacked.
#[test]
fn at_standard_a_clear_a_review_and_a_volet_are_pushed_naming_the_commit_not_attacked() {
    let world = World::at(nunki::mission::Rigor::Standard);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Clear, T1);
    assert_eq!(world.stage(), Stage::Verified);
    world.review();
    let first = world.commit("notes.rs", "// first\n", "the volet begins");
    let volet = world.coded("pub fn one() -> u8 { 3 }\n", "the volet ends");
    assert_eq!(world.stage(), Stage::Verified, "no second round");

    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(pushed.head, volet);
    assert_eq!(
        pushed.not_attacked,
        vec![
            named(&first, "the volet begins"),
            named(&volet, "the volet ends")
        ],
        "every commit after the round, oldest first"
    );
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(volet.as_str()));
}

/// At `standard`, FINDINGS spends the round. The HQ iterates, the fix is
/// gated green, and the mission is back on the findings — not pushed. The
/// human lifts them on the fix, and push takes the lift for the commits
/// after the round. A lift given in the same second as the verdict counts:
/// a human cannot be faster than the clock's resolution.
#[test]
fn at_standard_findings_a_fix_and_an_accept_are_pushed_naming_the_fix() {
    let world = World::at(nunki::mission::Rigor::Standard);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.event(Event::Iterate);
    let fix = world.coded("pub fn one() -> u8 { 3 }\n", "the fix");
    assert!(matches!(world.stage(), Stage::Findings { .. }));
    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(matches!(err, PushError::NotVerified { .. }), "{err}");

    world.accepted(&fix, T1);
    assert_eq!(world.stage(), Stage::Verified);
    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(pushed.not_attacked, vec![named(&fix, "the fix")]);
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(fix.as_str()));
}

/// After an accept at the cap, an HQ review and a volet: the old report does
/// not come back, and push still finds the lift, which now stands for two
/// commits.
#[test]
fn after_an_accept_at_the_cap_a_review_and_a_volet_are_pushed() {
    let world = World::at(nunki::mission::Rigor::Standard);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.accepted(&lot, T2);
    world.review();
    let volet = world.coded("pub fn one() -> u8 { 3 }\n", "the volet");
    assert_eq!(
        world.stage(),
        Stage::Verified,
        "the lifted report stays lifted"
    );

    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(pushed.not_attacked, vec![named(&volet, "the volet")]);
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(volet.as_str()));
}

/// The same at `critical`, once its third round is played: a CLEAR on the
/// third, a review, a volet.
#[test]
fn at_critical_a_clear_on_the_third_round_stands_for_the_volet_after_it() {
    let world = World::at(nunki::mission::Rigor::Critical);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.event(Event::Iterate);
    world.coded("pub fn one() -> u8 { 3 }\n", "fix one");
    world.security(Verdict::Findings, T2);
    world.event(Event::Iterate);
    world.coded("pub fn one() -> u8 { 4 }\n", "fix two");
    world.security(Verdict::Clear, T3);
    assert_eq!(world.state().flow.security_rounds(), 3);
    world.review();
    let volet = world.coded("pub fn one() -> u8 { 5 }\n", "the volet");
    assert_eq!(world.stage(), Stage::Verified, "no fourth round");

    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(pushed.not_attacked, vec![named(&volet, "the volet")]);
}

/// And FINDINGS on the third round at `critical`: a fix, an accept, and the
/// fix is pushed, named.
#[test]
fn at_critical_findings_on_the_third_round_a_fix_and_an_accept_are_pushed() {
    let world = World::at(nunki::mission::Rigor::Critical);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    for (round, date) in [(1, T1), (2, T2)] {
        world.security(Verdict::Findings, date);
        world.event(Event::Iterate);
        world.coded(
            &format!("pub fn one() -> u8 {{ {} }}\n", round + 2),
            "a fix",
        );
    }
    world.security(Verdict::Findings, T3);
    world.event(Event::Iterate);
    let fix = world.coded("pub fn one() -> u8 { 9 }\n", "the last fix");
    assert!(matches!(world.stage(), Stage::Findings { .. }));
    world.accepted(&fix, T3);

    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(pushed.not_attacked, vec![named(&fix, "the last fix")]);
}

/// While a round is left, nothing changed: the verdict must be on `HEAD`.
/// At `critical`, one CLEAR leaves two rounds, and a CLEAR on an older
/// commit does not stand for a newer one.
#[test]
fn rounds_not_spent_push_still_refuses_a_verdict_on_an_older_commit() {
    let world = World::at(nunki::mission::Rigor::Critical);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Clear, T1);
    assert_eq!(world.state().flow.security_rounds(), 1);
    let later = world.commit("src.rs", "pub fn one() -> u8 { 3 }\n", "unseen");
    let mut state = world.state();
    state.conclude(Role::Coder, None, &later);
    world.store().save(&state).unwrap();

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(
        matches!(
            &err,
            PushError::VerdictElsewhere { role: Role::Security, on, .. } if *on == lot
        ),
        "{err}"
    );
    assert!(world.on_forge("mission/x").is_none());
}

/// Rounds not spent, the push on the verdict's own commit is unchanged, and
/// names nothing.
#[test]
fn rounds_not_spent_a_clear_on_head_names_no_commit() {
    let world = World::at(nunki::mission::Rigor::Critical);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Clear, T1);
    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert!(pushed.not_attacked.is_empty(), "{:?}", pushed.not_attacked);
}

/// A FINDINGS at the cap that nobody lifted is still refused, whatever the
/// flow says. Forged here — the flow lets a mission reach `Verified` there
/// only through a lift — because push is the last check, and it is worth
/// what it refuses on its own.
#[test]
fn at_the_cap_findings_never_lifted_are_still_refused() {
    let world = World::at(nunki::mission::Rigor::Standard);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.event(Event::Iterate);
    world.coded("pub fn one() -> u8 { 3 }\n", "the fix");
    world.event(Event::HumanAccepted);
    assert_eq!(world.stage(), Stage::Verified);

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(matches!(err, PushError::NotLifted(_)), "{err}");
    assert!(world.on_forge("mission/x").is_none());
}

/// A lift is a decision about the report that was in front of the human.
/// At the cap, push refuses a lift given before the last verdict was
/// concluded, one given on a commit before it, one given on another branch,
/// and one that lifted a single finding rather than the verdict.
#[test]
fn at_the_cap_a_lift_that_is_not_about_the_last_verdict_is_refused() {
    let world = World::at(nunki::mission::Rigor::Standard);
    let base = git(&world.tree, &["rev-parse", "HEAD"]);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T2);
    world.event(Event::Iterate);
    let fix = world.coded("pub fn one() -> u8 { 3 }\n", "the fix");
    // A commit on another branch, which the pushed branch does not hold.
    git(&world.tree, &["checkout", "-q", "-b", "elsewhere"]);
    let elsewhere = world.commit("other.rs", "// elsewhere\n", "elsewhere");
    git(&world.tree, &["checkout", "-q", "mission/x"]);
    world.event(Event::HumanAccepted);

    for (finding, head, date, what) in [
        (None, fix.as_str(), T1, "before the verdict"),
        (None, base.as_str(), T3, "on a commit before the verdict's"),
        (None, elsewhere.as_str(), T3, "on another branch"),
        (Some("the redirect"), fix.as_str(), T3, "one finding only"),
    ] {
        let mut state = world.state();
        state.accepted.clear();
        world.store().save(&state).unwrap();
        world.lifted(finding, head, date);
        let err = push::push(&world.project, "m1", true).unwrap_err();
        assert!(matches!(err, PushError::NotLifted(_)), "{what}: {err}");
    }
    assert!(world.on_forge("mission/x").is_none());

    // And the one about it goes: on the verdict's own commit, after it.
    let mut state = world.state();
    state.accepted.clear();
    world.store().save(&state).unwrap();
    world.accepted(&lot, T3);
    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(pushed.not_attacked, vec![named(&fix, "the fix")]);
}

/// "What came after the last round" is read on the branch. A verdict on a
/// commit the branch no longer holds is not one the commits after it can
/// lean on.
#[test]
fn at_the_cap_a_verdict_off_the_branch_is_refused() {
    let world = World::at(nunki::mission::Rigor::Standard);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Clear, T1);
    world.review();
    git(&world.tree, &["reset", "-q", "--hard", "HEAD~1"]);
    world.coded("pub fn one() -> u8 { 3 }\n", "rewritten");
    assert_eq!(world.stage(), Stage::Verified);

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(
        matches!(&err, PushError::SecurityNotAnAncestor { on, .. } if *on == lot),
        "{err}"
    );
    assert!(world.on_forge("mission/x").is_none());
}

/// A spent cap never turns a red verdict green: a security verdict that is
/// neither CLEAR nor FINDINGS — forged, the agent cannot write one — is red
/// at the cap as it is anywhere.
#[test]
fn at_the_cap_a_red_security_verdict_is_still_red() {
    let world = World::at(nunki::mission::Rigor::Standard);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Clear, T1);
    let mut state = world.state();
    state.conclude(Role::Security, Some(Verdict::Broken), &lot);
    world.store().save(&state).unwrap();

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(
        matches!(
            err,
            PushError::Red {
                role: Role::Security,
                verdict: Verdict::Broken
            }
        ),
        "{err}"
    );
}

/// A prototype plays no security round, so push asks for no security
/// verdict — as for `security: gates`. A state frozen before the flow
/// refused a prototype with a security agent can still carry both.
#[test]
fn a_prototype_is_pushed_without_a_security_verdict() {
    let world = World::new(no_integration(), Security::Agent);
    let mut state = world.state();
    let mut json = serde_json::to_value(&state.flow).unwrap();
    json["header"]["rigor"] = serde_json::json!("prototype");
    state.flow = serde_json::from_value(json).unwrap();
    world.store().save(&state).unwrap();
    let head = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    assert_eq!(world.stage(), Stage::Verified);

    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert!(pushed.not_attacked.is_empty());
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
}

/// What `nunki push` prints is what the human reads before opening the pull
/// request, so the commits the security agent never saw are said there —
/// and only when there are some: a push on a verdict on `HEAD` names none,
/// and a heading over an empty list would read as a warning about nothing.
/// The real binary, because the decision is in what the CLI prints.
#[test]
fn the_push_command_names_the_commits_not_attacked_and_only_when_there_are_some() {
    let spent = World::opened_at(nunki::mission::Rigor::Standard);
    spent.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    spent.security(Verdict::Clear, T1);
    spent.review();
    let volet = spent.coded("pub fn one() -> u8 { 3 }\n", "the volet");
    let out = spent.pushed_by_the_binary();
    assert!(out.contains("not attacked by the security agent"), "{out}");
    assert!(out.contains(&named(&volet, "the volet")), "{out}");

    let on_head = World::opened_at(nunki::mission::Rigor::Critical);
    on_head.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    on_head.security(Verdict::Clear, T1);
    let out = on_head.pushed_by_the_binary();
    assert!(out.contains("pushed"), "the push happened: {out}");
    assert!(!out.contains("not attacked"), "{out}");
}

impl World {
    /// The real binary, run on this world with `args`: what it printed on
    /// both streams, whatever its exit.
    fn printed_by_the_binary(&self, args: &[&str]) -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_nunki"))
            .arg("-C")
            .arg(&self.project.root)
            .args(args)
            .env("HOME", &self.home)
            .env("HQ_NO_MONITOR", "1")
            .output()
            .unwrap();
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }
}

/// A commit subject is its author's text. Push names the commits the
/// security agent did not attack, on the terminal and in the follow-up, and
/// a subject carrying escape sequences reaches neither raw.
#[test]
fn a_commit_subject_not_attacked_is_printed_escaped_never_raw() {
    let world = World::opened_at(nunki::mission::Rigor::Standard);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Clear, T1);
    world.review();
    let volet = world.coded("pub fn one() -> u8 { 3 }\n", common::HOSTILE);

    let not_attacked = push::not_attacked(&world.tree, "HEAD~1", &volet).unwrap();
    assert_eq!(not_attacked.len(), 1);
    common::assert_printable(&not_attacked[0], "push::not_attacked");

    let printed = world.pushed_by_the_binary();
    assert!(printed.contains(&volet[..12]), "{printed}");
    common::assert_printable(&printed, "nunki push");
}

/// The security agent's report is its own text: `verify` prints it, and
/// `mission status` prints the stage that holds it, escaped and never raw.
#[test]
fn a_findings_report_is_printed_escaped_never_raw() {
    let world = World::opened_at(nunki::mission::Rigor::Standard);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    let mut state = world.state();
    state.conclude(Role::Security, Some(Verdict::Findings), &world.head());
    world.store().save(&state).unwrap();
    world.event(Event::Verdict {
        verdict: Verdict::Findings,
        report: common::HOSTILE.into(),
    });
    world.lifted(Some(common::HOSTILE), &world.head(), T1);

    let verified = world.printed_by_the_binary(&["verify", "m1"]);
    assert!(verified.contains("findings  "), "{verified}");
    assert!(verified.contains("lifted    "), "{verified}");
    common::assert_printable(&verified, "nunki verify");

    let status = world.printed_by_the_binary(&["mission", "status", "m1"]);
    assert!(status.contains("stage     Findings"), "{status}");
    assert!(!status.chars().any(common::raw_control), "{status}");
}

/// `nunki mission gates` prints one line per gate, its mark and why: a pass,
/// a failure and its reason, and a gate nobody could play — here gate 8,
/// with no profile up — said as such, never folded into a pass.
#[test]
fn the_gates_are_printed_one_line_each_with_their_mark_and_why() {
    let world = World::opened_at(nunki::mission::Rigor::Standard);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    // The base the gates measure the branch against, in the slot.
    git(&world.tree, &["branch", "-q", "dev", "HEAD~1"]);

    let out = world.printed_by_the_binary(&["mission", "gates", "m1"]);
    for line in [
        "gate 1    pass  clean tree",
        "gate 2    pass  branch not protected and ahead of its base",
        "gate 3    FAIL  the resume block names HEAD — the resume block does not name HEAD",
        "gate 4    pass  perimeter",
        "gate 8    ????  mechanical security — slot \"one\" has no profile up",
    ] {
        assert!(out.contains(line), "{line}\nin:\n{out}");
    }
}

// --- the lift nunki records itself (SPEC 4.5) ------------------------------

impl World {
    /// The lift `nunki` records on a FINDINGS whose every finding is LOW or
    /// INFO, on `HEAD`, as `verify` records it.
    fn lifted_by_nunki(&self) {
        let store = self.store();
        let mut state = store.load("m1").unwrap();
        nunki::findings::lift_by_nunki(
            &store,
            &mut state,
            &nunki::mission::dir::Paths::of(&self.project.hq_root, "m1").followup,
            &[
                nunki::mission::LowFinding {
                    severity: nunki::mission::Severity::Low,
                    title: "verbose error page".into(),
                    why_acceptable: "it names no path".into(),
                },
                nunki::mission::LowFinding {
                    severity: nunki::mission::Severity::Info,
                    title: "no security.txt".into(),
                    why_acceptable: "nothing is exposed".into(),
                },
            ],
            &self.head(),
        )
        .unwrap();
    }
}

const LIFTED: [&str; 2] = [
    "LOW — verbose error page: it names no path",
    "INFO — no security.txt: nothing is exposed",
];

/// A FINDINGS nunki lifted on `HEAD` is pushed as a human's lift would be,
/// and the push says what nunki accepted — while a human's lift names
/// nothing as nunki's.
#[test]
fn a_lift_by_nunki_on_head_is_pushed_naming_its_findings() {
    let world = World::at(nunki::mission::Rigor::Critical);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.lifted_by_nunki();
    assert_eq!(world.stage(), Stage::Verified);

    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(pushed.lifted_by_nunki, LIFTED.map(str::to_string).to_vec());
    assert_eq!(
        world.on_forge("mission/x").as_deref(),
        Some(world.head().as_str())
    );

    let human = World::at(nunki::mission::Rigor::Critical);
    let lot = human.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    human.security(Verdict::Findings, T1);
    human.accepted(&lot, T2);
    let pushed = push::push(&human.project, "m1", true).unwrap();
    assert!(
        pushed.lifted_by_nunki.is_empty(),
        "{:?}",
        pushed.lifted_by_nunki
    );
}

/// A lift is worth the verdict it answered. A later FINDINGS, on a later
/// commit, is not covered by nunki's lift of an earlier one — not even when
/// something forged the flow to `Verified`.
#[test]
fn a_later_findings_is_not_covered_by_an_earlier_lift_by_nunki() {
    let world = World::at(nunki::mission::Rigor::Critical);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.lifted_by_nunki();
    world.review();
    let volet = world.coded("pub fn one() -> u8 { 3 }\n", "the volet");
    world.security(Verdict::Findings, T2);
    assert!(matches!(world.stage(), Stage::Findings { .. }));
    world.event(Event::HumanAccepted);

    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(
        matches!(&err, PushError::NotLifted(head) if *head == volet),
        "{err}"
    );
    assert!(world.on_forge("mission/x").is_none());
}

/// At `standard`, nunki's lift of the one round's FINDINGS stands for the
/// volet after it, by the rule a human's lift follows at the cap: the
/// commits are named as not attacked, and the findings as nunki's.
#[test]
fn at_the_cap_a_lift_by_nunki_stands_for_the_commits_after_it() {
    let world = World::at(nunki::mission::Rigor::Standard);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, "2000-01-01T00:00:00Z");
    world.lifted_by_nunki();
    world.review();
    let volet = world.coded("pub fn one() -> u8 { 3 }\n", "the volet");
    assert_eq!(world.stage(), Stage::Verified, "no second round");

    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(pushed.not_attacked, vec![named(&volet, "the volet")]);
    assert_eq!(pushed.lifted_by_nunki, LIFTED.map(str::to_string).to_vec());
}

/// What `nunki push` and `nunki mission status` print: the findings nunki
/// accepted, under a heading that says it was nunki — and nothing of the
/// sort on a push no lift by nunki stands under. The real binary, because
/// the decision is in what the CLI prints.
#[test]
fn push_and_status_print_what_nunki_accepted_and_only_then() {
    let world = World::opened_at(nunki::mission::Rigor::Critical);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.lifted_by_nunki();

    let status = world.printed_by_the_binary(&["mission", "status", "m1"]);
    assert!(
        status.contains(&format!(
            "accepted  by nunki (LOW/INFO) on {}:",
            &world.head()[..12]
        )),
        "{status}"
    );
    for line in LIFTED {
        assert!(status.contains(line), "{line}\nin:\n{status}");
    }

    let out = world.pushed_by_the_binary();
    assert!(out.contains("accepted by nunki (LOW/INFO):"), "{out}");
    for line in LIFTED {
        assert!(out.contains(line), "{line}\nin:\n{out}");
    }

    let clear = World::opened_at(nunki::mission::Rigor::Critical);
    clear.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    clear.security(Verdict::Clear, T1);
    let out = clear.pushed_by_the_binary();
    assert!(out.contains("pushed"), "the push happened: {out}");
    assert!(!out.contains("accepted by nunki"), "{out}");
}

/// Gate 7 lets a coder's proposed equivalence through so the mission goes
/// on; push is where it waits. A verified mission whose campaign carries two
/// proposals is not pushed until the HQ ratified or refused both — and the
/// refusal names each survivor still waiting, and the two verbs. A refused
/// proposal reopens its survivor, and that holds the push too.
#[test]
fn push_refuses_until_every_proposed_equivalence_is_ratified_or_refused() {
    use nunki::mutants::{self, Campaign, Survivor, Triage};
    let world = World::new(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    let head = world.commit("src.rs", WITH_TESTS, "the lot");
    world.verified(&[(Role::Coder, None, head.clone())]);

    let dir = nunki::mission::dir::Paths::of(&world.project.hq_root, "m1").dir;
    let survivor = |id: &str| Survivor {
        id: id.into(),
        file: "src.rs".into(),
        line: 1,
        end_line: None,
        description: "replace 2 with 0".into(),
        outcome: None,
        refused: None,
    };
    mutants::write(
        &dir,
        &Campaign {
            fingerprint: "f".into(),
            head: head.clone(),
            date: "2026-10-06T12:00:00Z".into(),
            survivors: vec![survivor("s1"), survivor("s2"), survivor("s3")],
            tried: None,
        },
    )
    .unwrap();
    mutants::write_triage(
        &dir,
        &[
            (
                "s1".to_string(),
                Triage::EquivalentProposed {
                    why: "only a log line reads it".into(),
                },
            ),
            (
                "s2".to_string(),
                Triage::EquivalentProposed {
                    why: "both arms return the same constant".into(),
                },
            ),
            (
                "s3".to_string(),
                Triage::Killed {
                    test: "one_is_two".into(),
                },
            ),
        ]
        .into_iter()
        .collect(),
    )
    .unwrap();

    let err = push::push(&world.project, "m1", true).unwrap_err();
    match &err {
        PushError::ProposalsAwait { count, .. } => assert_eq!(*count, 2),
        other => panic!("two proposals nobody ruled on: {other}"),
    }
    let said = err.to_string();
    for part in [
        "`s1` (the coder's: only a log line reads it)",
        "`s2` (the coder's: both arms return the same constant)",
        "nunki mission mutants m1 --ratify <survivor>",
        "nunki mission mutants m1 --refuse <survivor> --because <why>",
    ] {
        assert!(said.contains(part), "{part:?} in {said}");
    }
    assert!(
        !said.contains("s3"),
        "a killed survivor awaits nothing: {said}"
    );
    assert!(world.on_forge("mission/x").is_none(), "nothing was pushed");

    // One ruled: the other still holds the push, and only it is named.
    mutants::ratify(&dir, "s1", None).unwrap();
    let err = push::push(&world.project, "m1", true).unwrap_err();
    let said = err.to_string();
    assert!(said.contains("`s2`"), "{said}");
    assert!(!said.contains("`s1`"), "{said}");
    assert!(world.on_forge("mission/x").is_none());

    // Both ruled — one ratified, one refused. The refusal reopened `s2`, so
    // the push is still refused: gate 7's rule no longer holds, and the
    // message names the survivor left without an outcome (security round 1,
    // MEDIUM).
    mutants::refuse(&dir, "s2", "the constant is printed").unwrap();
    let err = push::push(&world.project, "m1", true).unwrap_err();
    match &err {
        PushError::MutantsOwed { owed, .. } => {
            assert!(owed.contains("1 survivor(s) have no outcome"), "{owed}");
            assert!(owed.contains("src.rs:1 s2"), "{owed}");
        }
        other => panic!("a reopened survivor holds the push: {other}"),
    }
    assert!(
        err.to_string().contains("nunki mission iterate m1"),
        "{err}"
    );
    assert!(world.on_forge("mission/x").is_none());

    // The coder answers it with a test, and the push goes.
    let mut answers = mutants::read_triage(&dir).unwrap();
    answers.insert(
        "s2".to_string(),
        Triage::Killed {
            test: "two_is_printed".into(),
        },
    );
    mutants::write_triage(&dir, &answers).unwrap();
    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
    assert_eq!(pushed.head, head);
}

/// The HQ's two verbs, and where the proposals show, through the real
/// binary: `mission status` lists each proposal with its reason, `--ratify`
/// and `--refuse` rule on them, a `--refuse` without a reason and a lone
/// `--because` are refused, and the refusal reaches `FOLLOWUP_HQ.md`.
#[test]
fn the_hq_rules_on_proposals_with_two_verbs_and_status_lists_them() {
    use nunki::mutants::{self, Campaign, Survivor, Triage};
    let world = World::opened(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    let head = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    let paths = nunki::mission::dir::Paths::of(&world.project.hq_root, "m1");
    let survivor = |id: &str| Survivor {
        id: id.into(),
        file: "src.rs".into(),
        line: 1,
        end_line: None,
        description: "replace 2 with 0".into(),
        outcome: None,
        refused: None,
    };
    mutants::write(
        &paths.dir,
        &Campaign {
            fingerprint: "f".into(),
            head,
            date: "2026-10-06T12:00:00Z".into(),
            survivors: vec![survivor("s1"), survivor("s2")],
            tried: None,
        },
    )
    .unwrap();
    let propose = |id: &str, why: &str| {
        (
            id.to_string(),
            Triage::EquivalentProposed { why: why.into() },
        )
    };
    mutants::write_triage(
        &paths.dir,
        &[
            propose("s1", "only a log line reads it"),
            propose("s2", "both arms return the same constant"),
        ]
        .into_iter()
        .collect(),
    )
    .unwrap();

    let status = world.printed_by_the_binary(&["mission", "status", "m1"]);
    assert!(
        status.contains("proposals 2 equivalence proposal(s) await the HQ's ruling"),
        "{status}"
    );
    assert!(
        status.contains("s1 (the coder's) — only a log line reads it"),
        "{status}"
    );
    assert!(
        status.contains("s2 (the coder's) — both arms return the same constant"),
        "{status}"
    );

    let said = world.printed_by_the_binary(&["mission", "mutants", "m1", "--refuse", "s2"]);
    assert!(said.contains("--refuse needs --because"), "{said}");
    let said = world.printed_by_the_binary(&["mission", "mutants", "m1", "--because", "x"]);
    assert!(said.contains("--because goes with"), "{said}");
    assert_eq!(mutants::awaiting_ruling(&paths.dir).unwrap().len(), 2);

    let said = world.printed_by_the_binary(&["mission", "mutants", "m1", "--ratify", "s1"]);
    assert!(
        said.contains("s1 ruled equivalent — only a log line reads it"),
        "{said}"
    );
    let said = world.printed_by_the_binary(&[
        "mission",
        "mutants",
        "m1",
        "--refuse",
        "s2",
        "--because",
        "the constant is printed",
    ]);
    assert!(said.contains("s2: the proposal is refused"), "{said}");
    assert!(mutants::awaiting_ruling(&paths.dir).unwrap().is_empty());
    let followup = std::fs::read_to_string(&paths.followup).unwrap();
    assert!(followup.contains("the constant is printed"), "{followup}");

    let status = world.printed_by_the_binary(&["mission", "status", "m1"]);
    assert!(!status.contains("proposals "), "{status}");
}

/// At `standard`, push re-checks the share, with the threshold gate 7 used:
/// the header's when it froze one, the project's otherwise. 3 survivors
/// left open of 10 tried is 70% — under the project's 80%, over a frozen 60%.
#[test]
fn at_standard_push_refuses_a_share_below_the_threshold_gate_seven_used() {
    use nunki::mutants::{self, Campaign, Survivor};
    let world = World::at(nunki::mission::Rigor::Standard);
    let head = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Clear, "2026-10-06T12:00:00Z");
    assert_eq!(world.stage(), Stage::Verified);

    let dir = nunki::mission::dir::Paths::of(&world.project.hq_root, "m1").dir;
    let survivor = |line: u32| Survivor {
        id: format!("s{line}"),
        file: "src.rs".into(),
        line,
        end_line: None,
        description: "replace 2 with 0".into(),
        outcome: None,
        refused: None,
    };
    mutants::write(
        &dir,
        &Campaign {
            fingerprint: "f".into(),
            head: head.clone(),
            date: "2026-10-06T12:00:00Z".into(),
            survivors: (1..=3).map(survivor).collect(),
            tried: Some(10),
        },
    )
    .unwrap();

    let err = push::push(&world.project, "m1", true).unwrap_err();
    match &err {
        PushError::MutantsOwed { owed, .. } => {
            assert!(owed.contains("7 of 10 tried mutant(s) killed"), "{owed}");
            assert!(owed.contains("threshold of 80%"), "{owed}");
            assert!(owed.contains("src.rs:3 s3"), "{owed}");
        }
        other => panic!("70% is below the project's 80%: {other}"),
    }
    assert!(world.on_forge("mission/x").is_none());

    // The header froze 60%: that is the threshold the gate used, and 70%
    // clears it.
    let mut state = world.state();
    let mut header = state.flow.header().clone();
    header.mutation_threshold = Some(60);
    state.flow.reframe(header).unwrap();
    world.store().save(&state).unwrap();
    assert_eq!(world.stage(), Stage::Verified);
    push::push(&world.project, "m1", true).unwrap();
}

/// `--refuse` on a verified mission sends it back to the coder as a volet,
/// the path an HQ review takes, with the refusal as its cause: the reopened
/// survivor is answered by a coder run rather than left for a push that
/// refuses it. On a mission still being driven, it only refuses — gate 7
/// reads the survivor when the final gates are played.
#[test]
fn refusing_a_proposal_on_a_verified_mission_sends_it_back_to_the_coder() {
    use nunki::mutants::{self, Campaign, Survivor, Triage};
    let world = World::new(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    let head = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    let paths = nunki::mission::dir::Paths::of(&world.project.hq_root, "m1");
    let propose = || {
        mutants::write(
            &paths.dir,
            &Campaign {
                fingerprint: "f".into(),
                head: head.clone(),
                date: "2026-10-06T12:00:00Z".into(),
                survivors: vec![Survivor {
                    id: "s1".into(),
                    file: "src.rs".into(),
                    line: 1,
                    end_line: None,
                    description: "replace 2 with 0".into(),
                    outcome: None,
                    refused: None,
                }],
                tried: None,
            },
        )
        .unwrap();
        mutants::write_triage(
            &paths.dir,
            &[(
                "s1".to_string(),
                Triage::EquivalentProposed {
                    why: "only a log line reads it".into(),
                },
            )]
            .into_iter()
            .collect(),
        )
        .unwrap();
    };

    // Still coding: refused on file, and the flow is left where it is.
    propose();
    let before = world.stage();
    let refused =
        nunki::findings::refuse_proposal(&world.project, "m1", "s1", "the CLI prints it").unwrap();
    assert_eq!(refused.sent_back, None);
    assert_eq!(world.stage(), before);
    assert_eq!(mutants::open(&paths.dir).unwrap(), vec!["s1"]);

    // Verified: the refusal opens a volet whose cause is the refusal.
    propose();
    world.verified(&[(Role::Coder, None, head.clone())]);
    let refused =
        nunki::findings::refuse_proposal(&world.project, "m1", "s1", "the CLI prints it").unwrap();
    let cause = match world.stage() {
        Stage::Coding {
            work: nunki::mission::flow::Work::Volet { cause, .. },
            ..
        } => cause,
        other => panic!("a refusal on a verified mission is a volet: {other:?}"),
    };
    assert_eq!(refused.sent_back, Some(world.stage()));
    assert!(
        cause.contains("refused the equivalence proposed on `s1`"),
        "{cause}"
    );
    assert!(cause.contains("the CLI prints it"), "{cause}");
    assert_eq!(world.state().flow.volets(), 1);
    let followup = std::fs::read_to_string(&paths.followup).unwrap();
    assert!(followup.contains("the CLI prints it"), "{followup}");

    // A refusal refused — no reason — moves nothing.
    propose();
    world.verified(&[(Role::Coder, None, head)]);
    let err = nunki::findings::refuse_proposal(&world.project, "m1", "s1", " ").unwrap_err();
    assert!(err.to_string().contains("--because"), "{err}");
    assert_eq!(world.stage(), Stage::Verified);
}

/// A world verified at `rigor`, with no service and no security agent, and a
/// campaign holding `survivors` — each open — written over the empty one.
fn verified_with(rigor: nunki::mission::Rigor, survivors: &[&str]) -> (World, PathBuf, String) {
    use nunki::mutants::{self, Campaign, Survivor};
    let world = World::new(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    let mut h = header(
        Integration::None {
            reason: "none".into(),
        },
        Security::Gates,
    );
    h.rigor = rigor;
    let mut state = world.state();
    state.flow = Flow::new(h).unwrap();
    world.store().save(&state).unwrap();
    let head = world.commit("src.rs", WITH_TESTS, "the lot");
    world.verified(&[(Role::Coder, None, head.clone())]);
    let dir = nunki::mission::dir::Paths::of(&world.project.hq_root, "m1").dir;
    mutants::write(
        &dir,
        &Campaign {
            fingerprint: "f".into(),
            head: head.clone(),
            date: "2026-10-06T12:00:00Z".into(),
            survivors: survivors
                .iter()
                .map(|id| Survivor {
                    id: (*id).into(),
                    file: "src.rs".into(),
                    line: 1,
                    end_line: None,
                    description: "replace 2 with 0".into(),
                    outcome: None,
                    refused: None,
                })
                .collect(),
            tried: None,
        },
    )
    .unwrap();
    (world, dir, head)
}

/// At `critical`, push plays gate 7's rule again: a survivor with no
/// outcome at all — no test, no ruling, no proposal — holds the push, and
/// the refusal names it and points at the one verb that answers it.
#[test]
fn at_critical_push_refuses_a_survivor_left_without_an_outcome() {
    let (world, dir, head) = verified_with(nunki::mission::Rigor::Critical, &["s1"]);
    let err = push::push(&world.project, "m1", true).unwrap_err();
    match &err {
        PushError::MutantsOwed { owed, .. } => {
            assert!(
                owed.contains("1 survivor(s) have no outcome: src.rs:1 s1"),
                "{owed}"
            )
        }
        other => panic!("an open survivor at critical holds the push: {other}"),
    }
    // The hint names `iterate`, and never `--refuse`, which cannot answer a
    // survivor already refused.
    let said = err.to_string();
    assert!(
        said.contains("nunki mission iterate m1 --because <why>"),
        "{said}"
    );
    assert!(!said.contains("--refuse"), "{said}");
    assert!(world.on_forge("mission/x").is_none());

    nunki::mutants::write_triage(
        &dir,
        &[(
            "s1".to_string(),
            nunki::mutants::Triage::Bug {
                test: "one_is_two".into(),
            },
        )]
        .into_iter()
        .collect(),
    )
    .unwrap();
    push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
}

/// No campaign, no push: a mission whose rigor owes one and whose
/// `MUTANTS.json` is missing or empty is refused. A prototype owes none.
#[test]
fn push_fails_closed_when_the_campaign_is_missing_or_empty() {
    use nunki::mission::Rigor;
    for rigor in [Rigor::Critical, Rigor::Standard] {
        for leave in [None, Some("   \n")] {
            let (world, dir, _) = verified_with(rigor, &[]);
            let file = dir.join(nunki::mutants::FILE);
            match leave {
                None => std::fs::remove_file(&file).unwrap(),
                Some(text) => std::fs::write(&file, text).unwrap(),
            }
            let err = push::push(&world.project, "m1", true).unwrap_err();
            match &err {
                PushError::NoCampaign { .. } => {}
                other => panic!("{rigor} with {leave:?}: no campaign, no push: {other}"),
            }
            let said = err.to_string();
            assert!(said.contains("no campaign, no push"), "{said}");
            assert!(said.contains(&format!("`{rigor}`")), "{said}");
            // The verb that writes a campaign, and not `nunki verify`, which
            // replays no gate on a verified mission.
            assert!(said.contains("`nunki mission mutants m1`"), "{said}");
            assert!(!said.contains("nunki verify"), "{said}");
            assert!(world.on_forge("mission/x").is_none());
        }
    }

    let (world, dir, head) = verified_with(Rigor::Prototype, &[]);
    std::fs::remove_file(dir.join(nunki::mutants::FILE)).unwrap();
    push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
}

/// `--ratify`, `--equivalent` and `--lift` take the slot's lock, as
/// `--refuse` does: while something else holds it, each is refused and
/// writes nothing; once it is free, each goes through.
#[test]
fn the_hqs_rulings_take_the_slots_lock() {
    use nunki::mutants::{self, Triage};
    let (world, dir, _) = verified_with(nunki::mission::Rigor::Critical, &["s1", "s2"]);
    mutants::write_triage(
        &dir,
        &[(
            "s1".to_string(),
            Triage::EquivalentProposed {
                why: "only a log line reads it".into(),
            },
        )]
        .into_iter()
        .collect(),
    )
    .unwrap();
    let campaign = || std::fs::read_to_string(dir.join(mutants::FILE)).unwrap();
    let before = campaign();

    let held = nunki::state::SlotLock::acquire(
        &world.project.hq_root.join("locks"),
        "one",
        "a test holding the slot",
    )
    .unwrap();
    let locked = |err: nunki::findings::FindingsError| {
        assert!(
            matches!(err, nunki::findings::FindingsError::Lock(_)),
            "the lock is held: {err}"
        )
    };
    locked(nunki::findings::ratify_proposal(&world.project, "m1", "s1", None).unwrap_err());
    locked(nunki::findings::rule_equivalent(&world.project, "m1", "s2", "same").unwrap_err());
    locked(nunki::findings::lift_equivalent(&world.project, "m1", "s2").unwrap_err());
    locked(nunki::findings::refuse_proposal(&world.project, "m1", "s1", "printed").unwrap_err());
    assert_eq!(campaign(), before, "nothing was written under a held lock");
    drop(held);

    nunki::findings::ratify_proposal(&world.project, "m1", "s1", None).unwrap();
    nunki::findings::rule_equivalent(&world.project, "m1", "s2", "same").unwrap();
    nunki::findings::lift_equivalent(&world.project, "m1", "s2").unwrap();
    let after = mutants::read(&dir).unwrap().unwrap();
    assert!(matches!(
        after.survivors[0].outcome,
        Some(Triage::Equivalent { .. })
    ));
    assert_eq!(after.survivors[1].outcome, None);
}

/// Refusing a survivor that already holds the HQ's ruling is an error, not
/// a send-back: the mission stays verified, and the error points at `--lift`.
#[test]
fn refusing_a_ruled_survivor_on_a_verified_mission_sends_nothing_back() {
    use nunki::mutants::{self, Triage};
    let (world, dir, _) = verified_with(nunki::mission::Rigor::Critical, &["s1"]);
    mutants::write_triage(
        &dir,
        &[(
            "s1".to_string(),
            Triage::EquivalentProposed {
                why: "only a log line reads it".into(),
            },
        )]
        .into_iter()
        .collect(),
    )
    .unwrap();
    mutants::rule_equivalent(&dir, "s1", "the HQ's own words").unwrap();

    let err = nunki::findings::refuse_proposal(&world.project, "m1", "s1", "printed").unwrap_err();
    assert!(err.to_string().contains("--lift s1"), "{err}");
    assert_eq!(world.stage(), Stage::Verified);
    assert_eq!(world.state().flow.volets(), 0);
}

/// `nunki mission status` lists each proposal with its source, and a ruling
/// verb says on stderr when it left the registry as
/// it was: the mission's file changed, the next mission's will not. Both
/// are only ever printed by the binary.
#[test]
fn the_binary_lists_proposals_by_source_and_says_when_the_registry_is_left_alone() {
    use nunki::mutants::{Campaign, Survivor, Triage};
    let world = World::opened_at(nunki::mission::Rigor::Critical);
    let id = "src.rs:1:5: replace one -> u8 with 0";
    nunki::mutants::write(
        &nunki::mission::dir::Paths::of(&world.project.hq_root, "m1").dir,
        &Campaign {
            fingerprint: "f".into(),
            head: "0123456789abcdef0123456789abcdef01234567".into(),
            date: "2026-10-06T12:00:00Z".into(),
            survivors: vec![Survivor {
                id: id.into(),
                file: "src.rs".into(),
                line: 1,
                end_line: None,
                description: "replace one -> u8 with 0".into(),
                outcome: Some(Triage::EquivalentRegistered {
                    why: "nothing reads the value".into(),
                    mission: "earlier".into(),
                    commit: "fedcba9876543210fedcba9876543210fedcba98".into(),
                }),
                refused: None,
            }],
            tried: Some(1),
        },
    )
    .unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_nunki"))
            .arg("-C")
            .arg(&world.project.root)
            .args(args)
            .env("HOME", &world.home)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    let (ok, stdout, stderr) = run(&["mission", "status", "m1"]);
    assert!(ok, "{stdout}{stderr}");
    // An older file's registry ruling reads as what it is now: a proposal
    // from the registry, awaiting the HQ (HQ review 2).
    assert!(
        stdout.contains(
            "proposals 1 equivalence proposal(s) await the HQ's ruling (1 from the registry)"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "{id} (nunki's, from the registry: ruled on mission earlier at fedcba987654) — \
             nothing reads the value"
        )),
        "{stdout}"
    );

    std::fs::write(
        nunki::equivalences::path(&world.project.hq_root),
        "{ not a registry",
    )
    .unwrap();
    // A lift the registry cannot take fails, and changes nothing (HQ review,
    // item 4).
    let (ok, stdout, stderr) = run(&["mission", "mutants", "m1", "--lift", id]);
    assert!(!ok, "{stdout}{stderr}");
    assert!(stderr.contains("Nothing was lifted"), "{stderr}");
    assert!(stderr.contains("equivalences.json"), "{stderr}");
    // A ruling stands on its mission, and says on stderr that the registry
    // was left alone.
    let (ok, stdout, stderr) = run(&[
        "mission",
        "mutants",
        "m1",
        "--equivalent",
        id,
        "--because",
        "nothing reads it",
    ]);
    assert!(ok, "the ruling is the mission's: {stdout}{stderr}");
    assert!(
        stderr.contains("the registry of equivalences is unchanged"),
        "{stderr}"
    );
    let (_, stdout, _) = run(&["mission", "status", "m1"]);
    assert!(stdout.contains("registry  could not be read"), "{stdout}");
}

/// What `nunki` proposes from a ruling it matched holds the push exactly as
/// the coder's proposals do, and the refusal names each by source; `--ratify
/// --all` prints every pending proposal with its source and sentence, writes
/// the HQ's equivalence on each, and the push goes through (HQ review 2, D).
#[test]
fn push_waits_on_nunkis_proposals_and_ratify_all_rules_them() {
    use nunki::mutants::{self, Campaign, ProposedFrom, Survivor, Triage};
    let world = World::opened(no_integration(), Security::Gates);
    let head = world.commit("src.rs", "pub fn one() -> u8 { 2 }\n", "the lot");
    world.verified(&[(Role::Coder, None, head.clone())]);
    let paths = nunki::mission::dir::Paths::of(&world.project.hq_root, "m1");
    let survivor = |id: &str, outcome: Option<Triage>| Survivor {
        id: id.into(),
        file: "src.rs".into(),
        line: 1,
        end_line: None,
        description: format!("replace {id}"),
        outcome,
        refused: None,
    };
    mutants::write(
        &paths.dir,
        &Campaign {
            fingerprint: "f".into(),
            head,
            date: "2026-10-06T12:00:00Z".into(),
            survivors: vec![
                survivor("s1", None),
                survivor(
                    "s2",
                    Some(Triage::ProposedByNunki {
                        why: "ruled on another mission".into(),
                        from: ProposedFrom::Registry {
                            mission: "earlier".into(),
                            commit: "0123456789abcdef".into(),
                            date: "2026-10-01T00:00:00Z".into(),
                        },
                    }),
                ),
                survivor(
                    "s3",
                    Some(Triage::ProposedByNunki {
                        why: "ruled on its twin".into(),
                        from: ProposedFrom::Carried {
                            commit: "fedcba9876543210".into(),
                        },
                    }),
                ),
            ],
            tried: None,
        },
    )
    .unwrap();
    mutants::write_triage(
        &paths.dir,
        &[(
            "s1".to_string(),
            Triage::EquivalentProposed {
                why: "only a log line reads it".into(),
            },
        )]
        .into_iter()
        .collect(),
    )
    .unwrap();

    let err = push::push(&world.project, "m1", true).unwrap_err();
    let said = err.to_string();
    assert!(
        matches!(err, PushError::ProposalsAwait { count: 3, .. }),
        "{said}"
    );
    for part in [
        "`s1` (the coder's: only a log line reads it)",
        "`s2` (nunki's, from the registry: ruled on mission earlier at 0123456789ab: ruled on \
         another mission)",
        "`s3` (nunki's, carried by file and mutation: ruled at fedcba987654 on a survivor of \
         another id: ruled on its twin)",
        "--ratify --all",
    ] {
        assert!(said.contains(part), "{part:?} in {said}");
    }
    assert!(world.on_forge("mission/x").is_none());

    let status = world.printed_by_the_binary(&["mission", "status", "m1"]);
    assert!(
        status.contains(
            "3 equivalence proposal(s) await the HQ's ruling (1 from the coder, 1 from the \
             registry, 1 carried by file and mutation)"
        ),
        "{status}"
    );

    // Asked without a survivor, or with one and --all, it refuses.
    let said = world.printed_by_the_binary(&["mission", "mutants", "m1", "--ratify"]);
    assert!(said.contains("takes --all"), "{said}");
    let said =
        world.printed_by_the_binary(&["mission", "mutants", "m1", "--ratify", "s1", "--all"]);
    assert!(said.contains("names no survivor"), "{said}");
    assert_eq!(mutants::awaiting_ruling(&paths.dir).unwrap().len(), 3);

    let said = world.printed_by_the_binary(&["mission", "mutants", "m1", "--ratify", "--all"]);
    let listed = said.find("s2 (nunki's, from the registry").expect(&said);
    let ruled = said
        .find("s2 ruled equivalent — ruled on another mission")
        .expect(&said);
    assert!(listed < ruled, "each is printed before it is ruled: {said}");
    assert!(
        said.contains("s1 (the coder's) — only a log line reads it"),
        "{said}"
    );
    assert!(
        said.contains("s3 ruled equivalent — ruled on its twin"),
        "{said}"
    );

    let campaign = mutants::read(&paths.dir).unwrap().unwrap();
    for (s, why) in campaign.survivors.iter().zip([
        "only a log line reads it",
        "ruled on another mission",
        "ruled on its twin",
    ]) {
        assert_eq!(
            s.outcome,
            Some(Triage::Equivalent {
                why: why.into(),
                carried_from: None,
            }),
            "{}",
            s.id
        );
    }
    assert!(mutants::read_triage(&paths.dir).unwrap().is_empty());
    assert!(mutants::awaiting_ruling(&paths.dir).unwrap().is_empty());
    push::push(&world.project, "m1", true).unwrap();
    assert!(world.on_forge("mission/x").is_some());
}

/// The lot's source, holding the tests the outcomes in these worlds name:
/// push asks, as gate 7 does, that a named test exists (HQ review 3).
const WITH_TESTS: &str = "pub fn one() -> u8 { 2 }\n\
                          #[test]\nfn one_is_two() {}\n\
                          #[test]\nfn two_is_printed() {}\n";

/// The coder's file is read only for the outcomes the coder may give (HQ
/// review 3). A verified `critical` mission whose triage entry is rewritten
/// after the gates to `equivalent`, then to `equivalent_registered`, holds
/// no outcome for that survivor: push refuses, and names the entry refused.
#[test]
fn push_refuses_a_triage_rewritten_to_a_ruling() {
    let (world, dir, _) = verified_with(nunki::mission::Rigor::Critical, &["s1"]);
    for forged in [
        r#"{"s1": {"kind": "equivalent", "why": "trust me"}}"#,
        r#"{"s1": {"kind": "equivalent_registered", "why": "trust me", "mission": "m", "commit": "c"}}"#,
    ] {
        std::fs::write(dir.join(nunki::mutants::TRIAGE_FILE), forged).unwrap();
        match push::push(&world.project, "m1", true) {
            Err(PushError::MutantsOwed { owed, .. }) => {
                assert!(owed.contains("1 survivor(s) have no outcome"), "{owed}");
                assert!(owed.contains("not the coder's to give"), "{owed}");
                assert!(owed.contains("refused and read as no outcome"), "{owed}");
            }
            other => panic!("{forged}: a ruling forged by the coder was read: {other:?}"),
        }
        assert!(world.on_forge("mission/x").is_none());
    }
}

/// Push asks what gate 7 asks of a named test: that the tree holds it. A
/// `killed` naming a test nobody wrote, written after the gates, holds the
/// push with the gate's own words (HQ review 3).
#[test]
fn push_refuses_a_killed_naming_a_test_that_does_not_exist() {
    let (world, dir, head) = verified_with(nunki::mission::Rigor::Critical, &["s1"]);
    let killed_by = |test: &str| {
        nunki::mutants::write_triage(
            &dir,
            &[(
                "s1".to_string(),
                nunki::mutants::Triage::Killed { test: test.into() },
            )]
            .into_iter()
            .collect(),
        )
        .unwrap();
    };
    killed_by("a_test_nobody_wrote");
    match push::push(&world.project, "m1", true) {
        Err(PushError::MutantsOwed { owed, .. }) => assert!(
            owed.contains(
                "names the test \"a_test_nobody_wrote\", and nothing in the tree is called that"
            ),
            "{owed}"
        ),
        other => panic!("a test nobody wrote was taken: {other:?}"),
    }
    assert!(world.on_forge("mission/x").is_none());

    killed_by("one_is_two");
    push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
}

/// Push looks for a named test in the commit it pushes, never in the slot's
/// working tree: a name present only as an uncommitted edit of a tracked
/// file is refused (HQ review 4).
#[test]
fn push_refuses_a_test_named_only_in_an_uncommitted_edit() {
    let (world, dir, head) = verified_with(nunki::mission::Rigor::Critical, &["s1"]);
    nunki::mutants::write_triage(
        &dir,
        &[(
            "s1".to_string(),
            nunki::mutants::Triage::Killed {
                test: "only_in_the_working_tree".into(),
            },
        )]
        .into_iter()
        .collect(),
    )
    .unwrap();
    let source = world.tree.join("src.rs");
    let committed = std::fs::read_to_string(&source).unwrap();
    std::fs::write(
        &source,
        format!("{committed}#[test]\nfn only_in_the_working_tree() {{}}\n"),
    )
    .unwrap();
    match push::push(&world.project, "m1", true) {
        Err(PushError::MutantsOwed { owed, .. }) => assert!(
            owed.contains("names the test \"only_in_the_working_tree\""),
            "{owed}"
        ),
        other => panic!("an uncommitted edit stood for the pushed commit: {other:?}"),
    }
    assert!(world.on_forge("mission/x").is_none());
    std::fs::write(&source, committed).unwrap();
    let _ = head;
}

/// A named test is a name (HQ review 4): blank, a space, or a word of two
/// letters answers nothing at push — each is matched by nearly any line —
/// and a real test name, as a whole word, still does.
#[test]
fn push_takes_no_test_name_that_is_not_a_name() {
    let (world, dir, head) = verified_with(nunki::mission::Rigor::Critical, &["s1"]);
    let killed_by = |test: &str| {
        nunki::mutants::write_triage(
            &dir,
            &[(
                "s1".to_string(),
                nunki::mutants::Triage::Killed { test: test.into() },
            )]
            .into_iter()
            .collect(),
        )
        .unwrap();
    };
    for name in ["", " ", "fn", "u8", "one_is", "one-is-two"] {
        killed_by(name);
        match push::push(&world.project, "m1", true) {
            Err(PushError::MutantsOwed { owed, .. }) => assert!(
                owed.contains("1 survivor(s) have no outcome") || owed.contains("names the test"),
                "{name:?}: {owed}"
            ),
            other => panic!("{name:?} was taken for a test: {other:?}"),
        }
    }
    assert!(world.on_forge("mission/x").is_none());
    killed_by("one_is_two");
    push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
}

/// A prototype owes no campaign, so push asks nothing of the tests one
/// names: gate 7 does not either.
#[test]
fn a_prototype_is_pushed_whatever_test_its_triage_names() {
    let (world, dir, head) = verified_with(nunki::mission::Rigor::Prototype, &["s1"]);
    nunki::mutants::write_triage(
        &dir,
        &[(
            "s1".to_string(),
            nunki::mutants::Triage::Killed {
                test: "a_test_nobody_wrote".into(),
            },
        )]
        .into_iter()
        .collect(),
    )
    .unwrap();
    push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(head.as_str()));
}
