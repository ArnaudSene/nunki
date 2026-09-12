//! The verification gates 1 to 4 (SPEC 4.4), against real git repositories.
//!
//! Every test here builds an actual repository in a temporary directory and
//! makes actual commits. A gate that reads git is worth what it reads, and a
//! fake would only prove that the fake agrees with the code.

use std::path::{Path, PathBuf};
use std::process::Command;

use hq::gate::{self, Decision, Gate, Subject};
use hq::harness::Role;
use hq::mission::{Bounds, Header, Integration, Lot, Security, Service};
use hq::project::ProtectedPaths;

/// Run git in `at`, with an identity so commits work on a bare machine.
fn git(at: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["-c", "user.name=Gate Test", "-c", "user.email=gate@test"])
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

fn write(at: &Path, path: &str, body: &str) {
    let full = at.join(path);
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(full, body).unwrap();
}

/// A repository with a `dev` base holding `AGENTS.md` and one source file,
/// and a mission branch checked out from it.
fn repo(dir: &Path) -> PathBuf {
    let tree = dir.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    git(&tree, &["init", "-q", "-b", "dev"]);
    write(&tree, "AGENTS.md", "the rules of this place\n");
    write(&tree, "deny.toml", "[bans]\n");
    write(&tree, ".github/workflows/ci.yml", "name: ci\n");
    write(&tree, "src/lib.rs", "pub fn one() -> u8 { 1 }\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "base"]);
    git(&tree, &["checkout", "-q", "-b", "mission/x"]);
    tree
}

fn header() -> Header {
    Header {
        branch: "mission/x".into(),
        base: "dev".into(),
        lots: vec![Lot {
            id: "L1".into(),
            title: "do the thing".into(),
        }],
        integration: Integration::None {
            reason: "no external service is involved".into(),
        },
        security: Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Bounds::default(),
    }
}

fn protected() -> ProtectedPaths {
    ProtectedPaths {
        refuse: vec![
            ".github/workflows/**".into(),
            "vendor/**".into(),
            "AGENTS.md".into(),
            "deny.toml".into(),
        ],
        refuse_if_exists: vec!["src/*.rs".into()],
    }
}

/// A journal whose resume block names `head`.
fn journal(dir: &Path, head: &str) -> PathBuf {
    let path = dir.join("JOURNAL.md");
    std::fs::write(
        &path,
        format!("# Journal\n\n## ÉTAT DE REPRISE\n\nHEAD is `{head}`. Lot L1 done.\n\n## Notes\n\nolder work\n"),
    )
    .unwrap();
    path
}

struct Fixture {
    _dir: tempfile::TempDir,
    tree: PathBuf,
    journal: PathBuf,
    pr: PathBuf,
    verdict: PathBuf,
    header: Header,
    protected: ProtectedPaths,
    branches: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let tree = repo(dir.path());
        let journal = journal(dir.path(), "0000000");
        let pr = dir.path().join("PR.md");
        let verdict = dir.path().join("VERDICT.json");
        Self {
            pr,
            verdict,
            _dir: dir,
            tree,
            journal,
            header: header(),
            protected: protected(),
            branches: vec!["main".into(), "dev".into()],
        }
    }

    /// Rewrite the journal so its block names the current HEAD, which is what
    /// an agent honouring the run contract does.
    fn journal_names_head(&self) {
        let head = git(&self.tree, &["rev-parse", "HEAD"]);
        std::fs::write(
            &self.journal,
            format!("# Journal\n\n## ÉTAT DE REPRISE\n\nHEAD is `{head}`. Lot L1 done.\n"),
        )
        .unwrap();
    }

    fn gates(&self, role: Role) -> gate::Report {
        gate::after_run(&Subject {
            role,
            tree: &self.tree,
            journal: &self.journal,
            pr: &self.pr,
            verdict: &self.verdict,
            mission_dir: self._dir.path(),
            header: &self.header,
            protected_branches: &self.branches,
            protected_paths: &self.protected,
        })
        .unwrap()
    }

    fn decision(&self, role: Role, gate: Gate) -> Decision {
        self.gates(role)
            .outcomes
            .into_iter()
            .find(|o| o.gate == gate)
            .expect("every gate is reported")
            .decision
    }
}

fn commit(tree: &Path, path: &str, body: &str, message: &str) {
    write(tree, path, body);
    git(tree, &["add", "-A"]);
    git(tree, &["commit", "-q", "-m", message]);
}

