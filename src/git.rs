//! Just enough git, run as a subprocess.
//!
//! `nunki` never links a git library: what it needs is what the command line
//! does — clone, branch, log — and shelling out keeps the behaviour identical
//! to what a human sees in the same repository.
//!
//! # Two kinds of repository
//!
//! The human's own repository is run as it is, by [`run`]: its configuration
//! is the human's, and so is everything it executes.
//!
//! **A slot is never a repository host git runs in** (SPEC 3.1, 4.1 bis,
//! 4.2). Its `.git` is mounted read-write in an agent's container, and git
//! executes what a repository's configuration names: `core.fsmonitor`, a hook,
//! a filter or diff driver, a pager, `include.path` towards a file in the
//! tree. Overriding those keys one by one is a list git lengthens with every
//! release; what closes the class is that the slot's configuration is never
//! read at all. So every host-side git on a slot goes through [`SlotGit`]:
//!
//! - a **mirror** the host alone writes, beside the slot and outside every
//!   mount ([`mirror_of`]), holds the slot's commits;
//! - they are brought into it by a fetch from a **shim**: a repository
//!   `nunki` makes for the occasion, whose configuration is its own and whose
//!   object store borrows the slot's `objects/` as an alternate. The shim's
//!   refs are the slot's, read by `nunki` from the files themselves. The
//!   fetch's `index-pack` hashes every object it receives, so a loose object
//!   rewritten on disk, or an index that lies about a pack, fails the fetch
//!   rather than entering the mirror;
//! - every read — `log`, `diff`, `rev-parse`, `cat-file`, ancestry — runs in
//!   the mirror. What needs the working tree runs with the mirror as git
//!   directory, the slot's tree as work tree, and an index `nunki` builds
//!   from `HEAD` itself;
//! - what `nunki` writes back into a slot — a branch, its checkout, the
//!   objects that branch needs — it writes as files, never by running git
//!   there.
//!
//! Every such git runs with the system and global configuration switched
//! off, replace objects and grafts ignored, and no `GIT_*` variable of the
//! caller's: nothing the human configured for their own repositories applies
//! to an agent's tree either, and nothing an agent planted in that tree's
//! `.gitattributes` meets a driver the human's configuration defines.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git {verb} failed in {at}: {stderr}")]
    Failed {
        verb: String,
        at: String,
        stderr: String,
    },
    #[error("git is not on the path: {0}")]
    Missing(String),
    /// A slot whose `.git` the host cannot read as a plain clone: corrupted,
    /// missing objects, or not shaped like one. Said as such, and never a
    /// reason to run git in it after all.
    #[error(
        "the slot at {tree} cannot be read from the host: {why}. nunki ran no git in that \
         slot and will not; repair or recreate it (`nunki slot rm`, then `nunki slot add`)"
    )]
    Unreadable { tree: String, why: String },
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
}

/// Run git in `at` and return its stdout, trimmed.
///
/// **The human's own repository only** — never a slot, whose configuration
/// this would read and execute: a slot goes through [`SlotGit`].
///
/// In the C locale, always. What git says on failure is read by `nunki` — to
/// tell "this is not a repository" from "git could not run at all" — and
/// carried into messages a human reads in English (AGENTS.md § 2). A macOS
/// permission that stops git from reading its own working directory comes
/// back in the user's locale, and a check that reads git's words would take
/// it for an absent repository.
pub fn run(at: &Path, args: &[&str]) -> Result<String, GitError> {
    let mut git = Command::new("git");
    git.env("LC_ALL", "C").arg("-C").arg(at).args(args);
    finish(git, args, &at.display().to_string(), None)
}

/// Run git outside any repository — `clone`, essentially.
pub fn run_anywhere(args: &[&str]) -> Result<String, GitError> {
    let mut git = Command::new("git");
    git.env("LC_ALL", "C").args(args);
    finish(git, args, ".", None)
}

