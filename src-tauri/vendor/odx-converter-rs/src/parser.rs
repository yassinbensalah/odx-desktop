use std::collections::HashMap;
use std::fs::File;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use flatbuffers::{ForwardsUOffset, Table, Vector};
use prost::Message;
use serde::Serialize;

use crate::mdd::fileformat::MddFile;
use crate::writer::mdd_writer::FILE_MAGIC;

#[derive(Serialize)]
struct JsonChunk {
    #[serde(rename = "type")]
    type_name: String,
    name: Option<String>,
    compression_algorithm: Option<String>,
    uncompressed_size: Option<u64>,
    mime_type: Option<String>,
    has_data: bool,
    data_size: usize,
    signature_count: usize,
    metadata: HashMap<String, String>,
}

#[derive(Serialize)]
struct JsonParentRef {
    ref_type: String,
    ref_name: Option<String>,
    not_inherited_diag_comm_short_names: Vec<String>,
    /// The target's OWN parent_refs, walked recursively (Protocol, Variant,
    /// and FunctionalGroup targets can all have further parent_refs of
    /// their own — e.g. DIAGNOSTIC_SERVICES -> ISO_14229_CAN -> ISO_14229_BASE).
    /// EcuSharedData targets never have parent_refs (not part of that table).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    parent_refs: Vec<JsonParentRef>,
}

#[derive(Serialize)]
struct JsonFunctionalGroup {
    short_name: String,
    long_name: Option<String>,
    parent_refs: Vec<JsonParentRef>,
}

#[derive(Serialize)]
struct JsonDiagnosticPayload {
    version: Option<String>,
    ecu_name: Option<String>,
    variants: Vec<String>,
    functional_groups: Vec<JsonFunctionalGroup>,
}

#[derive(Serialize)]
struct JsonMddFile {
    version: String,
    ecu_name: String,
    revision: Option<String>,
    metadata: HashMap<String, String>,
    chunk_count: usize,
    chunks: Vec<JsonChunk>,
    diagnostic_payload: JsonDiagnosticPayload,
}

fn empty_diagnostic_payload() -> JsonDiagnosticPayload {
    JsonDiagnosticPayload {
        version: None,
        ecu_name: None,
        variants: vec![],
        functional_groups: vec![],
    }
}

fn parse_diagnostic_payload(data: &[u8]) -> JsonDiagnosticPayload {
    let parse = || -> Option<JsonDiagnosticPayload> {
        let root = unsafe { flatbuffers::root_unchecked::<Table>(data) };

        let version = read_string(&root, 4);
        let ecu_name = read_string(&root, 6);

        let variants = read_table_vector(&root, 14)
            .into_iter()
            .filter_map(|variant| read_diag_layer_short_name(&variant, 4))
            .collect::<Vec<_>>();

        let functional_groups = read_table_vector(&root, 16)
            .into_iter()
            .filter_map(|group| {
                let diag_layer = read_table_field(&group, 4)?;
                // `diag_layer` IS the DiagLayer table already (one hop from
                // `group` above) — read its fields directly, don't hop again.
                let short_name = read_string(&diag_layer, 4)?;
                let long_name = read_table_field(&diag_layer, 6)
                    .and_then(|ln| read_string(&ln, 4)); // LongName.value
                let parent_refs = read_table_vector(&group, 6)
                    .into_iter()
                    .filter_map(|pr| parse_parent_ref(pr, 0))
                    .collect();
                Some(JsonFunctionalGroup {
                    short_name,
                    long_name,
                    parent_refs,
                })
            })
            .collect::<Vec<_>>();

        Some(JsonDiagnosticPayload {
            version,
            ecu_name,
            variants,
            functional_groups,
        })
    };

    match panic::catch_unwind(AssertUnwindSafe(parse)) {
        Ok(Some(payload)) => payload,
        Ok(None) | Err(_) => empty_diagnostic_payload(),
    }
}