#[test]
fn a_run_that_honoured_its_contract_passes_all_four() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.journal_names_head();
    let report = f.gates(Role::Coder);
    assert!(report.passed(), "{:?}", report.failure());
    assert_eq!(report.outcomes.len(), 4);
}

#[test]
fn an_uncommitted_change_is_red_and_the_reason_names_it() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.journal_names_head();
    write(&f.tree, "src/half.rs", "fn unfinished(\n");
    match f.decision(Role::Coder, Gate::CleanTree) {
        Decision::Failed(why) => assert!(why.contains("src/half.rs"), "{why}"),
        other => panic!("a half-written file is not a clean tree: {other:?}"),
    }
}

#[test]
fn a_protected_branch_is_refused_and_so_is_one_that_is_not_ahead() {
    let f = Fixture::new();
    // Not ahead: the branch exists but carries nothing.
    f.journal_names_head();
    match f.decision(Role::Coder, Gate::BranchAhead) {
        Decision::Failed(why) => assert!(why.contains("no commit"), "{why}"),
        other => panic!("a branch with nothing on it is not ahead: {other:?}"),
    }

    // On a protected branch, whatever it carries.
    git(&f.tree, &["checkout", "-q", "dev"]);
    commit(
        &f.tree,
        "src/new.rs",
        "pub fn two() -> u8 { 2 }\n",
        "on dev",
    );
    f.journal_names_head();
    match f.decision(Role::Coder, Gate::BranchAhead) {
        Decision::Failed(why) => assert!(why.contains("dev"), "{why}"),
        other => panic!("dev is protected: {other:?}"),
    }
}

/// The gate reads **the block**, not the file. A journal that names an older
/// commit further down is exactly the stale-resume case it exists to catch.
#[test]
fn the_hash_must_be_in_the_resume_block_and_not_merely_in_the_file() {
    let f = Fixture::new();
    let first = git(&f.tree, &["rev-parse", "HEAD"]);
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    let head = git(&f.tree, &["rev-parse", "HEAD"]);

    // The block names the base commit; the current HEAD is further down,
    // outside the block. A file-wide search would call this green.
    std::fs::write(
        &f.journal,
        format!(
            "# Journal\n\n## ÉTAT DE REPRISE\n\nHEAD is `{first}`.\n\n## History\n\nlater work at {head}\n"
        ),
    )
    .unwrap();
    match f.decision(Role::Coder, Gate::ResumeNamesHead) {
        Decision::Failed(why) => assert!(why.contains(&head[..12]), "{why}"),
        other => panic!("a stale resume block must be caught: {other:?}"),
    }

    // Abbreviated is how an agent writes it, and it counts.
    std::fs::write(
        &f.journal,
        format!(
            "# Journal\n\n## ÉTAT DE REPRISE\n\nHEAD is {}.\n",
            &head[..8]
        ),
    )
    .unwrap();
    assert_eq!(
        f.decision(Role::Coder, Gate::ResumeNamesHead),
        Decision::Passed
    );

    // No block at all: the run contract was not honoured.
    std::fs::write(&f.journal, "# Journal\n\nI did some work.\n").unwrap();
    assert!(matches!(
        f.decision(Role::Coder, Gate::ResumeNamesHead),
        Decision::Failed(_)
    ));
}

/// The gate that only a per-commit pass can catch, and the reason SPEC 4.4
/// says "and commit by commit": a forbidden commit that is reverted leaves a
/// clean tree, an empty diff, and a dirty history.
#[test]
fn a_forbidden_commit_that_was_reverted_still_fails_the_perimeter() {
    let f = Fixture::new();
    commit(
        &f.tree,
        "AGENTS.md",
        "the rules, rewritten by the agent\n",
        "loosen the rules",
    );
    let bad = git(&f.tree, &["rev-parse", "HEAD"]);
    git(&f.tree, &["revert", "--no-edit", &bad]);
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.journal_names_head();

    // The diff is innocent: the file is back as it was.
    let diff = git(&f.tree, &["diff", "--name-only", "dev...HEAD"]);
    assert!(
        !diff.contains("AGENTS.md"),
        "the diff really is clean: {diff:?}"
    );

    let revert = git(&f.tree, &["rev-parse", "HEAD~1"]);
    match f.decision(Role::Coder, Gate::Perimeter) {
        Decision::Failed(why) => {
            assert!(why.contains("AGENTS.md"), "{why}");
            // Which of the two it names is not pinned: the revert touches the
            // file as surely as the commit it undoes, and either answer sends
            // the human to the same place in the history.
            assert!(
                why.contains(&bad[..7]) || why.contains(&revert[..7]),
                "it must name the commit that touched it: {why}"
            );
        }
        other => panic!("a reverted forbidden commit is still a forbidden commit: {other:?}"),
    }
}

