//! `nunki init`, asked rather than guessed (SPEC 4.2, `nunki init`).
//!
//! A first `init` on a terminal asks the questions whose wrong answer costs a
//! mission later: which stacks live where, the forge no agent may reach, who
//! holds the protected branches, and how the agents' silence falls. Each
//! question comes with what the repository already says — the stacks its
//! manifests declare, the forge its `origin` names, the branches it has — so
//! answering is mostly pressing Enter.
//!
//! What is detected is only ever **proposed**. nunki is not a scaffolder
//! (AGENTS.md § 1): it reads the manifests a project already has, and writes
//! nothing but its own configuration.

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
use std::path::Path;

use crate::init::{Answers, KNOWN_STACKS};
use crate::project::{ForgeProtection, Stack};

/// What the repository says before anyone is asked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detected {
    /// The stacks its manifests declare, each with the manifest that said
    /// so, the root's first.
    pub stacks: Vec<(Stack, String)>,
    /// The host `origin` points at.
    pub forge: Option<String>,
    /// Of the usual long-lived branches, those the repository has.
    pub branches: Vec<String>,
}

/// Directories no stack lives in: dependencies, build output, tooling.
const SKIPPED: [&str; 8] = [
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    "venv",
    "__pycache__",
    "graphify-out",
];

/// How deep the manifests are looked for. `frontend/`, `apps/web/`: two
/// levels hold every layout measured; deeper, a manifest is a package inside
/// a stack rather than a stack.
const DEPTH: usize = 2;

/// Read what the repository at `root` declares.
pub fn detect(root: &Path) -> Detected {
    let mut found: Vec<(Stack, String)> = Vec::new();
    walk(root, "", 0, &mut found);
    // The root first, then by path: the root's stack is the primary unless
    // the human says otherwise.
    found.sort_by(|(a, _), (b, _)| (!a.dir.is_empty(), &a.dir).cmp(&(!b.dir.is_empty(), &b.dir)));
    let forge = crate::git::run(root, &["remote", "get-url", "origin"])
        .ok()
        .and_then(|url| host_of_remote(&url));
    let mut branches = Vec::new();
    for name in ["main", "master", "dev", "develop"] {
        let local = format!("refs/heads/{name}");
        let remote = format!("refs/remotes/origin/{name}");
        let has = |r: &str| crate::git::run(root, &["rev-parse", "--verify", "--quiet", r]).is_ok();
        if has(&local) || has(&remote) {
            branches.push(name.to_string());
        }
    }
    Detected {
        stacks: found,
        forge,
        branches,
    }
}

fn walk(root: &Path, dir: &str, depth: usize, found: &mut Vec<(Stack, String)>) {
    let here = if dir.is_empty() {
        root.to_path_buf()
    } else {
        root.join(dir)
    };
    for (name, manifest) in stacks_in(&here) {
        // A stack already found above covers this directory: a workspace's
        // member crates are the workspace's, not stacks of their own.
        if found.iter().any(|(s, _)| {
            s.name == name && (s.dir.is_empty() || dir.starts_with(&format!("{}/", s.dir)))
        }) {
            continue;
        }
        let stack = Stack::new(name, dir).expect("a directory walked inside the root is valid");
        let said = if dir.is_empty() {
            manifest.to_string()
        } else {
            format!("{dir}/{manifest}")
        };
        found.push((stack, said));
    }
    if depth == DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&here) else {
        return;
    };
    let mut children: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.') && !SKIPPED.contains(&n.as_str()))
        .collect();
    children.sort();
    for child in children {
        let next = if dir.is_empty() {
            child
        } else {
            format!("{dir}/{child}")
        };
        walk(root, &next, depth + 1, found);
    }
}

/// The stacks a directory's manifests declare, with the manifest that says so.
///
/// Next.js is a `package.json` that depends on `next`, and nothing less: a
/// Node package that is not a Next.js application is not what the `next`
/// stack's battery, campaign and launch script know how to run.
fn stacks_in(dir: &Path) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    if dir.join("Cargo.toml").is_file() {
        out.push(("rust", "Cargo.toml"));
    }
    for manifest in ["pyproject.toml", "requirements.txt", "setup.py"] {
        if dir.join(manifest).is_file() {
            out.push(("python", manifest));
            break;
        }
    }
    if let Ok(text) = std::fs::read_to_string(dir.join("package.json"))
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
    {
        let depends = ["dependencies", "devDependencies"]
            .iter()
            .any(|k| json.get(k).and_then(|d| d.get("next")).is_some());
        if depends {
            out.push(("next", "package.json"));
        }
    }
    out
}

