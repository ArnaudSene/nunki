//! `git push`, authenticated with the project's token and with nothing else
//! (SPEC 4.2, `nunki push`).
//!
//! The owner's personal credentials are never used: not a credential helper
//! (system, global, local, or scoped to a URL — what `gh auth setup-git`
//! installs), not a keychain, not `~/.netrc`, not an askpass program, not an
//! SSH key. The push goes over HTTPS to the GitHub repository the remote
//! names, whatever form the remote has, and git is run so that it **cannot
//! read any of the owner's git configuration**:
//!
//! - from a scratch bare repository of its own, whose objects are borrowed
//!   from the project's through `objects/info/alternates`: the checkout's
//!   `.git/config` — its helpers, its URL rewrites, its askpass — is not read
//!   by the command at all, and is left exactly as it was;
//! - with an environment rebuilt from nothing but `PATH` ([`environment`]):
//!   no system configuration (`GIT_CONFIG_NOSYSTEM`), none global
//!   (`GIT_CONFIG_GLOBAL=/dev/null`), a `HOME` that is the empty scratch
//!   directory — so curl finds no `.netrc` — no inherited `GIT_*`, askpass or
//!   SSH variable, and no prompt (`GIT_TERMINAL_PROMPT=0`);
//! - with the configuration it does get on its command line only
//!   ([`config_args`]): every helper reset, then one helper scoped to the
//!   forge's address, which prints the token from the command's own
//!   environment ([`TOKEN_ENV`]) — so the token is never on a command line,
//!   in a URL, in a file or in any configuration — redirects not followed,
//!   and every protocol but the forge's refused.
//!
//! A URL rewrite (`url.<base>.insteadOf`, `pushInsteadOf`) is never followed
//! because no configuration that could hold one is read.

use std::ffi::OsString;
use std::path::Path;

use super::{Repo, Token};

/// The variable the token travels in, set on the one `git push` command and
/// read by the helper [`config_args`] declares. Its name only is ever written
/// anywhere.
pub const TOKEN_ENV: &str = "NUNKI_FORGE_TOKEN";

/// The user name the token is presented under. GitHub reads the token as
/// the password and accepts any user name; this one says what it is.
pub const USERNAME: &str = "x-access-token";

/// The only variable taken from the environment nunki was started with:
/// what finds `git` and the helpers git runs. Nothing else is inherited.
const INHERITED: &[&str] = &["PATH"];

#[derive(Debug, thiserror::Error)]
pub enum GitPushError {
    #[error(
        "{owner}/{name} is not a GitHub repository name nunki can push to: a name holds \
         letters, digits, `-`, `_` and `.` only"
    )]
    NotARepository { owner: String, name: String },
    #[error("nunki could not prepare the push in a scratch repository: {0}")]
    Scratch(String),
    #[error(
        "the forge refused the push to {url}: {said}. A token without Contents: read and \
         write on this repository, or an expired one, is refused this way — nothing was \
         pushed, and no pull request is opened"
    )]
    Refused { url: String, said: String },
}

/// What one push needs: the repository whose objects are pushed, where to,
/// what, and with which token.
pub struct Push<'a> {
    /// The project's repository, whose objects the push borrows and whose
    /// configuration it never reads.
    pub objects_of: &'a Path,
    /// The forge's address for git, with its trailing `/`:
    /// [`super::github::GIT`] for the binary, a local server in tests.
    pub base: &'a str,
    pub repo: &'a Repo,
    /// The commit pushed, by id.
    pub commit: &'a str,
    /// The branch it lands on, on the forge.
    pub branch: &'a str,
    pub token: &'a Token,
    /// The environment nunki was started with, of which only [`INHERITED`]
    /// is kept.
    pub inherited: &'a [(OsString, OsString)],
}

/// The address git pushes to: the forge's base, then `owner/name.git`. A
/// name GitHub would not accept is refused rather than put in a URL, where a
/// `?`, an `@` or a `/` could change what the address means.
pub fn url(base: &str, repo: &Repo) -> Result<String, GitPushError> {
    let plain = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if !plain(&repo.owner) || !plain(&repo.name) {
        return Err(GitPushError::NotARepository {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
        });
    }
    Ok(format!("{base}{}/{}.git", repo.owner, repo.name))
}

