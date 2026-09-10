//! `hq` — a mission orchestrator for AI coding agents (SPEC 4.2).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use hq::harness::Harness;
use hq::mission::{Header, Integration, Lot, Security, Service, dir as mission_dir};
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

    /// Missions: what an agent is asked to do, and where it reports.
    #[command(subcommand)]
    Mission(MissionCommand),

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

#[derive(Subcommand)]
enum MissionCommand {
    /// Write a new mission folder at the HQ.
    New {
        /// Identifier: letters, digits, - and _.
        id: String,
        /// The branch the agents work on.
        #[arg(long)]
        branch: String,
        /// What that branch starts from.
        #[arg(long, default_value = "dev")]
        base: String,
        /// A unit of work, as `id:title`. Repeat it; order is the order.
        #[arg(long = "lot", value_name = "ID:TITLE", required = true)]
        lots: Vec<String>,
        /// A service the integrator may reach, as `name=host[,host…]`.
        /// Declaring one makes this an integration mission.
        #[arg(long = "service", value_name = "NAME=REACH")]
        services: Vec<String>,
        /// Why there is no integration mission. Required when no service is
        /// declared: a shape is stated, never inferred from silence.
        #[arg(long, default_value = "no external service is involved")]
        no_integration: String,
        /// Call the security agent, rather than the mechanical gates alone.
        #[arg(long)]
        security_agent: bool,
        /// The prose an agent reads under the header.
        #[arg(long, default_value = "Describe the mission here.")]
        about: String,
    },
    /// List the missions this project's HQ holds.
    List,
    /// What a mission is, and where it stands.
    Status { id: String },
    /// Start the mission: freeze its framing and launch the coder's first run.
    Start {
        id: String,
        /// The slot it happens in.
        #[arg(long)]
        slot: String,
    },
    /// End the run in progress properly: the agent finishes its turn and
    /// writes its resume block.
    Stop { id: String },
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

        Command::Mission(command) => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
            };
            mission(&project, command)
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