/// Spawn `git`, feed it `input` if any, and return its stdout trimmed, or
/// what it said on stderr.
fn finish(
    mut git: Command,
    args: &[&str],
    at: &str,
    input: Option<&[u8]>,
) -> Result<String, GitError> {
    let out = output(&mut git, input)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(GitError::Failed {
            verb: args.first().unwrap_or(&"?").to_string(),
            at: at.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

fn output(git: &mut Command, input: Option<&[u8]>) -> Result<std::process::Output, GitError> {
    let missing = |e: std::io::Error| GitError::Missing(e.to_string());
    let Some(input) = input else {
        return git.stdin(Stdio::null()).output().map_err(missing);
    };
    let mut child = git
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(missing)?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    // Written from a thread: git may answer before it has read everything,
    // and a pipe both sides wait on is a deadlock.
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output().map_err(missing)?;
    // A git that stopped reading early closed the pipe; what it said about
    // why is on its stderr, which the caller reads.
    let _ = writer.join();
    Ok(out)
}

pub fn current_branch(at: &Path) -> Result<String, GitError> {
    run(at, &["rev-parse", "--abbrev-ref", "HEAD"])
}

pub fn head(at: &Path) -> Result<String, GitError> {
    run(at, &["rev-parse", "HEAD"])
}

/// Whether the slot at `tree` has anything uncommitted, by `nunki`'s own
/// comparison of its tree with `HEAD` ([`SlotGit::is_clean`]).
pub fn is_clean(tree: &Path) -> Result<bool, GitError> {
    SlotGit::open(tree)?.is_clean()
}

/// The commits reachable from the `HEAD` of the slot at `tree` that the
/// human's repository at `elsewhere` does not have. This is what makes
/// `nunki slot rm` refuse rather than lose work (SPEC 3.3).
pub fn commits_not_in(tree: &Path, elsewhere: &Path) -> Result<Vec<String>, GitError> {
    // Ask the other repository what it knows, then ask the slot's mirror what
    // it has that the other does not. No fetch, no network, no writing in
    // either repository.
    let here = SlotGit::open(tree)?.run(&["rev-list", "HEAD"])?;
    let there: std::collections::BTreeSet<String> = run(elsewhere, &["rev-list", "--all"])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    Ok(here
        .lines()
        .filter(|c| !there.contains(*c))
        .map(str::to_string)
        .collect())
}

/// Where a named branch is in `at`, whatever `HEAD` is on.
pub fn head_of(at: &Path, branch: &str) -> Result<String, GitError> {
    run(at, &["rev-parse", &format!("{branch}^{{commit}}")])
}

/// Git on the slot at `tree`, from the host: [`SlotGit::run`].
pub fn on_slot(tree: &Path, args: &[&str]) -> Result<String, GitError> {
    SlotGit::open(tree)?.run(args)
}

/// The commit the slot at `tree` is on, read in its mirror.
pub fn slot_head(tree: &Path) -> Result<String, GitError> {
    SlotGit::open(tree)?.head()
}

/// The keys of the slot's `.git/config` that git would execute — or run
/// something on behalf of — in a repository that honoured them: what an
/// agent planted for the host to run, named so a human can be told.
///
/// **A report, never the protection.** The protection is that no git on the
/// host reads a slot's configuration at all ([`SlotGit`]); this list only
/// says what was found, and a key git adds tomorrow is still never run. The
/// file is read as data by `git config --file --no-includes`, outside any
/// repository: an `include.path` is named, never followed.
pub fn executable_keys(tree: &Path) -> Result<Vec<String>, GitError> {
    let file = tree.join(".git").join("config");
    match std::fs::symlink_metadata(&file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(GitError::Io(file, e)),
        Ok(m) if !m.is_file() => {
            return Err(GitError::Unreadable {
                tree: tree.display().to_string(),
                why: format!(
                    "{} is not a regular file, and nunki follows no link in a slot's .git",
                    file.display()
                ),
            });
        }
        Ok(_) => {}
    }
    let mut git = host_git();
    // No repository: what `--file` names is all git reads.
    git.env("GIT_DIR", mirror_of(tree).join("no-repository"))
        .args(["config", "--no-includes", "--null", "--list", "--file"])
        .arg(&file);
    let listed = finish(git, &["config"], &tree.display().to_string(), None)?;
    let mut keys: Vec<String> = listed
        .split('\0')
        .filter_map(|entry| entry.split('\n').next())
        .filter(|key| executes(key))
        .map(str::to_string)
        .collect();
    keys.sort();
    keys.dedup();
    Ok(keys)
}

/// Whether git runs what `key` (as `git config --list` spells it: section and
/// name in lowercase) names, or reads another file because of it.
fn executes(key: &str) -> bool {
    const EXACT: &[&str] = &[
        "core.fsmonitor",
        "core.hookspath",
        "core.pager",
        "core.editor",
        "core.sshcommand",
        "core.askpass",
        "core.gitproxy",
        "core.alternaterefscommand",
        "sequence.editor",
        "diff.external",
        "credential.helper",
        "gpg.program",
        "uploadpack.packobjectshook",
        "include.path",
        "web.browser",
        "protocol.allow",
    ];
    if EXACT.contains(&key) {
        return true;
    }
    let Some((section, rest)) = key.split_once('.') else {
        return false;
    };
    // `section.<subsection>.name`: the subsection may itself hold dots.
    let name = rest.rsplit('.').next().unwrap_or(rest);
    let scoped = rest.contains('.');
    match section {
        "alias" | "pager" => true,
        "includeif" => scoped && name == "path",
        "filter" => scoped && matches!(name, "clean" | "smudge" | "process"),
        "diff" => scoped && matches!(name, "textconv" | "command"),
        "merge" => scoped && name == "driver",
        "credential" => scoped && name == "helper",
        "gpg" => scoped && name == "program",
        "remote" => scoped && matches!(name, "uploadpack" | "receivepack"),
        "difftool" | "mergetool" | "man" | "browser" => scoped && name == "cmd",
        "protocol" => scoped && name == "allow",
        _ => false,
    }
}

/// Where the host keeps the mirror of the slot at `tree`: beside the slot,
/// under `.nunki-git/`.
///
/// Beside and never inside: a slot's tree is what its agent's container
/// mounts, and only the tree (SPEC 4.2), so a sibling is out of every
/// agent's reach. Derived from the tree alone because that is all most
/// callers hold of a slot.
pub fn mirror_of(tree: &Path) -> PathBuf {
    let parent = tree
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = tree
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "slot".into());
    parent.join(MIRRORS).join(name)
}

/// The directory, beside a project's slots, that holds their mirrors.
pub const MIRRORS: &str = ".nunki-git";

/// The ref namespace of the mirror that holds what the human's repository
/// said its branches were, the last time a slot was refreshed from it.
const ORIGIN_NS: &str = "refs/nunki/origin";

/// The file in the mirror that records which refs of the slot it last
/// mirrored. Equal to what the slot holds now, there is nothing to fetch.
const SYNCED: &str = "nunki-synced";

/// Git on a slot, from the host — the one way `nunki` runs git on a slot
/// (module documentation).
#[derive(Debug, Clone)]
pub struct SlotGit {
    tree: PathBuf,
    gitdir: PathBuf,
    mirror: PathBuf,
}

/// What a slot's refs say, read from its files: `HEAD`, and every branch,
/// remote-tracking branch and tag with the id it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Refs {
    head: Head,
    refs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Head {
    Branch(String),
    Detached(String),
}

impl SlotGit {
    /// Open the slot at `tree` and bring its mirror up to what its refs say
    /// now.
    ///
    /// A `.git` that is not a plain directory, refs that are not ids, objects
    /// missing or not what their id says: each is an error that names the
    /// slot, and none falls back to running git in it.
    pub fn open(tree: &Path) -> Result<Self, GitError> {
        let gitdir = tree.join(".git");
        let unreadable = |why: String| GitError::Unreadable {
            tree: tree.display().to_string(),
            why,
        };
        match std::fs::symlink_metadata(&gitdir) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => {
                return Err(unreadable(format!(
                    "{} is not a directory: a slot is a plain clone",
                    gitdir.display()
                )));
            }
            Err(e) => return Err(unreadable(format!("{}: {e}", gitdir.display()))),
        }
        let slot = Self {
            tree: tree.to_path_buf(),
            gitdir,
            mirror: mirror_of(tree),
        };
        slot.sync().map_err(|e| match e {
            GitError::Failed { verb, stderr, .. } => unreadable(format!("git {verb}: {stderr}")),
            other => other,
        })?;
        Ok(slot)
    }

    /// The slot's tree.
    pub fn tree(&self) -> &Path {
        &self.tree
    }

    /// The mirror's git directory.
    pub fn mirror(&self) -> &Path {
        &self.mirror
    }

    /// A git that reads the mirror and nothing of the slot's.
    fn git(&self) -> Command {
        let mut git = host_git();
        git.arg("--git-dir").arg(&self.mirror);
        git
    }

    /// A git with the mirror as git directory, the slot's tree as work tree,
    /// and `index` as index.
    fn git_in_tree(&self, index: &Path) -> Command {
        let mut git = host_git();
        git.env("GIT_INDEX_FILE", index)
            .current_dir(&self.tree)
            .arg("--git-dir")
            .arg(&self.mirror)
            .arg("--work-tree")
            .arg(&self.tree)
            .args(["-c", "core.bare=false"]);
        git
    }

    fn at(&self) -> String {
        format!("{} (through its host mirror)", self.tree.display())
    }

    /// Run git in the mirror and return its stdout, trimmed. `HEAD`, the
    /// branches, the remote-tracking branches and the tags are the slot's.
    pub fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let mut git = self.git();
        git.args(args);
        finish(git, args, &self.at(), None)
    }

    /// [`SlotGit::run`], with `input` on git's stdin.
    pub fn run_with_input(&self, args: &[&str], input: &[u8]) -> Result<String, GitError> {
        let mut git = self.git();
        git.args(args);
        finish(git, args, &self.at(), Some(input))
    }

    /// The raw output of git in the mirror: its stdout untrimmed when it
    /// succeeded, `None` otherwise.
    pub fn bytes(&self, args: &[&str]) -> Option<Vec<u8>> {
        let mut git = self.git();
        git.args(args);
        let out = output(&mut git, None).ok()?;
        out.status.success().then_some(out.stdout)
    }

    /// Run git against the slot's **working tree**, with an index `nunki`
    /// builds from `HEAD`: `status`, a `grep` of the tree. The slot's own
    /// index is never read.
    pub fn run_in_tree(&self, args: &[&str]) -> Result<String, GitError> {
        let index = self.fresh_index(false)?;
        let mut git = self.git_in_tree(index.path());
        git.args(args);
        finish(git, args, &self.at(), None)
    }

    /// The commit `HEAD` names.
    pub fn head(&self) -> Result<String, GitError> {
        self.run(&["rev-parse", "--verify", "HEAD^{commit}"])
    }

    /// The branch `HEAD` is on, or `HEAD` when it is detached.
    pub fn current_branch(&self) -> Result<String, GitError> {
        self.run(&["rev-parse", "--abbrev-ref", "HEAD"])
    }

    /// What differs between the slot's tree and its `HEAD`, as
    /// `git status --porcelain` says it, against `nunki`'s index.
    pub fn status(&self) -> Result<String, GitError> {
        self.run_in_tree(&["status", "--porcelain"])
    }

    /// Whether the slot's tree holds exactly its `HEAD`, untracked files
    /// that are not ignored included.
    pub fn is_clean(&self) -> Result<bool, GitError> {
        Ok(self.status()?.is_empty())
    }

    /// Whether `rev` names an object in the mirror.
    pub fn has(&self, rev: &str) -> bool {
        self.run(&["rev-parse", "--verify", "--quiet", rev]).is_ok()
    }

    /// An index of `HEAD` in a file of its own, built afresh: it carries no
    /// stat information, so git compares the tree's **content** with
    /// `HEAD`'s. Stat information is never kept from one call to the next:
    /// an agent that rewrites a file keeping its size, inode and time, within
    /// the second git compares a change time to, would read as unchanged
    /// against a kept index. `refresh` brings stat information up to the
    /// tree as it is now, for a two-way merge, which refuses an entry it
    /// cannot tell is unchanged.
    fn fresh_index(&self, refresh: bool) -> Result<Index, GitError> {
        let index = Index::new(&self.mirror);
        let args = ["read-tree", "HEAD"];
        let mut git = self.git_in_tree(index.path());
        git.args(args);
        finish(git, &args, &self.at(), None)?;
        if refresh {
            // `-q` carries on past a modified file, which is an answer and
            // not a failure.
            let args = ["update-index", "-q", "--refresh"];
            let mut git = self.git_in_tree(index.path());
            git.args(args);
            finish(git, &args, &self.at(), None)?;
        }
        Ok(index)
    }

    /// Put the slot's tree back to its `HEAD` and remove everything
    /// untracked, ignored files included: `reset --hard` then `clean -xdff`,
    /// played from the mirror. The slot's index is then rewritten from
    /// `nunki`'s, so that its own git sees what its tree now holds.
    pub fn reset_hard(&self) -> Result<(), GitError> {
        let index = self.fresh_index(false)?;
        for args in [
            &["read-tree", "--reset", "-u", "HEAD"][..],
            &["clean", "-qxdff"][..],
        ] {
            let mut git = self.git_in_tree(index.path());
            git.args(args);
            finish(git, args, &self.at(), None)?;
        }
        self.give_index(&index)?;
        Ok(())
    }

    /// Put the slot on its existing branch `branch`, carrying uncommitted
    /// changes the way `git checkout` does and refusing where it would.
    pub fn checkout(&self, branch: &str) -> Result<(), GitError> {
        let target = self.run(&[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{branch}^{{commit}}"),
        ])?;
        let index = self.fresh_index(true)?;
        let args = ["read-tree", "-m", "-u", "HEAD", &target];
        let mut git = self.git_in_tree(index.path());
        git.args(args);
        finish(git, &args, &self.at(), None)?;
        self.give_index(&index)?;
        self.write_into_slot("HEAD", format!("ref: refs/heads/{branch}\n").as_bytes())?;
        self.forget_sync()?;
        self.sync()
    }

    /// Create `branch` on `start` in the slot and put the slot on it.
    /// `start` must be in the mirror; what the slot does not have of it is
    /// handed to the slot first.
    pub fn create_branch(&self, branch: &str, start: &str) -> Result<(), GitError> {
        let start = self.run(&["rev-parse", "--verify", &format!("{start}^{{commit}}")])?;
        let name = format!("refs/heads/{branch}");
        let mut git = host_git();
        git.args(["check-ref-format", &name]);
        finish(git, &["check-ref-format"], &self.at(), None)?;
        if self.has(&name) {
            return Err(GitError::Failed {
                verb: "branch".into(),
                at: self.at(),
                stderr: format!("{name} already exists"),
            });
        }
        self.give_objects(&[start.as_str()])?;
        self.write_into_slot(&name, format!("{start}\n").as_bytes())?;
        self.forget_sync()?;
        self.sync()?;
        self.checkout(branch)
    }

    /// Bring the branches of the human's repository at `origin` into the
    /// mirror, and hand them to the slot as its `origin/*`: what
    /// `git fetch origin` did in the slot, without a git there.
    ///
    /// `origin` is the caller's, never read from the slot's configuration,
    /// where an agent could have pointed it anywhere.
    pub fn fetch_origin(&self, origin: &Path) -> Result<Vec<String>, GitError> {
        let spec = format!("+refs/heads/*:{ORIGIN_NS}/*");
        let origin = origin.display().to_string();
        let args = [
            "fetch",
            "--quiet",
            "--no-tags",
            "--prune",
            "--no-write-fetch-head",
            origin.as_str(),
            spec.as_str(),
        ];
        let mut git = self.git();
        git.args(args);
        finish(git, &args, &self.at(), None)?;
        let listed = self.run(&[
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            ORIGIN_NS,
        ])?;
        let branches: Vec<(String, String)> = listed
            .lines()
            .filter_map(|l| l.split_once(' '))
            .filter_map(|(id, name)| {
                let short = name.strip_prefix(ORIGIN_NS)?.strip_prefix('/')?;
                Some((id.to_string(), short.to_string()))
            })
            .collect();
        let tips: Vec<&str> = branches.iter().map(|(id, _)| id.as_str()).collect();
        self.give_objects(&tips)?;
        for (id, short) in &branches {
            self.write_into_slot(
                &format!("refs/remotes/origin/{short}"),
                format!("{id}\n").as_bytes(),
            )?;
        }
        self.forget_sync()?;
        self.sync()?;
        Ok(branches.into_iter().map(|(_, b)| b).collect())
    }

    /// The commit the human's repository had on `branch` at the last
    /// [`SlotGit::fetch_origin`].
    pub fn origin_branch(&self, branch: &str) -> Option<String> {
        self.run(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{ORIGIN_NS}/{branch}^{{commit}}"),
        ])
        .ok()
    }

    /// Write into the slot's object store, as one pack, every object
    /// reachable from `tips` that no ref of the slot already reaches.
    fn give_objects(&self, tips: &[&str]) -> Result<(), GitError> {
        if tips.is_empty() {
            return Ok(());
        }
        let mut revs = tips.join("\n");
        revs.push('\n');
        let mirrored = self.run(&[
            "for-each-ref",
            "--format=%(objectname)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?;
        for id in mirrored.lines().filter(|l| !l.is_empty()) {
            revs.push_str(&format!("^{id}\n"));
        }
        // Nothing to hand over is no pack at all, rather than an empty one.
        let wanted = self.run_with_input(&["rev-list", "--objects", "--stdin"], revs.as_bytes())?;
        if wanted.is_empty() {
            return Ok(());
        }
        let staging = self.mirror.join(format!("pack-{}", unique()));
        std::fs::create_dir_all(&staging).map_err(|e| GitError::Io(staging.clone(), e))?;
        let base = staging.join("pack");
        let base = base.display().to_string();
        let given = (|| {
            self.run_with_input(
                &["pack-objects", "--revs", "--quiet", &base],
                revs.as_bytes(),
            )?;
            let files: Vec<PathBuf> = std::fs::read_dir(&staging)
                .map_err(|e| GitError::Io(staging.clone(), e))?
                .flatten()
                .map(|e| e.path())
                .collect();
            for file in in_writing_order(files) {
                let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let bytes = std::fs::read(&file).map_err(|e| GitError::Io(file.clone(), e))?;
                self.write_into_slot(&format!("objects/pack/{name}"), &bytes)?;
            }
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&staging);
        given
    }

    /// Hand the slot `nunki`'s index as its own.
    fn give_index(&self, index: &Index) -> Result<(), GitError> {
        let bytes =
            std::fs::read(index.path()).map_err(|e| GitError::Io(index.path().into(), e))?;
        self.write_into_slot("index", &bytes)
    }

    /// Write `bytes` at `relative` under the slot's `.git`, as a file
    /// renamed into place.
    ///
    /// No directory on the way may be a symbolic link: the agent writes this
    /// `.git`, and a link would carry the host's write wherever it points. A
    /// missing directory is made; a link, or anything that is not a
    /// directory, is an error. The rename replaces a link at the last
    /// component rather than writing through it.
    fn write_into_slot(&self, relative: &str, bytes: &[u8]) -> Result<(), GitError> {
        let unsafe_path = |why: String| GitError::Unreadable {
            tree: self.tree.display().to_string(),
            why,
        };
        let parts: Vec<&str> = relative.split('/').collect();
        if parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == "..")
        {
            return Err(unsafe_path(format!("{relative:?} is no path inside .git")));
        }
        let mut dir = self.gitdir.clone();
        for part in &parts[..parts.len() - 1] {
            dir.push(part);
            match std::fs::symlink_metadata(&dir) {
                Ok(m) if m.is_dir() => {}
                Ok(_) => {
                    return Err(unsafe_path(format!(
                        "{} is not a directory, and nunki writes nothing through it",
                        dir.display()
                    )));
                }
                // Missing, or not even looked at: `mkdir` makes a directory or
                // fails, and never follows a link, so its own error is the one
                // worth reporting.
                Err(_) => {
                    std::fs::create_dir(&dir).map_err(|e| GitError::Io(dir.clone(), e))?;
                }
            }
        }
        let last = parts[parts.len() - 1];
        let file = dir.join(last);
        let staged = dir.join(format!(".{last}.nunki-{}", unique()));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&staged);
            return Err(GitError::Io(staged, e));
        }
        std::fs::rename(&staged, &file).map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            GitError::Io(file.clone(), e)
        })
    }

    /// Make the next [`SlotGit::sync`] fetch, whatever the record says.
    fn forget_sync(&self) -> Result<(), GitError> {
        let record = self.mirror.join(SYNCED);
        match std::fs::remove_file(&record) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(GitError::Io(record, e)),
        }
    }

    /// Bring the mirror to what the slot's refs say now.
    fn sync(&self) -> Result<(), GitError> {
        let _lock = MirrorLock::take(&self.mirror)?;
        let refs = read_refs(&self.tree, &self.gitdir)?;
        let said = refs.said();
        let record = self.mirror.join(SYNCED);
        if std::fs::read_to_string(&record).is_ok_and(|r| r == said) {
            return Ok(());
        }
        self.init_mirror(refs.format())?;
        self.forget_sync()?;

        let shim = self.mirror.join(format!("shim-{}", unique()));
        let fetched = self.fetch_through(&shim, &refs);
        let _ = std::fs::remove_dir_all(&shim);
        fetched?;

        match &refs.head {
            Head::Branch(name) => self.run(&["symbolic-ref", "HEAD", name])?,
            Head::Detached(id) => self.run(&["update-ref", "--no-deref", "HEAD", id])?,
        };
        std::fs::write(&record, said).map_err(|e| GitError::Io(record, e))
    }

    /// A mirror whose objects are in `format`: made when there is none, and
    /// made again when the slot's repository is no longer in the format it
    /// was mirrored in. Everything in it is the host's, and all of it comes
    /// back from the slot.
    fn init_mirror(&self, format: &str) -> Result<(), GitError> {
        if self.mirror.join("HEAD").is_file() {
            if self.run(&["rev-parse", "--show-object-format"])? == format {
                return Ok(());
            }
            std::fs::remove_dir_all(&self.mirror)
                .map_err(|e| GitError::Io(self.mirror.clone(), e))?;
        }
        init_bare(&self.mirror, format)
    }

    /// Fetch into the mirror, from a shim at `shim` that borrows the slot's
    /// objects and holds `refs`, every ref the slot has.
    fn fetch_through(&self, shim: &Path, refs: &Refs) -> Result<(), GitError> {
        self.make_shim(shim, refs.format())?;

        // `-z`: a ref name is a file name the agent chose, and a newline in
        // it must not become a second command.
        let mut commands = Vec::new();
        for (name, id) in &refs.refs {
            commands.extend_from_slice(format!("create {name}\0{id}\0").as_bytes());
        }
        if let Head::Detached(id) = &refs.head {
            commands.extend_from_slice(format!("create refs/slot-head/HEAD\0{id}\0").as_bytes());
        }
        let mut git = host_git();
        git.arg("--git-dir")
            .arg(shim)
            .args(["update-ref", "-z", "--stdin"]);
        finish(git, &["update-ref"], &self.at(), Some(&commands))?;

        self.fetch_from_shim(
            shim,
            &[
                "--prune",
                "+refs/heads/*:refs/heads/*",
                "+refs/remotes/*:refs/remotes/*",
                "+refs/tags/*:refs/tags/*",
                "+refs/slot-head/*:refs/slot-head/*",
            ],
        )
    }

    /// Make sure the commit `id` is in the mirror, when the slot holds it
    /// though no ref of the slot reaches it any more: a campaign's `HEAD`
    /// before an amend. Fetched through a shim like everything else, so it
    /// is hashed on the way in, and kept under `refs/nunki/kept/`.
    ///
    /// Nothing for a name that is not a full id: what a name means is what
    /// the mirrored refs say.
    pub fn fetch_commit(&self, id: &str) -> Result<(), GitError> {
        if !is_id(id) || self.has(&format!("{id}^{{commit}}")) {
            return Ok(());
        }
        let _lock = MirrorLock::take(&self.mirror)?;
        let format = if id.len() == 64 { "sha256" } else { "sha1" };
        let shim = self.mirror.join(format!("shim-{}", unique()));
        let fetched = self
            .make_shim(&shim, format)
            .and_then(|()| self.fetch_from_shim(&shim, &[&format!("+{id}:refs/nunki/kept/{id}")]));
        let _ = std::fs::remove_dir_all(&shim);
        fetched
    }

    /// A bare repository at `shim` whose objects are the slot's, borrowed:
    /// its configuration is `nunki`'s, its object store an alternate.
    fn make_shim(&self, shim: &Path, format: &str) -> Result<(), GitError> {
        init_bare(shim, format)?;
        let objects = std::fs::canonicalize(self.gitdir.join("objects")).map_err(|e| {
            GitError::Unreadable {
                tree: self.tree.display().to_string(),
                why: format!("its object store cannot be read: {e}"),
            }
        })?;
        let alternates = shim.join("objects").join("info").join("alternates");
        let line = objects.to_str().ok_or_else(|| GitError::Unreadable {
            tree: self.tree.display().to_string(),
            why: format!("{} is not a path git can be handed", objects.display()),
        })?;
        std::fs::write(&alternates, format!("{line}\n")).map_err(|e| GitError::Io(alternates, e))
    }

    /// Fetch `refspecs` into the mirror from the shim at `shim`.
    fn fetch_from_shim(&self, shim: &Path, refspecs: &[&str]) -> Result<(), GitError> {
        let mut git = self.git();
        git.args([
            // What borrows the slot's object store reads it as data only: a
            // commit-graph or a multi-pack index there could lie about
            // parents or offsets. `index-pack` would catch the result, and
            // these spare it having to.
            "-c",
            "core.commitGraph=false",
            "-c",
            "core.multiPackIndex=false",
            // The shim advertises the slot's refs, and a commit no ref
            // reaches can only be asked for by its id.
            "-c",
            "uploadpack.allowAnySHA1InWant=true",
            "-c",
            "gc.autoDetach=false",
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--update-head-ok",
        ])
        .arg(shim)
        .args(refspecs);
        finish(git, &["fetch"], &self.at(), None).map(|_| ())
    }
}

