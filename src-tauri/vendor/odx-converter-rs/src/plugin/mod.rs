// src/plugin/mod.rs – Compression plugin system.
//
// Mirrors the Kotlin `ConverterPlugin` SPI from eclipse-opensovd/odx-converter.
// Each plugin receives raw chunk bytes and may replace them with compressed
// bytes, updating the chunk's compression_algorithm field.
//
// Plugins are applied in priority order (lowest priority value = runs first).

pub mod lz4;
pub mod lzma;
pub mod zstd;

use anyhow::Result;
use log::{debug, info};

use crate::mdd::fileformat::{Chunk, Signature};

// ─── Plugin trait ─────────────────────────────────────────────────────────

/// Result returned when a plugin successfully compresses data.
pub struct CompressResult {
    pub data: Vec<u8>,
    /// Compression algorithm name stored in the MDD Chunk (e.g. "zstd", "lz4").
    pub algorithm: String,
}

/// Compression / processing plugin for MDD chunks.
///
/// Implement this trait and add an instance to [`PluginChain`] to hook into
/// the chunk processing pipeline.
pub trait CompressionPlugin: Send + Sync {
    /// Human-readable identifier (e.g. "zstd", "lz4").
    fn identifier(&self) -> &str;
    /// Semver-like version string for metadata.
    fn version(&self) -> &str;
    /// Processing priority – lower value runs first.
    fn priority(&self) -> i32 { 100 }

    /// Attempt to compress `data`.
    ///
    /// Return `Ok(Some(result))` if compression is beneficial (result.data is
    /// smaller than input).  Return `Ok(None)` to skip (leave data unchanged).
    fn compress(&self, data: &[u8]) -> Result<Option<CompressResult>>;
}

// ─── Plugin chain ──────────────────────────────────────────────────────────

/// An ordered list of [`CompressionPlugin`]s applied sequentially to every
/// chunk.  The FIRST plugin that returns `Some(result)` wins; subsequent
/// plugins are skipped for that chunk.
pub struct PluginChain {
    plugins: Vec<Box<dyn CompressionPlugin>>,
}

impl PluginChain {
    pub fn empty() -> Self {
        Self { plugins: Vec::new() }
    }

    /// Create a chain with the default zstd compression plugin (level 3).
    pub fn default_zstd() -> Self {
        Self {
            plugins: vec![Box::new(zstd::ZstdPlugin::new(3))],
        }
    }

    /// Create a chain with LZ4 compression (faster, lower ratio).
    pub fn default_lz4() -> Self {
        Self {
            plugins: vec![Box::new(lz4::Lz4Plugin::new())],
        }
    }

    /// Create a chain with zstd + LZ4 (zstd tried first).
    pub fn zstd_then_lz4(level: i32) -> Self {
        let mut plugins: Vec<Box<dyn CompressionPlugin>> = vec![
            Box::new(zstd::ZstdPlugin::with_priority(level, 10)),
            Box::new(lz4::Lz4Plugin::with_priority(20)),
        ];
        plugins.sort_by_key(|p| p.priority());
        Self { plugins }
    }

    pub fn add(&mut self, plugin: Box<dyn CompressionPlugin>) {
        self.plugins.push(plugin);
        self.plugins.sort_by_key(|p| p.priority());
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    // ── Apply to a chunk ───────────────────────────────────────────────────

    /// Apply the first successful plugin to `chunk`, updating its `data`,
    /// `compression_algorithm`, and `uncompressed_size` fields.
    ///
    /// The `original_data` is the raw bytes BEFORE any plugin has run (needed
    /// because `chunk.data` may already be set to the FlatBuffers payload).
    pub fn process_chunk(&self, chunk: &mut Chunk) -> Result<()> {
        if self.plugins.is_empty() {
            return Ok(());
        }

        let original = match &chunk.data {
            Some(d) if !d.is_empty() => d.clone(),
            _ => return Ok(()),
        };

        for plugin in &self.plugins {
            debug!(
                "Trying plugin '{}' on chunk '{}' ({} bytes)",
                plugin.identifier(),
                chunk.name.as_deref().unwrap_or("<unnamed>"),
                original.len()
            );

            match plugin.compress(&original)? {
                Some(result) => {
                    let ratio = result.data.len() as f64 / original.len() as f64;
                    info!(
                        "Plugin '{}' compressed chunk '{}': {} → {} bytes ({:.1}%)",
                        plugin.identifier(),
                        chunk.name.as_deref().unwrap_or("<unnamed>"),
                        original.len(),
                        result.data.len(),
                        ratio * 100.0
                    );
                    chunk.uncompressed_size = Some(original.len() as u64);
                    chunk.compression_algorithm = Some(result.algorithm);
                    chunk.data = Some(result.data);

                    // Mirrors Kotlin's default CompressionPlugin: sign the
                    // ORIGINAL (uncompressed) bytes with SHA-512, so
                    // integrity can be verified after decompression
                    // regardless of which compression algorithm was used.
                    use sha2::{Digest, Sha512};
                    let mut hasher = Sha512::new();
                    hasher.update(&original);
                    let digest = hasher.finalize();
                    chunk.signatures.push(Signature {
                        algorithm: "sha512_uncompressed".to_string(),
                        key_identifier: None,
                        metadata: Default::default(),
                        signature: digest.to_vec(),
                        certificates: Vec::new(),
                    });

                    return Ok(());
                }
                None => {
                    debug!(
                        "Plugin '{}' skipped chunk '{}' (no size benefit)",
                        plugin.identifier(),
                        chunk.name.as_deref().unwrap_or("<unnamed>")
                    );
                }
            }
        }

        Ok(())
    }
}
