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

    /// Push a verified mission's branch, and hand the pull request over.
    ///
    /// The one verb that touches the forge in write, and the one thing in
    /// `hq` that is not autonomous: it needs `--yes`, on the command line,
    /// from somebody who has read the pull request.
    Push {
        /// The mission.
        mission: String,
        /// Your explicit authorisation. There is no dialogue, and no flag to
        /// push past a red verdict.
        #[arg(long)]
        yes: bool,
    },

    /// Missions: what an agent is asked to do, and where it reports.
    ///
    /// Boxed because `mission new` carries every field of a mission header
    /// and would otherwise decide the size of every other subcommand: this
    /// value is parsed once per process, and its size is not worth spending
    /// on the seven variants beside it.
    #[command(subcommand)]
    Mission(Box<MissionCommand>),

    /// Accounts: which subscription a mission spends.
    #[command(subcommand)]
    Account(AccountCommand),

    /// Say who hq thinks you are, and where it got that from.
    Whoami,

    /// Play the verification phase of a mission: the gates, then the
    /// integration and security missions, each with theirs (SPEC 4.2).
    /// Resumable — running it again picks up where it stopped.
    Verify {
        /// The mission to verify.
        mission: String,
    },
    /// Run a command in a slot's container — how the HQ replays a proof
    /// without having the stack on this machine (SPEC 4.2).
    Exec {
        /// The slot whose container runs it.
        slot: String,
        /// Run on the working tree instead of the clean copy of `HEAD`.
        ///
        /// Dangerous, and not the default for a reason: a proof replayed in
        /// the tree an agent has been living in proves what that tree does,
        /// not what the commit does. Never while a run is in progress.
        #[arg(long)]
        tree: bool,
        /// The command, after `--`.
        #[arg(trailing_var_arg = true, required = true)]
        argv: Vec<String>,
    },
    /// Render the runs of a mission, readably.
    ///
    /// What replaces watching a screen: an agent nobody can look at is
    /// acceptable because its output is readable afterwards.
    Logs {
        /// The mission. Defaults to the one this HQ touched last.
        mission: Option<String>,
        /// Only the last run.
        #[arg(long)]
        last: bool,
    },

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
enum AccountCommand {
    /// List the accounts declared in ~/.hq/accounts.yaml.
    List,
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
        /// A path the integrator's commits may touch — configuration,
        /// system tests, fixtures, migrations. Repeatable, globs allowed.
        /// This list **is** "wiring" as SPEC 4.4 defines it: every commit
        /// outside it is out of perimeter, so an integration mission that
        /// declares none lets its integrator commit nothing.
        #[arg(long = "wiring", value_name = "PATH")]
        wiring: Vec<String>,
        /// Why there is no integration mission. Required when no service is
        /// declared: a shape is stated, never inferred from silence.
        #[arg(long, default_value = "no external service is involved")]
        no_integration: String,
        /// Call the security agent, rather than the mechanical gates alone.
        #[arg(long)]
        security_agent: bool,
        /// How `hq` starts the application for the integrator and the
        /// security agent, as a path inside the tree. Refines `hq.yaml` and
        /// the stack's `run.sh`; `none` when there is nothing to start.
        #[arg(long, value_name = "SCRIPT")]
        run: Option<String>,
        /// Which account this mission spends. Defaults to the project's, then
        /// to the one named in ~/.hq/accounts.yaml.
        #[arg(long, value_name = "NAME")]
        account: Option<String>,
        /// Who arbitrates when this mission comes back with a question.
        /// Defaults to whoever runs `hq` — `hq whoami` says who that is.
        #[arg(long = "for", value_name = "NAME")]
        arbiter: Option<String>,
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
    /// Follow the run in progress until it ends.
    ///
    /// Two states, and a third a human causes: it runs, it is paused, it is
    /// finished. Nothing is asked of the agent — this reads the container and
    /// the run's own stream, like everything else `hq` knows about a run.
    Watch {
        /// The mission.
        id: String,
        /// Seconds between readings.
        #[arg(long, default_value_t = 10)]
        every: u64,
    },