/// The configuration the push is given, on its command line, as `-c`
/// pairs. None of it holds the token.
pub fn config_args(base: &str) -> Vec<String> {
    let scheme = base.split_once("://").map_or("https", |(s, _)| s);
    let helper = format!(
        "!f() {{ test \"$1\" = get || exit 0; printf 'username={USERNAME}\\npassword=%s\\n' \
         \"${TOKEN_ENV}\"; }}; f"
    );
    [
        // Every helper any level declared is dropped — an empty value resets
        // the list — then the one that answers for the forge alone.
        "credential.helper=".to_string(),
        format!("credential.{base}.helper={helper}"),
        // An askpass from a configuration is not used either.
        "core.askPass=".to_string(),
        // A redirect could carry the request, and so the token, to another
        // address.
        "http.followRedirects=false".to_string(),
        // The forge's protocol, and no other: no SSH, no file, no ext.
        "protocol.allow=never".to_string(),
        format!("protocol.{scheme}.allow=always"),
    ]
    .into_iter()
    .flat_map(|pair| ["-c".to_string(), pair])
    .collect()
}

/// The push's whole environment: [`INHERITED`] from `inherited`, then what
/// closes every other source of a credential or a configuration. `home` is
/// the scratch directory, empty: curl looks for `.netrc` there, and finds
/// none — and an unset `HOME` would send it to the account's home directory.
pub fn environment(inherited: &[(OsString, OsString)], home: &Path) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = inherited
        .iter()
        .filter(|(k, _)| INHERITED.iter().any(|keep| k == keep))
        .cloned()
        .collect();
    for (k, v) in [
        ("GIT_CONFIG_NOSYSTEM", OsString::from("1")),
        ("GIT_CONFIG_GLOBAL", OsString::from("/dev/null")),
        ("GIT_TERMINAL_PROMPT", OsString::from("0")),
        ("HOME", home.as_os_str().to_owned()),
        ("LC_ALL", OsString::from("C")),
    ] {
        env.push((OsString::from(k), v));
    }
    env
}

/// Push `push.commit` to `push.branch` on the forge, with the token and with
/// nothing else. What git said when it fails is given back with the token, if
/// it ever appeared there, taken out.
pub fn push(push: &Push) -> Result<(), GitPushError> {
    let url = url(push.base, push.repo)?;
    let scratch = tempfile::Builder::new()
        .prefix("nunki-push-")
        .tempdir()
        .map_err(|e| GitPushError::Scratch(e.to_string()))?;
    let bare = scratch.path().join("repo.git");
    let home = scratch.path().join("home");
    std::fs::create_dir(&home).map_err(|e| GitPushError::Scratch(e.to_string()))?;
    let env = environment(push.inherited, &home);

    // The project's object format and objects, read from the project; the
    // only git run there, and a local one.
    let format = crate::git::run(push.objects_of, &["rev-parse", "--show-object-format"])
        .map_err(|e| GitPushError::Scratch(e.to_string()))?;
    let objects = crate::git::run(
        push.objects_of,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ],
    )
    .map_err(|e| GitPushError::Scratch(e.to_string()))?;

    let git = |args: &[&str]| {
        let mut command = std::process::Command::new("git");
        command.env_clear().envs(env.iter().cloned()).args(args);
        command
    };
    let init = git(&[
        "init",
        "--bare",
        "-q",
        &format!("--object-format={format}"),
        &bare.display().to_string(),
    ])
    .output()
    .map_err(|e| GitPushError::Scratch(e.to_string()))?;
    if !init.status.success() {
        return Err(GitPushError::Scratch(
            String::from_utf8_lossy(&init.stderr).trim().to_string(),
        ));
    }
    std::fs::write(bare.join("objects/info/alternates"), format!("{objects}\n"))
        .map_err(|e| GitPushError::Scratch(e.to_string()))?;

    let args = push_args(push.base, &bare, &url, push.commit, push.branch);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = git(&args)
        .env(TOKEN_ENV, push.token.secret())
        .output()
        .map_err(|e| GitPushError::Scratch(e.to_string()))?;
    if out.status.success() {
        return Ok(());
    }
    Err(GitPushError::Refused {
        url,
        said: redact(&String::from_utf8_lossy(&out.stderr), push.token.secret()),
    })
}

/// `text`, with every occurrence of `secret` replaced, and on one line.
pub fn redact(text: &str, secret: &str) -> String {
    let text = if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "[the token]")
    };
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The whole `git push` command line, after `git`: the configuration of
/// [`config_args`], the scratch repository, and the push of `commit` to
/// `branch` at `url` — by id, to the full ref, so no name in the scratch
/// repository is consulted. `--no-verify`, so no hook a template could have
/// put in the scratch repository runs with the token in its environment.
pub fn push_args(base: &str, git_dir: &Path, url: &str, commit: &str, branch: &str) -> Vec<String> {
    let mut args = config_args(base);
    args.extend([
        format!("--git-dir={}", git_dir.display()),
        "push".to_string(),
        "--no-verify".to_string(),
        "--quiet".to_string(),
        url.to_string(),
        format!("{commit}:refs/heads/{branch}"),
    ]);
    args
}
