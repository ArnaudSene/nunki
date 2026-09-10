//! Running an engine's command line and waiting for it.
//!
//! The seam exists for the same reason [`crate::harness::spawn::Spawner`]
//! does: the adapter builds plain data, something else executes it, and a
//! test can assert on the command without an engine on the machine.

use std::io;
use std::process::{Command, Output};

use crate::harness::spawn::CommandSpec;

pub trait Cli: Send + Sync {
    fn run(&self, spec: &CommandSpec) -> io::Result<Output>;
}

/// Runs it for real.
#[derive(Debug, Default)]
pub struct RealCli;

impl Cli for RealCli {
    fn run(&self, spec: &CommandSpec) -> io::Result<Output> {
        Command::new(&spec.program)
            .args(&spec.args)
            .envs(&spec.env)
            .output()
    }
}