    /// Freeze the agent's container where it is.
    ///
    /// Nothing is lost: the processes are suspended by the engine. A model
    /// call in flight may time out during a long freeze, and the harness
    /// replays it.
    Pause {
        /// The mission.
        id: String,
    },

    /// Unfreeze a paused container, exactly where it was.
    Resume {
        /// The mission.
        id: String,
    },

    /// The emergency brake: kill the agent's container, waiting for nothing.
    ///
    /// After it, the lot in progress is a failed attempt and the relaunch
    /// starts from the last resume block the agent wrote. Use `stop` if you
    /// want the agent to finish its turn.
    Kill {
        /// The mission.
        id: String,
    },

    /// Leave an instruction for the **next** run.
    ///
    /// There is no channel during a run. This lands in `FOLLOWUP_HQ.md`,
    /// which every role reads before anything else; if it is urgent,
    /// `hq mission stop` ends the current run first.
    Say {
        /// The mission.
        id: String,
        /// What to tell the next run.
        what: Vec<String>,
    },

    /// Bring a mission's commits from its slot into the repository.
    ///
    /// The only way commits leave a slot, and it goes one way. `hq push`
    /// does it for you; this is for looking at them yourself first.
    Fetch {
        /// The mission.
        id: String,
    },

    /// Lift a security finding, or what a `FINDINGS` verdict still carries.
    ///
    /// Never touches `VERDICT.json`: the verdict says what the agent found,
    /// `hq`'s state says what you decided, and `hq push` reads it there.
    Accept {
        /// The mission.
        id: String,
        /// One finding, as the report names it. Without it, what the report
        /// still carries is lifted and the mission concludes.
        #[arg(long, value_name = "NAME")]
        finding: Option<String>,
        /// Why the risk is acceptable. Required: a risk accepted without a
        /// reason is not accepted, it is forgotten.
        #[arg(long = "because", value_name = "WHY")]
        why: String,
    },

    /// Send a `FINDINGS` verdict back to the coder, as a volet.
    Iterate {
        /// The mission.
        id: String,
    },

    /// Start the mutation campaign, or say where the one in flight is
    /// (SPEC 4.4, gate 7). Long: it is launched detached and watched, and
    /// the call that finds it finished writes `MUTANTS.json`.
    Mutants {
        id: String,
        /// Which slot runs it. Defaults to the one the mission started in.
        #[arg(long)]
        slot: Option<String>,
        /// Rule that a survivor is equivalent rather than starting or
        /// following a campaign. This outcome is not the coder's to give —
        /// nothing can check it — so it is written here, by hand, on
        /// purpose (SPEC 4.4).
        #[arg(long = "equivalent", value_name = "SURVIVOR")]
        equivalent: Option<String>,
        /// Why it is equivalent, in one sentence. Required with
        /// `--equivalent`: a ruling nobody can check must at least say what
        /// it rests on.
        #[arg(long = "because", value_name = "SENTENCE", requires = "equivalent")]
        because: Option<String>,
    },
    /// Play the verification gates 1 to 4 on what the slot holds (SPEC 4.4).
    /// Deterministic, and nothing is asked of the agent.
    Gates {
        id: String,
        /// Whose gates. Each role has the gates that match what it produces.
        #[arg(long, default_value = "coder")]
        role: RoleArg,
        /// Play the final verification's gates too — the deliverable and the
        /// battery. The battery runs in the slot's container, on the clean
        /// copy of `HEAD`, so a profile has to be up.
        #[arg(long)]
        verification: bool,
        /// Which slot to judge. Defaults to the one the mission started in;
        /// naming one lets a human play the gates before a run exists.
        #[arg(long)]
        slot: Option<String>,
    },
}

/// The three roles, as a command-line word.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum RoleArg {
    Coder,
    Integrator,
    Security,
}

impl From<RoleArg> for hq::harness::Role {
    fn from(role: RoleArg) -> Self {
        match role {
            RoleArg::Coder => hq::harness::Role::Coder,
            RoleArg::Integrator => hq::harness::Role::Integrator,
            RoleArg::Security => hq::harness::Role::Security,
        }
    }
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

