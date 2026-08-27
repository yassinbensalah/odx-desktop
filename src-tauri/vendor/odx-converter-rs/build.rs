// build.rs – compiles the Protobuf definitions used for the MDD container
// format, and captures a short git commit hash for the "converter" metadata
// field, mirroring Kotlin's `ManifestReader.commitHash.take(7)`.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    prost_build::compile_protos(&["proto/file_format.proto"], &["proto/"])?;
    println!("cargo:rerun-if-changed=proto/file_format.proto");

    let commit_hash = std::process::Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=ODX_CONVERTER_COMMIT_HASH={commit_hash}");
    println!("cargo:rerun-if-changed=.git/HEAD");

    Ok(())
}
