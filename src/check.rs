//! `nunki check` — does this project hold what SPEC describes? (SPEC 4.2.)
//!
//! Two rules shape it. It is **red** when a restriction is not held, not
//! when something is merely absent. And it **says what it could not check**:
//! a check that quietly skips is worse than no check, because it reads as a
//! pass. Everything in [`run`] is a pure function of the project on disk;
//! probing containers is the caller's, and so is asking the forge
//! ([`forge_protection`]), the one check here that talks to the network.

use std::collections::BTreeMap;
use std::path::Path;

use crate::harness::Role;
use crate::perimeter::{PerimeterError, Sources, compute};
use crate::project::Project;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Held, with what was found.
    Green(String),
    /// Not held. The text says what to do about it.
    Red(String),
    /// Could not be established, and why. Never counted as a pass.
    NotChecked(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub what: String,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    fn add(&mut self, what: impl Into<String>, verdict: Verdict) {
        self.checks.push(Check {
            what: what.into(),
            verdict,
        });
    }

    pub fn is_red(&self) -> bool {
        self.checks
            .iter()
            .any(|c| matches!(c.verdict, Verdict::Red(_)))
    }

    pub fn unchecked(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| matches!(c.verdict, Verdict::NotChecked(_)))
            .count()
    }

    /// One line per check, then a summary that never calls an unchecked
    /// thing a pass.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for check in &self.checks {
            let (mark, detail) = match &check.verdict {
                Verdict::Green(d) => ("ok  ", d),
                Verdict::Red(d) => ("RED ", d),
                Verdict::NotChecked(d) => ("--  ", d),
            };
            out.push_str(&format!("{mark}{}\n      {detail}\n", check.what));
        }
        let red = self
            .checks
            .iter()
            .filter(|c| matches!(c.verdict, Verdict::Red(_)))
            .count();
        let unchecked = self.unchecked();
        out.push('\n');
        out.push_str(&match (red, unchecked) {
            (0, 0) => "everything checked is held.".to_string(),
            (0, n) => format!("everything checked is held; {n} could not be checked."),
            (r, 0) => format!("{r} restriction(s) not held."),
            (r, n) => format!("{r} restriction(s) not held; {n} could not be checked."),
        });
        out.push('\n');
        out
    }
}

/// Everything that can be established from the project on disk.
pub fn run(project: &Project) -> Report {
    let mut report = Report::default();
    git_repository(project, &mut report);
    shareable_path(project, &mut report);
    case_collisions(project, &mut report);
    line_endings(project, &mut report);
    hq_exists(project, &mut report);
    known_harness(project, &mut report);
    rules_are_reachable(project, &mut report);
    harness_can_authenticate(project, &mut report);
    somebody_to_hand_back_to(project, &mut report);
    credentials_outside_the_tree(project, &mut report);
    coder_perimeter(project, &mut report);
    report
}

fn git_repository(project: &Project, report: &mut Report) {
    let dot_git = project.root.join(".git");
    report.add(
        "the project is a git repository",
        if dot_git.exists() {
            Verdict::Green(format!("{} exists", dot_git.display()))
        } else {
            // A slot is a local clone with an unreachable origin (SPEC 3.2);
            // without git there is nothing to clone and no branch to gate.
            Verdict::Red(format!("no {} — a slot is a clone", dot_git.display()))
        },
    );
}

fn shareable_path(project: &Project, report: &mut Report) {
    // Resolved first: what the engine shares is the real path, and on macOS
    // `/var/...` and `/tmp/...` are both symlinks into `/private`. Judging
    // the unresolved path would refuse directories that work.
    let path = std::fs::canonicalize(&project.root)
        .unwrap_or_else(|_| project.root.clone())
        .display()
        .to_string();
    let verdict = if path.starts_with("/mnt/") {
        // SPEC 4.2 bis: `/mnt/c` under WSL is very slow and carries no unix
        // permissions, so git refuses the tree from inside a container.
        Verdict::Red(format!(
            "{path} is on a Windows drive mounted in WSL: slow, and without \
             the permissions git needs from inside a container"
        ))
    } else if cfg!(target_os = "macos") {
        const SHARED: [&str; 4] = ["/Users/", "/Volumes/", "/private/", "/tmp/"];
        if SHARED.iter().any(|p| path.starts_with(p)) {
            Verdict::Green(format!("{path} is under a directory the engine shares"))
        } else {
            Verdict::Red(format!(
                "{path} is outside what a container engine shares by default \
                 ({})",
                SHARED.join(", ")
            ))
        }
    } else {
        Verdict::Green(format!("{path} is a native path"))
    };
    report.add("the tree is on a path a container can mount", verdict);
}

