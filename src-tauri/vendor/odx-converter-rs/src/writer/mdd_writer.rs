// src/writer/mdd_writer.rs – Assembles an MDDFile protobuf message and writes
// it to disk.
//
// The MDD file format:
//   20 bytes magic → "MDD version 0      \0" (ASCII, space-padded, NUL-terminated)
//   remainder      → protobuf-serialised MDDFile
//
// Must match the Kotlin implementation's `FILE_MAGIC` constant exactly,
// byte for byte (database/src/main/kotlin/Constants.kt):
//   val FILE_MAGIC = "MDD version 0      \u0000".toByteArray(Charsets.US_ASCII)

use std::io::{BufWriter, Write};
use std::path::Path;
use anyhow::Result;
use prost::Message;
use log::info;

use crate::mdd::fileformat::{Chunk, MddFile};

/// Magic bytes prepended to every MDD file (byte-for-byte identical to the
/// Kotlin implementation's `FILE_MAGIC` in Constants.kt).
pub const FILE_MAGIC: &[u8; 20] = b"MDD version 0      \0";

/// Wraps a list of `Chunk`s in an `MddFile` and serialises it.
pub struct MddWriter;

impl MddWriter {
    pub fn write(
        ecu_name: &str,
        revision: Option<&str>,
        version: &str,
        chunks: Vec<Chunk>,
        metadata: std::collections::HashMap<String, String>,
        output_path: &Path,
    ) -> Result<u64> {
        let mdd_file = MddFile {
            version: version.to_string(),
            ecu_name: ecu_name.to_string(),
            revision: revision.map(|r| r.to_string()),
            chunks,
            metadata,
            feature_flags: vec![],
            chunks_signature: None,
        };

        let serialised = mdd_file.encode_to_vec();
        let output_size = (FILE_MAGIC.len() + serialised.len()) as u64;

        let file = std::fs::File::create(output_path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(FILE_MAGIC)?;
        writer.write_all(&serialised)?;
        writer.flush()?;

        info!(
            "Wrote MDD file '{}': {} bytes",
            output_path.display(),
            output_size
        );

        Ok(output_size)
    }
}
