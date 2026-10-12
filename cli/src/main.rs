use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
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
    /// Check a LatticePy module.
    Check {},
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
        Commands::Check {} => {
            println!("check: will validate a LatticePy module and count quotas");
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
