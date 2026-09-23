//! What this repository must not carry.
//!
//! `nunki` keeps a project's tooling at `~/.nunki/<project>/` — `nunki.yaml`,
//! the HQ, and the stack fragments the slots mount read-only. The project's
//! own history holds none of it, which is the whole point of that layout: a
//! development tool is not a deliverable, and a second copy under version
//! control is a copy nothing reads.
//!
//! Its own repository had kept six of those files since before the move.
//! Measured on 2026-09-23: four of the six differed from the live ones at
//! `~/.nunki/2564429a-…/stacks/rust/`, and four more files that home carries
//! — `security.sh`, `system.sh`, `advisories.txt`, `caches.txt` — were not
//! there at all. Nothing read them, nothing updated them, and nothing said
//! so. Someone editing one would have been editing a file with no effect.
//!
//! `.gitignore` alone would not have caught it: a path already tracked stays
//! tracked whatever `.gitignore` says. This asks git what it actually holds.

use std::path::Path;
use std::process::Command;

/// Paths this repository must not track, and why each one is out.
const REFUSED: [(&str, &str); 2] = [
    (
        ".nunki/",
        "the stack fragments and the HQ live at `~/.nunki/<project>/`; \
         a copy here is a second copy nothing reads",
    ),
    (
        "nunki.yaml",
        "a project's configuration lives beside its HQ, not in its history",
    ),
];

fn tracked_files(root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .expect("git ls-files");
    assert!(out.status.success(), "git ls-files failed in {root:?}");
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_repository_does_not_carry_its_own_tooling() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tracked = tracked_files(root);
    assert!(
        !tracked.is_empty(),
        "no tracked file found — the test is not looking at a repository"
    );

    let mut offending: Vec<String> = Vec::new();
    for (refused, why) in REFUSED {
        for path in &tracked {
            let hit = match refused.strip_suffix('/') {
                Some(dir) => path == dir || path.starts_with(&format!("{dir}/")),
                None => path == refused,
            };
            if hit {
                offending.push(format!("{path} — {why}"));
            }
        }
    }
    assert!(
        offending.is_empty(),
        "these are tracked and must not be:\n{}",
        offending.join("\n")
    );
}

/// The other direction: the check reads the real index rather than agreeing
/// with itself. A path this repository *does* carry is found by the same
/// matching, so a green verdict above means "not there", never "not looked
/// for".
#[test]
fn the_same_matching_finds_a_path_the_repository_does_carry() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tracked = tracked_files(root);
    assert!(
        tracked.iter().any(|p| p == "Cargo.toml"),
        "Cargo.toml is tracked and the exact-name match has to find it"
    );
    assert!(
        tracked.iter().any(|p| p.starts_with("src/")),
        "src/ is tracked and the directory-prefix match has to find it"
    );
}