#[test]
fn a_protected_directory_is_protected_at_every_depth() {
    let f = Fixture::new();
    commit(
        &f.tree,
        ".github/workflows/nested/evil.yml",
        "name: mine\n",
        "add a workflow",
    );
    f.journal_names_head();
    match f.decision(Role::Coder, Gate::Perimeter) {
        Decision::Failed(why) => assert!(why.contains("nested/evil.yml"), "{why}"),
        other => panic!("`.github/workflows/**` covers what is under it: {other:?}"),
    }
}

/// `refuse_if_exists` asks the **base**, not the slot's tree: a file the
/// agent has just created is not one that already existed.
/// `**` spans zero segments too. A project writing `vendor/**` means the
/// directory and everything under it — including the day `vendor` is a file
/// rather than a directory, which is exactly when a glob quietly stops
/// matching and a protected path stops being protected.
#[test]
fn a_protected_directory_pattern_also_covers_the_name_itself() {
    let f = Fixture::new();
    commit(
        &f.tree,
        "vendor",
        "not a directory today\n",
        "add a file named vendor",
    );
    f.journal_names_head();
    match f.decision(Role::Coder, Gate::Perimeter) {
        Decision::Failed(why) => assert!(why.contains("vendor"), "{why}"),
        other => panic!("`vendor/**` must cover `vendor`: {other:?}"),
    }
}

#[test]
fn refuse_if_exists_asks_the_base_and_not_the_working_tree() {
    let f = Fixture::new();
    // `src/*.rs` is refuse_if_exists, and `src/new.rs` is not on dev.
    commit(
        &f.tree,
        "src/new.rs",
        "pub fn two() -> u8 { 2 }\n",
        "a new file",
    );
    f.journal_names_head();
    assert_eq!(f.decision(Role::Coder, Gate::Perimeter), Decision::Passed);

    // `src/lib.rs` is on dev, so touching it is refused.
    commit(
        &f.tree,
        "src/lib.rs",
        "pub fn one() -> u8 { 9 }\n",
        "change an old file",
    );
    f.journal_names_head();
    match f.decision(Role::Coder, Gate::Perimeter) {
        Decision::Failed(why) => assert!(why.contains("src/lib.rs"), "{why}"),
        other => panic!("a file that exists on the base is protected there: {other:?}"),
    }
}

/// The integrator's gate 4 is the inverse of the coder's: an allowlist made
/// of the mission's wiring, and everything outside it is out of perimeter.
#[test]
fn the_integrator_may_commit_its_wiring_and_nothing_else() {
    let mut f = Fixture::new();
    f.header.integration = Integration::Services {
        services: vec![Service {
            name: "db".into(),
            reach: vec!["db".into()],
            shared: false,
        }],
        wiring: vec!["tests/system/**".into(), "config/*.yml".into()],
    };
    commit(
        &f.tree,
        "tests/system/smoke.rs",
        "// system test\n",
        "wire it",
    );
    commit(&f.tree, "config/test.yml", "url: db\n", "configure it");
    f.journal_names_head();
    assert_eq!(
        f.decision(Role::Integrator, Gate::Perimeter),
        Decision::Passed
    );

    // The coder's file is not the integrator's to touch, protected or not.
    commit(
        &f.tree,
        "src/new.rs",
        "pub fn two() -> u8 { 2 }\n",
        "rewrite the code",
    );
    f.journal_names_head();
    match f.decision(Role::Integrator, Gate::Perimeter) {
        Decision::Failed(why) => assert!(
            why.contains("src/new.rs") && why.contains("wiring"),
            "{why}"
        ),
        other => panic!("outside the wiring list is out of perimeter: {other:?}"),
    }
}

#[test]
fn an_integration_mission_with_no_wiring_declared_lets_the_integrator_commit_nothing() {
    let mut f = Fixture::new();
    f.header.integration = Integration::Services {
        services: vec![Service {
            name: "db".into(),
            reach: vec!["db".into()],
            shared: false,
        }],
        wiring: Vec::new(),
    };
    commit(&f.tree, "config/test.yml", "url: db\n", "configure it");
    f.journal_names_head();
    match f.decision(Role::Integrator, Gate::Perimeter) {
        Decision::Failed(why) => {
            assert!(why.contains("--wiring"), "it must say how to fix it: {why}")
        }
        other => panic!("nothing declared means nothing allowed: {other:?}"),
    }
}

