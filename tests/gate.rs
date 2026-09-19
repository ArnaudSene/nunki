//! The verification gates 1 to 4 (SPEC 4.4), against real git repositories.
//!
//! Every test here builds an actual repository in a temporary directory and
//! makes actual commits. A gate that reads git is worth what it reads, and a
//! fake would only prove that the fake agrees with the code.

use std::path::{Path, PathBuf};
use std::process::Command;

use nunki::gate::{self, Decision, Gate, Subject};
use nunki::harness::Role;
use nunki::mission::{Bounds, Header, Integration, Lot, Security, Service};
use nunki::project::ProtectedPaths;

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
    /// Where the coder's gates were green. The base by default: a coder
    /// that added nothing, so every commit on the branch is the integrator's.
    coder_head: Option<String>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let tree = repo(dir.path());
        let journal = journal(dir.path(), "0000000");
        let pr = dir.path().join("PR.md");
        let verdict = dir.path().join("VERDICT.json");
        let coder_head = Some(git(&tree, &["rev-parse", "HEAD"]));
        Self {
            pr,
            verdict,
            _dir: dir,
            tree,
            journal,
            header: header(),
            protected: protected(),
            branches: vec!["main".into(), "dev".into()],
            coder_head,
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

    /// The project and slot a `Verification` borrows. Built here because
    /// gate 8 needs a container at the end of **every** run, not only at the
    /// final verification — so both phases carry the same context.
    fn context(&self) -> (nunki::project::Project, nunki::slot::Slot) {
        let nunki = self._dir.path().join("nunki");
        let project = nunki::project::Project::at(
            self._dir.path().join("repo"),
            nunki::project::Config {
                root: None,
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
            nunki,
        );
        let slot = nunki::slot::Slot {
            name: "nowhere".into(),
            tree: self.tree.clone(),
        };
        (project, slot)
    }

    fn gates(&self, role: Role) -> gate::Report {
        let (project, slot) = self.context();
        gate::after_run(
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
                coder_head: self.coder_head.as_deref(),
            },
            &gate::Verification {
                project: &project,
                slot: &slot,
                engine: std::sync::Arc::new(nunki::engine::fake::FakeEngine::default()),
                stack: "rust",
            },
        )
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
fn a_run_that_honoured_its_contract_passes_the_gates_of_its_end() {
    let f = Fixture::new();
    commit(&f.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    f.journal_names_head();
    let report = f.gates(Role::Coder);

    // 1 to 4, and 8 — the gates SPEC 4.4 owes at the end of every run.
    assert_eq!(report.outcomes.len(), 5, "{:?}", report.outcomes);
    for gate in [
        Gate::CleanTree,
        Gate::BranchAhead,
        Gate::ResumeNamesHead,
        Gate::Perimeter,
    ] {
        let one = report.outcomes.iter().find(|o| o.gate == gate).unwrap();
        assert_eq!(one.decision, Decision::Passed, "{gate:?}");
    }

    // Gate 8 needs a container, and there is none here. `Unplayed` and not
    // red: a verdict on the machine rather than on the agent, and a security
    // gate that could not look must not report green either.
    let eight = report
        .outcomes
        .iter()
        .find(|o| o.gate == Gate::MechanicalSecurity)
        .expect("gate 8 is played at the end of a run");
    assert!(
        matches!(eight.decision, Decision::Unplayed(_)),
        "{:?}",
        eight.decision
    );
    assert!(!report.passed(), "an unplayed gate is not a green report");
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

/// The integrator works on the coder's branch (SPEC 2), so the coder's
/// commits come first on it — and none of them is wiring.
///
/// Found on 2026-09-15, before the first integration mission was launched:
/// gate 4 judged the whole branch against its base, so the coder's `src/`
/// held every integrator red on a perimeter it never crossed —
/// `src/new.rs is not in this mission's wiring list, and the diff against
/// the base touches it`.
#[test]
fn the_integrator_is_judged_on_what_it_added_after_the_coder() {
    let mut f = Fixture::new();
    f.header.integration = Integration::Services {
        services: vec![Service {
            name: "db".into(),
            reach: vec!["db".into()],
            shared: false,
        }],
        wiring: vec!["tests/system/**".into()],
    };
    commit(
        &f.tree,
        "src/new.rs",
        "pub fn two() -> u8 { 2 }\n",
        "the coder's lot",
    );
    f.coder_head = Some(git(&f.tree, &["rev-parse", "HEAD"]));
    commit(
        &f.tree,
        "tests/system/smoke.rs",
        "// system test\n",
        "wire it",
    );
    f.journal_names_head();
    assert_eq!(
        f.decision(Role::Integrator, Gate::Perimeter),
        Decision::Passed,
        "the coder's src/new.rs is not the integrator's commit"
    );

    // Not an exemption for the coder's files: one the integrator commits
    // after the coder is still outside its wiring.
    commit(
        &f.tree,
        "src/new.rs",
        "pub fn two() -> u8 { 3 }\n",
        "rewrite the coder's decision",
    );
    f.journal_names_head();
    match f.decision(Role::Integrator, Gate::Perimeter) {
        Decision::Failed(why) => assert!(why.contains("src/new.rs"), "{why}"),
        other => panic!("a coder's file the integrator rewrites is out of perimeter: {other:?}"),
    }
}

/// Without the commit the coder's gates were green on, nothing separates the
/// two roles' commits. That is the machine's gap, not the integrator's fault,
/// so the gate is not played rather than red.
#[test]
fn without_the_coders_green_commit_the_integrators_perimeter_is_not_played() {
    let mut f = Fixture::new();
    f.header.integration = Integration::Services {
        services: vec![Service {
            name: "db".into(),
            reach: vec!["db".into()],
            shared: false,
        }],
        wiring: vec!["tests/system/**".into()],
    };
    f.coder_head = None;
    commit(
        &f.tree,
        "tests/system/smoke.rs",
        "// system test\n",
        "wire it",
    );
    f.journal_names_head();
    match f.decision(Role::Integrator, Gate::Perimeter) {
        Decision::Unplayed(why) => assert!(why.contains("coder"), "{why}"),
        other => panic!("nothing tells the two roles' commits apart: {other:?}"),
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
        let (project, slot) = self.context();
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
                coder_head: self.coder_head.as_deref(),
            },
            &gate::Verification {
                project: &project,
                slot: &slot,
                engine: std::sync::Arc::new(nunki::engine::fake::FakeEngine::default()),
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
        Decision::Unplayed(why) => assert!(why.contains("nunki mission start"), "{why}"),
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
    // And told apart, which is what the caller deciding the next move needs.
    // `failure()` folds the two on purpose — "is this green" has one answer —
    // but "what do I do about it" has two: a red gate is the agent's to fix,
    // and a gate nobody could play is not, so sending an agent back at it
    // spends a run to be told the same thing again. `verdict()` is that
    // second question, and the flow asks it and nothing else.
    let nunki::gate::Verdict::Wall(why) = report.verdict() else {
        panic!(
            "a gate nobody could play is not a red one: {:?}",
            report.verdict()
        );
    };
    assert!(why.contains("could not be played"), "{why}");
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
/// anything: the battery runs on the clean copy of `HEAD`, and the script
/// that runs is the stack's, mounted read-only from the project's home.
///
/// ```text
/// cargo test --test gate -- --ignored --nocapture
/// ```
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_the_battery_is_the_stacks_mounted_one_and_an_absent_one_is_red() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    git(&tree, &["init", "-q", "-b", "work"]);
    write(&tree, "src/lib.rs", "pub fn one() -> u8 { 1 }\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "base"]);
    git(&tree, &["branch", "-q", "dev"]);
    git(&tree, &["checkout", "-q", "-b", "mission/x"]);

    let project = nunki::project::Project::at(
        dir.path().join("repo"),
        nunki::project::Config {
            root: None,
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
        dir.path().join("nunki"),
    );
    let slot = nunki::slot::Slot {
        name: "gatelive".into(),
        tree: tree.clone(),
    };
    // The same shape as `tests/exec.rs`: alpine plus git, because the copy of
    // HEAD is made with git inside the container. What is being proved here
    // is which script runs, not what a stack image carries.
    let volume = nunki::exec::proof_volume(&slot.name);
    let file = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let script = project.fragment("rust").join(gate::BATTERY);
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();

    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::docker::Docker::real());
    let compose_project = nunki::compose::project_name(&project.session(), &slot.name).unwrap();
    // The profile as `nunki` writes it: the battery mounted when it exists and
    // not otherwise — a bind mount of a missing file would be a directory.
    let lift = |with_battery: bool| {
        let mount = if with_battery {
            format!(
                "\x20     - {}:{}/{}:ro\n",
                script.display(),
                nunki::run::STACK_AT,
                gate::BATTERY
            )
        } else {
            String::new()
        };
        std::fs::write(
            &file,
            format!(
                "services:\n\
                 \x20 agent:\n\
                 \x20   image: alpine:3.20\n\
                 \x20   volumes:\n\
                 \x20     - {tree}:{tree_at}\n\
                 \x20     - {volume}:{proof}\n\
                 {mount}\
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
                tree_at = nunki::run::TREE_AT,
                proof = nunki::exec::PROOF_AT,
            ),
        )
        .unwrap();
        engine.up(&file, &compose_project).unwrap();
    };
    std::fs::write(&file, "services: {}\n").unwrap();
    let _ = engine.down(&file, &compose_project, true);
    lift(false);

    let journal = dir.path().join("JOURNAL.md");
    let pr = dir.path().join("PR.md");
    std::fs::write(&pr, "# What this changes\n").unwrap();
    let verdict = dir.path().join("VERDICT.json");
    let header = header();
    let branches = vec!["main".to_string(), "dev".to_string()];
    let protected = ProtectedPaths::default();

    // Rewritten in place: the mount follows the file, and nothing is committed
    // — the battery is not the commit's.
    let battery = |body: &str, executable: bool| {
        std::fs::write(&script, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(mode)).unwrap();
        }
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
                coder_head: None,
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

    // No battery in the stack at all.
    write(&tree, "note.md", "a lot\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "a lot"]);
    std::fs::write(
        &journal,
        format!(
            "# Journal\n\n## ÉTAT DE REPRISE\n\nHEAD is `{}`.\n",
            git(&tree, &["rev-parse", "HEAD"])
        ),
    )
    .unwrap();
    match played(Role::Coder) {
        Decision::Failed(why) => assert!(why.contains("no battery"), "{why}"),
        other => panic!("a proof nobody can run is not one that passed: {other:?}"),
    }

    // Present and executable and green — and the profile lifted again, now
    // that there is a file to mount.
    battery("#!/bin/sh\nset -eu\ntest -f src/lib.rs\n", true);
    lift(true);
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

use nunki::mutants::{Campaign, Survivor, Triage};
use std::collections::BTreeMap;

impl Fixture {
    /// The campaign file this mission holds, written on the current content.
    fn campaign(&self, survivors: Vec<Survivor>) {
        let base = if git(&self.tree, &["rev-parse", "--verify", "--quiet", "dev"]).is_empty() {
            "origin/dev".to_string()
        } else {
            "dev".to_string()
        };
        let touched = nunki::gate::touched_paths(&self.tree, &base).unwrap();
        let fingerprint = nunki::mutants::fingerprint(&self.tree, &touched).unwrap();
        nunki::mutants::write(
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

    fn gate_seven(&self, role: Role) -> nunki::gate::Outcome {
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
        nunki::mutants::write_triage(self._dir.path(), &map).unwrap();
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
        Decision::Unplayed(why) => assert!(why.contains("nunki mission mutants"), "{why}"),
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

/// Built by hand rather than from the fixture: what is asked here is which
/// unplayed gates mean what, and the fixture leaves gate 6 unplayed too —
/// it has no profile up — so it can only ever show the case where the
/// campaign is *not* owed. In a real mission the battery is green in the
/// container and gate 7 stands alone, which is the case that matters.
fn report_of(decisions: &[(Gate, Decision)]) -> gate::Report {
    gate::Report {
        role: Role::Coder,
        head: "0123456789ab".into(),
        outcomes: decisions
            .iter()
            .map(|(gate, decision)| nunki::gate::Outcome {
                gate: *gate,
                decision: decision.clone(),
                note: None,
                waits_on_campaign: false,
            })
            .collect(),
    }
}

/// The four answers a report can give the flow, and the order they are asked
/// in — which is the doctrine, not a detail.
///
/// They used to be three questions recombined by hand at five sites in
/// `verify`, and the recombination is where the defects were: a new reason
/// for a gate to be unplayed changed what the old rule meant at all five,
/// silently. This is the whole decision, in one place, so a fifth reason
/// meets one rule.
#[test]
fn a_report_says_one_thing_to_the_flow() {
    use nunki::gate::Verdict;

    let green = report_of(&[
        (Gate::Battery, Decision::Passed),
        (Gate::Mutation, Decision::Passed),
    ]);
    assert_eq!(green.verdict(), Verdict::Green);

    // Red first: it is the agent's to fix, and it outranks a gate nobody
    // could play, because then there is something to send an agent back for.
    let red = report_of(&[
        (
            Gate::Battery,
            Decision::Failed("the battery came back 101".into()),
        ),
        (
            Gate::Mutation,
            Decision::Unplayed("no mutation campaign has run on this mission".into()),
        ),
    ]);
    let Verdict::Red(why) = red.verdict() else {
        panic!("{:?}", red.verdict());
    };
    assert!(why.contains("gate 6"), "{why}");

    // Gate 7 without a usable campaign is a campaign to run, and the gate's
    // own sentence is what the flow acts on — not one written a second time.
    let owed = report_of(&[
        (Gate::Battery, Decision::Passed),
        (
            Gate::Mutation,
            Decision::Unplayed("no mutation campaign has run on this mission".into()),
        ),
    ]);
    assert_eq!(
        owed.verdict(),
        Verdict::CampaignOwed("no mutation campaign has run on this mission".into())
    );

    // A gate nobody could play *beside* gate 7 is a wall, and an hour of
    // mutation in front of it would be an hour spent for nothing.
    let wall = report_of(&[
        (
            Gate::Battery,
            Decision::Unplayed("the profile did not come up".into()),
        ),
        (
            Gate::Mutation,
            Decision::Unplayed("no mutation campaign has run on this mission".into()),
        ),
    ]);
    let Verdict::Wall(why) = wall.verdict() else {
        panic!(
            "an hour of mutation in front of a wall: {:?}",
            wall.verdict()
        );
    };
    assert!(why.contains("gate 6"), "{why}");

    // And a gate 7 that is answered leaves nothing owed.
    for decision in [
        Decision::Passed,
        Decision::NotApplicable("system tests are not mutated".into()),
    ] {
        let report = report_of(&[
            (Gate::Battery, Decision::Passed),
            (Gate::Mutation, decision),
        ]);
        assert_eq!(report.verdict(), Verdict::Green, "{:?}", report.outcomes);
    }
}

/// A gate standing down **while** a campaign runs is the same obstacle seen
/// from the other side, in every phase — including the one where gate 7 is
/// not played at all.
///
/// After a run, `nunki` plays gates 1 to 4 and 8, never 7. So gate 8 standing
/// down for a campaign was the only unplayed gate in the report, gate 7 was
/// not there to name the obstacle, and the old rule read it as a wall: the
/// flow stopped, nothing read the campaign back, and the gate stood down for
/// ever. The same for a gate 7 that has already answered while gates 6 and 8
/// wait.
#[test]
fn a_gate_standing_down_for_a_campaign_is_never_a_wall() {
    use nunki::gate::Verdict;

    // After a run: gate 7 is not in the report at all.
    let mut after_run = report_of(&[(Gate::MechanicalSecurity, Decision::Passed)]);
    after_run.outcomes[0].decision = Decision::Unplayed("a campaign has been rewriting".into());
    after_run.outcomes[0].waits_on_campaign = true;
    assert_eq!(
        after_run.verdict(),
        Verdict::CampaignOwed("a campaign has been rewriting".into()),
        "{:?}",
        after_run.outcomes
    );

    // And at the final gates, with gate 7 already answered.
    let mut answered = report_of(&[
        (Gate::Battery, Decision::Passed),
        (Gate::Mutation, Decision::Passed),
    ]);
    answered.outcomes[0].decision = Decision::Unplayed("a campaign has been rewriting".into());
    answered.outcomes[0].waits_on_campaign = true;
    assert!(
        matches!(answered.verdict(), Verdict::CampaignOwed(_)),
        "{:?}",
        answered.verdict()
    );
}

// --- gate 8: the mechanical security ---------------------------------------

impl Fixture {
    /// Gate 8 with a container that answers what `security.sh` printed.
    fn security(&self, role: Role, out: nunki::engine::ExecOutput) -> Decision {
        self.security_seen(role, out).0
    }

    /// The same, and what the container was actually asked to run — the probe
    /// is half of gate 8, and a test that only reads the decision cannot see
    /// what was handed to the script.
    fn security_seen(
        &self,
        role: Role,
        out: nunki::engine::ExecOutput,
    ) -> (Decision, Vec<nunki::engine::fake::Call>) {
        // The refresh of the clean copy answers first, and green: what this
        // asks about is the script's own status, not the refresh's.
        let outs = vec![said(0, ""), out];
        let (project, slot) = self.context();
        let profile = nunki::run::profile_path(&project, &slot.name);
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        std::fs::write(&profile, "services: {}\n").unwrap();
        let engine =
            std::sync::Arc::new(nunki::engine::fake::FakeEngine::default().with_execs(outs));
        let decision = gate::after_run(
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
                coder_head: self.coder_head.as_deref(),
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
        .find(|o| o.gate == Gate::MechanicalSecurity)
        .expect("gate 8 is played")
        .decision;
        (decision, engine.calls())
    }
}

/// What the container was asked to run, last first: the refresh of the clean
/// copy goes through the same engine, so the probe is the one that follows it.
fn probe_of(calls: &[nunki::engine::fake::Call]) -> String {
    calls
        .iter()
        .filter_map(|c| match c {
            nunki::engine::fake::Call::Exec(_, _, argv) => Some(argv.join(" ")),
            _ => None,
        })
        .find(|argv| argv.contains("security.sh"))
        .expect("the probe runs security.sh")
}

fn said(status: i32, stdout: &str) -> nunki::engine::ExecOutput {
    nunki::engine::ExecOutput {
        status,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

/// What the branch brought and nobody ruled on stops it. The finding is
/// named, because a gate that says only "red" sends nobody anywhere.
#[test]
fn a_finding_this_branch_brought_is_red_and_the_reason_names_it() {
    let f = Fixture::new();
    let decision = f.security(
        Role::Coder,
        said(
            0,
            r#"{"id":"RUSTSEC-2020-0071","kind":"vulnerability","where":"time 0.1.45","via":"chrono","fix":">=0.2.23","accepted":"","was_at_base":false}"#,
        ),
    );

    let Decision::Failed(why) = decision else {
        panic!("{decision:?}");
    };
    assert!(why.contains("RUSTSEC-2020-0071"), "{why}");
    assert!(
        why.contains("via chrono"),
        "the parent is what can be replaced: {why}"
    );
    assert!(why.contains(">=0.2.23"), "{why}");
}

/// What the repository already carried is not this branch's to answer.
/// Blocking on it would punish the wrong change, for a cause outside the
/// agent's reach (SPEC 4.4).
#[test]
fn what_the_base_already_carried_does_not_stop_this_branch() {
    let f = Fixture::new();
    let decision = f.security(
        Role::Coder,
        said(
            0,
            r#"{"id":"RUSTSEC-2020-0071","kind":"vulnerability","where":"time 0.1.45","via":"","fix":">=0.2.23","accepted":"","was_at_base":true}"#,
        ),
    );

    assert_eq!(decision, Decision::Passed);
}

/// An acceptance a fix has overtaken is red: the exception was written when
/// nothing could be done, and something can now. This is what replaces an
/// expiry date.
#[test]
fn an_exception_a_fix_has_overtaken_is_red() {
    let f = Fixture::new();
    let decision = f.security(
        Role::Coder,
        said(
            0,
            r#"{"id":"RUSTSEC-2020-0159","kind":"vulnerability","where":"chrono 0.4.19","via":"","fix":">=0.4.20","accepted":"pas atteignable","was_at_base":true}"#,
        ),
    );

    let Decision::Failed(why) = decision else {
        panic!("{decision:?}");
    };
    assert!(why.contains("overtaken"), "{why}");
    assert!(why.contains("RUSTSEC-2020-0159"), "{why}");
}

/// An acceptance no fix can overtake lets the mission through, and the
/// `alloy`/`paste` case SPEC 4.4 cites is exactly that shape.
#[test]
fn an_acceptance_no_fix_can_overtake_lets_it_through() {
    let f = Fixture::new();
    let decision = f.security(
        Role::Coder,
        said(
            0,
            r#"{"id":"RUSTSEC-2024-0436","kind":"unmaintained","where":"paste 1.0.15","via":"alloy","fix":"","accepted":"archivé en amont","was_at_base":true}"#,
        ),
    );

    assert_eq!(decision, Decision::Passed);
}

/// No advisory database: the script's own 69. `Unplayed` and not red — a
/// verdict on the machine rather than on the agent, and a security gate that
/// could not consult its database must not report green either.
#[test]
fn a_gate_that_could_not_consult_its_database_says_so() {
    let f = Fixture::new();

    let decision = f.security(Role::Coder, said(69, ""));

    let Decision::Unplayed(why) = decision else {
        panic!("{decision:?}");
    };
    assert!(why.contains("advisory database"), "{why}");
    assert!(why.contains("cargo deny check advisories"), "{why}");
}

/// A stack shipping no script fails, as the battery does: a proof nobody can
/// run is not a proof that passed. And the message says how to get one.
#[test]
fn a_stack_with_no_security_script_is_red_and_says_how_to_get_one() {
    let f = Fixture::new();

    let decision = f.security(Role::Coder, said(66, ""));

    let Decision::Failed(why) = decision else {
        panic!("{decision:?}");
    };
    assert!(why.contains("nunki init --stack"), "{why}");
}

/// The script runs in the clean copy of `HEAD`, a detached clone that holds no
/// branch at all, so a base named `dev` resolves to nothing there and the
/// script reports every finding as new — a gate red on what the branch never
/// brought.
///
/// Measured on 2026-09-18 against a real container, the first time gate 8 ran
/// in one: `git rev-parse --verify dev` in the copy answered "Needed a single
/// revision", and notes-api's one finding — a test credential its base already
/// carried — came back `was_at_base: false`. The same script, in the same
/// container, given the commit instead, came back `was_at_base: true`.
#[test]
fn the_base_reaches_the_script_as_a_commit_and_not_a_branch_name() {
    let f = Fixture::new();
    let fork = git(&f.tree, &["merge-base", "HEAD", "dev"]);

    let (_, calls) = f.security_seen(Role::Coder, said(0, ""));
    let probe = probe_of(&calls);

    assert!(probe.contains(&fork), "the probe names no commit: {probe}");
    assert!(
        !probe.contains(" dev "),
        "the probe hands over a branch name the copy cannot resolve: {probe}"
    );
}

/// The fork point, and not the base's tip. Gate 2 has already said the branch
/// is ahead of its base, so the fork point is an ancestor of `HEAD` and is in
/// the copy by construction; a base that has moved since the copy was cloned
/// has a tip that is not.
#[test]
fn the_commit_is_the_fork_point_and_not_wherever_the_base_has_got_to() {
    let f = Fixture::new();
    let fork = git(&f.tree, &["rev-parse", "HEAD"]);
    // The base moves on, as a base does while a mission runs.
    git(&f.tree, &["checkout", "-q", "dev"]);
    write(&f.tree, "src/other.rs", "pub fn two() -> u8 { 2 }\n");
    git(&f.tree, &["add", "-A"]);
    git(&f.tree, &["commit", "-q", "-m", "the base moves"]);
    let tip = git(&f.tree, &["rev-parse", "HEAD"]);
    git(&f.tree, &["checkout", "-q", "mission/x"]);

    let (_, calls) = f.security_seen(Role::Coder, said(0, ""));
    let probe = probe_of(&calls);

    assert!(probe.contains(&fork), "not the fork point: {probe}");
    assert!(
        !probe.contains(&tip),
        "the base's tip is not in the copy: {probe}"
    );
}

/// A slot's own `dev` is written once, when the clone is made, and nothing
/// moves it again. The mission branch comes from `origin/dev`, which
/// `run::branch` fetches first — so from the second mission onwards the two
/// disagree, and every gate that reads "what this branch touched" reads the
/// wrong answer.
///
/// Measured on 2026-09-18 on `notes-api`'s slot, with a branch that had
/// touched nothing: `dev...HEAD` named eight files, every one of them the
/// previous mission's, and `origin/dev...HEAD` named none. Gate 7 would have
/// run a mutation campaign over seven source files the mission never opened,
/// and handed the coder survivors in code it had not written; gate 8's fork
/// point was one merge early, so what the previous mission brought counted as
/// brought by this one; and gate 4 turns red the moment anything on the base
/// between the two is a protected path — which is what this test uses,
/// because it is the one of the three that answers in a decision.
#[test]
fn the_gates_judge_against_the_base_the_branch_came_from() {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "dev"]);
    write(&origin, "AGENTS.md", "the rules of this place\n");
    write(&origin, "deny.toml", "[bans]\n");
    write(&origin, "src/lib.rs", "pub fn one() -> u8 { 1 }\n");
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-q", "-m", "base"]);

    // The slot: a clone, so it has its own `dev`, here, for good.
    let tree = dir.path().join("slot");
    git(
        dir.path(),
        &["clone", "-q", origin.to_str().unwrap(), "slot"],
    );

    // The base moves on, and what lands on it touches a protected path — a
    // human tightening `deny.toml`, which no agent may edit and every agent
    // inherits.
    write(
        &origin,
        "deny.toml",
        "[bans]\nmultiple-versions = \"deny\"\n",
    );
    git(&origin, &["add", "-A"]);
    git(
        &origin,
        &["commit", "-q", "-m", "a human tightens the bans"],
    );

    // This mission's branch, from `origin/dev` as `run::branch` makes it, and
    // one commit of its own, well inside its perimeter.
    git(&tree, &["fetch", "-q", "origin"]);
    git(&tree, &["checkout", "-q", "-b", "mission/x", "origin/dev"]);
    write(&tree, "src/mine.rs", "pub fn three() -> u8 { 3 }\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "feat(L1): mine"]);

    let mut f = Fixture::new();
    f.tree = tree;
    f.journal = journal(dir.path(), &git(&f.tree, &["rev-parse", "HEAD"]));

    let report = f.gates(Role::Coder);

    let perimeter = report
        .outcomes
        .iter()
        .find(|o| o.gate == Gate::Perimeter)
        .expect("gate 4 is played");
    assert_eq!(
        perimeter.decision,
        Decision::Passed,
        "the gate judged against the slot's stale `dev`, and blamed this branch \
         for a protected path the base already carried"
    );
}

/// A campaign in flight owns the clean copy of `HEAD`, and every gate that
/// runs there stands down while it does.
///
/// Gate 7's campaign is `cargo mutants --in-place` in that copy; gates 6 and 8
/// reach it through `exec::run(On::Proof)`, which refreshes it first — `git
/// reset --hard`, `git clean`. Run together they wreck each other both ways:
/// the gate judges a mutant, and the reset pulls the tree out from under the
/// campaign.
///
/// Measured on `notes-4`, 2026-09-18, the first three-agent mission run end to
/// end. The battery came back 101 on `warning: unused variable: value` at
/// `src/api.rs:160` — a function whose body cargo-mutants had replaced, and
/// which uses its argument in the coder's own code. Gate 6 red, a volet, and
/// again until the volets were spent: six runs, 59M tokens, and neither the
/// integrator nor the security agent ever ran.
#[test]
fn the_gates_that_run_in_the_copy_stand_down_while_a_campaign_rewrites_it() {
    let f = Fixture::new();
    let (project, slot) = f.context();
    nunki::mutants::write_running(
        &project.hq_root,
        &slot.name,
        &nunki::mutants::Running {
            fingerprint: "a62d271".into(),
            head: git(&f.tree, &["rev-parse", "HEAD"]),
            started_at: "2026-09-18T20:56:38Z".into(),
            container: "64e0796ca1de".into(),
            pid: Some(2736),
            log: f._dir.path().join("mutants.log"),
            deadline_minutes: 45,
        },
    )
    .unwrap();

    let report = f.verification(Role::Coder);

    for gate in [Gate::Battery, Gate::MechanicalSecurity] {
        let outcome = report
            .outcomes
            .iter()
            .find(|o| o.gate == gate)
            .unwrap_or_else(|| panic!("{gate:?} is played"));
        let Decision::Unplayed(why) = &outcome.decision else {
            panic!(
                "{gate:?} ran in a copy a campaign is rewriting: {:?}",
                outcome.decision
            );
        };
        assert!(why.contains("2026-09-18T20:56:38Z"), "{why}");
    }
}

/// Gate 7 and the campaign it judges read one set, and there is one call that
/// can produce it.
///
/// The base has two readings in a slot, `dev` and `origin/dev`, and from the
/// second mission onwards they differ. Gate 7 resolved the name; `nunki
/// verify` and `nunki mission mutants` worked the paths out themselves and
/// passed them in raw. So the campaign ran on one set and the gate judged
/// another, their fingerprints could never agree, and gate 7 asked for a
/// campaign that had just run.
///
/// Measured live on `notes-4`, 2026-09-18: eight paths on the launcher's side
/// and four on the gate's. Fifty-seven turns of `verify` went round it.
///
/// `mutants::campaign` takes the base's **name** now and makes the call
/// itself, so there is no second reading to get wrong — which is why this
/// test asserts on the paths that call gives, and not on two callers agreeing.
#[test]
fn what_a_branch_brought_has_one_reading() {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "dev"]);
    write(&origin, "AGENTS.md", "the rules of this place\n");
    write(&origin, "src/lib.rs", "pub fn one() -> u8 { 1 }\n");
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-q", "-m", "base"]);

    let tree = dir.path().join("slot");
    git(
        dir.path(),
        &["clone", "-q", origin.to_str().unwrap(), "slot"],
    );

    // The base moves on, as it does between two missions, and the slot's own
    // `dev` stays where the clone wrote it.
    write(&origin, "src/theirs.rs", "pub fn two() -> u8 { 2 }\n");
    git(&origin, &["add", "-A"]);
    git(
        &origin,
        &["commit", "-q", "-m", "what the last mission merged"],
    );

    git(&tree, &["fetch", "-q", "origin"]);
    git(&tree, &["checkout", "-q", "-b", "mission/x", "origin/dev"]);
    write(&tree, "src/mine.rs", "pub fn three() -> u8 { 3 }\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "feat(L1): mine"]);

    let asked = gate::touched_since_base(&tree, "dev").unwrap();

    assert_eq!(
        asked,
        vec!["src/mine.rs".to_string()],
        "the campaign would run on what the branch never opened: {asked:?}"
    );
}

/// The rulings a human left reach the script as an argument, at the path
/// `nunki` fixes. An environment variable would be the agent's to set, and a
/// path it could set is a file it could write.
#[test]
fn the_script_is_told_where_the_rulings_are() {
    let f = Fixture::new();

    let (_, calls) = f.security_seen(Role::Coder, said(0, ""));
    let probe = probe_of(&calls);

    assert!(probe.contains(nunki::secrets::AT), "{probe}");
}

/// A secret is the one finding no agent can close: taking it out in a later
/// commit leaves it in the branch's history, and `nunki` rewrites none. So
/// the line says whose move it is and what the move is, with the id it names
/// as that move's argument.
#[test]
fn a_secret_that_stops_the_gate_names_the_gesture_that_lifts_it() {
    let f = Fixture::new();

    let decision = f.security(
        Role::Coder,
        said(
            0,
            r#"{"id":"abc123:src/store.rs:Postgres:22","kind":"secret","where":"src/store.rs:22","via":"","fix":"","accepted":"","was_at_base":false}"#,
        ),
    );

    let Decision::Failed(why) = decision else {
        panic!("{decision:?}");
    };
    assert!(why.contains("abc123:src/store.rs:Postgres:22"), "{why}");
    assert!(why.contains("nunki secret accept"), "{why}");
}

/// And an advisory does not, because that gesture is not the one: an advisory
/// with a fix is an agent's to apply, and one without is `deny.toml`'s.
#[test]
fn an_advisory_that_stops_the_gate_does_not_send_a_human_to_the_secret_verb() {
    let f = Fixture::new();

    let decision = f.security(
        Role::Coder,
        said(
            0,
            r#"{"id":"RUSTSEC-2020-0071","kind":"vulnerability","where":"time 0.1.45","via":"chrono","fix":">=0.2.23","accepted":"","was_at_base":false}"#,
        ),
    );

    let Decision::Failed(why) = decision else {
        panic!("{decision:?}");
    };
    assert!(!why.contains("nunki secret accept"), "{why}");
}

/// And the script's own word for it, for the same reason. `nunki` resolves the
/// commit above and should never hand over one the copy lacks — this is the
/// second lock, not the first.
#[test]
fn a_script_that_cannot_read_the_base_is_unplayed_and_not_red() {
    let f = Fixture::new();

    let decision = f.security(Role::Coder, said(70, ""));

    let Decision::Unplayed(why) = decision else {
        panic!("{decision:?}");
    };
    assert!(why.contains("clean copy of HEAD"), "{why}");
}

/// The security agent attacks what is built and owes findings, not a tool's
/// report — the per-role table, and the battery two gates above.
#[test]
fn the_security_agent_owes_findings_and_not_a_tools_report() {
    let f = Fixture::new();

    let decision = f.security(Role::Security, said(0, ""));

    assert!(
        matches!(decision, Decision::NotApplicable(_)),
        "{decision:?}"
    );
}
