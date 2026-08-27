// src/main.rs – CLI entry point.
//
// Mirrors the Kotlin `Converter` CliktCommand with the same flags and
// semantics.

use std::path::PathBuf;

use clap::Parser;
use log::{error, info};

use odx_converter::{Converter, ConverterOptions, CompressionConfig};

#[derive(Debug, Parser)]
#[command(
    name = "odx-converter",
    about = "Converts PDX (packed ODX, ISO-22901) files to the MDD \
             (Marvelous Diagnostic Description) format for embedded systems.\n\n\
             Inspired by eclipse-opensovd/odx-converter (Kotlin) – re-implemented in Rust.",
    version = env!("CARGO_PKG_VERSION"),
    long_about = None
)]
struct Cli {
    /// PDX files to convert.
    #[arg(
        value_name = "pdx-files",
        required_unless_present_any = ["version_flag", "decode"],
        num_args = 0..
    )]
    pdx_files: Vec<PathBuf>,

    /// Output directory for generated .mdd files
    /// (default: same directory as the input PDX file).
    #[arg(short = 'O', long = "output-directory", value_name = "path")]
    output_dir: Option<PathBuf>,

    /// Lenient mode: emit a warning instead of aborting on ODX resolution errors.
    #[arg(short = 'L', long = "lenient", default_value_t = false)]
    lenient: bool,

    /// Include all referenced job files and libraries as CODE_FILE chunks.
    #[arg(long = "include-job-files", default_value_t = false)]
    include_job_files: bool,

    /// Include job file entries partially.
    /// Repeat for multiple patterns.  Format: --partial-job-files <job-regex> <content-regex>
    #[arg(
        long = "partial-job-files",
        value_names = ["job-pattern", "content-pattern"],
        num_args = 2
    )]
    partial_job_files: Vec<String>,

    /// Include services only when the specified audience short-name(s) match.
    /// Services with no enabled audience are always included.
    /// Can be repeated.
    #[arg(long = "with-audience")]
    with_audiences: Vec<String>,

    /// Compression algorithm for the diagnostic-description chunk.
    /// Choices: none | lzma | zstd | zstd:LEVEL (1-22) | lz4
    /// Default is "lzma" to match the original eclipse-opensovd/odx-converter
    /// (Kotlin) reference implementation's output format.
    #[arg(long = "compression", default_value = "lzma")]
    compression: String,

    /// Maximum number of PDX files to process in parallel
    /// (default: number of logical CPU cores).
    #[arg(short = 'j', long = "parallel", default_value_t = available_parallelism())]
    parallel: usize,

    /// Also export a human-readable JSON dump of each generated .mdd file
    /// (decoding the container, decompressing chunks, and decoding the
    /// diagnostic-description FlatBuffers payload), written alongside the
    /// .mdd with a .json extension.
    #[arg(long = "json", default_value_t = false)]
    json: bool,

    /// Console log level: trace | debug | info | warn | error.
    #[arg(long = "log-level", default_value = "info")]
    log_level: String,

    /// Decode an existing .mdd file to JSON and exit (does not convert any
    /// PDX). Useful for inspecting/comparing .mdd files produced by other
    /// tools (e.g. the Kotlin reference implementation).
    #[arg(long = "decode", value_name = "mdd-file")]
    decode: Option<PathBuf>,

    /// Print version information and exit.
    #[arg(long = "version-flag", hide = true)]
    version_flag: bool,
}

fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Parse a user-supplied compression string into a `CompressionConfig`.
///
/// Accepted forms: `none`, `zstd`, `zstd:3`, `lz4`.
fn parse_compression(s: &str) -> CompressionConfig {
    match s.to_ascii_lowercase().as_str() {
        "none" | "off" => CompressionConfig::None,
        "lzma" => CompressionConfig::Lzma,
        "lz4" => CompressionConfig::Lz4,
        s if s.starts_with("zstd:") => {
            let level = s.trim_start_matches("zstd:")
                .parse::<i32>()
                .unwrap_or(3)
                .clamp(1, 22);
            CompressionConfig::Zstd { level }
        }
        "zstd" => CompressionConfig::Zstd { level: 3 },
        _ => CompressionConfig::Lzma, // default, matches Kotlin reference
    }
}