/// Three of the four say nothing about a role that commits nothing, and
/// "not applicable" is reported as itself — a skipped gate called green is
/// how a report stops being worth reading.
#[test]
fn the_security_agent_is_not_asked_the_gates_it_cannot_answer() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    // A dirty tree and a forbidden path, neither of which is its business.
    write(&f.tree, "AGENTS.md", "touched by something else\n");
    f.journal_names_head();

    let report = f.gates(Role::Security);
    assert!(report.passed(), "{:?}", report.failure());
    for gate in [Gate::CleanTree, Gate::BranchAhead, Gate::Perimeter] {
        match f.decision(Role::Security, gate) {
            Decision::NotApplicable(why) => assert!(why.contains("read-only"), "{why}"),
            other => panic!("{gate:?} does not apply to the security agent: {other:?}"),
        }
    }
    // Its journal, however, is its own, and gate 3 holds for it.
    assert_eq!(
        f.decision(Role::Security, Gate::ResumeNamesHead),
        Decision::Passed
    );
}

#[test]
fn the_report_names_the_first_red_gate_in_specification_order() {
    let f = Fixture::new();
    // Two reds at once: an uncommitted change (1) and a forbidden commit (4).
    commit(
        &f.tree,
        "deny.toml",
        "[bans]\ndeny = []\n",
        "loosen the bans",
    );
    write(&f.tree, "src/half.rs", "fn unfinished(\n");
    f.journal_names_head();
    let failure = f.gates(Role::Coder).failure().expect("it is red");
    assert!(failure.starts_with("gate 1 "), "{failure}");
}

// ---------------------------------------------------------------------------
// Gates 5 and 6, the final verification's own (SPEC 4.4).
// ---------------------------------------------------------------------------

impl Fixture {
    /// The gates of the final verification, against a slot whose profile is
    /// not up. Gate 6 cannot be played then, and says so — which is exactly
    /// what these tests need in order to judge gate 5 on its own.
    fn verification(&self, role: Role) -> gate::Report {
        let hq = self._dir.path().join("hq");
        let project = hq::project::Project::at(
            self._dir.path().join("repo"),
            hq::project::Config {
                harness: "claude-code".into(),
                forge: vec![],
                stacks: vec!["rust".into()],
                protected_branches: self.branches.clone(),
                protected_paths: Default::default(),
                account: None,
                model: None,
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: None,
                permission_mode: "auto".to_string(),
                forge_protection: Default::default(),
            },
            hq,
        );
        let slot = hq::slot::Slot {
            name: "nowhere".into(),
            tree: self.tree.clone(),
        };
        gate::at_verification(
            &Subject {
                role,
                tree: &self.tree,
                journal: &self.journal,
                pr: &self.pr,
                verdict: &self.verdict,
                mission_dir: self._dir.path(),
                header: &self.header,
                protected_branches: &self.branches,
                protected_paths: &self.protected,
            },
            &gate::Verification {
                project: &project,
                slot: &slot,
                engine: std::sync::Arc::new(hq::engine::fake::FakeEngine::default()),
                stack: "rust",
            },
        )
        .unwrap()
    }

    fn at_verification(&self, role: Role, gate: Gate) -> Decision {
        self.verification(role)
            .outcomes
            .into_iter()
            .find(|o| o.gate == gate)
            .expect("every gate is reported")
            .decision
    }
}

#[test]
fn the_deliverable_is_the_pull_request_and_an_empty_one_is_not_one() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.journal_names_head();

    // Never written at all, and written empty, are the same thing.
    match f.at_verification(Role::Coder, Gate::Deliverable) {
        Decision::Failed(why) => assert!(why.contains("PR.md"), "{why}"),
        other => panic!("a mission with no pull request has no deliverable: {other:?}"),
    }
    std::fs::write(&f.pr, "   \n\n").unwrap();
    assert!(matches!(
        f.at_verification(Role::Coder, Gate::Deliverable),
        Decision::Failed(_)
    ));

    std::fs::write(&f.pr, "# What this changes\n\nA thing.\n").unwrap();
    assert_eq!(
        f.at_verification(Role::Coder, Gate::Deliverable),
        Decision::Passed
    );
}

