// src/plugin/lzma.rs – LZMA-Alone compression compatible with Kotlin.
//
// The Kotlin reference converter uses Apache Commons Compress, which is backed
// by the XZ for Java LZMA encoder.  The previous pure-Rust `lzma-rs` encoder
// produced valid streams, but its compression ratio was much worse for large
// FlatBuffers payloads.  This implementation uses liblzma through `xz2` and
// emits the same legacy .lzma / LZMA-Alone container format.

use std::io::Write;

use anyhow::{anyhow, Context, Result};
use xz2::{
    stream::{LzmaOptions, Stream},
    write::XzEncoder,
};

use crate::plugin::{CompressResult, CompressionPlugin};

/// Compression preset used by the reference-style LZMA encoder.
///
/// Preset 6 is liblzma's normal default and produces the expected 8 MiB
/// dictionary header (`5D 00 00 80 00`) for LZMA-Alone streams.
const LZMA_PRESET: u32 = 6;

/// Compresses chunk data using the legacy LZMA-Alone stream format.
pub struct LzmaPlugin {
    priority: i32,
}

impl LzmaPlugin {
    pub fn new() -> Self {
        Self { priority: 100 }
    }

    pub fn with_priority(priority: i32) -> Self {
        Self { priority }
    }
}

impl Default for LzmaPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressionPlugin for LzmaPlugin {
    fn identifier(&self) -> &str {
        "lzma"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn priority(&self) -> i32 {
        self.priority
    }

    fn compress(&self, data: &[u8]) -> Result<Option<CompressResult>> {
        let options = LzmaOptions::new_preset(LZMA_PRESET)
            .map_err(|error| anyhow!("Failed to create LZMA options: {error:?}"))?;
        let stream = Stream::new_lzma_encoder(&options)
            .map_err(|error| anyhow!("Failed to create LZMA-Alone encoder: {error:?}"))?;

        let mut encoder = XzEncoder::new_stream(Vec::new(), stream);
        encoder
            .write_all(data)
            .context("LZMA compression failed while writing input")?;
        let compressed = encoder
            .finish()
            .context("LZMA compression failed while finalizing stream")?;

        if compressed.len() < data.len() {
            Ok(Some(CompressResult {
                data: compressed,
                algorithm: "lzma".to_string(),
            }))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_lzma_alone_with_reference_dictionary_header() {
        let data = vec![b'A'; 128 * 1024];
        let result = LzmaPlugin::new()
            .compress(&data)
            .expect("compression should succeed")
            .expect("repetitive data should be compressed");

        assert_eq!(result.algorithm, "lzma");
        assert!(result.data.len() < data.len());
        assert_eq!(&result.data[..5], &[0x5D, 0x00, 0x00, 0x80, 0x00]);
    }
}
