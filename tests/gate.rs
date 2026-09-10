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
        account: None,
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
    header: Header,
    protected: ProtectedPaths,
    branches: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let tree = repo(dir.path());
        let journal = journal(dir.path(), "0000000");
        Self {
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
        gate::run(&Subject {
            role,
            tree: &self.tree,
            journal: &self.journal,
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