/// The integrator completes the coder's pull request, and "completed" is
/// decided by a heading — a gate that needs a reader is not a gate.
#[test]
fn the_integrator_owes_its_own_section_of_the_pull_request() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.journal_names_head();
    std::fs::write(&f.pr, "# What this changes\n\nA thing.\n").unwrap();
    match f.at_verification(Role::Integrator, Gate::Deliverable) {
        Decision::Failed(why) => assert!(why.contains("Integration"), "{why}"),
        other => panic!("the coder's pull request alone is not the integrator's: {other:?}"),
    }
    std::fs::write(
        &f.pr,
        "# What this changes\n\nA thing.\n\n## Integration\n\nWired to the database.\n",
    )
    .unwrap();
    assert_eq!(
        f.at_verification(Role::Integrator, Gate::Deliverable),
        Decision::Passed
    );
}

/// The security agent commits nothing, so its deliverable is its report —
/// and a verdict without one is a verdict about nothing.
#[test]
fn the_security_agents_deliverable_is_its_report_not_a_pull_request() {
    let f = Fixture::new();
    f.journal_names_head();
    std::fs::write(&f.pr, "# a pull request it did not write\n").unwrap();
    match f.at_verification(Role::Security, Gate::Deliverable) {
        Decision::Failed(why) => assert!(why.contains("VERDICT.json"), "{why}"),
        other => panic!("a pull request is not the security agent's deliverable: {other:?}"),
    }

    std::fs::write(
        &f.verdict,
        r#"{"role":"Security","verdict":"CLEAR","head":"abc","date":"2026-09-10","report":""}"#,
    )
    .unwrap();
    match f.at_verification(Role::Security, Gate::Deliverable) {
        Decision::Failed(why) => assert!(why.contains("report"), "{why}"),
        other => panic!("a verdict without a report is not a report: {other:?}"),
    }

    std::fs::write(
        &f.verdict,
        r#"{"role":"Security","verdict":"CLEAR","head":"abc","date":"2026-09-10",
            "report":"Nothing reachable was exploitable; the admin panel was not looked at."}"#,
    )
    .unwrap();
    assert_eq!(
        f.at_verification(Role::Security, Gate::Deliverable),
        Decision::Passed
    );
}

/// A battery nobody could run is not a battery that passed, and it is not a
/// battery that failed either. The distinction is the whole point: red blames
/// the agent, and a profile that is not up is not the agent's doing.
#[test]
fn a_battery_that_could_not_be_run_is_neither_green_nor_red() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.journal_names_head();
    std::fs::write(&f.pr, "# What this changes\n").unwrap();

    match f.at_verification(Role::Coder, Gate::Battery) {
        Decision::Unplayed(why) => assert!(why.contains("hq mission start"), "{why}"),
        other => panic!("no profile is up, so nothing ran: {other:?}"),
    }
    // And a report with a gate nobody played is not a green report.
    let report = f.verification(Role::Coder);
    assert!(!report.passed());
    assert!(
        report.failure().unwrap().contains("could not be played"),
        "{:?}",
        report.failure()
    );
}

#[test]
fn the_security_agent_owes_findings_and_not_a_green_battery() {
    let f = Fixture::new();
    f.journal_names_head();
    match f.at_verification(Role::Security, Gate::Battery) {
        Decision::NotApplicable(why) => assert!(why.contains("findings"), "{why}"),
        other => panic!("gate 6 does not apply to the security agent: {other:?}"),
    }
}

