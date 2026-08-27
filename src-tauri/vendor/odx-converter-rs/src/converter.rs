// src/converter.rs – Main conversion orchestration.
//
// `FileConverter::convert` opens a .pdx file (ZIP), extracts all .odx XML
// entries, parses them into per-file `OdxCollection`s, merges them into an
// `OdxCollectionGroup`, builds the FlatBuffers diagnostic description, and
// emits the MDD file.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use log::{debug, info, warn};
use zip::ZipArchive;

use crate::collection::{OdxCollection, OdxCollectionGroup, parse_odx_file};
use crate::options::ConverterOptions;
use crate::writer::{ChunkBuilder, MddWriter};

// ─── Conversion statistics ────────────────────────────────────────────────

/// Summary of one successful file conversion.
#[derive(Debug, Default)]
pub struct ConversionStats {
    /// Total raw ODX bytes read from the ZIP.
    pub raw_size: u64,
    /// Uncompressed size of the diagnostic description chunk.
    pub uncompressed_size: u64,
    /// Final on-disk size of the .mdd file (magic + protobuf).
    pub compressed_size: u64,
    /// Wall-clock duration of the conversion.
    pub duration_ms: u64,
}

// ─── Converter ────────────────────────────────────────────────────────────

pub struct FileConverter {
    options: ConverterOptions,
}

impl FileConverter {
    pub fn new(options: ConverterOptions) -> Self {
        Self { options }
    }

    /// Convert one `.pdx` file to a `.mdd` file.
    pub fn convert(&self, input_path: &Path, output_path: &Path) -> Result<ConversionStats> {
        let start = Instant::now();

        info!("Opening PDX archive '{}'", input_path.display());
        let file = std::fs::File::open(input_path)
            .with_context(|| format!("Cannot open '{}'", input_path.display()))?;

        let mut archive = ZipArchive::new(file)
            .with_context(|| format!("'{}' is not a valid ZIP/PDX archive", input_path.display()))?;

        let mut odx_texts: Vec<(String, String)> = Vec::new(); // (entry_name, xml_text)
        let mut raw_odx_bytes: u64 = 0;
        let mut extra_files: HashMap<String, Vec<u8>> = HashMap::new();

        // ── Read all entries ──────────────────────────────────────────────
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)
                .with_context(|| format!("Cannot read entry {} from PDX", i))?;

            if entry.is_dir() {
                continue;
            }

            let name = entry.name().to_owned();
            let entry_size = entry.size();

            let mut buf = Vec::with_capacity(entry_size as usize);
            entry.read_to_end(&mut buf)
                .with_context(|| format!("Cannot read entry '{}'", name))?;

            if name.to_ascii_lowercase().ends_with(".odx")
                || name.to_ascii_lowercase().ends_with(".odx-f")
                || name.to_ascii_lowercase().ends_with(".odx-c")
                || name.to_ascii_lowercase().ends_with(".odx-cs")
                || name.to_ascii_lowercase().ends_with(".odx-d")
                || name.to_ascii_lowercase().ends_with(".odx-e")
            {
                raw_odx_bytes += entry_size;
                let xml_text = String::from_utf8_lossy(&buf).into_owned();
                odx_texts.push((name.clone(), xml_text));
            }

            extra_files.insert(name, buf);
        }

        info!(
            "Parsed {} ODX entries ({} raw bytes)",
            odx_texts.len(),
            raw_odx_bytes
        );

        if odx_texts.is_empty() {
            anyhow::bail!("No ODX files found in '{}'", input_path.display());
        }

        // ── Parse XML into per-file collections ───────────────────────────
        let collections: Vec<OdxCollection> = odx_texts
            .iter()
            .map(|(name, xml)| {
                parse_odx_file(xml, name)
                    .with_context(|| format!("Failed to parse ODX file '{}'", name))
            })
            .collect::<Result<Vec<_>>>()?;

        // ── Build merged collection group ─────────────────────────────────
        let group = OdxCollectionGroup::new(collections, raw_odx_bytes);

        // Check requested audiences are defined
        if !self.options.with_audiences.is_empty() {
            let valid_sn: Vec<&str> = group
                .all_additional_audiences()
                .map(|a| a.short_name.as_str())
                .collect();
            for requested in &self.options.with_audiences {
                if !valid_sn.iter().any(|v| v.eq_ignore_ascii_case(requested)) {
                    warn!(
                        "Audience '{}' not defined in the diagnostic description. Valid: {:?}",
                        requested, valid_sn
                    );
                }
            }
        }

        // ── Build chunks ──────────────────────────────────────────────────
        let chunk_builder = ChunkBuilder::new(&self.options);

        let diag_chunk = chunk_builder
            .create_ecu_data_chunk(&group, &self.options)
            .context("Failed to build diagnostic description chunk")?;

        let uncompressed_size = diag_chunk.uncompressed_size.unwrap_or(0);

        let mut chunks = vec![diag_chunk];

        let job_chunks = chunk_builder
            .create_job_chunks(&extra_files, &group, &self.options)?;
        chunks.extend(job_chunks);

        let partial_chunks = chunk_builder
            .create_partial_chunks(&extra_files, &group, &self.options)?;
        chunks.extend(partial_chunks);

        // ── Build metadata ────────────────────────────────────────────────
        let mut metadata = HashMap::new();
        metadata.insert(
            "created".to_string(),
            crate::timestamp::current_reproducible_timestamp(),
        );
        metadata.insert(
            "source".to_string(),
            input_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        );
        metadata.insert(
            "options".to_string(),
            "{}".to_string(),
        );

        // ── Write MDD file ────────────────────────────────────────────────
        let compressed_size = MddWriter::write(
            &group.ecu_name,
            group.odx_revision.as_deref(),
            "2025-05-21",
            chunks,
            metadata,
            output_path,
        )
        .with_context(|| format!("Failed to write MDD file '{}'", output_path.display()))?;

        let duration_ms = start.elapsed().as_millis() as u64;
        info!(
            "Conversion finished in {}ms – raw={} B, uncompressed={} B, mdd={} B",
            duration_ms, raw_odx_bytes, uncompressed_size, compressed_size
        );

        Ok(ConversionStats {
            raw_size: raw_odx_bytes,
            uncompressed_size,
            compressed_size,
            duration_ms,
        })
    }
}

// ─── Parallel multi-file converter ───────────────────────────────────────

use rayon::prelude::*;

/// High-level converter that processes multiple PDX files (optionally in
/// parallel) and mirrors the Kotlin `Converter` CLI command.
pub struct Converter {
    options: ConverterOptions,
    parallel: usize,
}

impl Converter {
    pub fn new(options: ConverterOptions, parallel: usize) -> Self {
        Self { options, parallel }
    }

    /// Convert one PDX → MDD.
    pub fn convert(&self, input_path: &Path, output_path: &Path) -> Result<ConversionStats> {
        FileConverter::new(self.options.clone()).convert(input_path, output_path)
    }

    /// Convert multiple PDX files using up to `self.parallel` threads.
    pub fn convert_all<'a>(
        &self,
        pairs: &'a [(std::path::PathBuf, std::path::PathBuf)],
    ) -> Vec<(&'a Path, Result<ConversionStats>)> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.parallel)
            .build()
            .unwrap();

        pool.install(|| {
            pairs
                .par_iter()
                .map(|(inp, out)| {
                    (inp.as_path(), self.convert(inp, out))
                })
                .collect()
        })
    }
}