fn case_collisions(project: &Project, report: &mut Report) {
    let mut paths = Vec::new();
    walk(&project.root, &mut |path| {
        paths.push(path.display().to_string())
    });
    let clashes = collisions(&paths);
    report.add(
        "no two paths differ only by case",
        if clashes.is_empty() {
            Verdict::Green(format!("{} paths, no collision", paths.len()))
        } else {
            Verdict::Red(clashes.join("; "))
        },
    );
}

/// Paths that differ only by case (SPEC 4.2 bis): APFS is case-insensitive by
/// default and ext4 is not, so a pair created on Linux survives a commit and
/// then collides — inside the container, or on the next clone to a Mac.
///
/// Separate from the walk on purpose: on a case-insensitive filesystem such a
/// pair cannot be created, so the only way to test this is to hand it the
/// names.
pub fn collisions(paths: &[String]) -> Vec<String> {
    let mut seen: BTreeMap<String, &String> = BTreeMap::new();
    let mut clashes = Vec::new();
    for path in paths {
        let key = path.to_lowercase();
        match seen.get(&key) {
            Some(first) if *first != path => clashes.push(format!("{first} and {path}")),
            _ => {
                seen.insert(key, path);
            }
        }
    }
    clashes
}

fn line_endings(project: &Project, report: &mut Report) {
    let file = project.root.join(".gitattributes");
    let verdict = match std::fs::read_to_string(&file) {
        Ok(text) if text.contains("eol=lf") => Verdict::Green(".gitattributes pins LF".to_string()),
        Ok(_) => Verdict::Red(format!(
            "{} exists but does not pin `eol=lf`, so an editor on Windows \
             can commit CRLF",
            file.display()
        )),
        Err(_) => Verdict::NotChecked(format!(
            "no {} — line endings depend on each machine's git configuration",
            file.display()
        )),
    };
    report.add("line endings are pinned", verdict);
}

fn hq_exists(project: &Project, report: &mut Report) {
    report.add(
        "the project's HQ exists",
        if project.hq_root.is_dir() {
            Verdict::Green(project.hq_root.display().to_string())
        } else {
            Verdict::Red(format!(
                "{} is missing — state, missions and the journal live there",
                project.hq_root.display()
            ))
        },
    );
}

fn known_harness(project: &Project, report: &mut Report) {
    const KNOWN: [&str; 1] = ["claude-code"];
    let harness = &project.config.harness;
    report.add(
        "the declared harness has an adapter",
        if KNOWN.contains(&harness.as_str()) {
            Verdict::Green(harness.clone())
        } else {
            Verdict::Red(format!(
                "{harness:?} has no adapter; known: {}",
                KNOWN.join(", ")
            ))
        },
    );
}

/// The rules of the place have to be read by the harness that is driving.
/// Claude Code reads `CLAUDE.md` and not `AGENTS.md`, so one must import the
/// other — SPEC 4.1 makes this check red until it does, because otherwise a
/// mission starts without the rules.
fn rules_are_reachable(project: &Project, report: &mut Report) {
    let agents = project.root.join("AGENTS.md");
    let claude = project.root.join("CLAUDE.md");
    let verdict = if !agents.is_file() {
        Verdict::Red(format!(
            "no {} — the rules of the place have to be written somewhere",
            agents.display()
        ))
    } else if !claude.exists() {
        Verdict::Red(format!(
            "no {}: Claude Code reads that file and not AGENTS.md, so the              rules would never be read",
            claude.display()
        ))
    } else if claude.is_symlink() {
        Verdict::Green(format!("{} is a link to AGENTS.md", claude.display()))
    } else {
        match std::fs::read_to_string(&claude) {
            Ok(text) if text.contains("AGENTS.md") => {
                Verdict::Green(format!("{} imports AGENTS.md", claude.display()))
            }
            Ok(_) => Verdict::Red(format!(
                "{} does not import AGENTS.md — add `@AGENTS.md` to it",
                claude.display()
            )),
            Err(e) => Verdict::NotChecked(format!("{} could not be read: {e}", claude.display())),
        }
    };
    report.add("the rules of the place reach the harness", verdict);
}