/// Gate 6 against a real container, which is the only place it means
/// anything: the battery runs on the clean copy of `HEAD`, and what is being
/// judged is the committed script, not the one in the tree.
///
/// ```text
/// cargo test --test gate -- --ignored --nocapture
/// ```
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_the_battery_is_the_committed_one_and_an_absent_one_is_red() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    git(&tree, &["init", "-q", "-b", "work"]);
    write(&tree, "src/lib.rs", "pub fn one() -> u8 { 1 }\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "base"]);
    git(&tree, &["branch", "-q", "dev"]);
    git(&tree, &["checkout", "-q", "-b", "mission/x"]);

    let project = hq::project::Project::at(
        dir.path().join("repo"),
        hq::project::Config {
            harness: "claude-code".into(),
            forge: vec![],
            stacks: vec!["rust".into()],
            protected_branches: vec!["main".into(), "dev".into()],
            protected_paths: Default::default(),
            account: None,
            model: None,
            bounds: Default::default(),
            credentials: None,
            run: None,
            services_file: None,
            permission_mode: "auto".to_string(),
            forge_protection: Default::default(),
        },
        dir.path().join("hq"),
    );
    let slot = hq::slot::Slot {
        name: "gatelive".into(),
        tree: tree.clone(),
    };
    // The same shape as `tests/exec.rs`: alpine plus git, because the copy of
    // HEAD is made with git inside the container. What is being proved here
    // is which script runs, not what a stack image carries.
    let volume = hq::exec::proof_volume(&slot.name);
    let file = hq::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        format!(
            "services:\n\
             \x20 agent:\n\
             \x20   image: alpine:3.20\n\
             \x20   volumes:\n\
             \x20     - {tree}:{tree_at}\n\
             \x20     - {volume}:{proof}\n\
             \x20   command: [\"sh\", \"-c\", \"apk add --no-cache git > /dev/null && \
             sleep 600\"]\n\
             \x20   healthcheck:\n\
             \x20     test: [\"CMD-SHELL\", \"command -v git > /dev/null\"]\n\
             \x20     interval: 1s\n\
             \x20     timeout: 2s\n\
             \x20     retries: 60\n\
             \x20     start_period: 1s\n\
             volumes:\n\
             \x20 {volume}:\n",
            tree = tree.display(),
            tree_at = hq::run::TREE_AT,
            proof = hq::exec::PROOF_AT,
        ),
    )
    .unwrap();

    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::docker::Docker::real());
    let compose_project = hq::compose::project_name(&slot.name).unwrap();
    let _ = engine.down(&file, &compose_project, true);
    engine.up(&file, &compose_project).unwrap();

    let journal = dir.path().join("JOURNAL.md");
    let pr = dir.path().join("PR.md");
    std::fs::write(&pr, "# What this changes\n").unwrap();
    let verdict = dir.path().join("VERDICT.json");
    let header = header();
    let branches = vec!["main".to_string(), "dev".to_string()];
    let protected = ProtectedPaths::default();

    let battery = |body: &str, executable: bool| {
        let at = tree.join(".hq/stacks/rust/prepush.sh");
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(&at, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            std::fs::set_permissions(&at, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        git(&tree, &["add", "-A"]);
        git(&tree, &["commit", "-q", "-m", "the battery"]);
        std::fs::write(
            &journal,
            format!(
                "# Journal\n\n## ÉTAT DE REPRISE\n\nHEAD is `{}`.\n",
                git(&tree, &["rev-parse", "HEAD"])
            ),
        )
        .unwrap();
    };

    let played = |role: Role| {
        gate::at_verification(
            &Subject {
                role,
                tree: &tree,
                journal: &journal,
                pr: &pr,
                verdict: &verdict,
                mission_dir: dir.path(),
                header: &header,
                protected_branches: &branches,
                protected_paths: &protected,
            },
            &gate::Verification {
                project: &project,
                slot: &slot,
                engine: engine.clone(),
                stack: "rust",
            },
        )
        .unwrap()
        .outcomes
        .into_iter()
        .find(|o| o.gate == Gate::Battery)
        .unwrap()
        .decision
    };

    // No battery on this commit at all.
    std::fs::write(&journal, "# Journal\n\n## ÉTAT DE REPRISE\n\n").unwrap();
    write(&tree, "note.md", "a lot\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "a lot"]);
    match played(Role::Coder) {
        Decision::Failed(why) => assert!(why.contains("no battery"), "{why}"),
        other => panic!("a proof nobody can run is not one that passed: {other:?}"),
    }

    // Present and executable and green.
    battery("#!/bin/sh\nset -eu\ntest -f src/lib.rs\n", true);
    assert_eq!(played(Role::Coder), Decision::Passed);

    // Present, executable and red — and the reason carries what it said.
    battery("#!/bin/sh\necho 'two tests failed' >&2\nexit 1\n", true);
    match played(Role::Coder) {
        Decision::Failed(why) => {
            assert!(why.contains("came back 1"), "{why}");
            assert!(why.contains("two tests failed"), "{why}");
        }
        other => panic!("a red battery is red: {other:?}"),
    }

    // Present and not executable: red, and said differently, because it
    // sends a human somewhere else (SPEC 4.4).
    battery("#!/bin/sh\nexit 0\n", false);
    match played(Role::Coder) {
        Decision::Failed(why) => assert!(why.contains("not executable"), "{why}"),
        other => panic!("nothing ran, so nothing passed: {other:?}"),
    }

    // The integrator's battery is not the coder's, and this stack declares
    // none — red by the same rule.
    battery("#!/bin/sh\nexit 0\n", true);
    match played(Role::Integrator) {
        Decision::Failed(why) => assert!(why.contains("system.sh"), "{why}"),
        other => panic!("the integrator's gate 6 is its system tests: {other:?}"),
    }

    engine.down(&file, &compose_project, true).unwrap();
}

// ---------------------------------------------------------------------------
// Gate 7, the mutation campaign (SPEC 4.4).
// ---------------------------------------------------------------------------

use hq::mutants::{Campaign, Survivor, Triage};
use std::collections::BTreeMap;

impl Fixture {
    /// The campaign file this mission holds, written on the current content.
    fn campaign(&self, survivors: Vec<Survivor>) {
        let base = if git(&self.tree, &["rev-parse", "--verify", "--quiet", "dev"]).is_empty() {
            "origin/dev".to_string()
        } else {
            "dev".to_string()
        };
        let touched = hq::gate::touched_paths(&self.tree, &base).unwrap();
        let fingerprint = hq::mutants::fingerprint(&self.tree, &touched).unwrap();
        hq::mutants::write(
            self._dir.path(),
            &Campaign {
                fingerprint,
                head: git(&self.tree, &["rev-parse", "HEAD"]),
                date: "2026-09-10T12:00:00Z".into(),
                survivors,
            },
        )
        .unwrap();
    }

    fn gate_seven(&self, role: Role) -> hq::gate::Outcome {
        self.verification(role)
            .outcomes
            .into_iter()
            .find(|o| o.gate == Gate::Mutation)
            .expect("gate 7 is reported")
    }
}

impl Fixture {
    /// What the coder wrote, in its own file.
    fn coder_answers(&self, answers: &[(&str, Triage)]) {
        let map: BTreeMap<String, Triage> = answers
            .iter()
            .map(|(id, t)| ((*id).to_string(), t.clone()))
            .collect();
        hq::mutants::write_triage(self._dir.path(), &map).unwrap();
    }
}

fn survivor(line: u32, outcome: Option<Triage>) -> Survivor {
    Survivor {
        id: format!("src/new.rs:{line}"),
        file: "src/new.rs".into(),
        line,
        description: "replace two with 0".into(),
        outcome,
    }
}

/// No campaign is not a green gate, and it is not a red one either: nobody
/// has asked the question yet.
#[test]
fn a_mission_with_no_campaign_has_not_passed_gate_seven() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.journal_names_head();
    match f.gate_seven(Role::Coder).decision {
        Decision::Unplayed(why) => assert!(why.contains("hq mission mutants"), "{why}"),
        other => panic!("no campaign has run: {other:?}"),
    }
}

/// A campaign that ran on other content says nothing about the code as it
/// stands — and saying nothing is not saying yes.
#[test]
fn a_campaign_overtaken_by_a_commit_is_not_an_answer_about_this_one() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.campaign(vec![]);
    f.journal_names_head();
    assert_eq!(f.gate_seven(Role::Coder).decision, Decision::Passed);

    // The code moves; the campaign does not.
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 3 }\n", "L2");
    f.journal_names_head();
    match f.gate_seven(Role::Coder).decision {
        Decision::Unplayed(why) => assert!(why.contains("other content"), "{why}"),
        other => panic!("the campaign is stale: {other:?}"),
    }
}