/// The files of a pack in the order they are handed to a slot: the pack
/// before its index. git finds a pack by its index, and must never find one
/// whose pack is not there yet.
fn in_writing_order(mut files: Vec<PathBuf>) -> Vec<PathBuf> {
    files.sort_by_key(|p| p.extension().is_some_and(|x| x == "idx"));
    files
}

/// `git init --bare` with no template: no sample hook, nothing but what git
/// needs.
fn init_bare(dir: &Path, format: &str) -> Result<(), GitError> {
    let mut git = host_git();
    git.args(["init", "--quiet", "--bare", "--template="])
        .arg(format!("--object-format={format}"))
        .arg(dir);
    finish(git, &["init"], &dir.display().to_string(), None).map(|_| ())
}

/// A git that reads no configuration but that of the repository it is
/// handed: not the system's, not the human's global one, none of the
/// caller's `GIT_*` variables, replace objects and grafts off.
fn host_git() -> Command {
    let mut git = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            git.env_remove(key);
        }
    }
    git.env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    git
}

fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// A name no other call of this process, nor another process, picks.
fn unique() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{nanos}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// An index file of one call's own, in the mirror.
struct Index(PathBuf);

impl Index {
    fn new(mirror: &Path) -> Self {
        Self(mirror.join(format!("index-{}", unique())))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Index {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// One sync of a mirror at a time.
struct MirrorLock(PathBuf);

impl MirrorLock {
    /// How long a lock may be held before it is taken for a crashed
    /// process's.
    const STALE: std::time::Duration = std::time::Duration::from_secs(300);
    const WAIT: std::time::Duration = std::time::Duration::from_secs(60);

    fn take(mirror: &Path) -> Result<Self, GitError> {
        Self::take_within(mirror, Self::STALE, Self::WAIT)
    }

    /// [`MirrorLock::take`], with a lock older than `stale` taken for a
    /// crashed process's, and `wait` the longest it waits for a live one.
    fn take_within(
        mirror: &Path,
        stale: std::time::Duration,
        wait: std::time::Duration,
    ) -> Result<Self, GitError> {
        // Beside the mirror rather than in it: a mirror made again is
        // removed whole, under this lock.
        let mut name = mirror.file_name().unwrap_or_default().to_os_string();
        name.push(".lock");
        let file = mirror.with_file_name(name);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| GitError::Io(parent.to_path_buf(), e))?;
        }
        let started = std::time::Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)
            {
                Ok(_) => return Ok(Self(file)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&file)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| beyond(age, stale));
                    if stale {
                        let _ = std::fs::remove_file(&file);
                        continue;
                    }
                    if beyond(started.elapsed(), wait) {
                        return Err(GitError::Io(file, e));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => return Err(GitError::Io(file, e)),
            }
        }
    }
}