fn read_string(table: &Table<'_>, slot: u16) -> Option<String> {
    panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        table.get::<ForwardsUOffset<&str>>(slot, None).map(|value| value.to_string())
    }))
    .ok()
    .flatten()
}

/// Reads a `[string]` field — a vector of raw string offsets — as opposed
/// to `read_table_vector`, which is for `[SomeTable]` fields (a vector of
/// sub-table offsets). Using the wrong one of these two on a given slot
/// misinterprets the raw string bytes as a nested table's vtable/fields,
/// which reads garbage offsets and can panic.
fn read_string_vector(table: &Table<'_>, slot: u16) -> Vec<String> {
    panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        table
            .get::<ForwardsUOffset<Vector<'_, ForwardsUOffset<&str>>>>(slot, None)
            .map(|vector| vector.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }))
    .unwrap_or_default()
}

fn read_table_field<'a>(table: &Table<'a>, slot: u16) -> Option<Table<'a>> {
    panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        table.get::<ForwardsUOffset<Table<'a>>>(slot, None)
    }))
    .ok()
    .flatten()
}

fn read_table_vector<'a>(table: &Table<'a>, slot: u16) -> Vec<Table<'a>> {
    panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        table
            .get::<ForwardsUOffset<Vector<'a, ForwardsUOffset<Table<'a>>>>>(slot, None)
            .map(|vector| {
                let mut items = Vec::new();
                for item in vector.iter() {
                    items.push(item);
                }
                items
            })
            .unwrap_or_default()
    }))
    .unwrap_or_default()
}

fn read_diag_layer_short_name(table: &Table<'_>, slot: u16) -> Option<String> {
    let diag_layer = read_table_field(table, slot)?;
    read_string(&diag_layer, 4)
}

/// Maximum recursion depth when walking a chain of parent_refs
/// (DIAGNOSTIC_SERVICES -> Protocol -> Protocol -> ...). This is just a
/// safety cap against a malformed/cyclic file; real ODX inheritance chains
/// are never anywhere near this deep.
const MAX_PARENT_REF_DEPTH: u32 = 32;

fn parse_parent_ref(parent_ref: Table<'_>, depth: u32) -> Option<JsonParentRef> {
    let ref_type_byte = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        parent_ref.get::<u8>(4, None).unwrap_or_default()
    }))
    .ok()?;

    let ref_type = match ref_type_byte {
        1 => "variant".to_string(),
        2 => "protocol".to_string(),
        3 => "functional_group".to_string(),
        5 => "ecu_shared_data".to_string(),
        other => format!("unknown({other})"),
    };

    let target = read_table_field(&parent_ref, 6)?;
    let ref_name = read_diag_layer_short_name(&target, 4);
    let not_inherited_diag_comm_short_names = read_string_vector(&parent_ref, 8);

    // Recurse into the target's OWN parent_refs, if that table type has one
    // and we're not too deep. Slot differs per target table:
    //   Protocol.parent_refs / Variant.parent_refs → slot 10 (field index 3)
    //   FunctionalGroup.parent_refs                → slot 6  (field index 1)
    //   EcuSharedData                               → has no parent_refs field
    let parent_refs = if depth < MAX_PARENT_REF_DEPTH {
        let nested_slot = match ref_type_byte {
            1 | 2 => Some(10u16), // variant, protocol
            3 => Some(6u16),      // functional_group
            _ => None,            // ecu_shared_data, unknown
        };
        nested_slot
            .map(|slot| {
                read_table_vector(&target, slot)
                    .into_iter()
                    .filter_map(|pr| parse_parent_ref(pr, depth + 1))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Some(JsonParentRef {
        ref_type,
        ref_name,
        not_inherited_diag_comm_short_names,
        parent_refs,
    })
}

fn looks_like_flatbuffer(data: &[u8]) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = unsafe { flatbuffers::root_unchecked::<Table>(data) };
        true
    }))
    .unwrap_or(false)
}

