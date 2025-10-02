//! JAC CLI - Command-line tool for JSON-Aware Compression
//!
//! This binary provides command-line interfaces for:
//! - pack: compress JSON/NDJSON → .jac
//! - unpack: decompress .jac → JSON/NDJSON/CSV
//! - ls: list blocks, fields, record counts (TODO)
//! - cat: stream values for a field (TODO)

use clap::{Parser, Subcommand};
use jac_io::{
    execute_compress, execute_decompress, Codec, CompressOptions, CompressRequest, CompressSummary,
    DecompressFormat, DecompressOptions, DecompressRequest, DecompressSummary, InputSource,
    JacInput, Limits, OutputSink,
};
use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};

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
        /// Input file (JSON array or NDJSON)
        input: PathBuf,
        /// Output file (.jac)
        #[arg(short, long)]
        output: PathBuf,
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
        /// Maximum dictionary entries per field
        #[arg(long, default_value = "4096")]
        max_dict_entries: usize,
        /// Emit index footer and pointer (enabled by default)
        #[arg(long)]
        no_index: bool,
        /// Explicitly treat input as NDJSON (overrides extension detection)
        #[arg(long)]
        ndjson: bool,
        /// Explicitly treat input as JSON array (overrides extension detection)
        #[arg(long)]
        json_array: bool,
    },
    /// Decompress .jac to JSON/NDJSON
    Unpack {
        /// Input file (.jac)
        input: PathBuf,
        /// Output file
        #[arg(short, long)]
        output: PathBuf,
        /// Output as NDJSON (default). Use --json-array for array output.
        #[arg(long)]
        ndjson: bool,
        /// Output as JSON array
        #[arg(long = "json-array")]
        json_array: bool,
    },
    /// List blocks, fields, and record counts (not yet implemented)
    Ls {
        /// Input file (.jac)
        input: PathBuf,
    },
    /// Stream values for a specific field (not yet implemented)
    Cat {
        /// Input file (.jac)
        input: PathBuf,
        /// Field name to extract
        #[arg(long)]
        field: String,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Pack {
            input,
            output,
            block_records,
            zstd_level,
            canonicalize_keys,
            canonicalize_numbers,
            max_dict_entries,
            no_index,
            ndjson,
            json_array,
        } => {
            handle_pack(
                input,
                output,
                block_records,
                zstd_level,
                canonicalize_keys,
                canonicalize_numbers,
                max_dict_entries,
                !no_index,
                ndjson,
                json_array,
            )?;
        }
        Commands::Unpack {
            input,
            output,
            ndjson,
            json_array,
        } => {
            handle_unpack(input, output, ndjson, json_array)?;
        }
        Commands::Ls { .. } => {
            eprintln!("`jac ls` is not implemented yet.");
        }
        Commands::Cat { .. } => {
            eprintln!("`jac cat` is not implemented yet.");
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_pack(
    input: PathBuf,
    output: PathBuf,
    block_records: usize,
    zstd_level: u8,
    canonicalize_keys: bool,
    canonicalize_numbers: bool,
    max_dict_entries: usize,
    emit_index: bool,
    force_ndjson: bool,
    force_json_array: bool,
) -> Result<(), Box<dyn Error>> {
    let input_source = resolve_input_source(&input, force_ndjson, force_json_array)?;
    let options = CompressOptions {
        block_target_records: block_records,
        default_codec: Codec::Zstd(zstd_level),
        canonicalize_keys,
        canonicalize_numbers,
        nested_opaque: true,
        max_dict_entries,
        limits: Limits::default(),
    };

    let request = CompressRequest {
        input: input_source,
        output: OutputSink::Path(output.clone()),
        options,
        emit_index,
    };

    let summary = execute_compress(request)?;
    report_compress_summary(&summary, &output)?;
    Ok(())
}

fn handle_unpack(
    input: PathBuf,
    output: PathBuf,
    _ndjson: bool,
    json_array: bool,
) -> Result<(), Box<dyn Error>> {
    let format = if json_array {
        DecompressFormat::JsonArray
    } else {
        // Default to NDJSON if both flags false or ndjson explicitly true
        DecompressFormat::Ndjson
    };

    let request = DecompressRequest {
        input: JacInput::Path(input.clone()),
        output: OutputSink::Path(output.clone()),
        format,
        options: DecompressOptions::default(),
    };

    let summary = execute_decompress(request)?;
    report_decompress_summary(&summary, &output)?;
    Ok(())
}

fn resolve_input_source(
    path: &Path,
    force_ndjson: bool,
    force_json_array: bool,
) -> Result<InputSource, Box<dyn Error>> {
    if force_ndjson && force_json_array {
        return Err("--ndjson and --json-array are mutually exclusive".into());
    }

    if force_ndjson {
        return Ok(InputSource::NdjsonPath(path.to_path_buf()));
    }
    if force_json_array {
        return Ok(InputSource::JsonArrayPath(path.to_path_buf()));
    }

    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase())
    {
        Some(ext) if ext == "json" => Ok(InputSource::JsonArrayPath(path.to_path_buf())),
        Some(ext) if ext == "ndjson" || ext == "jsonl" => {
            Ok(InputSource::NdjsonPath(path.to_path_buf()))
        }
        _ => Ok(InputSource::NdjsonPath(path.to_path_buf())),
    }
}

fn report_compress_summary(summary: &CompressSummary, output: &Path) -> Result<(), Box<dyn Error>> {
    let mut stderr = std::io::stderr().lock();
    writeln!(
        &mut stderr,
        "Compressed to {} (records: {}, blocks: {}, bytes written: {})",
        output.display(),
        summary.metrics.records_written,
        summary.metrics.blocks_written,
        summary.metrics.bytes_written
    )?;
    Ok(())
}

fn report_decompress_summary(
    summary: &DecompressSummary,
    output: &Path,
) -> Result<(), Box<dyn Error>> {
    let mut stderr = std::io::stderr().lock();
    writeln!(
        &mut stderr,
        "Decompressed to {} (records: {}, blocks processed: {})",
        output.display(),
        summary.records_written,
        summary.blocks_processed
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn pack_and_unpack_roundtrip() {
        let data = "{\"id\":1}\n{\"id\":2}\n";
        let paths = temp_paths("cli_roundtrip");

        fs::write(&paths.input_ndjson, data).unwrap();

        handle_pack(
            paths.input_ndjson.clone(),
            paths.output_jac.clone(),
            1000,
            3,
            false,
            false,
            4_096,
            true,
            true,
            false,
        )
        .unwrap();

        handle_unpack(
            paths.output_jac.clone(),
            paths.output_json.clone(),
            true,
            false,
        )
        .unwrap();

        let result = fs::read_to_string(&paths.output_json).unwrap();
        assert_eq!(normalize(&result), normalize(data));
    }

    #[test]
    fn pack_json_array_roundtrip() {
        let data = r#"[{"id":1},{"id":2}]"#;
        let paths = temp_json_paths("cli_json_roundtrip");

        fs::write(&paths.input_json, data).unwrap();

        handle_pack(
            paths.input_json.clone(),
            paths.output_jac.clone(),
            10,
            3,
            false,
            false,
            4_096,
            true,
            false,
            false,
        )
        .unwrap();

        handle_unpack(
            paths.output_jac.clone(),
            paths.output_json.clone(),
            false,
            true,
        )
        .unwrap();

        let result = fs::read_to_string(&paths.output_json).unwrap();
        let expected: Value = serde_json::from_str(data).unwrap();
        let actual: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(actual, expected);
    }

    fn normalize(input: &str) -> Vec<String> {
        input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .collect()
    }

    struct TempCliPaths {
        input_ndjson: PathBuf,
        output_jac: PathBuf,
        output_json: PathBuf,
    }

    fn temp_paths(label: &str) -> TempCliPaths {
        let base = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        TempCliPaths {
            input_ndjson: base.join(format!("{}_{}_input.ndjson", label, unique)),
            output_jac: base.join(format!("{}_{}_output.jac", label, unique)),
            output_json: base.join(format!("{}_{}_output.ndjson", label, unique)),
        }
    }

    impl Drop for TempCliPaths {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.input_ndjson);
            let _ = fs::remove_file(&self.output_jac);
            let _ = fs::remove_file(&self.output_json);
        }
    }

    struct TempCliJsonPaths {
        input_json: PathBuf,
        output_jac: PathBuf,
        output_json: PathBuf,
    }

    fn temp_json_paths(label: &str) -> TempCliJsonPaths {
        let base = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        TempCliJsonPaths {
            input_json: base.join(format!("{}_{}_input.json", label, unique)),
            output_jac: base.join(format!("{}_{}_output.jac", label, unique)),
            output_json: base.join(format!("{}_{}_output.json", label, unique)),
        }
    }

    impl Drop for TempCliJsonPaths {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.input_json);
            let _ = fs::remove_file(&self.output_jac);
            let _ = fs::remove_file(&self.output_json);
        }
    }
}
