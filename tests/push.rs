//! `nunki mission fetch` and `nunki push` (SPEC 4.2, 4.4, 4.5): the one verb that
//! touches the forge in write, and everything it refuses first.

use std::path::{Path, PathBuf};
use std::process::Command;

use nunki::harness::Role;
use nunki::mission::flow::{Event, Flow, Stage};
use nunki::mission::{Bounds, Header, Integration, Lot, Security, Service, Verdict};
use nunki::project::{Config, Project, ProtectedPaths};
use nunki::push::{self, PushError};
use nunki::state::{MissionState, Said, Store};

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
        hardening: Vec::new(),
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
        // The mission's base is `dev`, and a project home the binary opens
        // starts on `main`: the base the host records for the slot has to
        // exist, since `nunki push` at `critical` judges the campaign on
        // the content the branch changed since it.
        if opened {
            git(&root, &["branch", "dev"]);
        }
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
        // What `run::branch` does at launch: the host records the base from
        // the project's repository, and the gates judge against it alone.
        nunki::git::SlotGit::open(&tree)
            .unwrap()
            .fetch_origin(&root)
            .unwrap();

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
                mutation_jobs: None,
                forge_protection: Default::default(),
                security_findings: Default::default(),
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
                stopped: None,
                files: None,
                chain: Default::default(),
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
                decided: None,
                coder_session: None,
                pushed: None,
                updated_at: String::new(),
                revision: 0,
            })
            .unwrap();

        // What every mission's `PR.md` holds by the time it is pushed: a
        // title and the coder's words, and nothing of the security agent's,
        // which cannot write it (SPEC 4.5). The tests about the Security
        // section write their own.
        let mission = hq_root.join("missions/m1");
        std::fs::create_dir_all(&mission).unwrap();
        std::fs::write(
            mission.join("PR.md"),
            "# feat: one returns two\n\nBecause it had to.\n",
        )
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
        // At `critical`, the default, the final full campaign: the one on
        // file is put on the content of `HEAD`, full, as the final stage
        // leaves it, and its green gate verifies the mission.
        if matches!(state.flow.stage(), Stage::FinalCampaign) {
            self.campaign_on_head();
            store.apply(&mut state, Event::GatesPassed).unwrap();
        }
        assert!(
            matches!(state.flow.stage(), Stage::Verified),
            "{:?}",
            state.flow.stage()
        );
        for (role, verdict, head) in verdicts {
            // What `verify` keeps of a security verdict it concludes: what
            // the Security section of the pull request is written from.
            let said = (*role == Role::Security).then(clear_said);
            state.conclude_saying(*role, *verdict, head, said);
        }
        store.save(&state).unwrap();
    }

    /// The campaign on file, moved onto the content of `HEAD` as a full
    /// campaign there, everything else in it kept.
    fn campaign_on_head(&self) {
        let dir = nunki::mission::dir::Paths::of(&self.project.hq_root, "m1").dir;
        let mut campaign = nunki::mutants::read(&dir).unwrap().unwrap();
        let touched = nunki::gate::touched_since_base(&self.tree, "dev").unwrap();
        campaign.fingerprint = nunki::mutants::fingerprint(&self.tree, &touched).unwrap();
        campaign.head = self.head();
        campaign.chain.scope = nunki::mutants::Scope::Full {
            why: "the first campaign of this mission".into(),
        };
        nunki::mutants::write(&dir, &campaign).unwrap();
    }

    /// The fingerprint of `HEAD`'s content, as gate 7 and `nunki push`
    /// compute it.
    fn fingerprint(&self) -> String {
        let touched = nunki::gate::touched_since_base(&self.tree, "dev").unwrap();
        nunki::mutants::fingerprint(&self.tree, &touched).unwrap()
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
        fixed_by: None,
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
    // And the volet's final full campaign, on its `HEAD`.
    world.campaign_on_head();
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
    // `PR.md` as written, then the Security section nunki writes from the
    // CLEAR (SPEC 4.5).
    assert_eq!(
        body["body"],
        format!(
            "Because it had to.\n\n## Security\n\n\
             The security agent attacked commit `{head}` and concluded CLEAR.\n\n\
             Its report, as it wrote it:\n\n> Attacked the build; nothing found."
        ),
        "{sent}"
    );
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

    /// The flow moves on `event`. A step onto the final stage of a
    /// `critical` mission is met as `verify` meets it when the campaign on
    /// file passes: that campaign put on `HEAD`, full, and the mission
    /// verified.
    fn event(&self, event: Event) {
        let mut state = self.state();
        self.store().apply(&mut state, event).unwrap();
        self.final_stage_passes();
    }

    /// At the final stage of a `critical` mission, what `verify` does when
    /// the campaign on file passes: that campaign put on `HEAD`, full, and
    /// the mission verified. Anywhere else, nothing.
    fn final_stage_passes(&self) {
        let mut state = self.state();
        if matches!(state.flow.stage(), Stage::FinalCampaign) {
            self.campaign_on_head();
            self.store().apply(&mut state, Event::GatesPassed).unwrap();
        }
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

    /// What the security verdict concluded said, as `verify` keeps it in
    /// the state when it concludes one.
    fn said(&self, said: Said) {
        let mut state = self.state();
        state
            .verdicts
            .iter_mut()
            .find(|c| c.role == Role::Security)
            .expect("the security agent concluded")
            .said = Some(said);
        self.store().save(&state).unwrap();
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
            fixed_by: None,
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
    // The base the gates measure the branch against: in the project, where
    // the host records it from, never in the slot.
    git(&world.project.root, &["branch", "-q", "-f", "dev", "HEAD"]);
    nunki::git::SlotGit::open(&world.tree)
        .unwrap()
        .fetch_origin(&world.project.root)
        .unwrap();

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
        self.final_stage_passes();
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
        found_on: None,
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
            stopped: None,
            files: None,
            chain: Default::default(),
            fingerprint: world.fingerprint(),
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
        found_on: None,
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
            stopped: None,
            files: None,
            chain: Default::default(),
            fingerprint: world.fingerprint(),
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
        found_on: None,
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
            stopped: None,
            files: None,
            chain: Default::default(),
            fingerprint: world.fingerprint(),
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
                stopped: None,
                files: None,
                chain: Default::default(),
                fingerprint: world.fingerprint(),
                head: head.clone(),
                date: "2026-10-06T12:00:00Z".into(),
                survivors: vec![Survivor {
                    found_on: None,
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
            stopped: None,
            files: None,
            chain: Default::default(),
            fingerprint: world.fingerprint(),
            head: head.clone(),
            date: "2026-10-06T12:00:00Z".into(),
            survivors: survivors
                .iter()
                .map(|id| Survivor {
                    found_on: None,
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
            stopped: None,
            files: None,
            chain: Default::default(),
            fingerprint: world.fingerprint(),
            head: "0123456789abcdef0123456789abcdef01234567".into(),
            date: "2026-10-06T12:00:00Z".into(),
            survivors: vec![Survivor {
                found_on: None,
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
        found_on: None,
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
            stopped: None,
            files: None,
            chain: Default::default(),
            fingerprint: world.fingerprint(),
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

/// Push re-checks the chain as gate 7 judges it: once, on the
/// reconstruction — the kept file's count as it was measured, the measured
/// one's as now, over their sum (HQ review of c4f2533).
#[test]
fn push_judges_the_chain_once_on_its_reconstruction() {
    use nunki::mutants::{self, Campaign, Chain, Link, Scope, Survivor};
    let world = World::at(nunki::mission::Rigor::Standard);
    let head = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Clear, "2026-10-06T12:00:00Z");
    assert_eq!(world.stage(), Stage::Verified);

    let dir = nunki::mission::dir::Paths::of(&world.project.hq_root, "m1").dir;
    let earlier = "e".repeat(40);
    let survivor = |line: u32| Survivor {
        found_on: Some(earlier.clone()),
        id: format!("s{line}"),
        file: "src.rs".into(),
        line,
        end_line: None,
        description: "replace 2 with 0".into(),
        outcome: None,
        refused: None,
    };
    let measured = |tried: u32, on: &str| mutants::Measured {
        tried,
        found: None,
        on: on.to_string(),
    };
    // The file kept from the earlier campaign tried 50; the one measured
    // again, 60: one full campaign would try 110.
    let chain = |open: u32| Campaign {
        stopped: None,
        files: Some(
            [
                ("src/kept.rs".to_string(), measured(50, &earlier)),
                ("src.rs".to_string(), measured(60, &head)),
            ]
            .into_iter()
            .collect(),
        ),
        fingerprint: world.fingerprint(),
        head: head.clone(),
        date: "2026-10-06T12:00:00Z".into(),
        survivors: (1..=open).map(survivor).collect(),
        tried: Some(110),
        chain: Chain {
            sample: None,
            fork: None,
            handed: None,
            ran: Some(60),
            scope: Scope::Partial {
                since: earlier.clone(),
            },
            tooling: Some("tools".into()),
            earlier: vec![Link {
                head: earlier.clone(),
                date: "2026-10-05T12:00:00Z".into(),
                scope: Scope::Full { why: String::new() },
                tried: Some(100),
            }],
        },
    };
    // 30 open of 110: 80 killed, below 80%.
    mutants::write(&dir, &chain(30)).unwrap();
    match push::push(&world.project, "m1", true).unwrap_err() {
        PushError::MutantsOwed { owed, .. } => {
            assert!(owed.contains("80 of 110 tried mutant(s) killed"), "{owed}");
        }
        other => panic!("80 of 110 is below 80%: {other}"),
    }
    assert!(world.on_forge("mission/x").is_none());

    // 22 open of 110: 88 killed, 80% exactly, and the push goes.
    mutants::write(&dir, &chain(22)).unwrap();
    push::push(&world.project, "m1", true).unwrap();
}

/// `mission status` says each campaign of the chain, full or partial, from
/// which commit, with what it tried, killed and left.
#[test]
fn status_says_each_campaign_of_the_chain_with_its_counts() {
    use nunki::mutants::{self, Campaign, Chain, Link, Scope};
    let world = World::opened_at(nunki::mission::Rigor::Standard);
    let head = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    let dir = nunki::mission::dir::Paths::of(&world.project.hq_root, "m1").dir;
    let earlier = "e".repeat(40);
    mutants::write(
        &dir,
        &Campaign {
            stopped: None,
            files: None,
            fingerprint: world.fingerprint(),
            head: head.clone(),
            date: "2026-10-06T12:00:00Z".into(),
            survivors: vec![],
            tried: Some(24),
            chain: Chain {
                sample: None,
                fork: None,
                handed: None,
                ran: Some(4),
                scope: Scope::Partial {
                    since: earlier.clone(),
                },
                tooling: Some("tools".into()),
                earlier: vec![Link {
                    head: earlier,
                    date: "2026-10-05T12:00:00Z".into(),
                    scope: Scope::Full {
                        why: "the first campaign of this mission".into(),
                    },
                    tried: Some(20),
                }],
            },
        },
    )
    .unwrap();
    let status = world.printed_by_the_binary(&["mission", "status", "m1"]);
    assert!(
        status.contains(
            "campaign  full at eeeeeeeeeeee (the first campaign of this mission) — tried 20\n"
        ),
        "{status}"
    );
    assert!(
        status.contains(&format!(
            "campaign  partial since eeeeeeeeeeee at {} — tried 4\n",
            &head[..12]
        )),
        "{status}"
    );
    assert!(
        status.contains(&format!(
            "campaign  judged as one campaign at {} — tried 24, killed 24, 0 survivor(s), 0 \
             without an outcome",
            &head[..12]
        )),
        "{status}"
    );
}

/// The reviewer's case (security round 3), on a code-only mission: the
/// gates judged commit X; the agent then commits Y on the mission branch,
/// adding a CI workflow, and detaches `HEAD` at X. What nunki publishes is
/// the judged commit and nothing else: `mission fetch` and `push` both
/// refuse, naming both commits, and nothing reaches the project or the
/// forge.
#[test]
fn only_the_judged_commit_is_fetched_or_pushed() {
    let world = World::new(no_integration(), Security::Gates);
    let judged = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    assert_eq!(world.stage(), Stage::Verified);
    let added = world.commit(".github/workflows/ci.yml", "on: push\n", "ci");
    git(&world.tree, &["checkout", "-q", "--detach", &judged]);
    assert_eq!(nunki::git::slot_head(&world.tree).unwrap(), judged);

    let err = push::fetch(&world.project, "m1").unwrap_err().to_string();
    assert!(err.contains(&judged) && err.contains(&added), "{err}");
    assert!(err.contains("nothing was fetched"), "{err}");

    let err = push::push(&world.project, "m1", true)
        .unwrap_err()
        .to_string();
    assert!(err.contains(&judged) && err.contains(&added), "{err}");
    assert_eq!(world.on_forge("mission/x"), None);
    let in_project = Command::new("git")
        .arg("-C")
        .arg(&world.project.root)
        .args(["rev-parse", "--verify", "--quiet", "refs/heads/mission/x"])
        .output()
        .unwrap();
    assert!(
        !in_project.status.success(),
        "the project received a commit"
    );
}

// --- the final full campaign of a `critical` mission (SPEC 4.4, 4.5) -------

impl World {
    /// A mission with a security agent at `rigor`, coded, `CLEAR` on `HEAD`
    /// and verified, its campaign on file then rewritten by `change`.
    fn verified_then(
        rigor: nunki::mission::Rigor,
        change: impl FnOnce(&mut nunki::mutants::Campaign),
    ) -> Self {
        let world = World::at(rigor);
        world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
        world.security(Verdict::Clear, T1);
        if rigor != nunki::mission::Rigor::Critical {
            world.campaign_on_head();
        }
        assert_eq!(world.stage(), Stage::Verified);
        let dir = nunki::mission::dir::Paths::of(&world.project.hq_root, "m1").dir;
        let mut campaign = nunki::mutants::read(&dir).unwrap().unwrap();
        change(&mut campaign);
        nunki::mutants::write(&dir, &campaign).unwrap();
        world
    }
}

/// The chain holds to the push at `critical`: a partial campaign at the
/// pushed commit that passes is pushed on.
#[test]
fn at_critical_push_takes_a_partial_campaign_at_the_pushed_commit() {
    let world = World::verified_then(nunki::mission::Rigor::Critical, |c| {
        c.chain.scope = nunki::mutants::Scope::Partial {
            since: "e".repeat(40),
        };
    });
    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x"), Some(pushed.head));
}

/// It does not take a campaign that ran on other content than the pushed
/// commit, and names the verb that runs one on it.
#[test]
fn at_critical_push_refuses_a_campaign_on_other_content() {
    let world = World::verified_then(nunki::mission::Rigor::Critical, |c| {
        c.fingerprint = "f".into();
    });
    let err = push::push(&world.project, "m1", true).unwrap_err();
    assert!(matches!(err, PushError::NotAtPushed { .. }), "{err}");
    let said = err.to_string();
    assert!(said.contains("other content"), "{said}");
    assert!(
        said.contains("`nunki mission mutants m1` runs one"),
        "{said}"
    );
    assert!(world.on_forge("mission/x").is_none(), "nothing was pushed");
}

/// A full campaign at the pushed commit that passes is pushed on.
#[test]
fn at_critical_push_takes_a_full_campaign_at_the_pushed_commit() {
    let world = World::verified_then(nunki::mission::Rigor::Critical, |_| {});
    let pushed = push::push(&world.project, "m1", true).unwrap();
    assert_eq!(world.on_forge("mission/x"), Some(pushed.head));
}

/// At `standard`, the chain's partial campaign is what the push stands on,
/// as before.
#[test]
fn at_standard_push_takes_a_partial_campaign() {
    let world = World::verified_then(nunki::mission::Rigor::Standard, |c| {
        c.chain.scope = nunki::mutants::Scope::Partial {
            since: "e".repeat(40),
        };
        c.fingerprint = "f".into();
    });
    push::push(&world.project, "m1", true).unwrap();
}

/// `mission status` prints the volets taken against the cap, and past it how
/// many the HQ granted; `mission retry` on spent volets grants exactly one
/// and says the new count — through the real binary, on a mission handed
/// over at 3 / 3.
#[test]
fn status_counts_the_volets_against_the_cap_and_a_retry_grants_one() {
    let world = World::opened_at(nunki::mission::Rigor::Standard);
    let store = world.store();
    let mut state = world.state();
    let mut events = vec![Event::RunEnded {
        outcome: nunki::harness::Outcome::Finished(Default::default()),
        lot_done: true,
    }];
    for n in 1..=4 {
        events.push(Event::GatesFailed {
            reason: format!("red {n}"),
        });
        if n < 4 {
            events.push(Event::RunEnded {
                outcome: nunki::harness::Outcome::Finished(Default::default()),
                lot_done: true,
            });
        }
    }
    for event in events {
        store.apply(&mut state, event).unwrap();
    }

    let status = world.printed_by_the_binary(&["mission", "status", "m1"]);
    assert!(status.contains("volets    3 / 3\n"), "{status}");

    let retried =
        world.printed_by_the_binary(&["mission", "retry", "m1", "--because", "one more look"]);
    assert!(
        retried.contains("volets    4 / 3 (1 granted by the HQ)"),
        "{retried}"
    );
    assert!(!retried.contains("handed back whole"), "{retried}");
    let status = world.printed_by_the_binary(&["mission", "status", "m1"]);
    assert!(
        status.contains("volets    4 / 3 (1 granted by the HQ)\n"),
        "{status}"
    );
}

/// Put on `m1`'s state, as a write of it would, the departures from the
/// rigor the HQ made.
fn with_overrides_on_file(world: &World, overrides: serde_json::Value) {
    let store = world.store();
    let mut state = world.state();
    let mut flow = serde_json::to_value(&state.flow).unwrap();
    flow.as_object_mut()
        .unwrap()
        .insert("overrides".into(), overrides);
    state.flow = serde_json::from_value(flow).unwrap();
    store.save(&state).unwrap();
}

fn two_overrides() -> serde_json::Value {
    serde_json::json!([
        {
            "departure": {"ReportNotBlocking": {"rigor": "standard", "round": 1, "worst": "MEDIUM"}},
            "because": "the redirect is reachable before login",
            "date": "2026-10-08T09:00:00Z"
        },
        {
            "departure": {"AnotherReview": {"rigor": "standard", "reviews": 1}},
            "because": "the rename broke the API",
            "date": "2026-10-09T09:00:00Z"
        }
    ])
}

/// Every departure from the rigor reaches the pull request, with its
/// reason, after what the mission wrote in `PR.md` — which is left as the
/// agent wrote it.
#[test]
fn the_pull_request_lists_every_departure_from_the_rigor_with_its_reason() {
    let world = World::new(with_wiring(), Security::Agent);
    world.on_github();
    world.with_token();
    world.pr_says("# feat: one returns two\n\nBecause it had to.\n");
    world.ready();
    with_overrides_on_file(&world, two_overrides());

    let (api, server) = serve(vec![(
        201,
        r#"{"html_url":"https://github.com/o/r/pull/9"}"#,
    )]);
    push::push_to(&world.project, "m1", true, &api).unwrap();

    let sent = &server.join().unwrap()[0];
    let body: serde_json::Value = serde_json::from_str(sent.split("\r\n\r\n").nth(1).unwrap())
        .unwrap_or_else(|e| panic!("{e}: {sent}"));
    let body = body["body"].as_str().unwrap();
    assert!(
        body.starts_with("Because it had to.\n\n## Departures from the rigor\n"),
        "{body}"
    );
    assert!(
        body.contains("set the mission's rigor aside 2 time(s)"),
        "{body}"
    );
    assert!(
        body.contains(
            "- 2026-10-08 — at standard, security round 1, a report whose worst finding is \
             MEDIUM does not block — only a HIGH does; set aside because: the redirect is \
             reachable before login"
        ),
        "{body}"
    );
    assert!(
        body.contains("- 2026-10-09 — at standard, a verified mission is sent back on review once"),
        "{body}"
    );
    assert!(
        body.contains("set aside because: the rename broke the API"),
        "{body}"
    );
    let file = std::fs::read_to_string(world.project.hq_root.join("missions/m1/PR.md")).unwrap();
    assert_eq!(file, "# feat: one returns two\n\nBecause it had to.\n");
}

/// With no departure, the body is what `PR.md` says, and nothing more.
#[test]
fn a_pull_request_with_no_departure_carries_no_note() {
    assert_eq!(push::with_overrides("Because.", &[]), "Because.");
}

/// `mission status` lists every departure, one line each, with its reason —
/// through the real binary.
#[test]
fn status_lists_every_departure_from_the_rigor() {
    let world = World::opened_at(nunki::mission::Rigor::Standard);
    with_overrides_on_file(&world, two_overrides());
    let status = world.printed_by_the_binary(&["mission", "status", "m1"]);
    let lines: Vec<&str> = status
        .lines()
        .filter(|l| l.starts_with("override  "))
        .collect();
    assert_eq!(lines.len(), 2, "{status}");
    assert!(
        lines[0].starts_with("override  2026-10-08 — at standard"),
        "{status}"
    );
    assert!(lines[0].ends_with("set aside because: the redirect is reachable before login"));
    assert!(lines[1].ends_with("set aside because: the rename broke the API"));
}

/// `--override` without `--because` is refused by the binary before any
/// verb runs: a departure from the rigor always carries its reason.
#[test]
fn an_override_without_because_is_refused_by_the_binary() {
    let world = World::opened_at(nunki::mission::Rigor::Standard);
    let said = world.printed_by_the_binary(&["mission", "iterate", "m1", "--override"]);
    assert!(said.contains("--because"), "{said}");
    assert!(world.state().flow.overrides().is_empty());
}

// --- the Security section nunki writes (SPEC 4.5) ---------------------------

/// A `PR.md` as the coder and the integrator write it: nothing of the
/// security agent's, which cannot write it.
const PR_AS_WRITTEN: &str = "# feat: one returns two\n\
\n\
Because it had to.\n\
\n\
## Integration\n\
\n\
Wired in compose.yaml.\n";

/// [`PR_AS_WRITTEN`]'s body, as the forge is sent it: its title line is the
/// pull request's title.
const BODY_AS_WRITTEN: &str = "Because it had to.\n\n## Integration\n\nWired in compose.yaml.";

/// What a `CLEAR` says in every world [`World::verified`] stands on.
fn clear_said() -> Said {
    Said {
        report: "Attacked the build; nothing found.".into(),
        findings: Some(nunki::mission::FindingList::Listed(Vec::new())),
    }
}

/// Words of the security agent's report, findings and reasons, found
/// nowhere else in a world: what a withheld section must not hold.
const SECRETS: [&str; 5] = ["zanzibar", "quokka", "marzipan", "wombat", "mailer"];

/// A `FINDINGS` report: one MEDIUM, two LOW with their reasons.
fn findings_said() -> Said {
    let finding = |severity: &str, title: &str, why: Option<&str>| nunki::mission::Finding {
        severity: severity.into(),
        title: title.into(),
        why_acceptable: why.map(str::to_string),
    };
    Said {
        report: "Attacked the login form.\n\
                 Found the zanzibar redirect and the quokka page.\n\
                 \n\
                 Did not look at the mailer.\n"
            .into(),
        findings: Some(nunki::mission::FindingList::Listed(vec![
            finding("MEDIUM", "zanzibar redirect on /login", None),
            finding(
                "LOW",
                "quokka error page",
                Some("it names no marzipan path"),
            ),
            finding("LOW", "no rate limit", Some("the wombat tunnel")),
        ])),
    }
}

/// A `VERDICT.json` of `role` on `head`, made of words the state does not
/// hold: what another role, or a later round, leaves in the file.
fn verdict_file(world: &World, role: &str, head: &str) {
    let verdict = serde_json::json!({
        "role": role,
        "verdict": "FINDINGS",
        "head": head,
        "date": T1,
        "report": "the platypus report",
        "findings": [
            {"severity": "HIGH", "title": "platypus title"},
            {"severity": "LOW", "title": "axolotl", "why_acceptable": "narwhal reason"}
        ]
    });
    std::fs::write(
        world.project.hq_root.join("missions/m1/VERDICT.json"),
        verdict.to_string(),
    )
    .unwrap();
}

/// The words of [`verdict_file`], none of which a section may hold.
const FILE_WORDS: [&str; 4] = ["platypus", "axolotl", "narwhal", "HIGH"];

/// A world at `Verified` on a `FINDINGS` the HQ lifted, on GitHub with a
/// token, its `PR.md` saying `pr`, the verdict having said `said` — `None`
/// for a state an older `nunki` wrote, which kept nothing of it.
fn lifted_findings(pr: &str, said: Option<Said>) -> (World, String) {
    let world = World::at(nunki::mission::Rigor::Critical);
    world.on_github();
    world.with_token();
    world.pr_says(pr);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    if let Some(said) = said {
        world.said(said);
    }
    world.accepted(&lot, T2);
    assert_eq!(world.stage(), Stage::Verified);
    (world, lot)
}

/// Push `world`'s mission against a forge that would open the pull
/// request, and return what push answered and every request the forge
/// received. The forge is asked once more after the push, by the test
/// itself, so that "no request" is read from a forge that was listening.
fn pushed_and_asked(world: &World) -> (Result<push::Pushed, PushError>, Vec<String>) {
    use std::io::{Read, Write};
    let (api, server) = serve(vec![
        (201, r#"{"html_url":"https://github.com/o/r/pull/9"}"#),
        (200, "{}"),
    ]);
    let pushed = push::push_to(&world.project, "m1", true, &api);
    let address = api.strip_prefix("http://").unwrap();
    for _ in 0..2 {
        if let Ok(mut probe) = std::net::TcpStream::connect(address) {
            let _ = probe.write_all(b"GET /the-test-asks HTTP/1.1\r\n\r\n");
            let _ = probe.read_to_end(&mut Vec::new());
        }
    }
    let seen = server
        .join()
        .unwrap()
        .into_iter()
        .filter(|r| !r.starts_with("GET /the-test-asks"))
        .collect();
    (pushed, seen)
}

/// The one pull request the forge was asked to open, as JSON.
fn request_sent(seen: &[String]) -> serde_json::Value {
    assert_eq!(seen.len(), 1, "{seen:?}");
    serde_json::from_str(seen[0].split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

/// Push `world`'s mission, and return what push answered, the title and
/// the body the forge was sent.
fn pushed_request(world: &World) -> (push::Pushed, String, String) {
    let (outcome, seen) = pushed_and_asked(world);
    let request = request_sent(&seen);
    (
        outcome.unwrap(),
        request["title"].as_str().unwrap().to_string(),
        request["body"].as_str().unwrap().to_string(),
    )
}

fn pr_on_disk(world: &World) -> String {
    std::fs::read_to_string(world.project.hq_root.join("missions/m1/PR.md")).unwrap()
}

/// No security agent: the body is `PR.md`'s, as written, and nothing more —
/// not even from a security verdict left in the state.
#[test]
fn without_a_security_agent_the_body_is_pr_md_as_written_and_holds_no_section() {
    let world = World::new(with_wiring(), Security::Gates);
    world.on_github();
    world.with_token();
    world.pr_says(PR_AS_WRITTEN);
    world.ready();
    let (pushed, title, body) = pushed_request(&world);
    assert_eq!(title, "feat: one returns two");
    assert_eq!(body, BODY_AS_WRITTEN);
    assert_eq!(pushed.withheld, None);
}

/// On a `CLEAR`, `PR.md` as written, then nunki's section: the commit
/// attacked, and the report as the agent wrote it, quoted line by line —
/// a lone `\r`, which the forge reads as a line break, shown as its escape
/// so that no line of the report starts outside the quote. Whatever the
/// project says about findings: a `CLEAR` has none to withhold.
#[test]
fn on_a_clear_the_section_names_the_commit_and_quotes_the_report() {
    for setting in [
        nunki::project::SecurityFindings::Withhold,
        nunki::project::SecurityFindings::Publish,
    ] {
        let mut world = World::new(with_wiring(), Security::Agent);
        world.project.config.security_findings = setting;
        world.on_github();
        world.with_token();
        world.pr_says(PR_AS_WRITTEN);
        let head = world.ready();
        world.said(Said {
            report: "Attacked the login form.\nNothing found.\r## Not a heading\n\n\
                     Did not look at the mailer.\n"
                .into(),
            findings: Some(nunki::mission::FindingList::Listed(Vec::new())),
        });
        let (pushed, title, body) = pushed_request(&world);

        assert_eq!(title, "feat: one returns two");
        assert_eq!(
            body,
            format!(
                "{BODY_AS_WRITTEN}\n\n## Security\n\n\
                 The security agent attacked commit `{head}` and concluded CLEAR.\n\n\
                 Its report, as it wrote it:\n\n\
                 > Attacked the login form.\n\
                 > Nothing found.\\u{{d}}## Not a heading\n\
                 >\n\
                 > Did not look at the mailer."
            )
        );
        assert_eq!(pushed.withheld, None);
        assert_eq!(pr_on_disk(&world), PR_AS_WRITTEN);
    }
}

/// On a lifted `FINDINGS` the project withholds (the default): `PR.md` as
/// written, then the note — how many findings per severity, accepted by
/// whom, withheld until fixed — and not one word of the report, of a
/// finding's title or of a reason, in the title or in the body.
#[test]
fn on_a_withheld_findings_the_section_is_the_note_and_no_word_of_the_agents() {
    let (world, _) = lifted_findings(PR_AS_WRITTEN, Some(findings_said()));
    let (pushed, title, body) = pushed_request(&world);

    assert_eq!(title, "feat: one returns two");
    assert_eq!(
        body,
        format!(
            "{BODY_AS_WRITTEN}\n\n## Security\n\n\
             The security review of this branch reported 3 finding(s) — 1 MEDIUM, 2 LOW. They \
             were accepted, not fixed — by the HQ (Alex Martin) — and their details are \
             withheld from this pull request until they are fixed."
        )
    );
    for secret in SECRETS {
        assert!(!title.contains(secret), "{secret:?} in the title: {title}");
        assert!(!body.contains(secret), "{secret:?} published:\n{body}");
    }
    assert_eq!(pushed.withheld, Some(push::Withheld { findings: Some(3) }));
    assert_eq!(pr_on_disk(&world), PR_AS_WRITTEN);
}

/// On a lifted `FINDINGS` the project publishes: who accepted it, each
/// finding with its severity, its title and its reason, then the report.
#[test]
fn on_a_published_findings_the_section_names_who_accepted_each_finding_and_the_report() {
    let (mut world, lot) = lifted_findings(PR_AS_WRITTEN, Some(findings_said()));
    world.project.config.security_findings = nunki::project::SecurityFindings::Publish;
    let (pushed, _, body) = pushed_request(&world);

    assert_eq!(
        body,
        format!(
            "{BODY_AS_WRITTEN}\n\n## Security\n\n\
             The security agent attacked commit `{lot}` and concluded FINDINGS. They were \
             accepted, not fixed — by the HQ (Alex Martin).\n\n\
             Its findings:\n\n\
             - MEDIUM: zanzibar redirect on /login\n\
             - LOW: quokka error page — accepted because: it names no marzipan path\n\
             - LOW: no rate limit — accepted because: the wombat tunnel\n\n\
             Its report, as it wrote it:\n\n\
             > Attacked the login form.\n\
             > Found the zanzibar redirect and the quokka page.\n\
             >\n\
             > Did not look at the mailer."
        )
    );
    assert_eq!(pushed.withheld, None);
    assert_eq!(pr_on_disk(&world), PR_AS_WRITTEN);
}

/// A `FINDINGS` nunki lifted itself, withheld: the note names nunki as who
/// accepted it, and holds nothing of the findings but their count.
#[test]
fn a_findings_lifted_by_nunki_is_withheld_naming_nunki() {
    let world = World::at(nunki::mission::Rigor::Critical);
    world.on_github();
    world.with_token();
    world.pr_says(PR_AS_WRITTEN);
    world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.said(Said {
        report: "the quokka report".into(),
        findings: Some(nunki::mission::FindingList::Listed(vec![
            nunki::mission::Finding {
                severity: "LOW".into(),
                title: "zanzibar page".into(),
                why_acceptable: Some("marzipan".into()),
            },
            nunki::mission::Finding {
                severity: "INFO".into(),
                title: "wombat".into(),
                why_acceptable: Some("mailer".into()),
            },
        ])),
    });
    world.lifted_by_nunki();
    let (pushed, _, body) = pushed_request(&world);

    assert!(
        body.ends_with(
            "## Security\n\nThe security review of this branch reported 2 finding(s) — 1 LOW, \
             1 INFO. They were accepted, not fixed — by nunki, every one being LOW or INFO — \
             and their details are withheld from this pull request until they are fixed."
        ),
        "{body}"
    );
    for secret in SECRETS {
        assert!(!body.contains(secret), "{secret:?} published:\n{body}");
    }
    assert_eq!(pushed.withheld, Some(push::Withheld { findings: Some(2) }));
}

/// Where the push says that later commits were not attacked by the
/// security agent, its section says so too, naming them — on a `CLEAR` and
/// on a withheld `FINDINGS` alike, before anything of the agent's.
#[test]
fn the_section_names_the_commits_the_security_agent_did_not_attack() {
    // A CLEAR at `standard`, its one round spent, then a review and a volet.
    let world = World::at(nunki::mission::Rigor::Standard);
    world.on_github();
    world.with_token();
    world.pr_says(PR_AS_WRITTEN);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Clear, T1);
    world.said(clear_said());
    world.review();
    let volet = world.coded("pub fn one() -> u8 { 3 }\n", "the volet");
    let (pushed, _, body) = pushed_request(&world);
    assert_eq!(pushed.not_attacked, vec![named(&volet, "the volet")]);
    assert_eq!(
        body,
        format!(
            "{BODY_AS_WRITTEN}\n\n## Security\n\n\
             The security agent attacked commit `{lot}` and concluded CLEAR.\n\n\
             The security agent's rounds were spent, and it did not attack the commits pushed \
             after it:\n\n\
             - {}\n\n\
             Its report, as it wrote it:\n\n\
             > Attacked the build; nothing found.",
            named(&volet, "the volet")
        )
    );

    // A FINDINGS lifted at the cap, then a review and a volet: withheld.
    let world = World::at(nunki::mission::Rigor::Standard);
    world.on_github();
    world.with_token();
    world.pr_says(PR_AS_WRITTEN);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.said(findings_said());
    world.accepted(&lot, T2);
    world.review();
    let volet = world.coded("pub fn one() -> u8 { 3 }\n", "the volet");
    let (pushed, _, body) = pushed_request(&world);
    assert_eq!(pushed.not_attacked, vec![named(&volet, "the volet")]);
    assert_eq!(
        body,
        format!(
            "{BODY_AS_WRITTEN}\n\n## Security\n\n\
             The security review of this branch reported 3 finding(s) — 1 MEDIUM, 2 LOW. They \
             were accepted, not fixed — by the HQ (Alex Martin) — and their details are \
             withheld from this pull request until they are fixed.\n\n\
             The security agent's rounds were spent, and it did not attack the commits pushed \
             after it:\n\n\
             - {}",
            named(&volet, "the volet")
        )
    );
}

/// The section is written from what `nunki` kept when the verdict was
/// concluded, and never from `VERDICT.json`, which every role rewrites. A
/// file that is another role's, or another commit's, or even the security
/// agent's on the very commit, gives the section nothing: with nothing kept
/// it says it could not read the count, and holds no word of the file; with
/// something kept, it is what was kept.
#[test]
fn a_verdict_file_is_never_read_into_the_section() {
    for (role, on_the_lot) in [
        ("Integrator", true),
        ("Security", false),
        ("Security", true),
    ] {
        // A state an older nunki wrote: nothing kept.
        let (world, lot) = lifted_findings(PR_AS_WRITTEN, None);
        let head = if on_the_lot {
            lot.clone()
        } else {
            "0000000000000000000000000000000000000000".to_string()
        };
        verdict_file(&world, role, &head);
        let (pushed, _, body) = pushed_request(&world);
        assert_eq!(
            body,
            format!(
                "{BODY_AS_WRITTEN}\n\n## Security\n\n\
                 The security review of this branch reported findings whose number nunki \
                 could not read from the security agent's verdict. They were accepted, not \
                 fixed — by the HQ (Alex Martin) — and their details are withheld from this \
                 pull request until they are fixed."
            ),
            "{role} on {head}"
        );
        assert_eq!(pushed.withheld, Some(push::Withheld { findings: None }));

        // What was kept, whatever the file says.
        let (world, _) = lifted_findings(PR_AS_WRITTEN, Some(findings_said()));
        verdict_file(&world, role, &head);
        let (pushed, _, body) = pushed_request(&world);
        assert!(
            body.contains("reported 3 finding(s) — 1 MEDIUM, 2 LOW."),
            "{body}"
        );
        for word in FILE_WORDS {
            assert!(!body.contains(word), "{word:?} of the file:\n{body}");
        }
        assert_eq!(pushed.withheld, Some(push::Withheld { findings: Some(3) }));
    }
}

/// What `nunki` did not keep is said to be unreadable on every form of the
/// section — a `CLEAR`'s report, a published `FINDINGS`'s findings and
/// report — and a verdict file on the very commit is not read instead.
#[test]
fn what_nunki_did_not_keep_is_said_unreadable_and_not_read_from_the_file() {
    let world = World::new(with_wiring(), Security::Agent);
    world.on_github();
    world.with_token();
    world.pr_says(PR_AS_WRITTEN);
    let head = world.ready();
    let mut state = world.state();
    state.verdicts.iter_mut().for_each(|c| c.said = None);
    world.store().save(&state).unwrap();
    verdict_file(&world, "Security", &head);
    let (_, _, body) = pushed_request(&world);
    assert_eq!(
        body,
        format!(
            "{BODY_AS_WRITTEN}\n\n## Security\n\n\
             The security agent attacked commit `{head}` and concluded CLEAR.\n\n\
             Its report could not be read: nunki kept none when the verdict was concluded."
        )
    );

    let (mut world, lot) = lifted_findings(PR_AS_WRITTEN, None);
    world.project.config.security_findings = nunki::project::SecurityFindings::Publish;
    verdict_file(&world, "Security", &lot);
    let (_, _, body) = pushed_request(&world);
    assert_eq!(
        body,
        format!(
            "{BODY_AS_WRITTEN}\n\n## Security\n\n\
             The security agent attacked commit `{lot}` and concluded FINDINGS. They were \
             accepted, not fixed — by the HQ (Alex Martin).\n\n\
             Its findings could not be read: nunki kept no list it can read with the verdict.\n\n\
             Its report could not be read: nunki kept none when the verdict was concluded."
        )
    );
}

/// A published `FINDINGS` whose list was kept but lists nothing says so,
/// rather than an empty heading.
#[test]
fn a_published_findings_with_an_empty_list_says_none_listed() {
    let (mut world, _) = lifted_findings(
        PR_AS_WRITTEN,
        Some(Said {
            report: "r".into(),
            findings: Some(nunki::mission::FindingList::Listed(Vec::new())),
        }),
    );
    world.project.config.security_findings = nunki::project::SecurityFindings::Publish;
    let (_, _, body) = pushed_request(&world);
    assert!(
        body.ends_with("Its findings: none listed.\n\nIts report, as it wrote it:\n\n> r"),
        "{body}"
    );
}

/// With no `PR.md`, or an empty one, there is no pull request: a push that
/// must withhold goes through as one that withholds nothing does — the
/// branch pushed, no pull request opened, and it says it opened none.
#[test]
fn a_push_that_withholds_with_no_pull_request_file_pushes_the_branch_and_opens_none() {
    for written in [None, Some(""), Some("\n  \n")] {
        let (world, lot) = lifted_findings(PR_AS_WRITTEN, Some(findings_said()));
        let pr = world.project.hq_root.join("missions/m1/PR.md");
        match written {
            None => std::fs::remove_file(&pr).unwrap(),
            Some(text) => std::fs::write(&pr, text).unwrap(),
        }
        let (outcome, seen) = pushed_and_asked(&world);
        let pushed = outcome.unwrap_or_else(|e| panic!("{written:?}: {e}"));
        assert!(
            seen.is_empty(),
            "{written:?}: the forge was asked: {seen:?}"
        );
        assert_eq!(world.on_forge("mission/x").as_deref(), Some(lot.as_str()));
        match &pushed.pull_request {
            push::PullRequestState::ByHand { why, .. } => {
                assert!(why.contains("PR.md"), "{why}");
                assert!(
                    why.contains("nothing a pull request could be titled with"),
                    "{why}"
                );
            }
            other => panic!("{written:?}: {other:?}"),
        }
    }
}

/// `nunki push`, through the real binary, with no `PR.md` over a push that
/// must withhold: it pushes, and says it opened no pull request.
#[test]
fn push_says_it_opened_no_pull_request_when_there_is_no_pull_request_file() {
    let world = World::opened_at(nunki::mission::Rigor::Critical);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.said(findings_said());
    world.accepted(&lot, T2);
    std::fs::remove_file(world.project.hq_root.join("missions/m1/PR.md")).unwrap();
    let out = world.pushed_by_the_binary();
    assert!(out.contains("pushed    mission/x"), "{out}");
    assert!(out.contains("pull request not opened by nunki"), "{out}");
    assert!(
        out.contains("says nothing a pull request could be titled with"),
        "{out}"
    );
    assert_eq!(world.on_forge("mission/x").as_deref(), Some(lot.as_str()));
}

/// `nunki push`, through the real binary, says what it withheld and on how
/// many findings, in words that do not describe a cut: nothing is cut.
#[test]
fn push_says_what_it_withheld() {
    let world = World::opened_at(nunki::mission::Rigor::Critical);
    world.pr_says(PR_AS_WRITTEN);
    let lot = world.coded("pub fn one() -> u8 { 2 }\n", "the lot");
    world.security(Verdict::Findings, T1);
    world.said(findings_said());
    world.accepted(&lot, T2);
    let out = world.pushed_by_the_binary();
    assert!(
        out.contains(
            "security  withheld from the pull request: the security agent's report and its 3 \
             finding(s), accepted and not fixed — the Security section nunki wrote says how \
             many, and who accepted them"
        ),
        "{out}"
    );
    assert!(!out.contains("section(s) of PR.md"), "{out}");
    assert!(
        out.contains("the findings stay withheld: publish nothing of the security agent's report"),
        "{out}"
    );
}

/// The Security section comes after the note on the departures from the
/// rigor: the agent's prose is the last thing in the body, so nothing
/// `nunki` writes follows it.
#[test]
fn the_security_section_comes_after_the_departures_from_the_rigor() {
    let world = World::new(with_wiring(), Security::Agent);
    world.on_github();
    world.with_token();
    world.pr_says(PR_AS_WRITTEN);
    world.ready();
    with_overrides_on_file(&world, two_overrides());
    let (_, _, body) = pushed_request(&world);
    let departures = body.find("## Departures from the rigor").expect("the note");
    let security = body.find("## Security").expect("the section");
    assert!(departures < security, "{body}");
    assert!(body.starts_with(BODY_AS_WRITTEN), "{body}");
    assert!(
        body.ends_with("> Attacked the build; nothing found."),
        "{body}"
    );
}

/// The note counts a finding whose severity it cannot rank as unranked,
/// beside those it can.
#[test]
fn the_note_counts_findings_it_cannot_rank_as_unranked() {
    let verdict: nunki::mission::VerdictFile = serde_json::from_value(serde_json::json!({
        "role": "Security",
        "verdict": "FINDINGS",
        "head": "abc",
        "date": T1,
        "findings": [
            {"severity": "HIGH", "title": "one"},
            {"severity": "SEVERE", "title": "two"},
            {"severity": "", "title": "three"}
        ]
    }))
    .unwrap();
    let (note, count) = push::withheld_note(verdict.findings.as_ref(), &["the HQ (A)".into()]);
    assert!(
        note.contains("reported 3 finding(s) — 1 HIGH, 2 unranked."),
        "{note}"
    );
    assert_eq!(count, Some(3));
}

/// The note names only the severities it counted, and no `unranked` when
/// every finding is ranked; an empty list is a count and nothing more.
#[test]
fn the_note_names_no_severity_it_did_not_count() {
    let note = |findings: serde_json::Value| {
        let verdict: nunki::mission::VerdictFile = serde_json::from_value(serde_json::json!({
            "role": "Security",
            "verdict": "FINDINGS",
            "head": "abc",
            "date": T1,
            "findings": findings
        }))
        .unwrap();
        push::withheld_note(verdict.findings.as_ref(), &[]).0
    };
    let said = note(serde_json::json!([
        {"severity": "HIGH", "title": "one"},
        {"severity": "LOW", "title": "two"}
    ]));
    assert!(
        said.contains("reported 2 finding(s) — 1 HIGH, 1 LOW. They"),
        "{said}"
    );
    let said = note(serde_json::json!([]));
    assert!(said.contains("reported 0 finding(s). They"), "{said}");
    assert!(said.contains("by nobody nunki can name"), "{said}");
}

// --- the one transition case: a section an older nunki's agent wrote -------

/// A `PR.md` whose line `n` is `line`, followed by a finding of the kind a
/// security agent wrote there under an older `nunki`.
fn pr_holding(line: &str) -> String {
    format!(
        "# feat: one returns two\n\nBecause it had to.\n\n{line}\nthe zanzibar finding\n\n\
         ## Integration\n\nWired.\n"
    )
}

/// The forms the transition refusal catches, each with the line it names.
fn refused_forms() -> Vec<(String, &'static str)> {
    vec![
        (
            pr_holding("## Security review"),
            "line 5: ## Security review",
        ),
        (pr_holding("## security"), "line 5: ## security"),
        (pr_holding("## **Security**"), "line 5: ## **Security**"),
        (pr_holding("> ## Security"), "line 5: > ## Security"),
        (pr_holding("\t## Security"), "line 5: ## Security"),
        (
            "# feat: one returns two\n\n```x``` is inline.\n## Security\nthe zanzibar finding\n"
                .to_string(),
            "line 4: ## Security",
        ),
        (
            "# Security review of the branch\n\nthe zanzibar finding\n".to_string(),
            "line 1: # Security review of the branch",
        ),
        // `\r\n` is one line break, and the lines are counted so.
        (
            "# feat: one returns two\r\n\r\n## Security\r\nthe zanzibar finding\r\n".to_string(),
            "line 3: ## Security",
        ),
        // A lone `\r` is a line break to the forge, and to the rule.
        (
            "# feat: one returns two\r\rIntro.\r\r## Security\r\rthe zanzibar finding\r"
                .to_string(),
            "line 5: ## Security",
        ),
    ]
}

/// A push that must withhold, over a `PR.md` holding a line where a `#`
/// comes before the first letter and the first letters are `security`, in
/// any case: refused before anything leaves, naming the line and the two
/// ways past. Nothing is cut, and `PR.md` is left as it is.
#[test]
fn a_push_that_must_withhold_refuses_a_pr_md_holding_a_security_heading() {
    for (written, line) in refused_forms() {
        let (world, _) = lifted_findings(&written, Some(findings_said()));
        let (outcome, seen) = pushed_and_asked(&world);
        let err = match outcome {
            Err(e) => e,
            Ok(pushed) => panic!("{written:?} pushed: {pushed:?}"),
        };
        assert!(
            matches!(err, PushError::SecurityHeadingInPr { .. }),
            "{written:?}: {err}"
        );
        let said = err.to_string();
        for part in [
            line,
            "PR.md",
            "the HQ may edit it",
            "security_findings: publish",
            "nothing was pushed",
        ] {
            assert!(said.contains(part), "{written:?}: {part:?} not in: {said}");
        }
        assert!(
            seen.is_empty(),
            "{written:?}: the forge was asked: {seen:?}"
        );
        assert_eq!(world.on_forge("mission/x"), None, "{written:?}");
        assert_eq!(pr_on_disk(&world), written);
    }
}

/// Every line that fires is named, so the HQ can fix them in one pass.
#[test]
fn the_refusal_names_every_line_that_fires() {
    let written = "# feat: one returns two\n\n## Security review\n\nx\n\n### security round 2\n";
    let (world, _) = lifted_findings(written, Some(findings_said()));
    let err = push::push(&world.project, "m1", true).unwrap_err();
    let said = err.to_string();
    assert!(
        said.contains("line 3: ## Security review; line 7: ### security round 2"),
        "{said}"
    );
}

/// What the rule does not fire on, measured on past missions' `PR.md`:
/// prose that wraps onto `security`, a table row, a list item naming the
/// setting, a heading that begins with another word. Such a push goes
/// through: `PR.md` as written, then the withheld section.
#[test]
fn a_push_that_must_withhold_sends_lines_that_only_mention_security() {
    for line in [
        "The rounds were spent, and the\nsecurity agent did not attack the volet.",
        "| Security agent rounds | 3 |",
        "- `security_findings: publish` is not set here",
        "## The security round",
    ] {
        let written = pr_holding(line);
        let (world, _) = lifted_findings(&written, Some(findings_said()));
        let (pushed, _, body) = pushed_request(&world);
        let as_written = written
            .strip_prefix("# feat: one returns two\n\n")
            .unwrap()
            .trim_end();
        assert!(
            body.starts_with(&format!(
                "{as_written}\n\n## Security\n\nThe security review"
            )),
            "{line:?}:\n{body}"
        );
        assert_eq!(pushed.withheld, Some(push::Withheld { findings: Some(3) }));
    }
}

/// The refusal is only for a push that must withhold: over a `CLEAR`, or a
/// `FINDINGS` the project publishes, the same `PR.md` is sent as written.
#[test]
fn a_push_that_withholds_nothing_sends_the_same_pr_md_as_written() {
    for (written, _) in refused_forms() {
        let world = World::new(with_wiring(), Security::Agent);
        world.on_github();
        world.with_token();
        world.pr_says(&written);
        world.ready();
        let (outcome, seen) = pushed_and_asked(&world);
        let pushed = outcome.unwrap_or_else(|e| panic!("CLEAR, {written:?}: {e}"));
        assert_eq!(pushed.withheld, None);
        // Title and body together: as written, whichever line the title
        // was taken from.
        let request = request_sent(&seen);
        let body = request["body"].as_str().unwrap().to_string();
        assert!(
            request.to_string().contains("the zanzibar finding"),
            "{written:?}:\n{request}"
        );
        assert!(body.contains("concluded CLEAR"), "{body}");

        let (mut world, _) = lifted_findings(&written, Some(findings_said()));
        world.project.config.security_findings = nunki::project::SecurityFindings::Publish;
        let (outcome, seen) = pushed_and_asked(&world);
        let pushed = outcome.unwrap_or_else(|e| panic!("publish, {written:?}: {e}"));
        assert_eq!(pushed.withheld, None);
        // Title and body together: as written, whichever line the title
        // was taken from.
        let request = request_sent(&seen);
        let body = request["body"].as_str().unwrap().to_string();
        assert!(
            request.to_string().contains("the zanzibar finding"),
            "{written:?}:\n{request}"
        );
        assert!(body.contains("concluded FINDINGS"), "{body}");
    }
}

/// The stated limit, pinned so that nobody reads the refusal as promising
/// more: a Security heading in a form the rule does not catch — HTML, or
/// `Security` underlined with `===` — is sent as written by a push that
/// must withhold. What the coder or the integrator writes in `PR.md` is
/// published as written; the guard is that the security agent cannot write
/// it, and the HQ reads `PR.md` before a push.
#[test]
fn the_stated_limit_an_html_or_underlined_security_heading_is_sent_as_written() {
    for line in ["<h2>Security</h2>", "Security\n========"] {
        let written = pr_holding(line);
        let (world, _) = lifted_findings(&written, Some(findings_said()));
        let (pushed, _, body) = pushed_request(&world);
        assert!(
            body.contains(&format!("{line}\nthe zanzibar finding")),
            "{line:?}:\n{body}"
        );
        assert_eq!(pushed.withheld, Some(push::Withheld { findings: Some(3) }));
    }
}

impl World {
    /// A human's lift of the verdict as a whole on `head` at `date`, as
    /// `nunki mission accept --fixed-by <fix>` records it, or as a plain
    /// `accept` when `fix` is `None` (#162). The flow moves when it is on the
    /// findings.
    fn lifted_as(&self, fix: Option<&str>, head: &str, date: &str) {
        let mut state = self.state();
        state.accepted.push(nunki::state::Accepted {
            finding: None,
            why: match fix {
                Some(_) => "probed the redirect on the host: it is refused".into(),
                None => "behind the VPN".into(),
            },
            who: "Alex Martin".into(),
            head: head.to_string(),
            date: date.to_string(),
            by_nunki: false,
            fixed_by: fix.map(str::to_string),
        });
        self.store().save(&state).unwrap();
        if matches!(self.stage(), Stage::Findings { .. }) {
            self.event(Event::HumanAccepted);
        }
    }

    /// #162's path at `standard`: the one round concludes FINDINGS, the HQ
    /// sends it back, the coder fixes it, and the mission is back on that
    /// report with no round left. The fix.
    fn fixed_after_the_last_round(&self) -> String {
        self.on_github();
        self.with_token();
        self.pr_says(PR_AS_WRITTEN);
        self.coded("pub fn one() -> u8 { 2 }\n", "the lot");
        self.security(Verdict::Findings, T1);
        self.said(findings_said());
        self.event(Event::Iterate);
        let fix = self.coded("pub fn one() -> u8 { 3 }\n", "the fix");
        assert!(
            matches!(self.stage(), Stage::Findings { .. }),
            "{:?}",
            self.stage()
        );
        fix
    }
}

/// #162. A finding fixed after the last round and lifted as such is pushed
/// as fixed by its commit, not attacked by the agent and checked by the HQ —
/// never as "accepted, not fixed". Withheld, as the project withholds every
/// finding by default: the count, the commit and who checked it, and no word
/// of the report, of a finding or of the HQ's reason, which can describe the
/// finding as well as the fix; and no promise to publish "once fixed". The
/// fixing commit is still named as not attacked.
#[test]
fn a_finding_fixed_after_the_last_round_is_pushed_as_fixed_and_still_withheld() {
    let world = World::at(nunki::mission::Rigor::Standard);
    let fix = world.fixed_after_the_last_round();
    world.lifted_as(Some(&fix), &fix, T2);

    let (pushed, _, body) = pushed_request(&world);

    assert_eq!(pushed.not_attacked, vec![named(&fix, "the fix")]);
    assert_eq!(
        body,
        format!(
            "{BODY_AS_WRITTEN}\n\n## Security\n\n\
             The security review of this branch reported 3 finding(s) — 1 MEDIUM, 2 LOW. They \
             were fixed after the security agent's last round, by commit `{}`, which the agent \
             did not attack; the HQ (Alex Martin) checked the fix. Their details are withheld \
             from this pull request.\n\n\
             The security agent's rounds were spent, and it did not attack the commits pushed \
             after it:\n\n\
             - {}",
            &fix[..12],
            named(&fix, "the fix")
        )
    );
    for word in SECRETS.iter().chain(&["probed the redirect"]) {
        assert!(!body.contains(word), "{word:?}:\n{body}");
    }
}

/// Published, the same lift says how the HQ checked the fix, then the
/// findings and the report, as any published section does.
#[test]
fn a_published_finding_fixed_after_the_last_round_says_how_the_hq_checked_it() {
    let mut world = World::at(nunki::mission::Rigor::Standard);
    world.project.config.security_findings = nunki::project::SecurityFindings::Publish;
    let fix = world.fixed_after_the_last_round();
    world.lifted_as(Some(&fix), &fix, T2);

    let (_, _, body) = pushed_request(&world);

    assert!(
        body.contains(&format!(
            "They were fixed after its last round, by commit `{}`, which it did not attack; \
             the HQ (Alex Martin) checked the fix: probed the redirect on the host: it is \
             refused",
            &fix[..12]
        )),
        "{body}"
    );
    assert!(!body.contains("accepted, not fixed"), "{body}");
    assert!(
        body.contains("zanzibar"),
        "the published findings follow:\n{body}"
    );
}

/// When more than one lift follows the last report, the latest decides
/// whether it was fixed or accepted: a fix named after a plain `accept` is
/// the later word, and a plain `accept` after a fix takes the risk back.
#[test]
fn the_latest_lift_after_the_last_round_decides_fixed_or_accepted() {
    let world = World::at(nunki::mission::Rigor::Standard);
    let fix = world.fixed_after_the_last_round();
    world.lifted_as(None, &fix, T2);
    world.lifted_as(Some(&fix), &fix, T3);
    let (_, _, body) = pushed_request(&world);
    assert!(
        body.contains("They were fixed after the security agent's last round"),
        "{body}"
    );
    assert!(!body.contains("accepted, not fixed"), "{body}");

    let world = World::at(nunki::mission::Rigor::Standard);
    let fix = world.fixed_after_the_last_round();
    world.lifted_as(Some(&fix), &fix, T2);
    world.lifted_as(None, &fix, T3);
    let (_, _, body) = pushed_request(&world);
    assert!(body.contains("accepted, not fixed"), "{body}");
    assert!(!body.contains("They were fixed"), "{body}");
}
