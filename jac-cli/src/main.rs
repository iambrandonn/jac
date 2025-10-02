//! JAC CLI - Command-line tool for JSON-Aware Compression
//!
//! This binary provides command-line interfaces for:
//! - pack: compress JSON/NDJSON → .jac
//! - unpack: decompress .jac → JSON/NDJSON/CSV
//! - ls: list blocks, fields, and optional statistics
//! - cat: stream values for a field across blocks

use clap::{Parser, Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use jac_io::{
    execute_compress, execute_decompress, BlockHandle, Codec, CompressOptions, CompressRequest,
    CompressSummary, DecompressFormat, DecompressOptions, DecompressOpts, DecompressRequest,
    DecompressSummary, InputSource, JacInput, JacReader, Limits, OutputSink,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
        /// Show progress spinner while compressing
        #[arg(long)]
        progress: bool,
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
        /// Show progress spinner while decompressing
        #[arg(long)]
        progress: bool,
    },
    /// List blocks, fields, and record counts
    ///
    /// Examples:
    ///   jac ls data.jac
    ///   jac ls data.jac --verbose --format json
    ///   jac ls data.jac --fields-only
    Ls {
        /// Input file (.jac)
        input: PathBuf,
        /// Output format (table, json)
        #[arg(long, value_enum, default_value_t = LsFormat::Table)]
        format: LsFormat,
        /// Verbose output with additional statistics
        #[arg(long, short = 'v')]
        verbose: bool,
        /// Show only field names
        #[arg(long, conflicts_with = "blocks_only")]
        fields_only: bool,
        /// Show only block summary
        #[arg(long, conflicts_with = "fields_only")]
        blocks_only: bool,
        /// Compute and display detailed field statistics
        #[arg(long)]
        stats: bool,
    },
    /// Stream values for a specific field
    ///
    /// Examples:
    ///   jac cat data.jac --field user
    ///   jac cat data.jac --field user --format csv
    ///   jac cat data.jac --field level --blocks 2-5 --progress
    Cat {
        /// Input file (.jac)
        input: PathBuf,
        /// Field name to extract
        #[arg(long)]
        field: String,
        /// Output format (ndjson, json-array, csv)
        #[arg(long, value_enum, default_value_t = CatFormat::Ndjson)]
        format: CatFormat,
        /// Block range filter (e.g. "1-5" or "3")
        #[arg(long)]
        blocks: Option<String>,
        /// Display a progress spinner during streaming
        #[arg(long)]
        progress: bool,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum LsFormat {
    Table,
    Json,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum CatFormat {
    Ndjson,
    #[value(name = "json-array")]
    JsonArray,
    Csv,
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
            progress,
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
                progress,
            )?;
        }
        Commands::Unpack {
            input,
            output,
            ndjson,
            json_array,
            progress,
        } => {
            handle_unpack(input, output, ndjson, json_array, progress)?;
        }
        Commands::Ls {
            input,
            format,
            verbose,
            fields_only,
            blocks_only,
            stats,
        } => {
            handle_ls(input, format, verbose, fields_only, blocks_only, stats)?;
        }
        Commands::Cat {
            input,
            field,
            format,
            blocks,
            progress,
        } => {
            handle_cat(input, field, format, blocks, progress)?;
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
    show_progress: bool,
) -> Result<(), Box<dyn Error>> {
    let start = Instant::now();
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

    let mut progress_bar = show_progress.then(|| create_spinner("Compressing records"));
    let summary = execute_compress(request)?;
    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64().max(f64::EPSILON);
    let rec_rate = summary.metrics.records_written as f64 / secs;
    let mb_rate = summary.metrics.bytes_written as f64 / (1024.0 * 1024.0) / secs;
    if let Some(pb) = progress_bar.take() {
        pb.finish_with_message(format!(
            "Compressed {} records across {} blocks in {:.2?} ({:.1} rec/s, {:.2} MiB/s)",
            summary.metrics.records_written,
            summary.metrics.blocks_written,
            elapsed,
            rec_rate,
            mb_rate
        ));
    }
    report_compress_summary(&summary, &output, elapsed, rec_rate, mb_rate)?;
    Ok(())
}

fn handle_unpack(
    input: PathBuf,
    output: PathBuf,
    _ndjson: bool,
    json_array: bool,
    show_progress: bool,
) -> Result<(), Box<dyn Error>> {
    let start = Instant::now();
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

    let mut progress_bar = show_progress.then(|| create_spinner("Decompressing records"));
    let summary = execute_decompress(request)?;
    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64().max(f64::EPSILON);
    let rec_rate = summary.records_written as f64 / secs;
    let output_bytes = std::fs::metadata(&output).map(|m| m.len()).ok();
    let mb_rate = output_bytes.map(|bytes| bytes as f64 / (1024.0 * 1024.0) / secs);
    if let Some(pb) = progress_bar.take() {
        pb.finish_with_message(format!(
            "Decompressed {} records from {} blocks in {:.2?} ({:.1} rec/s{})",
            summary.records_written,
            summary.blocks_processed,
            elapsed,
            rec_rate,
            mb_rate
                .map(|rate| format!(", {:.2} MiB/s", rate))
                .unwrap_or_default()
        ));
    }
    report_decompress_summary(&summary, &output, elapsed, rec_rate, mb_rate)?;
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

fn report_compress_summary(
    summary: &CompressSummary,
    output: &Path,
    elapsed: Duration,
    rec_rate: f64,
    mb_rate: f64,
) -> Result<(), Box<dyn Error>> {
    let mut stderr = std::io::stderr().lock();
    writeln!(
        &mut stderr,
        "Compressed to {} (records: {}, blocks: {}, bytes written: {}, elapsed: {:.2?}, {:.1} rec/s, {:.2} MiB/s)",
        output.display(),
        summary.metrics.records_written,
        summary.metrics.blocks_written,
        summary.metrics.bytes_written,
        elapsed,
        rec_rate,
        mb_rate
    )?;
    Ok(())
}

fn report_decompress_summary(
    summary: &DecompressSummary,
    output: &Path,
    elapsed: Duration,
    rec_rate: f64,
    mb_rate: Option<f64>,
) -> Result<(), Box<dyn Error>> {
    let mut stderr = std::io::stderr().lock();
    let mut message = format!(
        "Decompressed to {} (records: {}, blocks processed: {}, elapsed: {:.2?}, {:.1} rec/s",
        output.display(),
        summary.records_written,
        summary.blocks_processed,
        elapsed,
        rec_rate
    );
    if let Some(rate) = mb_rate {
        message.push_str(&format!(", {:.2} MiB/s", rate));
    }
    message.push(')');
    writeln!(&mut stderr, "{}", message)?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
struct BlockSummary {
    block_index: usize,
    record_count: usize,
    field_count: usize,
    compressed_size: usize,
    fields: Vec<FieldSummary>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct FieldSummary {
    name: String,
    present_count: usize,
    encoding_flags: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    dict_entries: Option<usize>,
    compressed_size: usize,
    uncompressed_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    compression_ratio: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct DetailedFieldStats {
    field_name: String,
    total_records: usize,
    present_values: usize,
    null_count: usize,
    absent_values: usize,
    type_distribution: BTreeMap<String, usize>,
    sample_size: usize,
    sampled: bool,
}

#[derive(Default)]
struct FieldStatsAccumulator {
    total_records: usize,
    present_values: usize,
    absent_values: usize,
    null_count: usize,
    type_distribution: BTreeMap<String, usize>,
    sample_values: usize,
    sampled: bool,
}

#[derive(Debug, Default, Clone, Copy)]
struct ReaderMetrics {
    blocks_read: u64,
    records_observed: u64,
    bytes_observed: u64,
}

impl FieldStatsAccumulator {
    fn record_value(&mut self, value: &Value) {
        if self.sample_values >= STATS_SAMPLE_LIMIT_PER_FIELD {
            self.sampled = true;
            return;
        }

        self.sample_values += 1;

        match value {
            Value::Null => {
                self.null_count += 1;
            }
            Value::Bool(_) => {
                *self
                    .type_distribution
                    .entry("bool".to_string())
                    .or_default() += 1;
            }
            Value::Number(num) => {
                let kind = if num.is_i64() || num.is_u64() {
                    "int"
                } else {
                    "decimal"
                };
                *self.type_distribution.entry(kind.to_string()).or_default() += 1;
            }
            Value::String(_) => {
                *self
                    .type_distribution
                    .entry("string".to_string())
                    .or_default() += 1;
            }
            Value::Array(_) => {
                *self
                    .type_distribution
                    .entry("array".to_string())
                    .or_default() += 1;
            }
            Value::Object(_) => {
                *self
                    .type_distribution
                    .entry("object".to_string())
                    .or_default() += 1;
            }
        }
    }

    fn into_detailed(self, field_name: String) -> DetailedFieldStats {
        DetailedFieldStats {
            field_name,
            total_records: self.total_records,
            present_values: self.present_values,
            null_count: self.null_count,
            absent_values: self.absent_values,
            type_distribution: self.type_distribution,
            sample_size: self.sample_values,
            sampled: self.sampled,
        }
    }
}

#[derive(Debug, Clone)]
enum BlockRange {
    Single(usize),
    Range { start: usize, end: Option<usize> },
}

impl BlockRange {
    fn into_bounds(self, block_count: usize) -> Result<(usize, usize), Box<dyn Error>> {
        if block_count == 0 {
            return Err("Block range requested, but file contains no blocks".into());
        }

        match self {
            BlockRange::Single(idx) => {
                if idx >= block_count {
                    return Err(format!(
                        "Block {} exceeds file block count ({} blocks)",
                        idx + 1,
                        block_count
                    )
                    .into());
                }
                Ok((idx, idx))
            }
            BlockRange::Range { start, end } => {
                if start >= block_count {
                    return Err(format!(
                        "Block range start {} exceeds file block count ({} blocks)",
                        start + 1,
                        block_count
                    )
                    .into());
                }
                let end_idx = match end {
                    Some(e) => e,
                    None => block_count - 1,
                };
                if end_idx >= block_count {
                    return Err(format!(
                        "Block range end {} exceeds file block count ({} blocks)",
                        end_idx + 1,
                        block_count
                    )
                    .into());
                }
                if start > end_idx {
                    return Err(format!(
                        "Invalid block range: start ({}) > end ({})",
                        start + 1,
                        end_idx + 1
                    )
                    .into());
                }
                Ok((start, end_idx))
            }
        }
    }
}

fn handle_ls(
    input: PathBuf,
    format: LsFormat,
    verbose: bool,
    fields_only: bool,
    blocks_only: bool,
    stats: bool,
) -> Result<(), Box<dyn Error>> {
    let start = Instant::now();
    let file = File::open(&input)?;
    let options = DecompressOptions::default();
    let codec_opts = DecompressOpts {
        limits: options.limits.clone(),
        verify_checksums: options.verify_checksums,
    };
    let mut reader = JacReader::new(file, codec_opts)?;

    let mut scan_spinner = if verbose || stats {
        Some(create_spinner("Scanning blocks"))
    } else {
        None
    };

    let mut block_handles = Vec::new();
    let mut summaries = Vec::new();
    let mut all_fields = HashSet::new();
    let mut reader_metrics = ReaderMetrics::default();

    for (idx, block_result) in reader.blocks().enumerate() {
        let block = block_result?;
        if let Some(pb) = scan_spinner.as_ref() {
            pb.set_position((idx + 1) as u64);
        }

        let mut field_summaries = Vec::with_capacity(block.header.fields.len());
        for field in &block.header.fields {
            all_fields.insert(field.field_name.clone());
            let ratio = if field.segment_uncompressed_len > 0 {
                Some(field.segment_compressed_len as f64 / field.segment_uncompressed_len as f64)
            } else {
                None
            };

            field_summaries.push(FieldSummary {
                name: field.field_name.clone(),
                present_count: field.value_count_present,
                encoding_flags: field.encoding_flags,
                dict_entries: (field.dict_entry_count > 0).then(|| field.dict_entry_count),
                compressed_size: field.segment_compressed_len,
                uncompressed_size: field.segment_uncompressed_len,
                compression_ratio: ratio,
            });
        }

        reader_metrics.blocks_read += 1;
        reader_metrics.records_observed += block.record_count as u64;
        reader_metrics.bytes_observed += block.size as u64;

        summaries.push(BlockSummary {
            block_index: idx + 1,
            record_count: block.record_count,
            field_count: block.header.fields.len(),
            compressed_size: block.size,
            fields: field_summaries,
        });
        block_handles.push(block);
    }

    if let Some(pb) = scan_spinner.take() {
        pb.finish_with_message(format!("Scanned {} blocks", block_handles.len()));
    }

    let mut sorted_fields: Vec<_> = all_fields.iter().cloned().collect();
    sorted_fields.sort();

    let mut stats_bar = stats.then(|| create_spinner("Computing field statistics"));
    let detailed_stats = if stats {
        let stats_vec = compute_detailed_stats(
            &mut reader,
            &block_handles,
            &summaries,
            &sorted_fields,
            stats_bar.as_ref(),
        )?;
        if let Some(pb) = stats_bar.take() {
            pb.finish_with_message(format!("Computed stats for {} fields", sorted_fields.len()));
        }
        Some(stats_vec)
    } else {
        None
    };

    if verbose {
        let mut stderr = std::io::stderr().lock();
        let total_records: usize = summaries.iter().map(|s| s.record_count).sum();
        let bytes_mib = reader_metrics.bytes_observed as f64 / (1024.0 * 1024.0);
        writeln!(
            &mut stderr,
            "File summary → blocks: {}, unique fields: {}, records: {}, compressed bytes: {:.2} MiB",
            block_handles.len(),
            sorted_fields.len(),
            total_records,
            bytes_mib
        )?;
        if let Some(stats_vec) = detailed_stats.as_ref() {
            if let Some(top) = stats_vec.iter().max_by_key(|s| s.present_values) {
                writeln!(
                    &mut stderr,
                    "Most populated field '{}' → present {}, null {}, absent {}",
                    top.field_name, top.present_values, top.null_count, top.absent_values
                )?;
            }
            if stats_vec.iter().any(|s| s.sampled) {
                writeln!(
                    &mut stderr,
                    "Note: stats sampling limited to {} values per field",
                    STATS_SAMPLE_LIMIT_PER_FIELD
                )?;
            }
        }
        writeln!(&mut stderr, "Listing completed in {:.2?}", start.elapsed())?;
    }

    match format {
        LsFormat::Table => {
            let mut stdout = std::io::stdout().lock();
            print_ls_table(
                &mut stdout,
                &summaries,
                &sorted_fields,
                verbose,
                fields_only,
                blocks_only,
                detailed_stats.as_deref(),
            )?;
        }
        LsFormat::Json => {
            let mut stdout = std::io::stdout().lock();
            print_ls_json(
                &mut stdout,
                &summaries,
                &sorted_fields,
                verbose,
                fields_only,
                blocks_only,
                detailed_stats.as_deref(),
            )?;
        }
    }

    Ok(())
}

fn print_ls_table(
    writer: &mut dyn Write,
    summaries: &[BlockSummary],
    all_fields: &[String],
    verbose: bool,
    fields_only: bool,
    blocks_only: bool,
    stats: Option<&[DetailedFieldStats]>,
) -> Result<(), Box<dyn Error>> {
    if fields_only {
        for field in all_fields {
            writeln!(writer, "{}", field)?;
        }
        if let Some(stats_entries) = stats {
            writeln!(writer)?;
            print_table_stats(writer, stats_entries)?;
        }
        return Ok(());
    }

    if blocks_only {
        writeln!(writer, "Block\tRecords\tFields\tSize")?;
        for summary in summaries {
            writeln!(
                writer,
                "{}\t{}\t{}\t{}",
                summary.block_index,
                summary.record_count,
                summary.field_count,
                summary.compressed_size
            )?;
        }
        if let Some(stats_entries) = stats {
            writeln!(writer)?;
            print_table_stats(writer, stats_entries)?;
        }
        return Ok(());
    }

    writeln!(writer, "Block\tRecords\tFields\tSize\tField Details")?;
    for summary in summaries {
        let mut sorted_fields = summary.fields.clone();
        sorted_fields.sort_by(|a, b| a.name.cmp(&b.name));
        let field_list = sorted_fields
            .iter()
            .map(|field| {
                if verbose {
                    let ratio_str = field
                        .compression_ratio
                        .map(|r| format!(", ratio: {:.2}", r))
                        .unwrap_or_default();
                    let dict_str = field
                        .dict_entries
                        .map(|d| format!(", dict: {}", d))
                        .unwrap_or_default();
                    format!(
                        "{} (present: {}{}, compressed: {} bytes{})",
                        field.name, field.present_count, ratio_str, field.compressed_size, dict_str
                    )
                } else {
                    field.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            writer,
            "{}\t{}\t{}\t{}\t{}",
            summary.block_index,
            summary.record_count,
            summary.field_count,
            summary.compressed_size,
            field_list
        )?;
    }

    if let Some(stats_entries) = stats {
        writeln!(writer)?;
        print_table_stats(writer, stats_entries)?;
    }

    Ok(())
}

fn print_table_stats(
    writer: &mut dyn Write,
    stats: &[DetailedFieldStats],
) -> Result<(), Box<dyn Error>> {
    writeln!(writer, "Field\tPresent\tNull\tAbsent\tTypes\tSampled")?;
    for entry in stats {
        let formatted_types = if entry.type_distribution.is_empty() {
            "-".to_string()
        } else {
            entry
                .type_distribution
                .iter()
                .map(|(kind, count)| format!("{}:{}", kind, count))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let sampled_note = if entry.sampled {
            format!("yes ({} values)", entry.sample_size)
        } else {
            "no".to_string()
        };
        writeln!(
            writer,
            "{}\t{}\t{}\t{}\t{}\t{}",
            entry.field_name,
            entry.present_values,
            entry.null_count,
            entry.absent_values,
            formatted_types,
            sampled_note
        )?;
    }
    Ok(())
}

fn print_ls_json(
    writer: &mut dyn Write,
    summaries: &[BlockSummary],
    all_fields: &[String],
    verbose: bool,
    fields_only: bool,
    blocks_only: bool,
    stats: Option<&[DetailedFieldStats]>,
) -> Result<(), Box<dyn Error>> {
    let mut root = serde_json::Map::new();

    if !blocks_only {
        root.insert("fields".to_string(), serde_json::to_value(all_fields)?);
    }

    if !fields_only {
        root.insert("blocks".to_string(), serde_json::to_value(summaries)?);
    }

    root.insert("verbose".to_string(), serde_json::Value::Bool(verbose));

    if let Some(stats_entries) = stats {
        root.insert("stats".to_string(), serde_json::to_value(stats_entries)?);
    }

    serde_json::to_writer_pretty(&mut *writer, &serde_json::Value::Object(root))?;
    writeln!(writer)?;
    Ok(())
}

fn compute_detailed_stats(
    reader: &mut JacReader<File>,
    blocks: &[BlockHandle],
    summaries: &[BlockSummary],
    all_fields: &[String],
    progress: Option<&ProgressBar>,
) -> Result<Vec<DetailedFieldStats>, Box<dyn Error>> {
    let mut stats_map: BTreeMap<String, FieldStatsAccumulator> = all_fields
        .iter()
        .cloned()
        .map(|name| (name, FieldStatsAccumulator::default()))
        .collect();

    reader.rewind()?;

    for (idx, block) in blocks.iter().enumerate() {
        if let Some(pb) = progress {
            pb.set_position((idx + 1) as u64);
        }

        let summary = &summaries[idx];

        let mut present_fields: HashSet<&str> = HashSet::with_capacity(summary.fields.len());

        for field_summary in &summary.fields {
            present_fields.insert(field_summary.name.as_str());
            let accumulator = stats_map
                .get_mut(&field_summary.name)
                .expect("field should exist in stats map");

            accumulator.total_records += block.record_count;
            accumulator.present_values += field_summary.present_count;
            accumulator.absent_values += block.record_count - field_summary.present_count;

            let iterator = reader.project_field(block, &field_summary.name)?;
            for value_result in iterator {
                let maybe_value = value_result?;
                if let Some(value) = maybe_value {
                    accumulator.record_value(&value);
                }
                if accumulator.sampled {
                    break;
                }
            }
        }

        for field_name in all_fields {
            if present_fields.contains(field_name.as_str()) {
                continue;
            }
            if let Some(accumulator) = stats_map.get_mut(field_name) {
                accumulator.total_records += block.record_count;
                accumulator.absent_values += block.record_count;
            }
        }
    }

    Ok(stats_map
        .into_iter()
        .map(|(name, accumulator)| accumulator.into_detailed(name))
        .collect())
}

fn handle_cat(
    input: PathBuf,
    field: String,
    format: CatFormat,
    blocks: Option<String>,
    progress: bool,
) -> Result<(), Box<dyn Error>> {
    let file = File::open(&input)?;
    let options = DecompressOptions::default();
    let codec_opts = DecompressOpts {
        limits: options.limits.clone(),
        verify_checksums: options.verify_checksums,
    };
    let mut reader = JacReader::new(file, codec_opts)?;
    let range = parse_block_range(blocks)?;

    let (available_fields, field_present) = collect_available_fields(&mut reader, &field)?;
    if !field_present {
        let mut sorted: Vec<_> = available_fields.into_iter().collect();
        sorted.sort();
        if sorted.is_empty() {
            return Err(format!(
                "Field '{}' not found in JAC file (no fields detected)",
                field
            )
            .into());
        }
        return Err(format!(
            "Field '{}' not found in JAC file. Available fields: {}",
            field,
            sorted.join(", ")
        )
        .into());
    }

    reader.rewind()?;
    let mut writer = CatWriter::new(format)?;
    let mut progress_bar = progress.then(|| create_spinner("Streaming field values"));
    let mut values_emitted: u64 = 0;
    let start = Instant::now();
    let mut reader_metrics = ReaderMetrics::default();

    if let Some(range) = range {
        let block_handles: Vec<_> = reader.blocks().collect::<Result<Vec<_>, _>>()?;
        let block_count = block_handles.len();
        let (start_idx, end_idx) = range.into_bounds(block_count)?;

        for (block_idx, block) in block_handles.into_iter().enumerate() {
            if block_idx < start_idx || block_idx > end_idx {
                continue;
            }

            reader_metrics.blocks_read += 1;
            reader_metrics.records_observed += block.record_count as u64;
            reader_metrics.bytes_observed += block.size as u64;

            let field_iter = reader.project_field(&block, &field)?;
            for value_result in field_iter {
                let maybe_value = value_result?;
                if let Some(value) = maybe_value {
                    writer.write_value(value)?;
                    values_emitted += 1;
                    if let Some(pb) = &progress_bar {
                        pb.set_position(values_emitted);
                    }
                }
            }
        }
    } else {
        let mut projection_stream = reader.projection_stream(field)?;
        for value_result in projection_stream.by_ref() {
            let maybe_value = value_result?;
            if let Some(value) = maybe_value {
                writer.write_value(value)?;
                values_emitted += 1;
                if let Some(pb) = &progress_bar {
                    pb.set_position(values_emitted);
                }
            }
        }
        reader_metrics.records_observed = values_emitted;
    }

    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64().max(f64::EPSILON);
    let value_rate = if values_emitted > 0 {
        values_emitted as f64 / secs
    } else {
        0.0
    };

    if let Some(pb) = progress_bar.take() {
        pb.finish_with_message(format!(
            "Emitted {} values in {:.2?} ({:.1} val/s)",
            values_emitted, elapsed, value_rate
        ));
    }

    writer.finish()?;
    let mut stderr = std::io::stderr().lock();
    writeln!(
        &mut stderr,
        "Streamed {} values in {:.2?} ({:.1} val/s, blocks: {}, observed records: {}, bytes: {:.2} MiB)",
        values_emitted,
        elapsed,
        value_rate,
        reader_metrics.blocks_read,
        reader_metrics.records_observed,
        reader_metrics.bytes_observed as f64 / (1024.0 * 1024.0)
    )?;
    Ok(())
}

fn parse_block_range(spec: Option<String>) -> Result<Option<BlockRange>, Box<dyn Error>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("Block range cannot be empty".into());
    }

    if let Ok(n) = trimmed.parse::<usize>() {
        if n == 0 {
            return Err("Block numbers start from 1".into());
        }
        return Ok(Some(BlockRange::Single(n - 1)));
    }

    if let Some((start_part, end_part)) = trimmed.split_once('-') {
        let start_idx = if start_part.is_empty() {
            0
        } else {
            let start = start_part.parse::<usize>()?;
            if start == 0 {
                return Err("Block numbers start from 1".into());
            }
            start - 1
        };

        let end_idx = if end_part.is_empty() {
            None
        } else {
            let end = end_part.parse::<usize>()?;
            if end == 0 {
                return Err("Block numbers start from 1".into());
            }
            Some(end - 1)
        };

        if let Some(end) = end_idx {
            if start_idx > end {
                return Err(format!(
                    "Invalid block range: start ({}) > end ({})",
                    start_idx + 1,
                    end + 1
                )
                .into());
            }
        }

        return Ok(Some(BlockRange::Range {
            start: start_idx,
            end: end_idx,
        }));
    }

    Err(format!(
        "Invalid block range syntax: '{}'. Expected 'N' or 'N-M'",
        trimmed
    )
    .into())
}

fn collect_available_fields<R: Read + Seek>(
    reader: &mut JacReader<R>,
    target_field: &str,
) -> Result<(HashSet<String>, bool), Box<dyn Error>> {
    let mut available = HashSet::new();
    let mut found = false;
    let mut blocks = reader.blocks();
    while let Some(block_res) = blocks.next() {
        let block = block_res?;
        for entry in &block.header.fields {
            if entry.field_name == target_field {
                found = true;
            }
            available.insert(entry.field_name.clone());
        }
    }
    Ok((available, found))
}

struct CatWriter {
    format: CatFormat,
    writer: Box<dyn Write>,
    first: bool,
}

impl CatWriter {
    fn new(format: CatFormat) -> Result<Self, Box<dyn Error>> {
        let stdout = std::io::stdout();
        let writer: Box<dyn Write> = Box::new(BufWriter::new(stdout));
        Self::with_writer(format, writer)
    }

    fn with_writer(format: CatFormat, mut writer: Box<dyn Write>) -> Result<Self, Box<dyn Error>> {
        if matches!(format, CatFormat::JsonArray) {
            writer.write_all(b"[")?;
        }

        Ok(Self {
            format,
            writer,
            first: true,
        })
    }

    fn write_value(&mut self, value: Value) -> Result<(), Box<dyn Error>> {
        match self.format {
            CatFormat::Ndjson => {
                serde_json::to_writer(&mut self.writer, &value)?;
                self.writer.write_all(b"\n")?;
            }
            CatFormat::JsonArray => {
                if !self.first {
                    self.writer.write_all(b",")?;
                }
                serde_json::to_writer(&mut self.writer, &value)?;
            }
            CatFormat::Csv => {
                let text = csv_serialize(&value)?;
                self.writer.write_all(text.as_bytes())?;
                self.writer.write_all(b"\n")?;
            }
        }

        if self.first {
            self.first = false;
        }

        Ok(())
    }

    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        if matches!(self.format, CatFormat::JsonArray) {
            self.writer.write_all(b"]")?;
            self.writer.write_all(b"\n")?;
        }
        self.writer.flush()?;
        Ok(())
    }
}

fn csv_serialize(value: &Value) -> Result<String, serde_json::Error> {
    match value {
        Value::Null => Ok("null".to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(num) => Ok(num.to_string()),
        Value::String(s) => Ok(s.clone()),
        other => serde_json::to_string(other),
    }
}

fn create_spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} [{elapsed_precise}] {pos} {msg}")
            .unwrap(),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
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
            false,
        )
        .unwrap();

        handle_unpack(
            paths.output_jac.clone(),
            paths.output_json.clone(),
            true,
            false,
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
            false,
        )
        .unwrap();

        handle_unpack(
            paths.output_jac.clone(),
            paths.output_json.clone(),
            false,
            true,
            false,
        )
        .unwrap();

        let result = fs::read_to_string(&paths.output_json).unwrap();
        let expected: Value = serde_json::from_str(data).unwrap();
        let actual: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn parse_block_range_single_block() {
        let range = parse_block_range(Some("3".to_string()))
            .unwrap()
            .expect("expected range");
        match range {
            BlockRange::Single(idx) => assert_eq!(idx, 2),
            _ => panic!("expected single block"),
        }
    }

    #[test]
    fn parse_block_range_open_ended() {
        let range = parse_block_range(Some("5-".to_string()))
            .unwrap()
            .expect("expected range");
        match range {
            BlockRange::Range { start, end } => {
                assert_eq!(start, 4);
                assert!(end.is_none());
            }
            _ => panic!("expected range variant"),
        }
    }

    #[test]
    fn block_range_into_bounds_normalizes_end() {
        let range = BlockRange::Range {
            start: 1,
            end: None,
        };
        let (start, end) = range.into_bounds(4).unwrap();
        assert_eq!(start, 1);
        assert_eq!(end, 3);
    }

    #[test]
    fn csv_serialize_roundtrip_examples() {
        assert_eq!(
            csv_serialize(&Value::String("hello".into())).unwrap(),
            "hello"
        );
        assert_eq!(csv_serialize(&Value::Bool(true)).unwrap(), "true");
        assert_eq!(
            csv_serialize(&serde_json::json!({"a":1})).unwrap(),
            "{\"a\":1}"
        );
    }

    #[test]
    fn print_ls_table_fields_only_sorts() {
        let mut buf = Vec::new();
        let summaries = Vec::new();
        let all_fields = vec![
            "id".to_string(),
            "timestamp".to_string(),
            "user".to_string(),
        ];
        print_ls_table(&mut buf, &summaries, &all_fields, false, true, false, None).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines, vec!["id", "timestamp", "user"]);
    }

    #[test]
    fn print_ls_json_includes_blocks_and_fields() {
        let mut buf = Vec::new();
        let summaries = vec![BlockSummary {
            block_index: 1,
            record_count: 10,
            field_count: 2,
            compressed_size: 256,
            fields: vec![
                FieldSummary {
                    name: "a".to_string(),
                    present_count: 5,
                    encoding_flags: 0,
                    dict_entries: None,
                    compressed_size: 100,
                    uncompressed_size: 120,
                    compression_ratio: Some(100.0 / 120.0),
                },
                FieldSummary {
                    name: "b".to_string(),
                    present_count: 5,
                    encoding_flags: 0,
                    dict_entries: Some(3),
                    compressed_size: 80,
                    uncompressed_size: 150,
                    compression_ratio: Some(80.0 / 150.0),
                },
            ],
        }];
        let all_fields = vec!["a".to_string(), "b".to_string()];
        print_ls_json(&mut buf, &summaries, &all_fields, false, false, false, None).unwrap();
        let value: Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(value["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(value.get("fields"), Some(&serde_json::json!(["a", "b"])));
        let block_fields = value["blocks"][0]["fields"].as_array().unwrap();
        assert_eq!(block_fields.len(), 2);
        assert_eq!(block_fields[0]["name"], "a");
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
const STATS_SAMPLE_LIMIT_PER_FIELD: usize = 50_000;