/// There is no threshold to hide behind: one survivor without an outcome is
/// a red gate, whatever the other ninety-nine did.
#[test]
fn one_survivor_without_an_outcome_is_a_red_gate() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.campaign(vec![
        survivor(
            1,
            Some(Triage::Killed {
                test: "a_run_that_honoured_its_contract_passes_all_four".into(),
            }),
        ),
        survivor(2, None),
    ]);
    f.journal_names_head();
    match f.gate_seven(Role::Coder).decision {
        Decision::Failed(why) => {
            assert!(why.contains("src/new.rs:2"), "{why}");
            assert!(why.contains("no threshold"), "{why}");
        }
        other => panic!("an untriaged survivor is unanswered: {other:?}"),
    }
}

/// "A test covers this" is not an outcome. A test called `x` is, and whether
/// `x` exists is a fact.
#[test]
fn a_named_test_has_to_exist() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    commit(
        &f.tree,
        "tests/thing.rs",
        "#[test]\nfn the_thing_holds() {}\n",
        "a test",
    );
    f.campaign(vec![survivor(
        1,
        Some(Triage::Killed {
            test: "a_test_nobody_wrote".into(),
        }),
    )]);
    f.journal_names_head();
    match f.gate_seven(Role::Coder).decision {
        Decision::Failed(why) => assert!(why.contains("a_test_nobody_wrote"), "{why}"),
        other => panic!("a test that does not exist kills nothing: {other:?}"),
    }

    f.campaign(vec![survivor(
        1,
        Some(Triage::Killed {
            test: "the_thing_holds".into(),
        }),
    )]);
    assert_eq!(f.gate_seven(Role::Coder).decision, Decision::Passed);
}

