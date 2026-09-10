//! `hq` — a mission orchestrator for AI coding agents (SPEC 4.2).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use hq::project::Project;
use hq::{check, image, init, probe, slot};

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
        /// Probe the mission profile from inside this slot's containers.
        #[arg(long, value_name = "NAME")]
        slot: Option<String>,
        /// Probe the system profile of this mission as well.
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
    /// Build this project's images: the stack's, and the firewall sidecar's.
    Rebuild {
        /// Which stack's image. Defaults to the only one declared.
        #[arg(long, value_name = "NAME")]
        stack: Option<String>,
    },
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
                SlotCommand::Rebuild { stack } => {
                    let stack = match stack.or_else(|| project.config.stacks.first().cloned()) {
                        Some(s) => s,
                        None => {
                            eprintln!("hq: no stack declared in hq.yaml; pass --stack");
                            return ExitCode::FAILURE;
                        }
                    };
                    let engine = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".into());
                    println!("building the images for {stack}, this takes a while…");
                    match image::build(&project, &stack, &engine) {
                        Ok(images) => {
                            println!("agent     {}", images.agent);
                            println!("firewall  {}", images.firewall);
                            println!("prober    {}", images.prober);
                            ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("hq: {e}");
                            ExitCode::FAILURE
                        }
                    }
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

        Command::Check {
            slot: which,
            mission,
        } => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
            };
            let mut report = check::run(&project);
            report.checks.extend(probes(&project, which.as_deref()));
            if let Some(id) = &mission {
                // Said, not skipped: a system profile is a mission's, and
                // missions do not exist yet.
                report.checks.push(check::Check {
                    what: format!("the perimeter holds from inside the system profile of {id}"),
                    verdict: check::Verdict::NotChecked(
                        "hq has no missions yet, so there is no system profile to lift".to_string(),
                    ),
                });
            }
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

/// The container probes of SPEC 4.1 bis rule 7, when a slot was named. Never
/// silently skipped: without a slot, that is said as plainly as a result.
fn probes(project: &Project, which: Option<&str>) -> Vec<check::Check> {
    let Some(name) = which else {
        return vec![check::Check {
            what: "the perimeter holds from inside the mission profile".to_string(),
            verdict: check::Verdict::NotChecked(
                "no slot named: `hq check --slot <name>` lifts one and probes it".to_string(),
            ),
        }];
    };
    let slot = match slot::find(project, name) {
        Ok(s) => s,
        Err(e) => {
            return vec![check::Check {
                what: "the perimeter holds from inside the mission profile".to_string(),
                verdict: check::Verdict::Red(e.to_string()),
            }];
        }
    };
    let Some(stack) = project.config.stacks.first().cloned() else {
        return vec![check::Check {
            what: "the perimeter holds from inside the mission profile".to_string(),
            verdict: check::Verdict::NotChecked(
                "no stack declared in hq.yaml, so no image to lift".to_string(),
            ),
        }];
    };
    let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".into());
    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::docker::Docker::real());

    match probe::mission_profile(project, &slot, &stack, engine, &engine_bin) {
        Ok(checks) => checks,
        Err(e) => vec![check::Check {
            what: "the perimeter holds from inside the mission profile".to_string(),
            verdict: match e {
                // Missing images are something to do, not something broken.
                probe::ProbeError::NoImages(..) => check::Verdict::NotChecked(e.to_string()),
                _ => check::Verdict::Red(e.to_string()),
            },
        }],
    }
}