fn try_decode_bytes(data: &[u8], algorithm: &str) -> Result<Option<Vec<u8>>> {
    match algorithm {
        "raw" => Ok(Some(data.to_vec())),
        algo if algo.starts_with("lzma") => {
            let mut reader = std::io::Cursor::new(data);
            let mut out = Vec::new();
            lzma_rs::lzma_decompress(&mut reader, &mut out)
                .map_err(|e| anyhow!("Failed to decompress LZMA payload: {e:?}"))?;
            Ok(Some(out))
        }
        algo if algo.starts_with("lz4") => {
            let decoded = lz4_flex::decompress_size_prepended(data)
                .map_err(|e| anyhow!("Failed to decompress LZ4 payload: {e:?}"))?;
            Ok(Some(decoded))
        }
        algo if algo.starts_with("zstd") => {
            let decoded = zstd::decode_all(std::io::Cursor::new(data))
                .map_err(|e| anyhow!("Failed to decompress Zstd payload: {e:?}"))?;
            Ok(Some(decoded))
        }
        _ => Ok(None),
    }
}

fn decode_chunk_payload(chunk: &crate::mdd::fileformat::Chunk) -> Result<Vec<u8>> {
    let Some(data) = chunk.data.as_ref() else {
        return Ok(Vec::new());
    };

    let algo = chunk.compression_algorithm.as_deref().unwrap_or_default();
    let algorithms = if algo.is_empty() {
        vec!["raw", "lzma", "lz4", "zstd"]
    } else {
        vec![algo, "raw"]
    };

    for algorithm in algorithms {
        match try_decode_bytes(data, algorithm) {
            Ok(Some(decoded)) if looks_like_flatbuffer(&decoded) => return Ok(decoded),
            Ok(Some(decoded)) => {
                if algorithm == "raw" {
                    return Ok(decoded);
                }
            }
            Ok(None) => {}
            Err(_) => {}
        }
    }

    Ok(data.clone())
}

