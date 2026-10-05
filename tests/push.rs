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
}

impl World {
    fn new(integration: Integration, security: Security) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let hq_root = dir.path().join("nunki").join(nunki::project::HQ_DIR);
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
        let world = World::new(no_integration(), Security::Agent);
        let mut h = header(no_integration(), Security::Agent);
        h.rigor = rigor;
        let mut state = world.state();
        state.flow = Flow::new(h).unwrap();
        world.store().save(&state).unwrap();
        world
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