/// Whether `spent` has gone past `limit`: a lock exactly `limit` old is not
/// yet stale, and a wait exactly `limit` long is not yet spent.
fn beyond(spent: std::time::Duration, limit: std::time::Duration) -> bool {
    spent > limit
}

impl Drop for MirrorLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Refs {
    /// The object format the ids are in: SHA-256 for 64 digits, SHA-1
    /// otherwise — and for a repository with no ref at all.
    fn format(&self) -> &'static str {
        let digits = match &self.head {
            Head::Detached(id) => Some(id.len()),
            Head::Branch(_) => self.refs.values().next().map(String::len),
        };
        if digits == Some(64) { "sha256" } else { "sha1" }
    }

    /// The record [`SYNCED`] holds: one line per ref, `HEAD` first.
    fn said(&self) -> String {
        let mut out = match &self.head {
            Head::Branch(name) => format!("HEAD ref {name}\n"),
            Head::Detached(id) => format!("HEAD {id}\n"),
        };
        for (name, id) in &self.refs {
            out.push_str(&format!("{id} {name}\n"));
        }
        out
    }
}

/// The namespaces of a slot's refs the mirror holds. `refs/replace/` is not
/// among them, and never will be.
const MIRRORED: [&str; 3] = ["refs/heads/", "refs/remotes/", "refs/tags/"];