pub fn export_mdd_to_json(input_path: &Path, output_path: &Path) -> Result<()> {
    let bytes = std::fs::read(input_path)
        .with_context(|| format!("Failed to read MDD file '{}'", input_path.display()))?;

    let payload = if bytes.starts_with(FILE_MAGIC) {
        bytes[FILE_MAGIC.len()..].to_vec()
    } else {
        bytes
    };

    let mdd_file = MddFile::decode(payload.as_slice())
        .with_context(|| format!("Failed to decode MDD file '{}'", input_path.display()))?;

    let diagnostic_payload = mdd_file.chunks.iter().find_map(|chunk| {
        if chunk.r#type == 0 {
            chunk.data.as_ref().map(|_| {
                let payload_bytes = decode_chunk_payload(chunk).unwrap_or_default();
                let mut payload = parse_diagnostic_payload(&payload_bytes);
                if payload.version.is_none() {
                    payload.version = Some("2025-05-10".to_string());
                }
                if payload.ecu_name.is_none() {
                    payload.ecu_name = Some(mdd_file.ecu_name.clone());
                }
                payload
            })
        } else {
            None
        }
    }).unwrap_or_else(|| JsonDiagnosticPayload {
        version: None,
        ecu_name: None,
        variants: vec![],
        functional_groups: vec![],
    });

    let json = JsonMddFile {
        version: mdd_file.version,
        ecu_name: mdd_file.ecu_name,
        revision: mdd_file.revision,
        metadata: mdd_file.metadata,
        chunk_count: mdd_file.chunks.len(),
        chunks: mdd_file
            .chunks
            .into_iter()
            .map(|chunk| JsonChunk {
                type_name: match chunk.r#type {
                    0 => "DIAGNOSTIC_DESCRIPTION".to_string(),
                    1 => "CODE_FILE".to_string(),
                    2 => "CODE_FILE_PARTIAL".to_string(),
                    3 => "EMBEDDED_FILE".to_string(),
                    1024 => "VENDOR_SPECIFIC".to_string(),
                    other => format!("UNKNOWN({other})"),
                },
                name: chunk.name,
                compression_algorithm: chunk.compression_algorithm,
                uncompressed_size: chunk.uncompressed_size,
                mime_type: chunk.mime_type,
                has_data: chunk.data.is_some(),
                data_size: chunk.data.as_ref().map_or(0, |data| data.len()),
                signature_count: chunk.signatures.len(),
                metadata: chunk.metadata,
            })
            .collect(),
        diagnostic_payload,
    };

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create output directory '{}'", parent.display()))?;
        }
    }

    let file = File::create(output_path)
        .with_context(|| format!("Failed to create JSON output '{}'", output_path.display()))?;
    serde_json::to_writer_pretty(file, &json)
        .with_context(|| format!("Failed to serialize JSON for '{}'", output_path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdd::fileformat::Chunk;
    use crate::writer::MddWriter;
    use std::fs;

    #[test]
    fn parses_parent_refs_from_flatbuffer_payload() {
        let bytes = vec![
            0x08, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];

        let payload = parse_diagnostic_payload(&bytes);
        assert!(payload.functional_groups.is_empty());
    }

    #[test]
    fn inspects_flatbuffer_payload_slots() {
        let root = std::env::current_dir().unwrap();
        let input_path = root.join("kotlin-version.mdd");
        let bytes = std::fs::read(&input_path).unwrap();
        let payload = if bytes.starts_with(FILE_MAGIC) {
            bytes[FILE_MAGIC.len()..].to_vec()
        } else {
            bytes
        };

        let mdd_file = crate::mdd::fileformat::MddFile::decode(payload.as_slice()).unwrap();
        let chunk = mdd_file.chunks.iter().find(|chunk| chunk.r#type == 0).unwrap();
        let payload_bytes = decode_chunk_payload(chunk).unwrap();
        let root_table = unsafe { flatbuffers::root_unchecked::<Table>(&payload_bytes) };
        let vtable = root_table.vtable();
        println!("num_fields = {}", vtable.num_fields());
        println!("vtable bytes = {:?}", vtable.as_bytes());

        for slot in [4, 6, 8, 10, 12, 14, 16, 18] {
            let string_try = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
                root_table.get::<ForwardsUOffset<&str>>(slot, None)
            }));
            println!("slot {slot} string -> {:?}", string_try.ok().flatten());

            let vector_try = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
                root_table.get::<ForwardsUOffset<Vector<'_, ForwardsUOffset<Table<'_>>>>>(slot, None)
            }));
            let vector = vector_try.ok().flatten();
            println!("slot {slot} vector -> {:?}", vector.as_ref().map(|v| v.len()));
        }
    }

    #[test]
    fn exports_basic_mdd_file_to_json() {
        let temp_dir = std::env::temp_dir().join(format!("odx_mdd_json_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let input_path = temp_dir.join("sample.mdd");
        let output_path = temp_dir.join("sample.json");

        let written = MddWriter::write(
            "TestECU",
            Some("r1"),
            "0.1",
            vec![Chunk {
                r#type: 0,
                name: Some("diag.bin".to_string()),
                metadata: Default::default(),
                signatures: vec![],
                compression_algorithm: None,
                uncompressed_size: None,
                encryption: None,
                mime_type: Some("application/octet-stream".to_string()),
                data: Some(vec![1, 2, 3]),
            }],
            Default::default(),
            &input_path,
        )
        .unwrap();
        assert!(written > 0);

        export_mdd_to_json(&input_path, &output_path).unwrap();

        let json = fs::read_to_string(&output_path).unwrap();
        assert!(json.contains("TestECU"));
        assert!(json.contains("diag.bin"));
        assert!(json.contains("\"has_data\": true"));
        assert!(json.contains("\"signature_count\": 0"));
        assert!(json.contains("\"functional_groups\""));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