/// The split the owner decided on 2026-09-10: the coder answers with the two
/// outcomes that rest on a committed test, and the one nobody can check is
/// not its to give. What decides who wrote a line is the mount, so an
/// `equivalent` in the coder's file is not a mistake to tolerate — it is
/// somebody granting themselves the gate.
#[test]
fn the_coder_may_not_call_a_survivor_equivalent() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.campaign(vec![survivor(1, None)]);
    f.coder_answers(&[(
        "src/new.rs:1",
        Triage::Equivalent {
            why: "nothing reads it, honest".into(),
        },
    )]);
    f.journal_names_head();
    match f.gate_seven(Role::Coder).decision {
        Decision::Failed(why) => {
            assert!(why.contains("not the coder's to give"), "{why}");
            assert!(
                why.contains("MUTANTS.json"),
                "it must say where it goes: {why}"
            );
        }
        other => panic!("the graded may not fill in the box nobody can check: {other:?}"),
    }

    // The same ruling from the HQ's own file is accepted.
    f.coder_answers(&[]);
    f.campaign(vec![survivor(
        1,
        Some(Triage::Equivalent {
            why: "no caller can reach that branch".into(),
        }),
    )]);
    assert_eq!(f.gate_seven(Role::Coder).decision, Decision::Passed);
}

/// The coder's own file answers, and it is enough on its own.
#[test]
fn the_coder_answers_with_the_two_outcomes_that_rest_on_a_test() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    commit(
        &f.tree,
        "tests/thing.rs",
        "#[test]\nfn the_thing_holds() {}\n#[test]\nfn the_known_hole() {}\n",
        "tests",
    );
    f.campaign(vec![survivor(1, None), survivor(2, None)]);
    f.coder_answers(&[
        (
            "src/new.rs:1",
            Triage::Killed {
                test: "the_thing_holds".into(),
            },
        ),
        (
            "src/new.rs:2",
            Triage::Bug {
                test: "the_known_hole".into(),
            },
        ),
    ]);
    f.journal_names_head();
    let outcome = f.gate_seven(Role::Coder);
    assert_eq!(outcome.decision, Decision::Passed);
    assert!(outcome.note.is_none(), "nothing rode on a judgement");

    // And a test it names still has to exist.
    f.coder_answers(&[(
        "src/new.rs:1",
        Triage::Killed {
            test: "a_test_nobody_wrote".into(),
        },
    )]);
    assert!(matches!(
        f.gate_seven(Role::Coder).decision,
        Decision::Failed(_)
    ));
}

/// The outcome no gate can check must not be silent, or it becomes the escape
/// hatch that empties the gate.
#[test]
fn the_equivalences_are_counted_out_loud() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.campaign(vec![
        survivor(
            1,
            Some(Triage::Equivalent {
                why: "no caller can reach that branch".into(),
            }),
        ),
        survivor(
            2,
            Some(Triage::Equivalent {
                why: "the constant is never read".into(),
            }),
        ),
    ]);
    f.journal_names_head();
    let outcome = f.gate_seven(Role::Coder);
    assert_eq!(outcome.decision, Decision::Passed);
    let note = outcome
        .note
        .expect("a green gate that owes the reader a number");
    assert!(note.contains("2 of 2"), "{note}");
    assert!(note.contains("HQ"), "{note}");
}

#[test]
fn system_tests_and_configuration_are_not_mutated() {
    let f = Fixture::new();
    f.journal_names_head();
    for role in [Role::Integrator, Role::Security] {
        match f.gate_seven(role).decision {
            Decision::NotApplicable(why) => assert!(why.contains("not mutated"), "{why}"),
            other => panic!("gate 7 is the coder's: {other:?}"),
        }
    }
}