/// The host of a git remote, SSH (`git@host:o/r`) or URL (`https://host/o/r`).
pub fn host_of_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest.split('/').next()?,
        None => url.split_once(':')?.0,
    };
    let host = rest.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty() && host.contains('.')).then(|| host.to_string())
}

/// Something that asks a human and hears the answer.
pub trait Ask {
    /// Put `question` with its `default`, and return the answer — the default
    /// when the human just pressed Enter.
    fn ask(&mut self, question: &str, default: &str) -> io::Result<String>;
    /// Say something that needs no answer.
    fn say(&mut self, text: &str);
}

/// The human at the terminal: questions on stderr, answers on stdin, so what
/// `init` prints on stdout stays what it always was.
pub struct Terminal;

impl Ask for Terminal {
    fn ask(&mut self, question: &str, default: &str) -> io::Result<String> {
        let mut err = io::stderr();
        write!(err, "{question} [{default}]: ")?;
        err.flush()?;
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "stdin closed before the question was answered",
            ));
        }
        let line = line.trim();
        Ok(if line.is_empty() {
            default.to_string()
        } else {
            line.to_string()
        })
    }

    fn say(&mut self, text: &str) {
        eprintln!("{text}");
    }
}

/// Nobody to ask: every question takes its proposed answer, and nothing is
/// said. What `init --yes` plays.
pub struct Proposed;

impl Ask for Proposed {
    fn ask(&mut self, _question: &str, default: &str) -> io::Result<String> {
        Ok(default.to_string())
    }

    fn say(&mut self, _text: &str) {}
}

/// Answers given in advance, for a test to play an interview.
#[derive(Debug, Default)]
pub struct Scripted {
    pub answers: VecDeque<String>,
    pub heard: Vec<String>,
}

impl Scripted {
    pub fn new(answers: &[&str]) -> Self {
        Self {
            answers: answers.iter().map(|a| a.to_string()).collect(),
            heard: Vec::new(),
        }
    }
}

