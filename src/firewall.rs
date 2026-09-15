//! The firewall sidecar's build context, carried by the binary (SPEC 4.1
//! bis).
//!
//! It does **not** live in a project's stack fragments, and it never will:
//! those are what `nunki init` writes for a project, in that project's home,
//! and this is `nunki`'s own asset. Two reasons beyond tidiness. A project's
//! fragments are the project's to edit, and the firewall is not; and the
//! file describing the agent's cage has no business anywhere an agent or a
//! project could change it.
//!
//! So the sources are embedded at compile time and written out to a build
//! context when a slot needs the image. The binary is self-contained, which
//! is one of the reasons the engine is written in Rust at all (SPEC 4.2).

use std::io;
use std::path::Path;

/// One file of the build context: its name, its bytes, and whether it must
/// come out executable.
pub struct Asset {
    pub name: &'static str,
    pub contents: &'static str,
    pub executable: bool,
}

/// Everything `docker build` needs, in the order it is written.
pub const BUILD_CONTEXT: [Asset; 3] = [
    Asset {
        name: "Dockerfile",
        contents: include_str!("../assets/firewall/Dockerfile"),
        executable: false,
    },
    Asset {
        name: "entrypoint.sh",
        contents: include_str!("../assets/firewall/entrypoint.sh"),
        executable: true,
    },
    Asset {
        name: "fw-ready",
        contents: include_str!("../assets/firewall/fw-ready"),
        executable: true,
    },
];

/// The prober's build context: one file, and its reason is in it.
pub const PROBER_CONTEXT: [Asset; 1] = [Asset {
    name: "Dockerfile",
    contents: include_str!("../assets/prober/Dockerfile"),
    executable: false,
}];

/// Write the prober's build context into `dir`.
pub fn materialise_prober(dir: &Path) -> io::Result<()> {
    write(dir, &PROBER_CONTEXT)
}

/// Write the build context into `dir`, which must exist. The two scripts come
/// out executable: a Dockerfile that `COPY`s them can chmod, but an
/// entrypoint that arrives without its bit is a sidecar that fails to start,
/// and a sidecar that fails to start is an agent that never runs.
pub fn materialise(dir: &Path) -> io::Result<()> {
    write(dir, &BUILD_CONTEXT)
}

fn write(dir: &Path, assets: &[Asset]) -> io::Result<()> {
    for asset in assets {
        let path = dir.join(asset.name);
        std::fs::write(&path, asset.contents)?;
        if asset.executable {
            set_executable(&path)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}
