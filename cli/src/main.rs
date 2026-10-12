use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use lattice_cli::check;
use lattice_cli::replay::{self, Outcome, Target};

/// LATTICE: 5v5 game for LLM players.
#[derive(Parser)]
#[command(name = "lattice", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a simulation.
    Sim {},
    /// Check LatticePy modules (and optionally a team library's quota).
    Check {
        /// Module files to check
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Ruleset to read limits from
        #[arg(long, default_value = "spec/rulesets/default.toml")]
        ruleset: PathBuf,
        /// Print one JSON array instead of text
        #[arg(long)]
        json: bool,
        /// Treat all paths as one team library and apply the node quota
        #[arg(long)]
        library: bool,
        /// Override the ruleset's team quota (needs --library)
        #[arg(long, requires = "library")]
        quota: Option<u32>,
    },
    /// Generate a map.
    Mapgen {},
    /// Replay a golden match and verify state hashes.
    Replay {
        /// Directory containing golden replay fixtures
        #[arg(short, long)]
        verify: Option<String>,
        /// Compare native and WASM replays
        #[arg(short, long, value_enum, default_value = "native")]
        target_parity: TargetParity,
    },
}

#[derive(Clone, ValueEnum)]
enum TargetParity {
    Native,
    Wasm,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sim {} => {
            println!("sim: will run a seeded simulation against a map");
        }
        Commands::Check {
            paths,
            ruleset,
            json,
            library,
            quota,
        } => {
            let opts = check::Options {
                ruleset: &ruleset,
                library,
                quota,
            };
            let outcome = match check::run(&paths, &opts) {
                Ok(o) => o,
                Err(err) => {
                    eprintln!("check: {err}");
                    return ExitCode::FAILURE;
                }
            };
            if json {
                match check::render_json(&outcome) {
                    Ok(text) => println!("{text}"),
                    Err(err) => {
                        eprintln!("check: {err}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                print!("{}", check::render_human(&outcome));
            }
            if let Some(q) = outcome.quota.as_ref().filter(|q| !q.holds()) {
                eprintln!(
                    "check: library is over quota ({} / {} nodes)",
                    q.nodes, q.limit
                );
            }
            if !outcome.ok() {
                return ExitCode::FAILURE;
            }
        }
        Commands::Mapgen {} => {
            println!("mapgen: will generate a symmetric map from a seed");
        }
        Commands::Replay {
            verify,
            target_parity,
        } => {
            if let Some(dir) = verify {
                let target = match target_parity {
                    TargetParity::Native => Target::Native,
                    TargetParity::Wasm => Target::Wasm,
                };
                match replay::verify(Path::new(&dir), target) {
                    Ok(Outcome::NoMatches) => {
                        println!("replay: no golden matches in '{dir}'");
                    }
                    Err(err) => {
                        eprintln!("replay: {err}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                println!("replay: will replay a match from seed and inputs");
            }
        }
    }
    ExitCode::SUCCESS
}