impl Ask for Scripted {
    fn ask(&mut self, question: &str, default: &str) -> io::Result<String> {
        self.heard.push(format!("{question} [{default}]"));
        let answer = self.answers.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("no answer to {question:?}"),
            )
        })?;
        Ok(if answer.is_empty() {
            default.to_string()
        } else {
            answer
        })
    }

    fn say(&mut self, text: &str) {
        self.heard.push(text.to_string());
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InterviewError {
    #[error("the interview stopped: {0}")]
    Io(#[from] io::Error),
    #[error("{question}: {why} — three answers were refused, so nothing was written")]
    Refused { question: String, why: String },
    #[error("nothing was written: the configuration was not confirmed")]
    NotConfirmed,
}

/// What a flag already answered, so the interview does not ask it again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Given {
    pub stacks: Vec<String>,
    pub forge: Vec<String>,
    pub forge_protection: Option<ForgeProtection>,
    pub permission_mode: Option<String>,
    pub protected_branches: Vec<String>,
}

/// The stacks, as `init --stack` takes them, and the rest of `nunki.yaml`,
/// from what was detected, what flags gave, and — when `ask` is a human —
/// what they said.
///
/// [`Proposed`] plays `--yes`: every question takes its proposed answer.
pub fn interview(
    detected: &Detected,
    given: &Given,
    ask: &mut dyn Ask,
) -> Result<(Vec<String>, Answers), InterviewError> {
    if detected.stacks.is_empty() {
        ask.say("No stack manifest found (Cargo.toml, pyproject.toml, a package.json using next).");
    } else {
        ask.say("Stacks this repository's manifests declare:");
        for (stack, manifest) in &detected.stacks {
            ask.say(&format!("  {:<24} ({manifest})", stack.to_string()));
        }
    }

    let stacks = if given.stacks.is_empty() {
        let proposed = detected
            .stacks
            .iter()
            .map(|(s, _)| as_flag(s))
            .collect::<Vec<_>>()
            .join(", ");
        let answer = until_valid(
            ask,
            "Stacks to declare, primary first, as name or name=dir",
            &proposed,
            parse_stacks,
        )?;
        answer.iter().map(as_flag).collect()
    } else {
        given.stacks.clone()
    };

    let forge = if given.forge.is_empty() {
        until_valid(
            ask,
            "Forge domain no agent may ever reach (empty: none)",
            detected.forge.as_deref().unwrap_or(""),
            |text| Ok(list(text)),
        )?
    } else {
        given.forge.clone()
    };

    let forge_protection = match given.forge_protection {
        Some(p) => p,
        None => until_valid(
            ask,
            "Who refuses a push to a protected branch: the forge, or you by hand \
             (a private repository on GitHub's free plan cannot protect one) — forge/by_hand",
            "forge",
            |text| match text.trim() {
                "forge" => Ok(ForgeProtection::Forge),
                "by_hand" => Ok(ForgeProtection::ByHand),
                other => Err(format!("{other:?} is neither forge nor by_hand")),
            },
        )?,
    };

    let permission_mode = match &given.permission_mode {
        Some(m) => m.clone(),
        None => until_valid(
            ask,
            "How the agents' unanswerable permissions fall: auto lets the harness's own \
             checks decide, dontAsk denies whatever is not pre-approved — auto/dontAsk",
            "auto",
            |text| match text.trim() {
                m @ ("auto" | "dontAsk") => Ok(m.to_string()),
                other => Err(format!("{other:?} is neither auto nor dontAsk")),
            },
        )?,
    };

    let protected_branches = if given.protected_branches.is_empty() {
        let proposed = if detected.branches.is_empty() {
            "main, dev".to_string()
        } else {
            detected.branches.join(", ")
        };
        until_valid(
            ask,
            "Branches no mission may target or push to",
            &proposed,
            |text| {
                let l = list(text);
                if l.is_empty() {
                    Err("at least one branch is protected: gate 2 refuses a push to it".into())
                } else {
                    Ok(l)
                }
            },
        )?
    } else {
        given.protected_branches.clone()
    };

    let answers = Answers {
        forge,
        forge_protection,
        permission_mode,
        protected_branches,
    };
    ask.say(&summary(&stacks, &answers));
    let yes = ask.ask("Write this configuration? y/n", "y")?;
    if !matches!(yes.trim(), "y" | "Y" | "yes") {
        return Err(InterviewError::NotConfirmed);
    }
    Ok((stacks, answers))
}

/// Ask until the answer reads, three times at most.
fn until_valid<T>(
    ask: &mut dyn Ask,
    question: &str,
    proposed: &str,
    read: impl Fn(&str) -> Result<T, String>,
) -> Result<T, InterviewError> {
    let mut last = String::new();
    for _ in 0..3 {
        let answer = ask.ask(question, proposed)?;
        match read(&answer) {
            Ok(value) => return Ok(value),
            Err(why) => {
                ask.say(&format!("  {why}"));
                last = why;
            }
        }
    }
    Err(InterviewError::Refused {
        question: question.to_string(),
        why: last,
    })
}

fn parse_stacks(text: &str) -> Result<Vec<Stack>, String> {
    let mut stacks: Vec<Stack> = Vec::new();
    for item in list(text) {
        let stack = Stack::parse(&item)?;
        if !KNOWN_STACKS.contains(&stack.name.as_str()) {
            return Err(format!(
                "{} is not a stack nunki knows — known: {}",
                stack.name,
                KNOWN_STACKS.join(", ")
            ));
        }
        if stacks.iter().any(|s| s.name == stack.name) {
            return Err(format!("{} is named twice", stack.name));
        }
        stacks.push(stack);
    }
    Ok(stacks)
}

fn list(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn as_flag(stack: &Stack) -> String {
    if stack.dir.is_empty() {
        stack.name.clone()
    } else {
        format!("{}={}", stack.name, stack.dir)
    }
}

fn summary(stacks: &[String], a: &Answers) -> String {
    format!(
        "\nnunki.yaml will declare:\n  stacks              {}\n  forge               {}\n  \
         forge_protection    {}\n  permission_mode     {}\n  protected_branches  {}\n",
        if stacks.is_empty() {
            "none".to_string()
        } else {
            stacks.join(", ")
        },
        if a.forge.is_empty() {
            "none".to_string()
        } else {
            a.forge.join(", ")
        },
        match a.forge_protection {
            ForgeProtection::Forge => "forge",
            ForgeProtection::ByHand => "by_hand",
        },
        a.permission_mode,
        a.protected_branches.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_names_its_host_whichever_way_it_is_written() {
        for (url, host) in [
            (
                "https://github.com/example-org/example-repo.git",
                Some("github.com"),
            ),
            ("git@github.com:example/nunki.git", Some("github.com")),
            (
                "ssh://git@gitlab.example.org:2222/team/x.git",
                Some("gitlab.example.org"),
            ),
            (
                "https://user:token@git.example.com/o/r",
                Some("git.example.com"),
            ),
            ("/srv/git/nunki.git", None),
            ("../elsewhere", None),
        ] {
            assert_eq!(host_of_remote(url).as_deref(), host, "{url}");
        }
    }
}
