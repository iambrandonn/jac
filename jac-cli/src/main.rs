//! JAC CLI - Command-line tool for JSON-Aware Compression
//!
//! This binary provides command-line interfaces for:
//! - pack: compress JSON/NDJSON → .jac
//! - unpack: decompress .jac → JSON/NDJSON
//! - ls: list blocks, fields, record counts
//! - cat: stream values for a field

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "jac")]
#[command(about = "JSON-Aware Compression CLI tool")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compress JSON/NDJSON to .jac format
    Pack {
        /// Input file (JSON or NDJSON)
        input: String,
        /// Output file (.jac)
        #[arg(short, long)]
        output: String,
        /// Target records per block
        #[arg(long, default_value = "100000")]
        block_records: usize,
        /// Zstd compression level
        #[arg(long, default_value = "15")]
        zstd_level: u8,
        /// Canonicalize keys (lexicographic order)
        #[arg(long)]
        canonicalize_keys: bool,
        /// Canonicalize numbers (scientific notation, trim trailing zeros)
        #[arg(long)]
        canonicalize_numbers: bool,
        /// Project only specified fields
        #[arg(long)]
        project: Option<String>,
    },
    /// Decompress .jac to JSON/NDJSON
    Unpack {
        /// Input file (.jac)
        input: String,
        /// Output file
        #[arg(short, long)]
        output: String,
        /// Output as NDJSON (one object per line)
        #[arg(long)]
        ndjson: bool,
    },
    /// List blocks, fields, and record counts
    Ls {
        /// Input file (.jac)
        input: String,
    },
    /// Stream values for a specific field
    Cat {
        /// Input file (.jac)
        input: String,
        /// Field name to extract
        #[arg(long)]
        field: String,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Pack { .. } => {
            println!("Pack command not yet implemented");
        }
        Commands::Unpack { .. } => {
            println!("Unpack command not yet implemented");
        }
        Commands::Ls { .. } => {
            println!("Ls command not yet implemented");
        }
        Commands::Cat { .. } => {
            println!("Cat command not yet implemented");
        }
    }

    Ok(())
}