/// Which account this project's missions spend, and whether it can actually
/// pay (SPEC 4.3). A human may hold several subscriptions — two Anthropic
/// ones, an OpenAI one — so the question is not "is there a token" but "is
/// the named account usable by this project".
fn harness_can_authenticate(project: &Project, report: &mut Report) {
    use crate::account::{Accounts, setup_command};

    let what = "the account this project spends can authenticate";
    let nunki_home = project.nunki_home();
    let accounts = match Accounts::load(&nunki_home) {
        Ok(a) => a,
        Err(e) => {
            report.add(what, Verdict::Red(e.to_string()));
            return;
        }
    };
    let (name, account) =
        match accounts.choose(&nunki_home, None, project.config.account.as_deref()) {
            Ok(pair) => pair,
            // Nothing named and nothing to guess from: something to do, not
            // something broken.
            Err(e) => {
                report.add(what, Verdict::NotChecked(e.to_string()));
                return;
            }
        };

    if account.harness != project.config.harness {
        report.add(
            what,
            Verdict::Red(format!(
                "account {name:?} authenticates {}, but this project runs {}",
                account.harness, project.config.harness
            )),
        );
        return;
    }

    let path = account.token_path(&nunki_home);
    if path.starts_with(&project.root) {
        // A token in the tree is a token in every slot and every container.
        report.add(
            what,
            Verdict::Red(format!(
                "{} is inside the repository, so every slot and every container \
                 would carry it",
                path.display()
            )),
        );
        return;
    }
    match account.token(&nunki_home, &name) {
        Err(_) => report.add(
            what,
            Verdict::NotChecked(format!(
                "account {name:?} has no token at {} — put the output of `{}` there",
                path.display(),
                setup_command(&account.harness)
            )),
        ),
        Ok(_) => {
            // A secret others can read is a secret.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path)
                    .map(|m| m.permissions().mode())
                    .unwrap_or(0);
                if mode & 0o077 != 0 {
                    report.add(
                        what,
                        Verdict::Red(format!(
                            "{} is readable by others ({:o}) — `chmod 600` it",
                            path.display(),
                            mode & 0o777
                        )),
                    );
                    return;
                }
            }
            report.add(what, Verdict::Green(format!("{name} ({})", path.display())));
        }
    }
}

/// A mission ends by giving something back to a person (SPEC 4.5). "Waiting
/// on the human" is enough while there is one; it stops being enough the
/// moment a second person works on the project, and an arbitration for
/// Arnaud is not an arbitration for Igor.
fn somebody_to_hand_back_to(project: &Project, report: &mut Report) {
    use crate::human::{ME_FILE, Source, me};

    let who = me(&project.nunki_home(), Some(&project.root));
    let what = "nunki knows who to hand a mission back to";
    let verdict = match (&who.name, &who.source) {
        (Some(name), Source::Declared(path)) => {
            Verdict::Green(format!("{name}, from {}", path.display()))
        }
        (Some(name), Source::Git) => Verdict::Green(format!("{name}, from git")),
        // A machine account name is a guess, not a statement — usable, and
        // said to be a guess.
        (Some(name), _) => Verdict::NotChecked(format!(
            "{name} is the machine's account name, not something you said; \
             write {} to be addressed properly",
            project.nunki_home().join(ME_FILE).display()
        )),
        (None, _) => Verdict::NotChecked(format!(
            "nothing says who you are; write {}",
            project.nunki_home().join(ME_FILE).display()
        )),
    };
    report.add(what, verdict);
}

fn credentials_outside_the_tree(project: &Project, report: &mut Report) {
    let Some(dir) = &project.config.credentials else {
        report.add(
            "test credentials live outside the tree",
            Verdict::Green("none declared".to_string()),
        );
        return;
    };
    let inside = dir.starts_with(&project.root);
    report.add(
        "test credentials live outside the tree",
        if inside {
            // The coder's profile mounts no credentials at all (SPEC 4.1); a
            // credentials directory inside the tree would be mounted with it.
            Verdict::Red(format!(
                "{} is inside the repository, so the coder would read it \
                 through the tree mount",
                dir.display()
            ))
        } else {
            Verdict::Green(dir.display().to_string())
        },
    );
}

fn coder_perimeter(project: &Project, report: &mut Report) {
    let harness_domains = crate::harness::claude_code::ClaudeCode::new(
        Default::default(),
        Box::new(crate::harness::spawn::LocalSpawner),
    );
    let harness_domains = {
        use crate::harness::Harness;
        harness_domains.provision().domains
    };

    if project.config.stacks.is_empty() {
        report.add(
            "the coder's allowlist names no forge domain",
            Verdict::NotChecked("no stack declared in nunki.yaml".to_string()),
        );
        return;
    }

    for stack in &project.config.stacks {
        let what = format!("the coder's allowlist for {stack} names no forge domain");
        let Some(domains) = project.stack_domains(stack) else {
            report.add(
                what,
                Verdict::NotChecked(format!(
                    "{} has no allow.txt",
                    project.fragment(stack).display()
                )),
            );
            continue;
        };
        let verdict = match compute(
            Role::Coder,
            &Sources {
                stack: &domains,
                harness: &harness_domains,
                services: &[],
                forge: &project.config.forge,
            },
        ) {
            Ok(perimeter) => Verdict::Green(format!(
                "{} domain(s), none of them the forge",
                perimeter.domains.len()
            )),
            Err(e @ PerimeterError::Forge { .. }) => Verdict::Red(e.to_string()),
            Err(e) => Verdict::Red(e.to_string()),
        };
        report.add(what, verdict);
    }
}