/// Read a slot's refs from its files, the way git stores them in a clone:
/// `HEAD`, `packed-refs`, then the loose refs, which win.
fn read_refs(tree: &Path, gitdir: &Path) -> Result<Refs, GitError> {
    let unreadable = |why: String| GitError::Unreadable {
        tree: tree.display().to_string(),
        why,
    };
    let head_file = gitdir.join("HEAD");
    let head = regular_file(&head_file).map_err(&unreadable)?;
    let head = head.trim();
    let head = match head.strip_prefix("ref:") {
        Some(name) if name.trim().starts_with("refs/heads/") => {
            Head::Branch(name.trim().to_string())
        }
        Some(name) => {
            return Err(unreadable(format!(
                "HEAD points at {:?}, which is not a branch",
                name.trim()
            )));
        }
        None if is_id(head) => Head::Detached(head.to_string()),
        None => {
            return Err(unreadable(format!(
                "HEAD holds {head:?}, which is no commit id"
            )));
        }
    };

    let mut refs = BTreeMap::new();
    let packed = gitdir.join("packed-refs");
    match std::fs::symlink_metadata(&packed) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => {
            let text = regular_file(&packed).map_err(&unreadable)?;
            for line in text.lines() {
                if line.starts_with('#') || line.starts_with('^') || line.trim().is_empty() {
                    continue;
                }
                let Some((id, name)) = line.split_once(' ') else {
                    return Err(unreadable(format!("packed-refs holds {line:?}")));
                };
                if !is_id(id) {
                    return Err(unreadable(format!("packed-refs holds {line:?}")));
                }
                if MIRRORED.iter().any(|ns| name.starts_with(ns)) {
                    refs.insert(name.to_string(), id.to_string());
                }
            }
        }
    }
    for ns in MIRRORED {
        let ns = ns.trim_end_matches('/');
        for dir in [gitdir.join("refs"), gitdir.join(ns)] {
            if std::fs::symlink_metadata(&dir).is_ok_and(|m| !m.is_dir()) {
                return Err(unreadable(format!(
                    "{} is not a directory, and nunki follows no link in a slot's .git",
                    dir.display()
                )));
            }
        }
        loose(gitdir, &gitdir.join(ns), ns, &mut refs).map_err(&unreadable)?;
    }
    for name in refs.keys() {
        if name.chars().any(|c| c.is_control() || c == ' ') {
            return Err(unreadable(format!("a ref is named {name:?}")));
        }
    }
    let lengths: std::collections::BTreeSet<usize> = refs
        .values()
        .chain(match &head {
            Head::Detached(id) => Some(id),
            Head::Branch(_) => None,
        })
        .map(String::len)
        .collect();
    if lengths.len() > 1 {
        return Err(unreadable("its refs mix SHA-1 and SHA-256 ids".to_string()));
    }
    Ok(Refs { head, refs })
}

