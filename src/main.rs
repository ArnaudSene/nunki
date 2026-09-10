//! `hq` — a mission orchestrator for AI coding agents (SPEC 4.2).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use hq::check;
use hq::project::Project;

#[derive(Parser)]
#[command(
    name = "hq",
    version,
    about = "Orchestrate missions run by AI coding agents",
    long_about = None
)]
struct Cli {
    /// Where to look for the project. Defaults to the current directory, and
    /// `hq` walks up from there the way git does.
    #[arg(long, short = 'C', global = true, value_name = "DIR")]
    directory: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Say whether the project holds what the specification describes.
    ///
    /// Red when a restriction is not held; and it always says what it could
    /// not check, because a check that quietly skips reads as a pass.
    Check {
        /// Probe the system profile of this mission as well as the mission
        /// profile.
        #[arg(long, value_name = "ID")]
        mission: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let start = cli
        .directory
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match cli.command {
        Command::Check { mission } => {
            let project = match Project::open(&start) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let mut report = check::run(&project);
            // Said, not skipped: the container probes of SPEC 4.1 bis rule 7
            // need a slot and an engine, and neither exists yet.
            report.checks.push(check::Check {
                what: match &mission {
                    Some(id) => format!("the perimeter holds from inside the profiles of {id}"),
                    None => "the perimeter holds from inside the mission profile".to_string(),
                },
                verdict: check::Verdict::NotChecked(
                    "hq cannot lift a slot yet, so the probes of SPEC 4.1 bis rule 7 \
                     were not run; `cargo test --test firewall -- --ignored` runs them \
                     by hand"
                        .to_string(),
                ),
            });
            print!("{}", report.render());
            if report.is_red() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}
