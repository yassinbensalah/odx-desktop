// src/options.rs – Converter options (mirrors Kotlin `ConverterOptions`).
//
// Serialised to JSON and stamped into the MDD file's "options" metadata
// field. To match Kotlin's `kotlinx.serialization` output byte-for-byte
// for equivalent settings:
//   - field names are camelCase ("includeJobFiles", not "include_job_files")
//   - fields at their default value are OMITTED entirely (kotlinx.serialization's
//     default `encodeDefaults = false` behaviour) rather than always written
//     — e.g. an all-defaults ConverterOptions serialises to "{}", exactly
//     like Kotlin's, instead of spelling out every key with its default value.
//   - each partial-job-files entry is an object {"jobFilePattern": ...,
//     "includePattern": ...}, not a bare 2-element array/tuple.
//
// `compression` is a genuine addition beyond what Kotlin's ConverterOptions
// has (Kotlin's compression is a fixed built-in plugin, not a user choice);
// it's kept here as a real capability, but still follows the same
// omit-if-default convention as everything else.

use serde::{Deserialize, Serialize};

fn is_false(b: &bool) -> bool {
    !*b
}

/// One `--partial-job-files <job-pattern> <content-pattern>` entry, shaped
/// to serialise identically to Kotlin's `PartialFilePattern` data class.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialFilePattern {
    pub job_file_pattern: String,
    pub include_pattern: String,
}

impl From<(String, String)> for PartialFilePattern {
    fn from((job_file_pattern, include_pattern): (String, String)) -> Self {
        Self { job_file_pattern, include_pattern }
    }
}

/// Which compression algorithm to apply to the diagnostic-description chunk.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompressionConfig {
    /// No compression (raw FlatBuffers bytes).
    #[default]
    None,
    /// Zstandard compression.  `level` 1–22 (default 3).
    Zstd {
        #[serde(default = "default_zstd_level")]
        level: i32,
    },
    /// LZ4 compression (faster, lower ratio than zstd).
    Lz4,
    /// LZMA compression (alone format) – matches the original Kotlin
    /// odx-converter reference implementation's output format.
    Lzma,
}

fn default_zstd_level() -> i32 { 3 }

fn is_default_compression(c: &CompressionConfig) -> bool {
    *c == CompressionConfig::None
}

/// Options that control conversion behaviour.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverterOptions {
    /// Continue on resolution errors instead of aborting.
    #[serde(skip_serializing_if = "is_false")]
    pub lenient: bool,

    /// Include referenced job / library code files as CODE_FILE chunks.
    #[serde(skip_serializing_if = "is_false")]
    pub include_job_files: bool,

    /// Patterns for partial job file extraction.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub partial_job_files: Vec<PartialFilePattern>,

    /// Audience short-names to include.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub with_audiences: Vec<String>,

    /// Compression to apply to the diagnostic-description chunk.
    #[serde(skip_serializing_if = "is_default_compression")]
    pub compression: CompressionConfig,
}
