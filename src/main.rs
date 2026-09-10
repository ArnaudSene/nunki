//! `hq` — a mission orchestrator for AI coding agents (SPEC 4.2).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use hq::project::Project;
use hq::{check, init, slot};

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
    /// Make an existing repository orchestrable.
    ///
    /// Creates what is absent, never overwrites a file a human edits — it
    /// deposits its version beside it — and keeps no manifest. Replayable.
    Init {
        /// Stack fragments to write under `.hq/stacks/`.
        #[arg(long = "stack", value_name = "NAME")]
        stacks: Vec<String>,
    },

    /// Slots: a local clone without hard links, where missions happen.
    #[command(subcommand)]
    Slot(SlotCommand),

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

#[derive(Subcommand)]
enum SlotCommand {
    /// Clone the project into a new slot.
    Add { name: String },
    /// List this project's slots.
    List,
    /// Remove a slot, refusing while it holds work the repository lacks.
    Rm {
        name: String,
        /// Remove it anyway. The commits in it are lost.
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let start = cli
        .directory
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match cli.command {
        Command::Init { stacks } => {
            // `init` is the one verb that runs before a project exists, so it
            // takes the directory as given rather than walking up to find one.
            let root = match std::fs::canonicalize(&start) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("hq: {}: {e}", start.display());
                    return ExitCode::FAILURE;
                }
            };
            let hq_root = match hq_root_for(&root) {
                Some(h) => h,
                None => {
                    eprintln!("hq: no home directory: the HQ lives under ~/.hq");
                    return ExitCode::FAILURE;
                }
            };
            match init::init(&root, &hq_root, &stacks) {
                Ok(actions) => {
                    for action in &actions {
                        println!("{}", action.render());
                    }
                    if actions.is_empty() {
                        println!("nothing to do: this project is already orchestrable.");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        Command::Slot(command) => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
            };
            match command {
                SlotCommand::Add { name } => match slot::add(&project, &name) {
                    Ok(s) => {
                        println!("slot {} at {}", s.name, s.tree.display());
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("hq: {e}");
                        ExitCode::FAILURE
                    }
                },
                SlotCommand::List => {
                    let slots = slot::list(&project);
                    if slots.is_empty() {
                        println!("no slot yet: `hq slot add <name>`");
                    }
                    for s in slots {
                        println!("{:<20} {}", s.name, s.tree.display());
                    }
                    ExitCode::SUCCESS
                }
                SlotCommand::Rm { name, force } => match slot::rm(&project, &name, force) {
                    Ok(s) => {
                        println!("removed {}", s.tree.display());
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("hq: {e}");
                        ExitCode::FAILURE
                    }
                },
            }
        }

        Command::Check { mission } => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
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

/// Open the project containing `start`, reporting the failure the way every
/// verb reports it.
fn open(start: &std::path::Path) -> Option<Project> {
    match Project::open(start) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("hq: {e}");
            None
        }
    }
}

fn hq_root_for(root: &std::path::Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let name = root.file_name()?.to_string_lossy().into_owned();
    Some(PathBuf::from(home).join(".hq").join(name))
}