fn main() {
    let cli = Cli::parse();

    // Version banner
    if cli.version_flag {
        println!("odx-converter-rs");
        println!("Version: {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if let Some(mdd_path) = &cli.decode {
        let json_out = mdd_path.with_extension("json");
        match odx_converter::parser::export_mdd_to_json(mdd_path, &json_out) {
            Ok(()) => {
                println!("Decoded {} -> {}", mdd_path.display(), json_out.display());
                return;
            }
            Err(e) => {
                eprintln!("Failed to decode {}: {:#}", mdd_path.display(), e);
                std::process::exit(1);
            }
        }
    }

    // Logging
    let level_filter = match cli.log_level.to_ascii_lowercase().as_str() {
        "trace" => log::LevelFilter::Trace,
        "debug" => log::LevelFilter::Debug,
        "warn" | "warning" => log::LevelFilter::Warn,
        "error" => log::LevelFilter::Error,
        _ => log::LevelFilter::Info,
    };
    env_logger::Builder::new()
        .filter_level(level_filter)
        .format_timestamp_secs()
        .init();

    println!(
        "odx-converter-rs {} – ODX (ISO-22901) → MDD converter written in Rust\n",
        env!("CARGO_PKG_VERSION")
    );

    if cli.pdx_files.is_empty() {
        eprintln!("No PDX files specified.  Run with --help for usage.");
        std::process::exit(1);
    }

    // Build partial-job-files pairs
    let partial_job_files: Vec<odx_converter::options::PartialFilePattern> = cli
        .partial_job_files
        .chunks_exact(2)
        .map(|c| (c[0].clone(), c[1].clone()).into())
        .collect();

    let compression = parse_compression(&cli.compression);

    let options = ConverterOptions {
        lenient: cli.lenient,
        include_job_files: cli.include_job_files,
        partial_job_files,
        with_audiences: cli.with_audiences,
        compression,
    };

    let converter = Converter::new(options, cli.parallel);

    // Build (input, output) pairs
    let pairs: Vec<(PathBuf, PathBuf)> = cli
        .pdx_files
        .iter()
        .map(|pdx| {
            let out_dir = cli
                .output_dir
                .clone()
                .unwrap_or_else(|| {
                    pdx.parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .to_path_buf()
                });
            let stem = pdx
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let out = out_dir.join(format!("{}.mdd", stem));
            (pdx.clone(), out)
        })
        .collect();

    // Convert (parallel when multiple files)
    if pairs.len() == 1 {
        let (inp, out) = &pairs[0];
        println!("Processing {} ...", inp.display());
        match converter.convert(inp, out) {
            Ok(stats) => {
                println!(
                    "Done in {}ms  |  raw={} B  uncompressed={} B  mdd={} B",
                    stats.duration_ms, stats.raw_size, stats.uncompressed_size,
                    stats.compressed_size
                );
                println!("Output: {}", out.display());

                if cli.json {
                    let json_out = out.with_extension("json");
                    match odx_converter::parser::export_mdd_to_json(out, &json_out) {
                        Ok(()) => println!("JSON:   {}", json_out.display()),
                        Err(e) => eprintln!("Failed to export JSON for {}: {:#}", out.display(), e),
                    }
                }
            }
            Err(e) => {
                error!("{:#}", e);
                std::process::exit(1);
            }
        }
    } else {
        let mut had_errors = false;

        let results = converter.convert_all(&pairs);

        for (input, result) in results {
            match result {
                Ok(stats) => {
                    println!(
                        "OK  {}  ({}ms, mdd={} B)",
                        input.display(),
                        stats.duration_ms,
                        stats.compressed_size
                    );

                    if cli.json {
                        let out = pairs
                            .iter()
                            .find(|(inp, _)| inp == &input)
                            .map(|(_, out)| out.clone());
                        if let Some(out) = out {
                            let json_out = out.with_extension("json");
                            if let Err(e) = odx_converter::parser::export_mdd_to_json(&out, &json_out) {
                                eprintln!("Failed to export JSON for {}: {:#}", out.display(), e);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("ERR {}  {:#}", input.display(), e);
                    had_errors = true;
                }
            }
        }

        // Print totals
        if had_errors {
            eprintln!("One or more files failed to convert.");
            std::process::exit(1);
        }
    }
}
