// src/writer/chunk_builder.rs – Builds `Chunk` Protobuf messages and applies
// the compression plugin chain.
//
// Mirrors the Kotlin `ChunkBuilder` class: the core diagnostic description
// chunk is a FlatBuffers-encoded EcuData payload wrapped in a protobuf Chunk.
// Optional code file chunks are also built here.
// Compression is applied via the PluginChain selected by ConverterOptions.

use anyhow::Result;
use log::info;

use crate::collection::OdxCollectionGroup;
use crate::mdd::fileformat::{Chunk, chunk};
use crate::options::{ConverterOptions, CompressionConfig};
use crate::plugin::{PluginChain, lz4::Lz4Plugin, lzma::LzmaPlugin, zstd::ZstdPlugin};
use crate::writer::DatabaseWriter;

/// Builds typed `Chunk` messages for inclusion in an `MDDFile`.
pub struct ChunkBuilder {
    plugins: PluginChain,
}

impl ChunkBuilder {
    /// Build a ChunkBuilder with a plugin chain derived from `options.compression`.
    pub fn new(options: &ConverterOptions) -> Self {
        let plugins = match &options.compression {
            CompressionConfig::None => PluginChain::empty(),
            CompressionConfig::Zstd { level } => {
                let mut chain = PluginChain::empty();
                chain.add(Box::new(ZstdPlugin::new(*level)));
                chain
            }
            CompressionConfig::Lz4 => {
                let mut chain = PluginChain::empty();
                chain.add(Box::new(Lz4Plugin::new()));
                chain
            }
            CompressionConfig::Lzma => {
                let mut chain = PluginChain::empty();
                chain.add(Box::new(LzmaPlugin::new()));
                chain
            }
        };
        Self { plugins }
    }

    // ── Diagnostic description chunk ──────────────────────────────────────

    /// Serialise the entire ODX collection into a FlatBuffers blob, apply
    /// compression, and wrap in a DIAGNOSTIC_DESCRIPTION chunk.
    pub fn create_ecu_data_chunk(
        &self,
        odx: &OdxCollectionGroup,
        options: &ConverterOptions,
    ) -> Result<Chunk> {
        let writer = DatabaseWriter::new();
        let data = writer.create_ecu_data(odx, options)?;
        let uncompressed_size = data.len() as u64;

        info!(
            "Built FlatBuffers diagnostic description for '{}': {} bytes (uncompressed)",
            odx.ecu_name, uncompressed_size
        );

        let mut chunk = Chunk {
            r#type: chunk::DataType::DiagnosticDescription as i32,
            name: Some(odx.ecu_name.clone()),
            data: Some(data),
            uncompressed_size: Some(uncompressed_size),
            metadata: Default::default(),
            signatures: vec![],
            compression_algorithm: None,
            encryption: None,
            mime_type: None,
        };

        // Apply compression plugin chain
        self.plugins.process_chunk(&mut chunk)?;

        Ok(chunk)
    }

    // ── Job file chunks ───────────────────────────────────────────────────

    pub fn create_job_chunks(
        &self,
        input_files: &std::collections::HashMap<String, Vec<u8>>,
        odx: &OdxCollectionGroup,
        options: &ConverterOptions,
    ) -> Result<Vec<Chunk>> {
        if !options.include_job_files {
            return Ok(vec![]);
        }

        let referenced: std::collections::HashSet<String> = odx
            .collections
            .values()
            .flat_map(|c| c.single_ecu_job_store.iter())
            .flat_map(|j| j.prog_codes.iter())
            .filter_map(|pc| pc.code_file.clone())
            .collect();

        let mut chunks = Vec::new();
        for file_name in &referenced {
            if let Some(data) = input_files.get(file_name) {
                let size = data.len() as u64;
                info!("Including job file '{}' ({} bytes)", file_name, size);
                let mut ch = Chunk {
                    r#type: chunk::DataType::CodeFile as i32,
                    name: Some(file_name.clone()),
                    data: Some(data.clone()),
                    uncompressed_size: Some(size),
                    metadata: Default::default(),
                    signatures: vec![],
                    compression_algorithm: None,
                    encryption: None,
                    mime_type: None,
                };
                self.plugins.process_chunk(&mut ch)?;
                chunks.push(ch);
            } else {
                let msg = format!("Job file '{}' not found in PDX", file_name);
                if options.lenient {
                    log::warn!("{}", msg);
                } else {
                    anyhow::bail!(msg);
                }
            }
        }

        Ok(chunks)
    }

    // ── Partial job file chunks ───────────────────────────────────────────

    pub fn create_partial_chunks(
        &self,
        input_files: &std::collections::HashMap<String, Vec<u8>>,
        odx: &OdxCollectionGroup,
        options: &ConverterOptions,
    ) -> Result<Vec<Chunk>> {
        if options.partial_job_files.is_empty() {
            return Ok(vec![]);
        }

        let mut chunks = Vec::new();

        let referenced: std::collections::HashSet<String> = odx
            .collections
            .values()
            .flat_map(|c| c.single_ecu_job_store.iter())
            .flat_map(|j| j.prog_codes.iter())
            .filter_map(|pc| pc.code_file.clone())
            .collect();

        for file_name in &referenced {
            for pattern in &options.partial_job_files {
                let job_pattern = &pattern.job_file_pattern;
                let content_pattern = &pattern.include_pattern;
                let job_regex = regex_from_pattern(job_pattern);
                if !job_regex.is_match(file_name) {
                    continue;
                }

                let data = match input_files.get(file_name) {
                    Some(d) => d,
                    None => {
                        log::warn!("Partial job file '{}' not found in PDX", file_name);
                        continue;
                    }
                };

                let content_regex = regex_from_pattern(content_pattern);
                let matches = extract_matching_entries_from_zip(data, &content_regex)?;

                for (entry_name, entry_data) in matches {
                    let size = entry_data.len() as u64;
                    let chunk_name = format!("{}::{}", file_name, entry_name);
                    info!("Including partial '{}' ({} bytes)", chunk_name, size);
                    chunks.push(Chunk {
                        r#type: chunk::DataType::CodeFilePartial as i32,
                        name: Some(chunk_name),
                        data: Some(entry_data),
                        uncompressed_size: Some(size),
                        metadata: Default::default(),
                        signatures: vec![],
                        compression_algorithm: None,
                        encryption: None,
                        mime_type: None,
                    });
                }
            }
        }

        Ok(chunks)
    }
}

// ── helpers ───────────────────────────────────────────────────────────────

fn regex_from_pattern(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).unwrap_or_else(|_| {
        // Fallback: escape the pattern and match literally.
        regex::Regex::new(&regex::escape(pattern)).unwrap()
    })
}

fn extract_matching_entries_from_zip(
    data: &[u8],
    content_regex: &regex::Regex,
) -> Result<Vec<(String, Vec<u8>)>> {
    use std::io::Read;
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)?;
    let mut results = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_owned();
        if content_regex.is_match(&name) {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            results.push((name, buf));
        }
    }

    Ok(results)
}
