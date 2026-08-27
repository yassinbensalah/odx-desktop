// src/build_info.rs – Build-time metadata embedded by build.rs.

/// Short (7-char) git commit hash of the build, or "unknown" if unavailable
/// (e.g. building outside a git checkout). Mirrors the role of Kotlin's
/// `ManifestReader.commitHash.take(7)`.
pub const COMMIT_HASH_SHORT: &str = env!("ODX_CONVERTER_COMMIT_HASH");