/// The loose refs under `dir`, named from `prefix`.
fn loose(
    gitdir: &Path,
    dir: &Path,
    prefix: &str,
    refs: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Err(format!("{} is not a name a ref can have", path.display()));
        };
        let full = format!("{prefix}/{name}");
        let meta =
            std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.is_dir() {
            loose(gitdir, &path, &full, refs)?;
            continue;
        }
        if name.ends_with(".lock") {
            continue;
        }
        let text = regular_file(&path)?;
        let text = text.trim();
        if text.starts_with("ref:") {
            // A symbolic ref — `origin/HEAD`, as a clone writes it. What it
            // points at is mirrored under its own name.
            continue;
        }
        if !is_id(text) {
            return Err(format!(
                "{} holds {text:?}, which is no object id",
                path.strip_prefix(gitdir).unwrap_or(&path).display()
            ));
        }
        refs.insert(full, text.to_string());
    }
    Ok(())
}

/// The content of `path`, refused when it is anything but a regular file.
fn regular_file(path: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!(
            "{} is not a regular file, and nunki follows no link in a slot's .git",
            path.display()
        ));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// A full object id, as git writes one: SHA-1 or SHA-256.
fn is_id(text: &str) -> bool {
    (text.len() == 40 || text.len() == 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn slot_in(dir: &Path) -> SlotGit {
        let tree = dir.join("slot");
        std::fs::create_dir_all(tree.join(".git")).unwrap();
        SlotGit {
            gitdir: tree.join(".git"),
            mirror: mirror_of(&tree),
            tree,
        }
    }

    /// A path that leaves `.git`, or names nothing, is refused before
    /// anything is written: an empty component, `.` and `..` each.
    #[test]
    fn a_write_into_a_slot_refuses_a_path_that_is_not_one() {
        let dir = tempfile::tempdir().unwrap();
        let slot = slot_in(dir.path());
        for bad in ["refs//x", "./x", "refs/./x", "../x", "refs/../../x", ""] {
            let err = slot.write_into_slot(bad, b"x").unwrap_err();
            assert!(
                err.to_string().contains("is no path inside .git"),
                "{bad}: {err}"
            );
        }
        assert!(!dir.path().join("x").exists());
        slot.write_into_slot("refs/heads/ok", b"x").unwrap();
        assert_eq!(
            std::fs::read(slot.gitdir.join("refs/heads/ok")).unwrap(),
            b"x"
        );
    }

    #[test]
    fn a_pack_is_handed_over_before_its_index() {
        let order = in_writing_order(vec![
            PathBuf::from("pack-a.idx"),
            PathBuf::from("pack-a.pack"),
        ]);
        assert_eq!(
            order,
            [PathBuf::from("pack-a.pack"), PathBuf::from("pack-a.idx")]
        );
        let order = in_writing_order(vec![
            PathBuf::from("pack-a.pack"),
            PathBuf::from("pack-a.idx"),
        ]);
        assert_eq!(
            order,
            [PathBuf::from("pack-a.pack"), PathBuf::from("pack-a.idx")]
        );
    }

    /// A lock exactly as old as the limit is still its holder's, and a wait
    /// exactly as long as the limit is not yet given up: only beyond it.
    #[test]
    fn a_limit_is_reached_only_beyond_it() {
        let limit = Duration::from_secs(300);
        assert!(!beyond(limit, limit));
        assert!(!beyond(limit - Duration::from_nanos(1), limit));
        assert!(beyond(limit + Duration::from_nanos(1), limit));
    }

    fn lock_file(mirror: &Path) -> PathBuf {
        mirror.with_file_name(format!(
            "{}.lock",
            mirror.file_name().unwrap().to_string_lossy()
        ))
    }

    /// A lock somebody holds is waited for, and taken once it is released:
    /// neither stolen while fresh nor refused at once.
    #[test]
    fn a_held_mirror_lock_is_waited_for() {
        let dir = tempfile::tempdir().unwrap();
        let mirror = dir.path().join(MIRRORS).join("one");
        std::fs::create_dir_all(mirror.parent().unwrap()).unwrap();
        let file = lock_file(&mirror);
        std::fs::write(&file, "").unwrap();
        let released = file.clone();
        let other = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            std::fs::remove_file(released).unwrap();
        });
        let started = Instant::now();
        let lock =
            MirrorLock::take_within(&mirror, Duration::from_secs(300), Duration::from_secs(20));
        other.join().unwrap();
        assert!(lock.is_ok(), "{:?}", lock.err());
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "a fresh lock was taken without waiting for its holder"
        );
        drop(lock);
        assert!(!file.exists(), "the lock is released when dropped");
    }

    /// A lock older than `stale` is a crashed process's, and taken at once.
    #[test]
    fn a_stale_mirror_lock_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let mirror = dir.path().join(MIRRORS).join("one");
        std::fs::create_dir_all(mirror.parent().unwrap()).unwrap();
        let file = lock_file(&mirror);
        std::fs::write(&file, "").unwrap();
        let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(hour_ago)
            .unwrap();
        let started = Instant::now();
        let lock =
            MirrorLock::take_within(&mirror, Duration::from_secs(300), Duration::from_secs(10));
        assert!(lock.is_ok(), "{:?}", lock.err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A lock nobody releases is an error once `wait` is spent — not a
    /// wait without end.
    #[test]
    fn a_mirror_lock_never_released_is_an_error_after_the_wait() {
        let dir = tempfile::tempdir().unwrap();
        let mirror = dir.path().join(MIRRORS).join("one");
        std::fs::create_dir_all(mirror.parent().unwrap()).unwrap();
        std::fs::write(lock_file(&mirror), "").unwrap();
        // In a thread of its own, so that a wait without end fails this test
        // rather than hanging it.
        let (sent, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let started = Instant::now();
            let lock = MirrorLock::take_within(
                &mirror,
                Duration::from_secs(300),
                Duration::from_millis(200),
            );
            let _ = sent.send((lock.is_err(), started.elapsed()));
        });
        let (refused, spent) = answer
            .recv_timeout(Duration::from_secs(5))
            .expect("a lock never released is waited for without end");
        assert!(refused);
        assert!(spent >= Duration::from_millis(200), "{spent:?}");
    }

    /// A lock that cannot be created for another reason than its being
    /// held is an error at once, not a wait.
    #[test]
    fn a_mirror_lock_that_cannot_be_written_is_an_error_at_once() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mirrors = dir.path().join(MIRRORS);
        std::fs::create_dir_all(&mirrors).unwrap();
        std::fs::set_permissions(&mirrors, std::fs::Permissions::from_mode(0o555)).unwrap();
        let started = Instant::now();
        let lock = MirrorLock::take_within(
            &mirrors.join("one"),
            Duration::from_secs(300),
            Duration::from_secs(10),
        );
        std::fs::set_permissions(&mirrors, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(lock.is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