        Command::Whoami => {
            let project = open(&start);
            let hq_home = project
                .as_ref()
                .map(|p| p.hq_home())
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".hq")))
                .unwrap_or_else(|| PathBuf::from("."));
            let me = hq::human::me(&hq_home, project.as_ref().map(|p| p.root.as_path()));
            match &me.name {
                Some(name) => {
                    println!("{name}");
                    if let Some(email) = &me.email {
                        println!("{email}");
                    }
                    println!("from {}", me.source.describe());
                }
                None => {
                    println!("hq does not know who you are.");
                    println!();
                    println!("Write {}:", hq_home.join(hq::human::ME_FILE).display());
                    println!("name: Arnaud");
                    println!("email: you@example.com");
                }
            }
            ExitCode::SUCCESS
        }

        Command::Account(AccountCommand::List) => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
            };
            let hq_home = project.hq_home();
            let accounts = match hq::account::Accounts::load(&hq_home) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if accounts.accounts.is_empty() {
                println!(
                    "no account declared. Write {}:\n",
                    hq_home.join(hq::account::INDEX_FILE).display()
                );
                println!("default: perso");
                println!("accounts:");
                println!("  perso:");
                println!("    harness: claude-code");
                println!("    token_file: accounts/perso");
                println!("    note: my own subscription");
                println!();
                println!(
                    "then put the token in {}/accounts/perso, mode 600.",
                    hq_home.display()
                );
                return ExitCode::SUCCESS;
            }
            for (name, account) in &accounts.accounts {
                let path = account.token_path(&hq_home);
                let state = match account.token(&hq_home, name) {
                    Ok(_) => "ready",
                    Err(_) => "no token",
                };
                let chosen = if accounts.default.as_deref() == Some(name.as_str()) {
                    " (default)"
                } else {
                    ""
                };
                println!(
                    "{name:<12} {:<14} {state:<9} {}{chosen}",
                    account.harness,
                    path.display()
                );
                if let Some(note) = &account.note {
                    println!("{:<12} {note}", "");
                }
            }
            ExitCode::SUCCESS
        }

        Command::Mission(command) => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
            };
            mission(&project, *command)
        }

        Command::Logs { mission, last } => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
            };
            let mission = match mission {
                Some(id) => id,
                None => match hq::logs::most_recent(&project) {
                    Ok(Some(id)) => {
                        // Said out loud: choosing for the reader without
                        // telling them which is a guess dressed as a
                        // convenience.
                        println!("mission   {id} — the one this HQ touched last");
                        id
                    }
                    Ok(None) => {
                        eprintln!("hq: no mission has run yet; name one");
                        return ExitCode::FAILURE;
                    }
                    Err(e) => {
                        eprintln!("hq: {e}");
                        return ExitCode::FAILURE;
                    }
                },
            };
            let harness = harness_for(&project, "", None);
            match hq::logs::of(&project, &mission, &harness) {
                Ok(runs) => {
                    let runs = if last {
                        runs.into_iter().next_back().into_iter().collect()
                    } else {
                        runs
                    };
                    for run in runs {
                        println!();
                        println!("── run {} ({})", run.session, run.log.display());
                        for line in run.lines {
                            let mark = match line.kind {
                                hq::harness::LineKind::Start => "start",
                                hq::harness::LineKind::Said => "said ",
                                hq::harness::LineKind::Did => "did  ",
                                hq::harness::LineKind::Ended => "ended",
                                hq::harness::LineKind::Unread => "?    ",
                                hq::harness::LineKind::Noted => "…    ",
                            };
                            for (n, text) in line.text.lines().enumerate() {
                                match n {
                                    0 => println!("{mark} {text}"),
                                    _ => println!("      {text}"),
                                }
                            }
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        Command::Push { mission, yes } => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
            };
            match hq::push::push(&project, &mission, yes) {
                Ok(pushed) => {
                    println!("pushed    {} → {}", pushed.branch, pushed.remote);
                    println!("head      {}", &pushed.head[..12.min(pushed.head.len())]);
                    match pushed.pull_request {
                        Some(url) => {
                            println!("pull request, to open yourself:");
                            println!("  {url}");
                        }
                        None => println!(
                            "the pull request is yours to open: hq does not talk to the \
                             forge yet"
                        ),
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        Command::Verify { mission } => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
            };
            let engine: std::sync::Arc<dyn hq::engine::Engine> =
                std::sync::Arc::new(hq::engine::docker::Docker::real());
            let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".into());
            match hq::verify::verify(&project, &mission, engine, &engine_bin) {
                Ok(steps) => {
                    let mut owed = false;
                    for step in &steps {
                        match step {
                            hq::verify::Step::Gates { role, report } => {
                                println!("gates     {role:?}, on {}", &report.head[..12]);
                                print_gates(report);
                            }
                            hq::verify::Step::Moved { to } => println!("stage     {to:?}"),
                            hq::verify::Step::NeedsRun { role, why } => {
                                owed = true;
                                println!("owed      a {role:?} run — {why}");
                            }
                            hq::verify::Step::Launched { role, application } => {
                                owed = true;
                                println!("launched  a {role:?} run — {application}");
                                println!(
                                    "          it runs detached; `hq verify {mission}` \
                                     again reads it back"
                                );
                            }
                            hq::verify::Step::Unreachable { role, why } => {
                                owed = true;
                                println!("unknown   the {role:?} run could not be asked — {why}");
                            }
                            hq::verify::Step::Verified => println!(
                                "VERIFIED  every declared stage is green; read it, then \
                                 `hq push {mission}`"
                            ),
                            hq::verify::Step::AwaitingHuman(handover) => {
                                owed = true;
                                println!("stopped   {handover:?}");
                            }
                            hq::verify::Step::Findings { report, lifted } => {
                                owed = true;
                                println!("findings  {report}");
                                for one in lifted {
                                    println!("lifted    {one}");
                                }
                                println!(
                                    "          `hq mission iterate {mission}` sends it back \
                                     to the coder;"
                                );
                                println!(
                                    "          `hq mission accept {mission} --because <why>` \
                                     lifts what remains."
                                );
                            }
                        }
                    }
                    if owed {
                        ExitCode::FAILURE
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        Command::Exec { slot, tree, argv } => {
            let project = match open(&start) {
                Some(p) => p,
                None => return ExitCode::FAILURE,
            };
            let slot = match hq::slot::find(&project, &slot) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let engine: std::sync::Arc<dyn hq::engine::Engine> =
                std::sync::Arc::new(hq::engine::docker::Docker::real());
            let on = if tree {
                hq::exec::On::Tree
            } else {
                hq::exec::On::Proof
            };
            match hq::exec::run(&project, &slot, engine, &argv, on) {
                Ok(out) => {
                    // The command's own output, on the streams it wrote to,
                    // and its own status: `hq exec` is a way through, not a
                    // reporter.
                    print!("{}", out.stdout);
                    eprint!("{}", out.stderr);
                    if out.ok() {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(u8::try_from(out.status).unwrap_or(1))
                    }
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
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

/// Follow a run until it ends, printing only what changed.
///
/// Only what changed, because a line every ten seconds saying the same thing
/// is a log nobody reads to the end — and the two facts worth seeing, the
/// transitions, would be lost in it.
fn watch(project: &Project, id: &str, every: u64) -> ExitCode {
    let store = match hq::state::Store::open(&project.hq_root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hq: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut said = String::new();
    loop {
        let state = match store.load(id) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("hq: {e}");
                return ExitCode::FAILURE;
            }
        };
        let Some(handle) = &state.run else {
            println!("run       none in progress");
            return ExitCode::SUCCESS;
        };
        let now = match harness_for(project, &state.slot, Some(&handle.session.0)).state(handle) {
            Ok(hq::harness::RunState::Running(p)) => format!(
                "running — {} event(s), {} tool call(s)",
                p.events, p.tool_calls
            ),
            Ok(hq::harness::RunState::Paused(p)) => format!(
                "paused — {} event(s), {} tool call(s); `hq mission resume {id}` unfreezes it",
                p.events, p.tool_calls
            ),
            Ok(hq::harness::RunState::Finished(outcome)) => {
                println!("run       finished — {outcome:?}");
                println!("          `hq logs {id} --last` renders it");
                return ExitCode::SUCCESS;
            }
            // Not a reason to stop watching: a machine that slept comes
            // back, and a `watch` that gave up on the first silence would
            // give up exactly when the human left it running.
            Err(e) => format!("unknown — {e}"),
        };
        if now != said {
            println!("run       {now}");
            said = now;
        }
        std::thread::sleep(std::time::Duration::from_secs(every.max(1)));
    }
}

/// `pause` and `resume` are one gesture in two directions, and printing them
/// from one place keeps the two messages saying the same thing.
fn freeze(project: &Project, id: &str, which: hq::gesture::Freeze) -> ExitCode {
    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::docker::Docker::real());
    match hq::gesture::freeze(project, id, engine, which) {
        Ok(_) => {
            match which {
                hq::gesture::Freeze::On => println!(
                    "paused    the agent and its sidecar are frozen; \
                     `hq mission resume {id}` unfreezes them exactly there"
                ),
                hq::gesture::Freeze::Off => println!("resumed   exactly where it was"),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hq: {e}");
            ExitCode::FAILURE
        }
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
            wiring,
            no_integration,
            security_agent,
            run,
            account,
            arbiter,
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
                    Ok(services) => Integration::Services { services, wiring },
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
                account,
                run,
                // Said rather than assumed: whoever frames a mission is who
                // it comes back to, until somebody says otherwise.
                arbiter: arbiter
                    .or_else(|| hq::human::me(&project.hq_home(), Some(&project.root)).name),
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

        MissionCommand::Watch { id, every } => watch(project, &id, every),

        MissionCommand::Pause { id } => freeze(project, &id, hq::gesture::Freeze::On),
        MissionCommand::Resume { id } => freeze(project, &id, hq::gesture::Freeze::Off),

        MissionCommand::Kill { id } => {
            let engine: std::sync::Arc<dyn hq::engine::Engine> =
                std::sync::Arc::new(hq::engine::docker::Docker::real());
            match hq::gesture::kill(project, &id, engine) {
                Ok(_) => {
                    println!("killed    nothing was waited for");
                    println!(
                        "          the lot in progress is a failed attempt; the relaunch \
                         starts from the last resume block"
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        MissionCommand::Say { id, what } => match hq::gesture::say(project, &id, &what.join(" ")) {
            Ok(()) => {
                println!("said      written to FOLLOWUP_HQ.md, for the next run");
                println!(
                    "          there is no channel during a run; `hq mission stop {id}` \
                     ends this one first"
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("hq: {e}");
                ExitCode::FAILURE
            }
        },

        MissionCommand::Fetch { id } => match hq::push::fetch(project, &id) {
            Ok(fetched) => {
                let head = &fetched.head[..12.min(fetched.head.len())];
                match fetched.was {
                    Some(was) if was == fetched.head => {
                        println!("fetched   {} already at {head}", fetched.branch)
                    }
                    Some(was) => println!(
                        "fetched   {} {} → {head}",
                        fetched.branch,
                        &was[..12.min(was.len())]
                    ),
                    None => println!("fetched   {} at {head}, new here", fetched.branch),
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("hq: {e}");
                ExitCode::FAILURE
            }
        },

        MissionCommand::Accept { id, finding, why } => {
            let lift = match finding {
                Some(name) => hq::findings::Lift::Finding(name),
                None => hq::findings::Lift::Verdict,
            };
            match hq::findings::accept(project, &id, lift, &why) {
                Ok(state) => {
                    println!("accepted  written to FOLLOWUP_HQ.md and to hq's state");
                    println!("stage     {:?}", state.flow.stage());
                    println!(
                        "          VERDICT.json still says FINDINGS — that is the agent's \
                         answer, and it is not yours to edit"
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        MissionCommand::Iterate { id } => match hq::findings::iterate(project, &id) {
            Ok(state) => {
                println!("stage     {:?}", state.flow.stage());
                println!("          `hq verify {id}` plays it from there");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("hq: {e}");
                ExitCode::FAILURE
            }
        },

        MissionCommand::Mutants {
            id,
            slot,
            equivalent,
            because,
        } => {
            if let Some(survivor) = equivalent {
                let Some(why) = because else {
                    eprintln!(
                        "hq: --equivalent needs --because: a ruling nobody can check must \
                         say what it rests on"
                    );
                    return ExitCode::FAILURE;
                };
                let paths = hq::mission::dir::Paths::of(&project.hq_root, &id);
                return match hq::mutants::rule_equivalent(&paths.dir, &survivor, &why) {
                    Ok(()) => {
                        println!("{survivor} ruled equivalent — {why}");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("hq: {e}");
                        ExitCode::FAILURE
                    }
                };
            }
            let header = match hq::mission::dir::read_header(&project.hq_root, &id) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let Some(in_slot) = slot.or_else(|| {
                hq::state::Store::open(&project.hq_root)
                    .and_then(|s| s.load(&id))
                    .ok()
                    .map(|s| s.slot)
            }) else {
                eprintln!("hq: mission {id} has not started — name a slot with --slot");
                return ExitCode::FAILURE;
            };
            let slot = match hq::slot::find(project, &in_slot) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let base = match hq::gate::touched_paths(&slot.tree, &header.base) {
                Ok(paths) => paths,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let stack = project
                .config
                .stacks
                .first()
                .cloned()
                .unwrap_or_else(|| "rust".to_string());
            let engine: std::sync::Arc<dyn hq::engine::Engine> =
                std::sync::Arc::new(hq::engine::docker::Docker::real());
            let paths = hq::mission::dir::Paths::of(&project.hq_root, &id);
            match hq::mutants::campaign(
                project,
                &slot,
                engine,
                &paths.dir,
                &stack,
                &base,
                header.bounds.mutation_minutes,
            ) {
                Ok(progress) => {
                    use hq::mutants::Progress;
                    match progress {
                        Progress::Fresh { survivors } => println!(
                            "a campaign on this exact content is already on file — \
                             {survivors} survivor(s); it replays only when the touched \
                             files change"
                        ),
                        Progress::Started { fingerprint } => println!(
                            "campaign {} started; `hq mission mutants {id}` follows it",
                            &fingerprint[..7.min(fingerprint.len())]
                        ),
                        Progress::Running { started_at, lines } => {
                            println!("running since {started_at} — {lines} line(s) so far")
                        }
                        Progress::Finished { survivors } => println!(
                            "finished — {survivors} survivor(s) in {}; each needs one of \
                             the three outcomes before gate 7 is green",
                            hq::mutants::FILE
                        ),
                        Progress::Overrun { minutes } => {
                            eprintln!(
                                "hq: the campaign passed its {minutes}-minute deadline and was stopped"
                            );
                            return ExitCode::FAILURE;
                        }
                        Progress::Lost(why) => {
                            eprintln!("hq: the campaign cannot be reached: {why}");
                            return ExitCode::FAILURE;
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hq: {e}");
                    ExitCode::FAILURE
                }
            }
        }

        MissionCommand::Gates {
            id,
            role,
            verification,
            slot,
        } => {
            let role: hq::harness::Role = role.into();
            let started = hq::state::Store::open(&project.hq_root)
                .and_then(|s| s.load(&id))
                .ok();
            // The frozen header once a mission has started, and the file only
            // before that: `hq` never re-reads the header during a mission
            // (SPEC 4.1). A perimeter a run is judged against must be the one
            // it was launched with, or an agent could widen it between two
            // gates.
            let header = match &started {
                Some(state) => state.flow.header().clone(),
                None => match hq::mission::dir::read_header(&project.hq_root, &id) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("hq: {e}");
                        return ExitCode::FAILURE;
                    }
                },
            };
            let in_slot = match slot.or_else(|| started.as_ref().map(|s| s.slot.clone())) {
                Some(s) => s,
                None => {
                    eprintln!(
                        "hq: mission {id} has not started, so there is no slot to judge — \
                         name one with --slot"
                    );
                    return ExitCode::FAILURE;
                }
            };
            let slot = match hq::slot::find(project, &in_slot) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let paths = hq::mission::dir::Paths::of(&project.hq_root, &id);
            let subject = hq::gate::Subject {
                role,
                tree: &slot.tree,
                journal: &paths.journal,
                pr: &paths.pr,
                verdict: &paths.verdict,
                mission_dir: &paths.dir,
                header: &header,
                protected_branches: &project.config.protected_branches,
                protected_paths: &project.config.protected_paths,
            };
            let played = if verification {
                let engine: std::sync::Arc<dyn hq::engine::Engine> =
                    std::sync::Arc::new(hq::engine::docker::Docker::real());
                let stack = project
                    .config
                    .stacks
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "rust".to_string());
                hq::gate::at_verification(
                    &subject,
                    &hq::gate::Verification {
                        project,
                        slot: &slot,
                        engine,
                        stack: &stack,
                    },
                )
            } else {
                hq::gate::after_run(&subject)
            };
            let report = match played {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("hq: {e}");
                    return ExitCode::FAILURE;
                }
            };
            println!("mission   {id}");
            println!("role      {:?}", report.role);
            println!("head      {}", report.head);
            print_gates(&report);
            match report.failure() {
                Some(_) => ExitCode::FAILURE,
                None => ExitCode::SUCCESS,
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
            match harness_for(project, &state.slot, Some(&handle.session.0)).stop(handle) {
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
                Integration::Services { services, wiring } => {
                    for s in services {
                        println!("service   {} → {}", s.name, s.reach.join(", "));
                    }
                    if wiring.is_empty() {
                        println!("wiring    none declared — the integrator may commit nothing");
                    } else {
                        println!("wiring    {}", wiring.join(", "));
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
                        match harness_for(project, &state.slot, Some(&handle.session.0))
                            .state(handle)
                        {
                            Ok(hq::harness::RunState::Running(p)) => println!(
                                "run       running — {} event(s), {} tool call(s)",
                                p.events, p.tool_calls
                            ),
                            Ok(hq::harness::RunState::Paused(p)) => println!(
                                "run       paused — {} event(s), {} tool call(s); \
                                 `hq mission resume {id}` unfreezes it",
                                p.events, p.tool_calls
                            ),
                            Ok(hq::harness::RunState::Finished(outcome)) => {
                                println!("run       finished — {outcome:?}")
                            }
                            // Not "finished", not "running": `hq` says it
                            // does not know, and why. Calling an unreachable
                            // run a dead one is how a container taken down
                            // becomes a harness failure in the record.
                            Err(e) => println!("run       unknown — {e}"),
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

/// One line per gate, the same wherever gates are reported — `hq mission
/// gates` and `hq verify` must not describe the same report differently.
fn print_gates(report: &hq::gate::Report) {
    for outcome in &report.outcomes {
        let (mark, detail) = match &outcome.decision {
            hq::gate::Decision::Passed => ("pass", String::new()),
            hq::gate::Decision::Failed(why) => ("FAIL", format!(" — {why}")),
            // Said, never folded into a pass: a skipped gate reported as
            // green is how a report stops being worth reading (SPEC 4.4,
            // the per-role table).
            hq::gate::Decision::NotApplicable(why) => ("n/a ", format!(" — {why}")),
            // Neither green nor red: nobody managed to play it.
            hq::gate::Decision::Unplayed(why) => ("????", format!(" — {why}")),
        };
        println!(
            "gate {}    {mark}  {}{detail}",
            outcome.gate.number(),
            outcome.gate.title()
        );
        // What a green gate still owes the reader.
        if let Some(note) = &outcome.note {
            println!("          note  {note}");
        }
    }
}

/// The harness as it must be addressed for a run that lives in a slot's
/// container: through the engine, because the pid `hq` holds is inside it.
fn harness_for(
    project: &Project,
    slot: &str,
    session: Option<&str>,
) -> hq::harness::claude_code::ClaudeCode {
    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::docker::Docker::real());
    let file = hq::run::profile_path(project, slot);
    let compose_project = hq::compose::project_name(slot).unwrap_or_else(|_| format!("hq-{slot}"));
    let mut spawner = hq::engine::spawn::ContainerSpawner::new(
        engine,
        file,
        &compose_project,
        hq::compose::AGENT_SERVICE,
    );
    if let Some(session) = session {
        spawner = spawner.identified_by(session);
    }
    hq::harness::claude_code::ClaudeCode::new(Default::default(), Box::new(spawner))
}