/// Everything under `root`, skipping what no container ever mounts.
fn walk(root: &Path, visit: &mut impl FnMut(&Path)) {
    const SKIP: [&str; 4] = [".git", "target", "node_modules", ".venv"];
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if SKIP.iter().any(|s| name == *s) {
            continue;
        }
        visit(&path);
        if path.is_dir() {
            walk(&path, visit);
        }
    }
}

/// Whether each protected branch is protected **on the forge** (SPEC 4.1
/// bis, "branche protégée"): locally nothing stops an agent committing to
/// `main` in its clone — gate 2 sees it, and the forge is what refuses the
/// push. When the forge would not refuse, gate 2 is the only guard left, and
/// that is a restriction not held — unless `nunki.yaml` declares
/// `forge_protection: by_hand`, the written decision that the human holds the
/// rule. Then the forge is not asked, and every branch is named as held by
/// hand: never green, since nunki cannot see a human keep a rule, and never red.
///
/// It asks with the human's credential at the HQ, and without one it says it
/// did not ask. Anything the forge declines to answer is "not checked",
/// never green and never red: a check that cannot say "I do not know" lies.
///
/// It runs on every `nunki check` where a credential is present — unlike the
/// container probes, which wait for `--slot`. The asymmetry is deliberate:
/// SPEC 4.1 bis asks for it "quand un credential de forge est présent sur
/// l'hôte", and it costs one read per protected branch, where a probe costs
/// a profile lifted and taken down. Without a credential it makes no network
/// call at all.
pub fn forge_protection(project: &Project, api: &str, report: &mut Report) {
    use crate::forge::{Protection, TOKEN_FILE, token};
    use crate::project::{CONFIG_FILE, ForgeProtection};

    let not_checked = |report: &mut Report, why: String| {
        for branch in &project.config.protected_branches {
            report.add(
                format!("{branch} is protected on the forge"),
                Verdict::NotChecked(why.clone()),
            );
        }
    };

    if project.config.forge_protection == ForgeProtection::ByHand {
        return not_checked(
            report,
            format!(
                "held by hand, as {CONFIG_FILE} declares (`forge_protection: by_hand`): the forge \
                 is not asked and would accept a push to it, so gate 2 and the human are the \
                 guards (SPEC 4.1 bis)"
            ),
        );
    }

    let Ok(remote) = crate::git::run(&project.root, &["remote", "get-url", crate::push::REMOTE])
    else {
        return not_checked(
            report,
            format!(
                "the repository has no `{}` remote, so there is no forge to ask",
                crate::push::REMOTE
            ),
        );
    };
    let Some((forge, repo)) = crate::forge::of_remote_at(&remote, api) else {
        return not_checked(
            report,
            format!(
                "the remote is on a forge nunki has no adapter for ({}): GitHub is the one it \
                 can ask",
                remote.trim()
            ),
        );
    };
    let Some(token) = token(&project.nunki_home()) else {
        return not_checked(
            report,
            format!(
                "no forge credential at {} — without one nunki cannot ask the forge, and says so",
                project.nunki_home().join(TOKEN_FILE).display()
            ),
        );
    };

    for branch in &project.config.protected_branches {
        let verdict = match forge.protection(&token, &repo, branch) {
            Ok(Protection::Protected) => Verdict::Green(format!(
                "{} reports {branch} protected on {}/{}",
                forge.name(),
                repo.owner,
                repo.name
            )),
            Ok(Protection::Unprotected) => Verdict::Red(format!(
                "{} reports {branch} unprotected on {}/{}: the forge would accept a push \
                 to it, and gate 2 is the only guard left (SPEC 4.1 bis). Protect it on the \
                 forge — on a private GitHub repository that needs a paid plan — or, if you \
                 hold the rule yourself, declare `forge_protection: by_hand` in {CONFIG_FILE}",
                forge.name(),
                repo.owner,
                repo.name
            )),
            Ok(Protection::NoSuchBranch) => Verdict::NotChecked(format!(
                "{branch} does not exist on {}/{}, so there is nothing to protect yet",
                repo.owner, repo.name
            )),
            Err(e) => Verdict::NotChecked(format!("the forge did not say: {e}")),
        };
        report.add(format!("{branch} is protected on the forge"), verdict);
    }
}
