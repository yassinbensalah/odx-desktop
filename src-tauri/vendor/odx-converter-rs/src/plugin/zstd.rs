// src/plugin/zstd.rs – Zstandard (zstd) compression plugin.
//
// Uses the `zstd` crate to compress chunk payloads.  Only applies if the
// compressed output is strictly smaller than the original.

use anyhow::Result;
use zstd::stream::encode_all;

use crate::plugin::{CompressResult, CompressionPlugin};

/// Compresses chunk data using Zstandard.
///
/// Level 1 = fastest, level 22 = best compression.
/// Recommended level 3 for a good speed/ratio trade-off.
pub struct ZstdPlugin {
    level: i32,
    priority: i32,
}

impl ZstdPlugin {
    /// Create a ZstdPlugin with the given compression level and default priority.
    pub fn new(level: i32) -> Self {
        Self { level, priority: 100 }
    }

    /// Create a ZstdPlugin with an explicit processing priority.
    pub fn with_priority(level: i32, priority: i32) -> Self {
        Self { level, priority }
    }
}

impl CompressionPlugin for ZstdPlugin {
    fn identifier(&self) -> &str {
        "zstd"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn priority(&self) -> i32 {
        self.priority
    }

    fn compress(&self, data: &[u8]) -> Result<Option<CompressResult>> {
        let compressed = encode_all(data, self.level)?;

        if compressed.len() < data.len() {
            Ok(Some(CompressResult {
                data: compressed,
                algorithm: format!("zstd;level={}", self.level),
            }))
        } else {
            Ok(None)
        }
    }
}
