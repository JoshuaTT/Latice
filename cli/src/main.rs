use clap::{Parser, Subcommand, ValueEnum};

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

fn main() {
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
            target_parity: _,
        } => {
            if let Some(dir) = verify {
                match std::fs::read_dir(&dir) {
                    Ok(entries) => {
                        let count = entries.filter_map(|e| e.ok()).count();
                        if count == 0 {
                            println!("replay: no golden replays found in '{dir}'");
                        } else {
                            println!(
                                "replay: will verify {} golden replays against '{dir}'",
                                count
                            );
                        }
                    }
                    Err(_) => {
                        println!(
                            "replay: no golden replays in '{dir}' — directory does not exist yet"
                        );
                    }
                }
            } else {
                println!("replay: will replay a match from seed and inputs");
            }
        }
    }
}
