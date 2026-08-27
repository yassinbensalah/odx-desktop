// src/plugin/lz4.rs – LZ4 compression plugin.
//
// Uses `lz4_flex` for fast, low-latency compression.  The compressed output
// includes a 4-byte prepended original-size header, which is compatible with
// lz4_flex's `decompress_size_prepended` decompressor.

use anyhow::Result;
use lz4_flex::compress_prepend_size;

use crate::plugin::{CompressResult, CompressionPlugin};

/// Compresses chunk data using LZ4.
///
/// LZ4 prioritises speed over compression ratio.  Suitable for data that
/// needs fast decompression at runtime on the embedded CDA.
pub struct Lz4Plugin {
    priority: i32,
}

impl Lz4Plugin {
    pub fn new() -> Self {
        Self { priority: 100 }
    }

    pub fn with_priority(priority: i32) -> Self {
        Self { priority }
    }
}

impl CompressionPlugin for Lz4Plugin {
    fn identifier(&self) -> &str {
        "lz4"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn priority(&self) -> i32 {
        self.priority
    }

    fn compress(&self, data: &[u8]) -> Result<Option<CompressResult>> {
        let compressed = compress_prepend_size(data);

        if compressed.len() < data.len() {
            Ok(Some(CompressResult {
                data: compressed,
                // Use a standard algorithm name so the CDA knows how to decompress.
                algorithm: "lz4;size-prepended".to_string(),
            }))
        } else {
            Ok(None)
        }
    }
}