fn mission(project: &Project, command: MissionCommand) -> ExitCode {
    match command {
        MissionCommand::New {
            id,
            branch,
            base,
            lots,
            services,
            no_integration,
            security_agent,
            about,
        } => {
            let lots = match lots
                .iter()
                .map(|l| parse_lot(l))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let integration = if services.is_empty() {
                // The shape is declared, never inferred from silence: a
                // mission that needs services and does not say so would run
                // its integrator with an empty perimeter (SPEC 2).
                Integration::None {
                    reason: no_integration,
                }
            } else {
                match services
                    .iter()
                    .map(|s| parse_service(s))
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(services) => Integration::Services { services },
                    Err(e) => {
                        eprintln!("hq: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            };
            let header = Header {
                branch,
                base,
                lots,
                integration,
                security: if security_agent {
                    Security::Agent
                } else {
                    Security::Gates
                },
                bounds: project.config.bounds.clone(),
            };
            match mission_dir::create(&project.hq_root, &id, &header, &about) {
                Ok(paths) => {
                    println!("mission {id} at {}", paths.dir.display());
                    println!(
                        "  {} — yours and the HQ's, read-only for the agent",
                        paths.mission.display()
                    );
                    println!("  {} — the agent's", paths.journal.display());
                    println!();
                    println!("Edit MISSION.md, then `hq mission status {id}`.");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        MissionCommand::List => {
            let ids = mission_dir::list(&project.hq_root);
            if ids.is_empty() {
                println!("no mission yet: `hq mission new <id> --branch <branch> --lot L1:…`");
            }
            for id in ids {
                match mission_dir::read_header(&project.hq_root, &id) {
                    Ok(h) => println!("{id:<20} {:<24} {} lot(s)", h.branch, h.lots.len()),
                    Err(e) => println!("{id:<20} unreadable: {e}"),
                }
            }
            ExitCode::SUCCESS
        }

        MissionCommand::Start {
            id,
            slot: slot_name,
        } => {
            let slot = match slot::find(project, &slot_name) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".into());
            let engine: std::sync::Arc<dyn hq::engine::Engine> =
                std::sync::Arc::new(hq::engine::docker::Docker::real());
            match hq::run::start(project, &id, &slot, engine, &engine_bin) {
                Ok(state) => {
                    println!("mission {id} started in slot {}", state.slot);
                    if let Some(run) = &state.run {
                        println!("session {}", run.session.0);
                        println!("log     {}", run.log.display());
                    }
                    println!();
                    println!("`hq mission status {id}` says where it is.");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        MissionCommand::Stop { id } => {
            let store = match hq::state::Store::open(&project.hq_root) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let state = match store.load(&id) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let Some(handle) = &state.run else {
                eprintln!("hq: mission {id} has no run in progress");
                return ExitCode::FAILURE;
            };
            match harness_for(project, &state.slot).stop(handle) {
                Ok(()) => {
                    // SIGINT, not SIGTERM: the agent ends its turn and writes
                    // its resume block (SPEC 4.3).
                    println!("asked the run to end its turn; `hq mission status {id}` follows it");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        MissionCommand::Status { id } => {
            let header = match mission_dir::read_header(&project.hq_root, &id) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            println!("mission   {id}");
            println!("branch    {} (from {})", header.branch, header.base);
            println!("shape     {:?}", header.shape());
            for lot in &header.lots {
                println!("lot       {} — {}", lot.id, lot.title);
            }
            match &header.integration {
                Integration::None { reason } => println!("services  none — {reason}"),
                Integration::Services { services } => {
                    for s in services {
                        println!("service   {} → {}", s.name, s.reach.join(", "));
                    }
                }
            }
            println!(
                "bounds    {} volet(s), {} attempt(s) per lot",
                header.bounds.max_volets, header.bounds.attempts_per_lot
            );

            // The state is the other half of the answer, and its absence is
            // an answer too: a mission exists before it has ever run.
            match hq::state::Store::open(&project.hq_root).and_then(|s| s.load(&id)) {
                Ok(state) => {
                    println!("stage     {:?} in slot {}", state.flow.stage(), state.slot);
                    if let Some(handle) = &state.run {
                        println!("session   {}", handle.session.0);
                        // Read from the run itself, not from what was
                        // recorded: a machine that slept leaves the state
                        // saying "running" (SPEC 4.2).
                        match harness_for(project, &state.slot).state(handle) {
                            Ok(hq::harness::RunState::Running(p)) => println!(
                                "run       running — {} event(s), {} tool call(s)",
                                p.events, p.tool_calls
                            ),
                            Ok(hq::harness::RunState::Finished(outcome)) => {
                                println!("run       finished — {outcome:?}")
                            }
                            Err(e) => println!("run       unreadable: {e}"),
                        }
                    }
                }
                Err(_) => println!("stage     not started"),
            }
            ExitCode::SUCCESS
        }
    }
}

fn parse_lot(spec: &str) -> Result<Lot, String> {
    let (id, title) = spec
        .split_once(':')
        .ok_or_else(|| format!("a lot is written `id:title`, and {spec:?} is not"))?;
    if id.trim().is_empty() || title.trim().is_empty() {
        return Err(format!(
            "a lot needs an id and a title, and {spec:?} lacks one"
        ));
    }
    Ok(Lot {
        id: id.trim().to_string(),
        title: title.trim().to_string(),
    })
}

fn parse_service(spec: &str) -> Result<Service, String> {
    let (name, reach) = spec
        .split_once('=')
        .ok_or_else(|| format!("a service is written `name=host[,host…]`, and {spec:?} is not"))?;
    let reach: Vec<String> = reach
        .split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .collect();
    if name.trim().is_empty() || reach.is_empty() {
        return Err(format!(
            "a service needs a name and something to reach, and {spec:?} lacks one"
        ));
    }
    Ok(Service {
        name: name.trim().to_string(),
        reach,
    })
}

/// The harness as it must be addressed for a run that lives in a slot's
/// container: through the engine, because the pid `hq` holds is inside it.
fn harness_for(project: &Project, slot: &str) -> hq::harness::claude_code::ClaudeCode {
    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::docker::Docker::real());
    let file = hq::run::profile_path(project, slot);
    let compose_project = hq::compose::project_name(slot).unwrap_or_else(|_| format!("hq-{slot}"));
    let spawner = hq::engine::spawn::ContainerSpawner::new(
        engine,
        file,
        &compose_project,
        hq::compose::AGENT_SERVICE,
    );
    hq::harness::claude_code::ClaudeCode::new(Default::default(), Box::new(spawner))
}
