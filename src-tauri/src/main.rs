// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use std::process::Command;
use odx_converter::{CompressionConfig, Converter, ConverterOptions};
use odx_converter::collection::{parse_odx_file, OdxCollectionGroup};
use odx_converter::writer::{ChunkBuilder, MddWriter};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::lookup_host;
use tokio::time::{timeout, Duration};
use doip_rw::message::UdsBuffer;
use doip_rw_tokio::{send_uds, DoIpTcpConnection, DoIpTcpMessage, Timings};

fn clean_path(raw: &str) -> String {
    raw.trim().trim_matches('"').trim_matches('\'').trim().to_string()
}

#[tauri::command]
async fn validate_pdx_file(
    app: tauri::AppHandle,
    pdx_file_path: String,
) -> Result<String, String> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        let _ = pdx_file_path;
        return Err("The bundled PDX validator is currently available only on Windows.".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        let pdx_file_path = clean_path(&pdx_file_path);
        if pdx_file_path.is_empty() {
            return Err("No PDX file was selected.".to_string());
        }
        if !Path::new(&pdx_file_path).exists() {
            return Err(format!("PDX file not found: {pdx_file_path}"));
        }

        // In a packaged application the validator is copied to Tauri's resource
        // directory. During `tauri dev`, fall back to the executable next to
        // Cargo.toml (src-tauri/pdx_validator.exe).
        #[cfg(target_os = "windows")]
        let validator_name = "pdx_validator.exe";
        #[cfg(not(target_os = "windows"))]
        let validator_name = "pdx_validator";

        let bundled_validator = app
            .path()
            .resource_dir()
            .ok()
            .map(|dir| dir.join(validator_name));

        let dev_validator = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(validator_name);

        let exe_path = bundled_validator
            .filter(|path| path.exists())
            .or_else(|| dev_validator.exists().then_some(dev_validator))
            .ok_or_else(|| {
                format!("{validator_name} was not found. On Windows use pdx_validator.exe. On Linux provide a native Linux pdx_validator binary; the Windows executable cannot be used natively.")
                    .to_string()
            })?;

        // Use a fresh extraction directory for each validation so files from a
        // previous PDX cannot affect the current result.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| format!("Could not create validation timestamp: {e}"))?
            .as_millis();
        let output_folder = std::env::temp_dir()
            .join(format!("odx_desktop_pdx_validation_{}_{}", std::process::id(), stamp));

        fs::create_dir_all(&output_folder)
            .map_err(|e| format!("Failed to create validation folder: {e}"))?;

        let exe_for_task = exe_path.clone();
        let pdx_for_task = pdx_file_path.clone();
        let output_for_task = output_folder.clone();

        let output = tauri::async_runtime::spawn_blocking(move || {
            Command::new(&exe_for_task)
                .arg(&pdx_for_task)
                .arg(&output_for_task)
                .output()
        })
        .await
        .map_err(|e| format!("Validator task failed: {e}"))?
        .map_err(|e| format!("Failed to start validator '{}': {e}", exe_path.display()))?;

        // Best-effort cleanup. Validation result must not depend on whether
        // Windows allows immediate deletion of the extracted temporary files.
        let _ = fs::remove_dir_all(&output_folder);

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if output.status.success() {
            if stdout.is_empty() {
                Ok("PDX file is valid.".to_string())
            } else {
                Ok(stdout)
            }
        } else {
            let message = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("PDX validation failed with exit code {:?}.", output.status.code())
            };
            Err(message)
        }
    }
}


#[derive(Serialize, Deserialize, Default)]
struct DesktopPreferences {
    project_output_dir: Option<String>,
}

#[derive(Serialize)]
struct ProjectOutputFolderInfo {
    path: String,
    is_default: bool,
}

fn desktop_preferences_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir()
        .map_err(|e| format!("Could not resolve app config directory: {e}"))?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create app config directory '{}': {e}", dir.display()))?;
    Ok(dir.join("desktop_preferences.json"))
}

fn load_desktop_preferences(app: &tauri::AppHandle) -> Result<DesktopPreferences, String> {
    let path = desktop_preferences_path(app)?;
    if !path.exists() {
        return Ok(DesktopPreferences::default());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("Could not read desktop preferences '{}': {e}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|e| format!("Could not parse desktop preferences '{}': {e}", path.display()))
}

fn save_desktop_preferences(app: &tauri::AppHandle, prefs: &DesktopPreferences) -> Result<(), String> {
    let path = desktop_preferences_path(app)?;
    let raw = serde_json::to_string_pretty(prefs)
        .map_err(|e| format!("Could not serialize desktop preferences: {e}"))?;
    fs::write(&path, raw)
        .map_err(|e| format!("Could not save desktop preferences '{}': {e}", path.display()))
}

fn default_project_output_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(diagnostic_library_files_dir(app)?.join("generated_mdd"))
}

fn project_output_dir(app: &tauri::AppHandle) -> Result<(PathBuf, bool), String> {
    let prefs = load_desktop_preferences(app)?;
    if let Some(raw) = prefs.project_output_dir {
        let clean = clean_path(&raw);
        if !clean.is_empty() {
            let path = PathBuf::from(clean);
            fs::create_dir_all(&path)
                .map_err(|e| format!("Could not create project output folder '{}': {e}", path.display()))?;
            return Ok((path, false));
        }
    }
    let path = default_project_output_dir(app)?;
    fs::create_dir_all(&path)
        .map_err(|e| format!("Could not create default project output folder '{}': {e}", path.display()))?;
    Ok((path, true))
}

fn project_log_path(app: &tauri::AppHandle, filename: &str) -> Result<PathBuf, String> {
    let (dir, _) = project_output_dir(app)?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create project output folder '{}': {e}", dir.display()))?;
    Ok(dir.join(filename))
}

#[tauri::command]
fn get_project_output_folder(app: tauri::AppHandle) -> Result<ProjectOutputFolderInfo, String> {
    let (path, is_default) = project_output_dir(&app)?;
    Ok(ProjectOutputFolderInfo {
        path: path.to_string_lossy().to_string(),
        is_default,
    })
}

#[tauri::command]
async fn pick_project_output_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let folder = app.dialog().file().blocking_pick_folder();
    let Some(folder) = folder else { return Ok(None); };
    let path = PathBuf::from(folder.to_string());
    fs::create_dir_all(&path)
        .map_err(|e| format!("Could not create selected output folder '{}': {e}", path.display()))?;
    let mut prefs = load_desktop_preferences(&app)?;
    prefs.project_output_dir = Some(path.to_string_lossy().to_string());
    save_desktop_preferences(&app, &prefs)?;
    Ok(Some(path.to_string_lossy().to_string()))
}

#[tauri::command]
fn reset_project_output_folder(app: tauri::AppHandle) -> Result<ProjectOutputFolderInfo, String> {
    let mut prefs = load_desktop_preferences(&app)?;
    prefs.project_output_dir = None;
    save_desktop_preferences(&app, &prefs)?;
    get_project_output_folder(app)
}

#[derive(Serialize)]
struct ConvertResult {
    success: bool,
    stdout: String,
    stderr: String,
    output_path: Option<String>,
}

#[tauri::command]
async fn convert_pdx(app: tauri::AppHandle, input_path: String) -> Result<ConvertResult, String> {
    let input_path = clean_path(&input_path);
    if input_path.is_empty() {
        return Err("No PDX file was selected.".to_string());
    }

    let input = PathBuf::from(&input_path);
    if !input.exists() {
        return Err(format!("PDX file not found: {}", input.display()));
    }

    // Generated MDD files go to the user's selected project output folder.
    // The path is persisted so conversion and diagnostic logs share one folder.
    let (generated_root, _) = project_output_dir(&app)?;
    fs::create_dir_all(&generated_root)
        .map_err(|e| format!("Could not create project output directory: {e}"))?;
    let stem = input
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let output_path = generated_root.join(format!(
        "{}_{}.mdd",
        safe_folder_name(&stem, "diagnostic"),
        now_ms()
    ));

    let input_for_task = input.clone();
    let output_for_task = output_path.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let options = ConverterOptions {
            compression: CompressionConfig::Lzma,
            ..ConverterOptions::default()
        };
        let converter = Converter::new(options, 1);
        converter.convert(&input_for_task, &output_for_task)
    })
    .await
    .map_err(|e| format!("Converter task failed: {e}"))?;

    match result {
        Ok(stats) => {
            let mut library = load_diagnostic_library(&app)?;
            let output_string = output_path.to_string_lossy().to_string();

            // Preserve the link when the input PDX already exists in Workspace.
            for source in &mut library.sources {
                if source.kind == "pdx" && Path::new(&source.stored_path) == input.as_path() {
                    source.generated_mdd_path = Some(output_string.clone());
                }
            }

            // Register the generated MDD itself as a persistent Workspace entry.
            // Avoid duplicate registration if this exact path is already present.
            if !library.sources.iter().any(|source| source.stored_path == output_string) {
                let ecu_names = scan_source_ecus("pdx", &input);
                library.sources.push(DiagnosticSourceEntry {
                    id: format!("mdd_{}", now_ms()),
                    name: output_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    kind: "mdd".to_string(),
                    original_path: input.to_string_lossy().to_string(),
                    stored_path: output_string.clone(),
                    ecu_names,
                    generated_mdd_path: None,
                    imported_at_ms: now_ms(),
                });
            }
            save_diagnostic_library(&app, &library)?;

            let stdout = format!(
                "Conversion succeeded.\nOutput saved in Workspace: {}\nDuration: {} ms\nRaw ODX: {} bytes\nUncompressed diagnostic data: {} bytes\nMDD size: {} bytes",
                output_path.display(),
                stats.duration_ms,
                stats.raw_size,
                stats.uncompressed_size,
                stats.compressed_size
            );

            Ok(ConvertResult {
                success: true,
                stdout,
                stderr: String::new(),
                output_path: Some(output_string),
            })
        }
        Err(e) => Ok(ConvertResult {
            success: false,
            stdout: String::new(),
            stderr: format!("{e:#}"),
            output_path: None,
        }),
    }
}

#[derive(Serialize)]
struct ParseResult {
    success: bool,
    stdout: String,
    stderr: String,
    json: Option<serde_json::Value>,
    json_source: Option<String>,
}

#[tauri::command]
async fn parse_mdd(input_path: String) -> Result<ParseResult, String> {
    let input_path = clean_path(&input_path);
    if input_path.is_empty() {
        return Err("No MDD file was selected.".to_string());
    }

    let input = PathBuf::from(&input_path);
    if !input.exists() {
        return Err(format!("MDD file not found: {}", input.display()));
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Could not create parser timestamp: {e}"))?
        .as_millis();
    let temp_json = std::env::temp_dir().join(format!(
        "odx_desktop_mdd_{}_{}.json",
        std::process::id(),
        stamp
    ));

    let input_for_task = input.clone();
    let json_for_task = temp_json.clone();
    let parse_result = tauri::async_runtime::spawn_blocking(move || {
        odx_converter::parser::export_mdd_to_json(&input_for_task, &json_for_task)
    })
    .await
    .map_err(|e| format!("MDD parser task failed: {e}"))?;

    if let Err(e) = parse_result {
        let _ = fs::remove_file(&temp_json);
        return Ok(ParseResult {
            success: false,
            stdout: String::new(),
            stderr: format!("{e:#}"),
            json: None,
            json_source: None,
        });
    }

    let raw = fs::read_to_string(&temp_json)
        .map_err(|e| format!("Failed to read generated JSON: {e}"))?;
    let json: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("Generated JSON is invalid: {e}"))?;
    let _ = fs::remove_file(&temp_json);

    Ok(ParseResult {
        success: true,
        stdout: "Parsed with the bundled Rust MDD parser.".to_string(),
        stderr: String::new(),
        json: Some(json),
        json_source: Some("bundled Rust parser".to_string()),
    })
}
#[tauri::command]
async fn pick_pdx_file(app: tauri::AppHandle) -> Option<String> {
    let file = app
        .dialog()
        .file()
        .add_filter("PDX files", &["pdx"])
        .add_filter("All files", &["*"])
        .blocking_pick_file();
    file.map(|f| f.to_string())
}

#[tauri::command]
async fn pick_mdd_file(app: tauri::AppHandle) -> Option<String> {
    let file = app
        .dialog()
        .file()
        .add_filter("MDD files", &["mdd"])
        .add_filter("All files", &["*"])
        .blocking_pick_file();
    file.map(|f| f.to_string())
}




// ======================================================
// Persistent diagnostic source workspace
// ======================================================

#[derive(Serialize, Deserialize, Clone, Default)]
struct DiagnosticSourceEntry {
    id: String,
    name: String,
    kind: String,
    original_path: String,
    stored_path: String,
    ecu_names: Vec<String>,
    generated_mdd_path: Option<String>,
    imported_at_ms: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct DiagnosticSourceLibrary {
    format_version: u16,
    sources: Vec<DiagnosticSourceEntry>,
}

impl Default for DiagnosticSourceLibrary {
    fn default() -> Self {
        Self { format_version: 1, sources: Vec::new() }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn diagnostic_library_state_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir()
        .map_err(|e| format!("Could not resolve app config directory: {e}"))?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create app config directory '{}': {e}", dir.display()))?;
    Ok(dir.join("diagnostic_sources.json"))
}

fn diagnostic_library_files_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir()
        .map_err(|e| format!("Could not resolve app data directory: {e}"))?
        .join("diagnostic_sources");
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create diagnostic source directory '{}': {e}", dir.display()))?;
    Ok(dir)
}

fn load_diagnostic_library(app: &tauri::AppHandle) -> Result<DiagnosticSourceLibrary, String> {
    let path = diagnostic_library_state_path(app)?;
    if !path.exists() {
        return Ok(DiagnosticSourceLibrary::default());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("Could not read diagnostic source library '{}': {e}", path.display()))?;
    let mut library: DiagnosticSourceLibrary = serde_json::from_str(&raw)
        .map_err(|e| format!("Could not parse diagnostic source library '{}': {e}", path.display()))?;
    if library.format_version != 1 {
        return Err(format!("Unsupported diagnostic source library version: {}", library.format_version));
    }
    // Remove entries whose copied source was deleted outside the application.
    library.sources.retain(|entry| Path::new(&entry.stored_path).exists());
    Ok(library)
}

fn save_diagnostic_library(app: &tauri::AppHandle, library: &DiagnosticSourceLibrary) -> Result<(), String> {
    let path = diagnostic_library_state_path(app)?;
    let raw = serde_json::to_string_pretty(library)
        .map_err(|e| format!("Could not serialize diagnostic source library: {e}"))?;
    fs::write(&path, raw)
        .map_err(|e| format!("Could not save diagnostic source library '{}': {e}", path.display()))
}

fn diagnostic_kind(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".pdx") {
        Some("pdx")
    } else if name.ends_with(".mdd") {
        Some("mdd")
    } else if name.ends_with(".odx") || name.ends_with(".odx-d") || name.ends_with(".odx-f")
        || name.ends_with(".odx-c") || name.ends_with(".odx-cs") || name.ends_with(".odx-e") {
        Some("odx")
    } else {
        None
    }
}

fn is_odx_archive_entry(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".odx") || lower.ends_with(".odx-d") || lower.ends_with(".odx-f")
        || lower.ends_with(".odx-c") || lower.ends_with(".odx-cs") || lower.ends_with(".odx-e")
}

fn collect_ecu_names(collection: &odx_converter::collection::OdxCollection, names: &mut BTreeSet<String>) {
    for variant in &collection.ecu_variant_store {
        let value = variant.core.short_name.trim();
        if !value.is_empty() { names.insert(value.to_string()); }
    }
    for variant in &collection.base_variant_store {
        let value = variant.core.short_name.trim();
        if !value.is_empty() { names.insert(value.to_string()); }
    }
    if names.is_empty() {
        if let Some(container) = &collection.diag_layer_container {
            let value = container.short_name.trim();
            if !value.is_empty() { names.insert(value.to_string()); }
        }
    }
}

fn scan_ecu_names_from_odx(path: &Path) -> Result<Vec<String>, String> {
    let xml = fs::read_to_string(path)
        .map_err(|e| format!("Could not read ODX file '{}': {e}", path.display()))?;
    let source_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let collection = parse_odx_file(&xml, &source_name)
        .map_err(|e| format!("Could not parse ODX file '{}': {e:#}", path.display()))?;
    let mut names = BTreeSet::new();
    collect_ecu_names(&collection, &mut names);
    Ok(names.into_iter().collect())
}

fn scan_ecu_names_from_pdx(path: &Path) -> Result<Vec<String>, String> {
    let file = fs::File::open(path)
        .map_err(|e| format!("Could not open PDX '{}': {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("'{}' is not a valid PDX/ZIP archive: {e}", path.display()))?;
    let mut names = BTreeSet::new();

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)
            .map_err(|e| format!("Could not read PDX entry #{index}: {e}"))?;
        if entry.is_dir() || !is_odx_archive_entry(entry.name()) {
            continue;
        }
        let entry_name = entry.name().to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)
            .map_err(|e| format!("Could not read ODX entry '{entry_name}': {e}"))?;
        let xml = String::from_utf8_lossy(&bytes);
        if let Ok(collection) = parse_odx_file(&xml, &entry_name) {
            collect_ecu_names(&collection, &mut names);
        }
    }
    Ok(names.into_iter().collect())
}

fn scan_source_ecus(kind: &str, path: &Path) -> Vec<String> {
    let result = match kind {
        "pdx" => scan_ecu_names_from_pdx(path),
        "odx" => scan_ecu_names_from_odx(path),
        _ => Ok(Vec::new()),
    };
    result.unwrap_or_default()
}

#[tauri::command]
async fn import_diagnostic_sources(
    app: tauri::AppHandle,
) -> Result<Vec<DiagnosticSourceEntry>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();

    app.dialog()
        .file()
        .add_filter(
            "Diagnostic files",
            &["pdx", "odx", "odx-d", "odx-f", "odx-c", "odx-cs", "odx-e", "mdd"],
        )
        .add_filter("All files", &["*"])
        .pick_files(move |files| {
            let _ = sender.send(files);
        });

    let selected = tauri::async_runtime::spawn_blocking(move || receiver.recv())
        .await
        .map_err(|e| format!("File dialog task failed: {e}"))?
        .map_err(|e| format!("File dialog failed: {e}"))?;

    let Some(selected) = selected else {
        return Ok(load_diagnostic_library(&app)?.sources);
    };

    let mut library = load_diagnostic_library(&app)?;
    let root = diagnostic_library_files_dir(&app)?;
    let stamp = now_ms();

    for (index, file) in selected.into_iter().enumerate() {
        let original = PathBuf::from(file.to_string());
        let Some(kind) = diagnostic_kind(&original) else { continue; };
        if !original.exists() { continue; }

        let id = format!("src_{}_{}", stamp, index);
        let entry_dir = root.join(&id);
        fs::create_dir_all(&entry_dir)
            .map_err(|e| format!("Could not create source directory '{}': {e}", entry_dir.display()))?;
        let name = original.file_name().unwrap_or_default().to_string_lossy().to_string();
        let stored = entry_dir.join(&name);
        fs::copy(&original, &stored)
            .map_err(|e| format!("Could not import '{}': {e}", original.display()))?;

        // Keep Workspace import fast and non-blocking on Linux/WSL.
        // ECU/service discovery is performed when a diagnostic source is loaded.
        let ecu_names: Vec<String> = Vec::new();

        library.sources.push(DiagnosticSourceEntry {
            id,
            name,
            kind: kind.to_string(),
            original_path: original.to_string_lossy().to_string(),
            stored_path: stored.to_string_lossy().to_string(),
            ecu_names,
            generated_mdd_path: None,
            imported_at_ms: now_ms(),
        });
    }

    save_diagnostic_library(&app, &library)?;
    Ok(library.sources)
}

#[tauri::command]
fn list_diagnostic_sources(app: tauri::AppHandle) -> Result<Vec<DiagnosticSourceEntry>, String> {
    Ok(load_diagnostic_library(&app)?.sources)
}

#[tauri::command]
fn remove_diagnostic_source(app: tauri::AppHandle, source_id: String) -> Result<Vec<DiagnosticSourceEntry>, String> {
    let mut library = load_diagnostic_library(&app)?;
    if let Some(entry) = library.sources.iter().find(|entry| entry.id == source_id).cloned() {
        let stored = PathBuf::from(&entry.stored_path);
        // Only delete files that live inside ODX Desktop's managed workspace.
        // A generated MDD in a user-selected project folder must never be
        // deleted just because its Workspace entry is removed.
        if let Ok(managed_root) = diagnostic_library_files_dir(&app) {
            if stored.starts_with(&managed_root) {
                if let Some(parent) = stored.parent() {
                    let owns_directory = parent.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name == entry.id)
                        .unwrap_or(false);
                    if owns_directory {
                        let _ = fs::remove_dir_all(parent);
                    } else {
                        let _ = fs::remove_file(&stored);
                    }
                }
            }
        }
        for source in &mut library.sources {
            if source.generated_mdd_path.as_deref() == Some(entry.stored_path.as_str()) {
                source.generated_mdd_path = None;
            }
        }
    }
    library.sources.retain(|entry| entry.id != source_id);
    save_diagnostic_library(&app, &library)?;
    Ok(library.sources)
}

fn convert_odx_group_to_mdd(paths: &[PathBuf], output: &Path) -> Result<String, String> {
    if paths.is_empty() {
        return Err("Select at least one ODX file.".to_string());
    }
    let mut collections = Vec::new();
    let mut raw_size = 0u64;
    for path in paths {
        let bytes = fs::read(path)
            .map_err(|e| format!("Could not read ODX '{}': {e}", path.display()))?;
        raw_size = raw_size.saturating_add(bytes.len() as u64);
        let xml = String::from_utf8_lossy(&bytes);
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let collection = parse_odx_file(&xml, &name)
            .map_err(|e| format!("Could not parse ODX '{}': {e:#}", path.display()))?;
        collections.push(collection);
    }

    let group = OdxCollectionGroup::new(collections, raw_size);
    let options = ConverterOptions {
        compression: CompressionConfig::Lzma,
        ..ConverterOptions::default()
    };
    let chunk_builder = ChunkBuilder::new(&options);
    let diag_chunk = chunk_builder.create_ecu_data_chunk(&group, &options)
        .map_err(|e| format!("Could not build diagnostic description: {e:#}"))?;
    let mut metadata = HashMap::new();
    metadata.insert("source".to_string(), "ODX Desktop saved ODX group".to_string());
    MddWriter::write(
        &group.ecu_name,
        group.odx_revision.as_deref(),
        "2025-05-21",
        vec![diag_chunk],
        metadata,
        output,
    )
    .map_err(|e| format!("Could not write MDD '{}': {e:#}", output.display()))?;
    Ok(group.ecu_name)
}

#[tauri::command]
async fn convert_saved_sources(
    app: tauri::AppHandle,
    source_ids: Vec<String>,
) -> Result<DiagnosticSourceEntry, String> {
    if source_ids.is_empty() {
        return Err("Select one saved PDX or one/more saved ODX files.".to_string());
    }
    let mut library = load_diagnostic_library(&app)?;
    let selected = source_ids.iter()
        .filter_map(|id| library.sources.iter().find(|entry| &entry.id == id).cloned())
        .collect::<Vec<_>>();
    if selected.len() != source_ids.len() {
        return Err("One or more selected diagnostic sources no longer exist.".to_string());
    }

    let kinds = selected.iter().map(|entry| entry.kind.as_str()).collect::<BTreeSet<_>>();
    if kinds.len() != 1 || (!kinds.contains("pdx") && !kinds.contains("odx")) {
        return Err("Convert either one PDX file or a group containing only ODX files.".to_string());
    }
    if kinds.contains("pdx") && selected.len() != 1 {
        return Err("Select exactly one PDX file for conversion.".to_string());
    }

    let (generated_root, _) = project_output_dir(&app)?;
    fs::create_dir_all(&generated_root)
        .map_err(|e| format!("Could not create project output directory: {e}"))?;
    let output = if selected.len() == 1 {
        let stem = Path::new(&selected[0].name).file_stem().unwrap_or_default().to_string_lossy();
        generated_root.join(format!("{}_{}.mdd", safe_folder_name(&stem, "diagnostic"), now_ms()))
    } else {
        generated_root.join(format!("odx_group_{}.mdd", now_ms()))
    };

    let output_for_task = output.clone();
    let selected_for_task = selected.clone();
    let generated_ecu_name = tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
        if selected_for_task[0].kind == "pdx" {
            let options = ConverterOptions { compression: CompressionConfig::Lzma, ..ConverterOptions::default() };
            let converter = Converter::new(options, 1);
            converter.convert(Path::new(&selected_for_task[0].stored_path), &output_for_task)
                .map_err(|e| format!("PDX conversion failed: {e:#}"))?;
            Ok(None)
        } else {
            let paths = selected_for_task.iter().map(|entry| PathBuf::from(&entry.stored_path)).collect::<Vec<_>>();
            Ok(Some(convert_odx_group_to_mdd(&paths, &output_for_task)?))
        }
    }).await.map_err(|e| format!("Conversion task failed: {e}"))??;

    let mdd_id = format!("mdd_{}", now_ms());
    let mdd_name = output.file_name().unwrap_or_default().to_string_lossy().to_string();
    let mut ecu_names = selected.iter().flat_map(|entry| entry.ecu_names.clone()).collect::<BTreeSet<_>>();
    if let Some(name) = generated_ecu_name {
        if !name.trim().is_empty() { ecu_names.insert(name); }
    }

    // The imported source metadata may intentionally be lightweight/empty.
    // Discover the authoritative ECU variant names from the generated MDD so
    // Diagnostics can immediately offer the correct ECU list after conversion.
    let output_string = output.to_string_lossy().to_string();
    if let Ok(services) = cached_uds_services(&output_string) {
        ecu_names.extend(uds_ecu_names_for_services(services.as_ref()));
    }

    let mdd_entry = DiagnosticSourceEntry {
        id: mdd_id,
        name: mdd_name,
        kind: "mdd".to_string(),
        original_path: String::new(),
        stored_path: output.to_string_lossy().to_string(),
        ecu_names: ecu_names.into_iter().collect(),
        generated_mdd_path: None,
        imported_at_ms: now_ms(),
    };

    for source in &mut library.sources {
        if source_ids.iter().any(|id| id == &source.id) {
            source.generated_mdd_path = Some(output.to_string_lossy().to_string());
        }
    }
    library.sources.push(mdd_entry.clone());
    save_diagnostic_library(&app, &library)?;
    Ok(mdd_entry)
}

#[derive(Serialize, Clone)]
struct UdsMddChoice {
    label: String,
    value_hex: String,
}

#[derive(Serialize, Clone)]
struct UdsMddParameter {
    name: String,
    param_type: String,
    byte_position: Option<u32>,
    bit_position: Option<u32>,
    fixed: bool,
    value_hex: Option<String>,
    default_value_hex: Option<String>,
    bit_length: Option<u32>,
    byte_length: Option<u32>,
    data_type: Option<String>,
    min_value: Option<String>,
    max_value: Option<String>,
    unit: Option<String>,
    choices: Vec<UdsMddChoice>,
    children: Vec<UdsMddParameter>,
    input_hint: Option<String>,
}

#[derive(Serialize, Clone)]
struct UdsMddService {
    id: String,
    name: String,
    long_name: Option<String>,
    source_layer: String,
    source_ecu: String,
    sid_hex: Option<String>,
    positive_sid_hex: Option<String>,
    parameters: Vec<UdsMddParameter>,
    positive_parameters: Vec<UdsMddParameter>,
    negative_parameters: Vec<UdsMddParameter>,
}

fn read_u16_le(data: &[u8], pos: usize) -> Result<u16, String> {
    let bytes = data.get(pos..pos + 2).ok_or_else(|| "Unexpected end of FlatBuffer".to_string())?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32_le(data: &[u8], pos: usize) -> Result<u32, String> {
    let bytes = data.get(pos..pos + 4).ok_or_else(|| "Unexpected end of FlatBuffer".to_string())?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_i32_le(data: &[u8], pos: usize) -> Result<i32, String> {
    let bytes = data.get(pos..pos + 4).ok_or_else(|| "Unexpected end of FlatBuffer".to_string())?;
    Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn fb_field_addr(data: &[u8], table: usize, field_index: usize) -> Result<Option<usize>, String> {
    // The first 32-bit value of a FlatBuffers table is a signed offset
    // from the table to its vtable. It is usually positive, but it can
    // legitimately be negative when FlatBuffers reuses/deduplicates a
    // vtable that ends up after the current table in the final buffer.
    let vtable_offset = read_i32_le(data, table)? as isize;
    let vtable_signed = (table as isize)
        .checked_sub(vtable_offset)
        .ok_or_else(|| "Invalid FlatBuffer vtable address".to_string())?;
    if vtable_signed < 0 || vtable_signed as usize >= data.len() {
        return Err(format!(
            "Invalid FlatBuffer vtable address (table={table}, offset={vtable_offset})"
        ));
    }
    let vtable = vtable_signed as usize;
    let vtable_len = read_u16_le(data, vtable)? as usize;
    let entry = 4 + field_index * 2;
    if entry + 2 > vtable_len { return Ok(None); }
    let off = read_u16_le(data, vtable + entry)? as usize;
    if off == 0 {
        Ok(None)
    } else {
        let addr = table.checked_add(off).ok_or_else(|| "FlatBuffer field offset overflow".to_string())?;
        if addr >= data.len() { return Err("FlatBuffer field address out of range".to_string()); }
        Ok(Some(addr))
    }
}

fn fb_indirect(data: &[u8], addr: usize) -> Result<usize, String> {
    let rel = read_u32_le(data, addr)? as usize;
    let target = addr.checked_add(rel).ok_or_else(|| "FlatBuffer offset overflow".to_string())?;
    if target >= data.len() {
        return Err(format!("FlatBuffer indirect target out of range (addr={addr}, rel={rel}, len={})", data.len()));
    }
    Ok(target)
}

fn fb_table_field(data: &[u8], table: usize, field_index: usize) -> Result<Option<usize>, String> {
    match fb_field_addr(data, table, field_index)? {
        Some(addr) => Ok(Some(fb_indirect(data, addr)?)),
        None => Ok(None),
    }
}

fn fb_string_field(data: &[u8], table: usize, field_index: usize) -> Result<Option<String>, String> {
    let Some(addr) = fb_field_addr(data, table, field_index)? else { return Ok(None); };
    let target = fb_indirect(data, addr)?;
    let len = read_u32_le(data, target)? as usize;
    let start = target.checked_add(4).ok_or_else(|| "FlatBuffer string offset overflow".to_string())?;
    let end = start.checked_add(len).ok_or_else(|| "FlatBuffer string length overflow".to_string())?;
    let bytes = data.get(start..end).ok_or_else(|| "Invalid FlatBuffer string length".to_string())?;
    Ok(Some(String::from_utf8_lossy(bytes).to_string()))
}

fn fb_u8_field(data: &[u8], table: usize, field_index: usize, default: u8) -> Result<u8, String> {
    let Some(addr) = fb_field_addr(data, table, field_index)? else { return Ok(default); };
    Ok(*data.get(addr).ok_or_else(|| "Invalid FlatBuffer byte field".to_string())?)
}

fn fb_u32_field(data: &[u8], table: usize, field_index: usize) -> Result<Option<u32>, String> {
    let Some(addr) = fb_field_addr(data, table, field_index)? else { return Ok(None); };
    Ok(Some(read_u32_le(data, addr)?))
}

fn fb_vector_tables(data: &[u8], table: usize, field_index: usize) -> Result<Vec<usize>, String> {
    let Some(addr) = fb_field_addr(data, table, field_index)? else { return Ok(Vec::new()); };
    let vector = fb_indirect(data, addr)?;
    let len = read_u32_le(data, vector)? as usize;
    let start = vector.checked_add(4).ok_or_else(|| "FlatBuffer vector offset overflow".to_string())?;
    let remaining = data.len().saturating_sub(start);
    let max_elements = remaining / 4;
    if len > max_elements {
        return Err(format!("Invalid FlatBuffer vector length {len}; only {max_elements} table slot(s) fit in the remaining buffer"));
    }
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let slot = start
            .checked_add(i.checked_mul(4).ok_or_else(|| "FlatBuffer vector index overflow".to_string())?)
            .ok_or_else(|| "FlatBuffer vector slot overflow".to_string())?;
        out.push(fb_indirect(data, slot)?);
    }
    Ok(out)
}

fn read_varint(data: &[u8], mut pos: usize) -> Result<(u64, usize), String> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let b = *data.get(pos).ok_or_else(|| "Unexpected end of protobuf data".to_string())?;
        pos += 1;
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 { return Ok((value, pos)); }
        shift += 7;
        if shift >= 64 { return Err("Invalid protobuf varint".to_string()); }
    }
}

fn protobuf_length_field(data: &[u8], wanted: u32) -> Result<Vec<u8>, String> {
    let mut pos = 0usize;
    while pos < data.len() {
        let (tag, p) = read_varint(data, pos)?;
        pos = p;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x07) as u8;
        match wire {
            0 => { let (_, p2) = read_varint(data, pos)?; pos = p2; }
            1 => { pos = pos.checked_add(8).ok_or("protobuf overflow")?; }
            2 => {
                let (len, p2) = read_varint(data, pos)?;
                pos = p2;
                let end = pos.checked_add(len as usize).ok_or("protobuf overflow")?;
                let bytes = data.get(pos..end).ok_or_else(|| "Invalid protobuf length field".to_string())?;
                if field == wanted { return Ok(bytes.to_vec()); }
                pos = end;
            }
            5 => { pos = pos.checked_add(4).ok_or("protobuf overflow")?; }
            _ => return Err(format!("Unsupported protobuf wire type {wire}")),
        }
    }
    Err(format!("Protobuf field {wanted} not found"))
}

struct LimitedVecWriter {
    data: Vec<u8>,
    max_len: usize,
}

impl LimitedVecWriter {
    fn new(max_len: usize) -> Self {
        Self { data: Vec::new(), max_len }
    }
}

impl std::io::Write for LimitedVecWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let next = self.data.len().checked_add(buf.len()).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "MDD decompressed size overflow")
        })?;
        if next > self.max_len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("MDD decompressed FlatBuffer exceeds safety limit of {} bytes", self.max_len),
            ));
        }
        self.data.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

fn extract_flatbuffer_from_mdd(path: &str) -> Result<Vec<u8>, String> {
    const MAX_DECOMPRESSED_MDD_BYTES: usize = 512 * 1024 * 1024;

    let data = fs::read(path).map_err(|e| format!("Failed to read MDD '{path}': {e}"))?;
    if data.len() < 8 { return Err("MDD file is too small".to_string()); }

    let payload = if data.starts_with(b"MDD version 0   ") {
        data.get(20..).ok_or_else(|| "Invalid MDD header".to_string())?
    } else {
        data.get(4..).ok_or_else(|| "Invalid MDD header".to_string())?
    };

    let chunk = protobuf_length_field(payload, 6)?;
    let compressed = protobuf_length_field(&chunk, 8)?;
    let mut output = LimitedVecWriter::new(MAX_DECOMPRESSED_MDD_BYTES);
    lzma_rs::lzma_decompress(&mut std::io::Cursor::new(compressed), &mut output)
        .map_err(|e| format!("Failed to decompress MDD FlatBuffer safely: {e}"))?;
    Ok(output.data)
}

fn decimal_string_to_hex(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let parsed = if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<u64>().ok()
    }?;
    let width = if parsed <= 0xFF { 2 } else if parsed <= 0xFFFF { 4 } else if parsed <= 0xFFFFFF { 6 } else { 8 };
    Some(format!("{parsed:0width$X}"))
}

fn param_type_name(value: u8) -> &'static str {
    match value {
        0 => "CODED_CONST",
        1 => "DYNAMIC",
        2 => "LENGTH_KEY",
        3 => "MATCHING_REQUEST_PARAM",
        4 => "NRC_CONST",
        5 => "PHYS_CONST",
        6 => "RESERVED",
        7 => "SYSTEM",
        8 => "TABLE_ENTRY",
        9 => "TABLE_KEY",
        10 => "TABLE_STRUCT",
        11 => "VALUE",
        _ => "UNKNOWN",
    }
}

fn data_type_name(value: u8) -> &'static str {
    match value {
        0 => "A_INT_32",
        1 => "A_UINT_32",
        2 => "A_FLOAT_32",
        3 => "A_ASCIISTRING",
        4 => "A_UTF_8_STRING",
        5 => "A_UNICODE_2_STRING",
        6 => "A_BYTEFIELD",
        7 => "A_FLOAT_64",
        _ => "UNKNOWN",
    }
}

#[derive(Default)]
struct DopGuide {
    bit_length: Option<u32>,
    byte_length: Option<u32>,
    data_type: Option<String>,
    min_value: Option<String>,
    max_value: Option<String>,
    unit: Option<String>,
    choices: Vec<UdsMddChoice>,
    children: Vec<UdsMddParameter>,
}

fn limit_value(data: &[u8], table: Option<usize>) -> Result<Option<String>, String> {
    let Some(table) = table else { return Ok(None); };
    fb_string_field(data, table, 0)
}

fn compu_text_choices(data: &[u8], compu_method: usize) -> Result<Vec<UdsMddChoice>, String> {
    // CompuMethod: category=0, internal_to_phys=1. TEXT_TABLE category = 3.
    if fb_u8_field(data, compu_method, 0, 0)? != 3 {
        return Ok(Vec::new());
    }
    let Some(i2p) = fb_table_field(data, compu_method, 1)? else { return Ok(Vec::new()); };
    let mut out = Vec::new();
    for scale in fb_vector_tables(data, i2p, 0)? {
        let lower = limit_value(data, fb_table_field(data, scale, 1)?)?;
        let upper = limit_value(data, fb_table_field(data, scale, 2)?)?;
        let exact = match (&lower, &upper) {
            (Some(a), Some(b)) if a == b => Some(a.clone()),
            (Some(a), None) => Some(a.clone()),
            _ => None,
        };
        let Some(internal) = exact else { continue; };
        let Some(value_hex) = decimal_string_to_hex(&internal) else { continue; };

        let label = if let Some(consts) = fb_table_field(data, scale, 4)? {
            fb_string_field(data, consts, 1)?.filter(|v| !v.trim().is_empty())
        } else { None }
        .or_else(|| {
            fb_table_field(data, scale, 0).ok().flatten()
                .and_then(|t| fb_string_field(data, t, 0).ok().flatten())
        })
        .unwrap_or_else(|| format!("0x{}", value_hex));

        out.push(UdsMddChoice { label, value_hex });
    }
    Ok(out)
}

fn diag_coded_guide(data: &[u8], dct: usize, guide: &mut DopGuide) -> Result<(), String> {
    guide.data_type = Some(data_type_name(fb_u8_field(data, dct, 2, 0)?).to_string());
    let dct_kind = fb_u8_field(data, dct, 0, 0)?;
    let specific_type = fb_u8_field(data, dct, 4, 0)?;
    let specific = fb_table_field(data, dct, 5)?;
    match (dct_kind, specific_type, specific) {
        // STANDARD_LENGTH_TYPE / StandardLengthType union variant 4.
        (3, 4, Some(table)) | (3, _, Some(table)) => {
            let bits = fb_u32_field(data, table, 0)?.unwrap_or(0);
            if bits > 0 {
                guide.bit_length = Some(bits);
                guide.byte_length = Some((bits + 7) / 8);
            }
        }
        // LEADING_LENGTH_INFO_TYPE / LeadingLengthInfoType union variant 1.
        (0, 1, Some(table)) | (0, _, Some(table)) => {
            let bits = fb_u32_field(data, table, 0)?.unwrap_or(0);
            if bits > 0 { guide.bit_length = Some(bits); guide.byte_length = Some((bits + 7) / 8); }
        }
        // MIN_MAX_LENGTH_TYPE / MinMaxLengthType union variant 2: lengths are bytes.
        (1, 2, Some(table)) | (1, _, Some(table)) => {
            if let Some(min_len) = fb_u32_field(data, table, 0)? {
                guide.byte_length = Some(min_len);
                guide.bit_length = Some(min_len.saturating_mul(8));
            }
        }
        _ => {}
    }
    Ok(())
}

fn dop_guide(data: &[u8], dop: usize) -> Result<DopGuide, String> {
    let mut guide = DopGuide::default();
    let dop_type = fb_u8_field(data, dop, 0, 0)?;
    let specific_type = fb_u8_field(data, dop, 3, 0)?;
    let specific = fb_table_field(data, dop, 4)?;

    // REGULAR DOP / NormalDOP union variant 1.
    if dop_type == 0 || specific_type == 1 {
        if let Some(normal) = specific {
            if let Some(compu_method) = fb_table_field(data, normal, 0)? {
                guide.choices = compu_text_choices(data, compu_method)?;
            }
            if let Some(dct) = fb_table_field(data, normal, 1)? {
                diag_coded_guide(data, dct, &mut guide)?;
            }
            if let Some(internal) = fb_table_field(data, normal, 3)? {
                guide.min_value = limit_value(data, fb_table_field(data, internal, 0)?)?;
                guide.max_value = limit_value(data, fb_table_field(data, internal, 1)?)?;
            }
            if let Some(unit) = fb_table_field(data, normal, 4)? {
                guide.unit = fb_string_field(data, unit, 1)?.or_else(|| fb_string_field(data, unit, 0).ok().flatten());
            }
        }
    }

    // STRUCTURE DOP / Structure union variant 7.
    if dop_type == 8 || specific_type == 7 {
        if let Some(structure) = specific {
            guide.byte_length = fb_u32_field(data, structure, 1)?;
            if let Some(bytes) = guide.byte_length { guide.bit_length = Some(bytes.saturating_mul(8)); }
            for child in fb_vector_tables(data, structure, 0)? {
                guide.children.push(parse_param(data, child)?);
            }
            if guide.byte_length.is_none() {
                let mut inferred = 0u32;
                for child in &guide.children {
                    let start = child.byte_position.unwrap_or(0);
                    let len = child.byte_length.unwrap_or_else(|| child.bit_length.map(|b| (b + 7) / 8).unwrap_or(1));
                    inferred = inferred.max(start.saturating_add(len));
                }
                if inferred > 0 { guide.byte_length = Some(inferred); guide.bit_length = Some(inferred.saturating_mul(8)); }
            }
        }
    }

    // DTC DOP / DTCDOP union variant 6.
    if dop_type == 9 || specific_type == 6 {
        if let Some(dtc) = specific {
            if let Some(dct) = fb_table_field(data, dtc, 0)? { diag_coded_guide(data, dct, &mut guide)?; }
        }
    }

    Ok(guide)
}

fn table_key_guide(data: &[u8], table_key: usize) -> Result<DopGuide, String> {
    let mut guide = DopGuide::default();
    let reference_type = fb_u8_field(data, table_key, 0, 0)?;
    let reference = fb_table_field(data, table_key, 1)?;
    let Some(reference) = reference else { return Ok(guide); };

    // TableKeyReference::TableDop = 1, TableRow = 2.
    if reference_type == 1 {
        if let Some(key_dop) = fb_table_field(data, reference, 4)? {
            let key_guide = dop_guide(data, key_dop)?;
            guide.bit_length = key_guide.bit_length;
            guide.byte_length = key_guide.byte_length;
            guide.data_type = key_guide.data_type;
            guide.min_value = key_guide.min_value;
            guide.max_value = key_guide.max_value;
            guide.unit = key_guide.unit;
        }
        for row in fb_vector_tables(data, reference, 5)? {
            let Some(key) = fb_string_field(data, row, 2)? else { continue; };
            let Some(value_hex) = decimal_string_to_hex(&key) else { continue; };
            let label = if let Some(long_name) = fb_table_field(data, row, 1)? {
                fb_string_field(data, long_name, 0)?.filter(|v| !v.trim().is_empty())
            } else { None }
            .or_else(|| fb_string_field(data, row, 0).ok().flatten())
            .unwrap_or_else(|| format!("0x{}", value_hex));
            guide.choices.push(UdsMddChoice { label, value_hex });
        }
    } else if reference_type == 2 {
        if let Some(key) = fb_string_field(data, reference, 2)? {
            if let Some(value_hex) = decimal_string_to_hex(&key) {
                let label = fb_string_field(data, reference, 0)?.unwrap_or_else(|| format!("0x{}", value_hex));
                guide.choices.push(UdsMddChoice { label, value_hex });
            }
        }
    }
    Ok(guide)
}

fn input_hint_for(parameter: &UdsMddParameter) -> Option<String> {
    if parameter.fixed { return Some("Value is fully defined by the MDD; no input is required.".to_string()); }
    if !parameter.choices.is_empty() {
        return Some(format!("Choose one of {} value(s) defined by the MDD.", parameter.choices.len()));
    }
    if let Some(ref default_hex) = parameter.default_value_hex {
        let bytes = default_hex.len() / 2;
        return Some(format!("The MDD provides a default value. It has been pre-filled for you: {} ({} byte{}). You may keep it or replace it with another ECU/test-specific value of the same length.", default_hex, bytes, if bytes == 1 { "" } else { "s" }));
    }
    if !parameter.children.is_empty() {
        return Some("This DataRecord is a structure. Complete only the variable fields shown below; fixed fields are inserted automatically.".to_string());
    }

    let mut parts = Vec::new();
    if let Some(bits) = parameter.bit_length {
        let bytes = (bits + 7) / 8;
        if bits % 8 == 0 {
            parts.push(format!("{} byte{} ({} bits)", bytes, if bytes == 1 { "" } else { "s" }, bits));
        } else {
            parts.push(format!("{} bits", bits));
        }
    } else if let Some(bytes) = parameter.byte_length {
        parts.push(format!("{} byte{}", bytes, if bytes == 1 { "" } else { "s" }));
    }
    if let Some(ref data_type) = parameter.data_type { parts.push(data_type.clone()); }
    match (&parameter.min_value, &parameter.max_value) {
        (Some(min), Some(max)) => parts.push(format!("allowed internal range {}..{}", min, max)),
        (Some(min), None) => parts.push(format!("minimum {}", min)),
        (None, Some(max)) => parts.push(format!("maximum {}", max)),
        _ => {}
    }
    if let Some(ref unit) = parameter.unit { parts.push(format!("unit {}", unit)); }

    if parts.is_empty() {
        Some("Runtime value required. The MDD does not provide a fixed value or selectable table for this field; enter the ECU/test-specific value as hexadecimal bytes.".to_string())
    } else {
        let example = match parameter.byte_length.or_else(|| parameter.bit_length.map(|b| (b + 7) / 8)) {
            Some(1) => " For example, hexadecimal 0x20 is entered as 20.",
            Some(2) => " For example, hexadecimal 0x1234 is entered as 12 34.",
            _ => "",
        };
        Some(format!("Runtime value required: {}. Enter the value as hexadecimal bytes.{}", parts.join(" • "), example))
    }
}

fn clean_hex_string(value: &str) -> Option<String> {
    let mut clean = value.trim().to_string();
    if clean.starts_with("0x") || clean.starts_with("0X") {
        clean = clean[2..].to_string();
    }
    clean.retain(|c| c.is_ascii_hexdigit());
    if clean.is_empty() { return None; }
    if clean.len() % 2 != 0 { clean.insert(0, '0'); }
    Some(clean.to_uppercase())
}

fn physical_default_hex(data: &[u8], param: usize, kind: u8, specific_type: u8, specific: Option<usize>, data_type: Option<&str>) -> Option<String> {
    // Param.physical_default_value is field 5. VALUE also carries its own
    // physical_default_value in field 0. Prefer the Value-level entry.
    let mut default_value = None;
    if kind == 11 && specific_type == 7 {
        if let Some(value) = specific {
            default_value = fb_string_field(data, value, 0).ok().flatten();
        }
    }
    if default_value.is_none() {
        default_value = fb_string_field(data, param, 5).ok().flatten();
    }
    let raw = default_value?;

    // A_BYTEFIELD physical defaults are already hexadecimal byte strings in
    // the MDD (for example 0000000000000000 for an 8-byte field).
    if data_type == Some("A_BYTEFIELD") {
        return clean_hex_string(&raw);
    }
    None
}

fn parse_param(data: &[u8], param: usize) -> Result<UdsMddParameter, String> {
    let kind = fb_u8_field(data, param, 1, 0)?;
    let name = fb_string_field(data, param, 2)?.unwrap_or_else(|| "Unnamed parameter".to_string());
    let byte_position = fb_u32_field(data, param, 6)?;
    let bit_position = fb_u32_field(data, param, 7)?;
    // Guidance/fixed-value decoding is best-effort. Some MDDs contain
    // optional/partially populated DOPs that are valid for the main model but
    // cannot be fully interpreted by this lightweight reader. Never fail the
    // whole MDD because an optional DataRecord guide cannot be decoded.
    let fixed_bytes = fixed_param_bytes(data, param).unwrap_or(None);
    let fixed = fixed_bytes.is_some();
    let value_hex = fixed_bytes.as_deref().map(bytes_to_hex);

    let mut guide = DopGuide::default();
    let specific_type = fb_u8_field(data, param, 8, 0)?;
    let specific = fb_table_field(data, param, 9)?;

    // CODED_CONST also carries its own DiagCodedType.
    if kind == 0 && specific_type == 1 {
        if let Some(cc) = specific {
            if let Ok(Some(dct)) = fb_table_field(data, cc, 1) {
                let _ = diag_coded_guide(data, dct, &mut guide);
            }
        }
    }
    // VALUE -> DOP.
    if kind == 11 && specific_type == 7 {
        if let Some(value) = specific {
            if let Ok(Some(dop)) = fb_table_field(data, value, 1) {
                if let Ok(parsed) = dop_guide(data, dop) { guide = parsed; }
            }
        }
    }
    // PHYS_CONST -> DOP.
    if kind == 5 && specific_type == 5 {
        if let Some(pc) = specific {
            if let Ok(Some(dop)) = fb_table_field(data, pc, 1) {
                if let Ok(parsed) = dop_guide(data, dop) { guide = parsed; }
            }
        }
    }
    // TABLE_KEY -> TableDop/TableRow. When the MDD contains table rows,
    // expose them as selectable values instead of a raw DataRecord input.
    if kind == 9 && specific_type == 9 {
        if let Some(table_key) = specific {
            if let Ok(parsed) = table_key_guide(data, table_key) { guide = parsed; }
        }
    }
    // RESERVED has an explicit bit length.
    if kind == 6 && specific_type == 6 {
        if let Some(reserved) = specific {
            let bits = fb_u32_field(data, reserved, 0).ok().flatten().unwrap_or(0);
            if bits > 0 { guide.bit_length = Some(bits); guide.byte_length = Some((bits + 7) / 8); }
        }
    }
    // SYSTEM -> DOP.
    if kind == 7 && specific_type == 11 {
        if let Some(system) = specific {
            if let Ok(Some(dop)) = fb_table_field(data, system, 0) {
                if let Ok(parsed) = dop_guide(data, dop) { guide = parsed; }
            }
        }
    }

    // If the MDD gives a fixed bit width but no explicit constraints, provide
    // a useful numeric range for common integer types.
    if guide.min_value.is_none() && guide.max_value.is_none() {
        if let (Some(bits), Some(data_type)) = (guide.bit_length, guide.data_type.as_ref()) {
            if bits > 0 && bits <= 32 {
                if data_type == "A_UINT_32" {
                    guide.min_value = Some("0".to_string());
                    guide.max_value = Some(((1u64 << bits) - 1).to_string());
                } else if data_type == "A_INT_32" {
                    let max = (1i64 << (bits - 1)) - 1;
                    let min = -(1i64 << (bits - 1));
                    guide.min_value = Some(min.to_string());
                    guide.max_value = Some(max.to_string());
                }
            }
        }
    }

    let default_value_hex = physical_default_hex(data, param, kind, specific_type, specific, guide.data_type.as_deref());

    let mut parameter = UdsMddParameter {
        name,
        param_type: param_type_name(kind).to_string(),
        byte_position,
        bit_position,
        fixed,
        value_hex,
        default_value_hex,
        bit_length: guide.bit_length,
        byte_length: guide.byte_length,
        data_type: guide.data_type,
        min_value: guide.min_value,
        max_value: guide.max_value,
        unit: guide.unit,
        choices: guide.choices,
        children: guide.children,
        input_hint: None,
    };
    parameter.input_hint = input_hint_for(&parameter);
    Ok(parameter)
}

fn fixed_structure_bytes(data: &[u8], structure: usize) -> Result<Option<Vec<u8>>, String> {
    const MAX_FIXED_STRUCTURE_BYTES: usize = 1024 * 1024;
    let nested = fb_vector_tables(data, structure, 0)?;
    if nested.is_empty() {
        return Ok(None);
    }

    let mut chunks: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut inferred_end = 0usize;
    for nested_param in nested {
        let Some(bytes) = fixed_param_bytes(data, nested_param)? else {
            return Ok(None);
        };
        let position = fb_u32_field(data, nested_param, 6)?
            .map(|value| value as usize)
            .unwrap_or(inferred_end);
        let end = position.checked_add(bytes.len())
            .ok_or_else(|| "Fixed structure position overflow".to_string())?;
        if end > MAX_FIXED_STRUCTURE_BYTES {
            return Err(format!("Fixed structure exceeds safe size limit ({MAX_FIXED_STRUCTURE_BYTES} bytes)"));
        }
        inferred_end = inferred_end.max(end);
        chunks.push((position, bytes));
    }

    chunks.sort_by_key(|(position, _)| *position);
    Ok(Some(chunks.into_iter().flat_map(|(_, bytes)| bytes).collect()))
}

fn fixed_param_bytes(data: &[u8], param: usize) -> Result<Option<Vec<u8>>, String> {
    let kind = fb_u8_field(data, param, 1, 0)?;
    let specific_type = fb_u8_field(data, param, 8, 0)?;

    // ParamSpecificData::CodedConst = union variant 1.
    if kind == 0 && specific_type == 1 {
        let Some(coded_const) = fb_table_field(data, param, 9)? else { return Ok(None); };
        let Some(value) = fb_string_field(data, coded_const, 0)? else { return Ok(None); };
        let Some(hex) = decimal_string_to_hex(&value) else { return Ok(None); };
        let compact: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if compact.is_empty() || compact.len() % 2 != 0 {
            return Ok(None);
        }

        let mut bytes = Vec::new();
        for i in (0..compact.len()).step_by(2) {
            let Some(byte) = u8::from_str_radix(&compact[i..i + 2], 16).ok() else {
                return Ok(None);
            };
            bytes.push(byte);
        }
        return Ok(Some(bytes));
    }

    // ParamSpecificData::Value = union variant 7.
    // In this MDD family, values such as DiagnosticSessionControl's
    // "DataRecord" contain a DOP -> STRUCTURE -> nested CODED_CONST values.
    // Those bytes are already fully defined by the MDD and must NOT be typed
    // by the user.
    if kind == 11 && specific_type == 7 {
        let Some(value_table) = fb_table_field(data, param, 9)? else { return Ok(None); };
        let Some(dop) = fb_table_field(data, value_table, 1)? else { return Ok(None); };

        let dop_type = fb_u8_field(data, dop, 0, 0)?;
        let dop_specific_type = fb_u8_field(data, dop, 3, 0)?;

        // DOPType::STRUCTURE = 8 and SpecificDOPData::Structure = 7.
        // Accept either discriminator as sufficient evidence. This makes the
        // reader robust against MDD writers that omit one discriminator while
        // still storing the Structure union payload correctly.
        if dop_type == 8 || dop_specific_type == 7 {
            if let Some(structure) = fb_table_field(data, dop, 4)? {
                if let Some(bytes) = fixed_structure_bytes(data, structure)? {
                    return Ok(Some(bytes));
                }
            }
        }
    }

    Ok(None)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

fn parse_request_params(data: &[u8], request: usize) -> Result<Vec<UdsMddParameter>, String> {
    let mut out = Vec::new();
    for param in fb_vector_tables(data, request, 0)? {
        match parse_param(data, param) {
            Ok(parsed) => out.push(parsed),
            Err(_) => {
                // Keep the service usable even when an optional complex DOP is
                // not supported by this lightweight reader.
                let name = fb_string_field(data, param, 2).ok().flatten()
                    .unwrap_or_else(|| "Runtime parameter".to_string());
                let kind = fb_u8_field(data, param, 1, 0).unwrap_or(255);
                let mut fallback = UdsMddParameter {
                    name,
                    param_type: param_type_name(kind).to_string(),
                    byte_position: fb_u32_field(data, param, 6).ok().flatten(),
                    bit_position: fb_u32_field(data, param, 7).ok().flatten(),
                    fixed: false, value_hex: None, default_value_hex: None, bit_length: None, byte_length: None,
                    data_type: None, min_value: None, max_value: None, unit: None,
                    choices: Vec::new(), children: Vec::new(), input_hint: Some(
                        "Runtime value required. This parameter uses an MDD construct that could not be fully decoded; consult the ECU/ODX definition for the exact value and length.".to_string()
                    ),
                };
                out.push(fallback);
            }
        }
    }
    Ok(out)
}

fn parse_response_params(data: &[u8], response: usize) -> Result<Vec<UdsMddParameter>, String> {
    let mut out = Vec::new();
    // Response.params is field 1 (field 0 is ResponseType).
    for param in fb_vector_tables(data, response, 1)? {
        match parse_param(data, param) {
            Ok(parsed) => out.push(parsed),
            Err(_) => {
                let name = fb_string_field(data, param, 2).ok().flatten()
                    .unwrap_or_else(|| "Response parameter".to_string());
                let kind = fb_u8_field(data, param, 1, 0).unwrap_or(255);
                out.push(UdsMddParameter {
                    name,
                    param_type: param_type_name(kind).to_string(),
                    byte_position: fb_u32_field(data, param, 6).ok().flatten(),
                    bit_position: fb_u32_field(data, param, 7).ok().flatten(),
                    fixed: false,
                    value_hex: None,
                    default_value_hex: None,
                    bit_length: None,
                    byte_length: None,
                    data_type: None,
                    min_value: None,
                    max_value: None,
                    unit: None,
                    choices: Vec::new(),
                    children: Vec::new(),
                    input_hint: Some("Response parameter could not be fully decoded by the lightweight MDD reader.".to_string()),
                });
            }
        }
    }
    Ok(out)
}

fn parse_service(data: &[u8], service: usize, id: String, source_layer: String, source_ecu: String) -> Result<UdsMddService, String> {
    let diag_comm = fb_table_field(data, service, 0)?;
    let name = match diag_comm {
        Some(comm) => fb_string_field(data, comm, 0)?.unwrap_or_else(|| "Unnamed service".to_string()),
        None => "Unnamed service".to_string(),
    };
    let long_name = match diag_comm {
        Some(comm) => match fb_table_field(data, comm, 1)? {
            Some(long_name_table) => fb_string_field(data, long_name_table, 0)?,
            None => None,
        },
        None => None,
    };
    let parameters = match fb_table_field(data, service, 1)? {
        Some(request) => parse_request_params(data, request)?,
        None => Vec::new(),
    };
    let sid_hex = parameters.iter()
        .find(|p| p.name.eq_ignore_ascii_case("SID") && p.fixed)
        .and_then(|p| p.value_hex.clone());
    let positive_responses = fb_vector_tables(data, service, 2).unwrap_or_default();
    let positive_parameters = positive_responses
        .first()
        .and_then(|response| parse_response_params(data, *response).ok())
        .unwrap_or_default();
    let negative_responses = fb_vector_tables(data, service, 3).unwrap_or_default();
    let negative_parameters = negative_responses
        .first()
        .and_then(|response| parse_response_params(data, *response).ok())
        .unwrap_or_default();

    let positive_sid_hex = positive_parameters.iter()
        .find(|p| (p.name.eq_ignore_ascii_case("SIDPR") || p.name.eq_ignore_ascii_case("SID")) && p.fixed)
        .and_then(|p| p.value_hex.clone())
        .or_else(|| sid_hex.as_ref().and_then(|sid| {
            u8::from_str_radix(sid, 16).ok().map(|v| format!("{:02X}", v.wrapping_add(0x40)))
        }));
    Ok(UdsMddService {
        id, name, long_name, source_layer, source_ecu, sid_hex, positive_sid_hex, parameters,
        positive_parameters, negative_parameters,
    })
}

fn collect_layer_services(
    data: &[u8],
    layer: usize,
    prefix: &str,
    source_ecu: &str,
    out: &mut Vec<UdsMddService>,
) -> Result<(), String> {
    let layer_name = fb_string_field(data, layer, 0)?.unwrap_or_else(|| "Unnamed layer".to_string());
    for (index, service) in fb_vector_tables(data, layer, 4)?.into_iter().enumerate() {
        out.push(parse_service(
            data,
            service,
            format!("{prefix}/service/{index}"),
            layer_name.clone(),
            source_ecu.to_string(),
        )?);
    }
    Ok(())
}

fn collect_parent_services(
    data: &[u8],
    parent_ref: usize,
    prefix: &str,
    source_ecu: &str,
    out: &mut Vec<UdsMddService>,
) -> Result<(), String> {
    // ParentRefType union: NONE=0, Variant=1, Protocol=2, FunctionalGroup=3,
    // TableDop=4, EcuSharedData=5. We use inherited Variant and EcuSharedData
    // services in this first version.
    let ref_type = fb_u8_field(data, parent_ref, 0, 0)?;
    let Some(reference) = fb_table_field(data, parent_ref, 1)? else { return Ok(()); };
    match ref_type {
        1 => {
            if let Some(layer) = fb_table_field(data, reference, 0)? {
                collect_layer_services(data, layer, &format!("{prefix}/variant-parent"), source_ecu, out)?;
            }
        }
        5 => {
            if let Some(layer) = fb_table_field(data, reference, 0)? {
                collect_layer_services(data, layer, &format!("{prefix}/shared-parent"), source_ecu, out)?;
            }
        }
        _ => {}
    }
    Ok(())
}



// Generic transport helpers kept for future read/write transports.
trait DiagnosticTransport: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> DiagnosticTransport for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

#[allow(dead_code)]
async fn transport_write<T: DiagnosticTransport>(transport: &mut T, bytes: &[u8]) -> Result<(), String> {
    transport
        .write_all(bytes)
        .await
        .map_err(|e| format!("Transport write failed: {e}"))?;
    transport
        .flush()
        .await
        .map_err(|e| format!("Transport flush failed: {e}"))?;
    Ok(())
}

#[allow(dead_code)]
async fn transport_read_to_end<T: DiagnosticTransport>(transport: &mut T) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    transport
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| format!("Transport read failed: {e}"))?;
    Ok(bytes)
}

// The UDS writer trait separates the Builder/Sequencer from the final target.
// Current implementation writes .bin files. Another team can implement the same
// trait for sockets, DoIP, CAN, shared memory, or any other transport.
struct UdsFrameWriteContext<'a> {
    bytes: &'a [u8],
    target_ecu: Option<&'a str>,
}

trait UdsFrameWriter {
    fn write_frame(&mut self, frame_index: usize, frame: &UdsFrameWriteContext<'_>) -> Result<PathBuf, String>;
}

struct SingleBinFrameWriter {
    output_path: PathBuf,
}

impl SingleBinFrameWriter {
    fn new(output_path: PathBuf) -> Self {
        Self { output_path }
    }
}

impl UdsFrameWriter for SingleBinFrameWriter {
    fn write_frame(&mut self, _frame_index: usize, frame: &UdsFrameWriteContext<'_>) -> Result<PathBuf, String> {
        if let Some(parent) = self.output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create '{}': {e}", parent.display()))?;
        }
        fs::write(&self.output_path, frame.bytes)
            .map_err(|e| format!("Could not write '{}': {e}", self.output_path.display()))?;
        Ok(self.output_path.clone())
    }
}

struct BinSequenceFrameWriter {
    output_dir: PathBuf,
}

impl BinSequenceFrameWriter {
    fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }
}

impl UdsFrameWriter for BinSequenceFrameWriter {
    fn write_frame(&mut self, frame_index: usize, frame: &UdsFrameWriteContext<'_>) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.output_dir)
            .map_err(|e| format!("Could not create '{}': {e}", self.output_dir.display()))?;
        let path = self.output_dir.join(format!("frame_{:03}.bin", frame_index + 1));
        fs::write(&path, frame.bytes)
            .map_err(|e| format!("Could not write '{}': {e}", path.display()))?;
        Ok(path)
    }
}

fn write_single_uds_frame<W: UdsFrameWriter>(
    writer: &mut W,
    frame: &[u8],
    target_ecu: Option<&str>,
) -> Result<PathBuf, String> {
    writer.write_frame(0, &UdsFrameWriteContext { bytes: frame, target_ecu })
}

fn write_uds_sequence<W: UdsFrameWriter>(writer: &mut W, frames: &[Vec<u8>]) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        let context = UdsFrameWriteContext { bytes: frame, target_ecu: None };
        written.push(writer.write_frame(index, &context)?);
    }
    Ok(written)
}

fn parse_hex_frame_text(frame: &str) -> Result<Vec<u8>, String> {
    let clean: String = frame
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ',' | ';' | ':' | '_' | '-'))
        .collect::<String>()
        .replace("0x", "")
        .replace("0X", "");

    if clean.is_empty() {
        return Err("A sequence contains an empty frame.".to_string());
    }
    if clean.len() % 2 != 0 {
        return Err(format!("Invalid hexadecimal frame '{frame}': odd number of digits."));
    }
    if !clean.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("Invalid hexadecimal frame '{frame}'."));
    }

    (0..clean.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&clean[index..index + 2], 16)
                .map_err(|_| format!("Invalid hexadecimal frame '{frame}'."))
        })
        .collect()
}

#[derive(Serialize, Deserialize, Clone)]
struct SequenceState {
    format_version: u16,
    active_sequence_id: Option<String>,
    sequences: Vec<SequenceStateItem>,
}

#[derive(Serialize, Deserialize, Clone)]
struct SequenceStateItem {
    id: String,
    name: String,
    frames: Vec<SequenceStateFrame>,
}

#[derive(Serialize, Deserialize, Clone)]
struct SequenceStateFrame {
    id: String,
    frame: String,
    label: String,
    source: String,
    timeout_ms: u64,
    delay_ms: u64,
    runtime_ms: u64,
    stop_on_nrc: bool,
    continue_on_failure: bool,
    expected_positive_sid: Option<String>,
    condition_type: String,
    condition_value: Option<String>,
}

fn sequence_state_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Could not resolve config dir: {e}"))?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create config dir: {e}"))?;
    Ok(dir.join("uds_sequences.bin"))
}

#[tauri::command]
async fn save_uds_sequence_state(
    app: tauri::AppHandle,
    mut state: SequenceState,
) -> Result<(), String> {
    state.format_version = 1;
    let encoded = bincode::serialize(&state)
        .map_err(|e| format!("Could not serialize sequence state: {e}"))?;
    let path = sequence_state_path(&app)?;
    tokio::fs::write(&path, encoded)
        .await
        .map_err(|e| format!("Could not save sequence state '{}': {e}", path.display()))
}

#[tauri::command]
async fn load_uds_sequence_state(app: tauri::AppHandle) -> Result<Option<SequenceState>, String> {
    let path = sequence_state_path(&app)?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("Could not read sequence state '{}': {e}", path.display()))?;
    let state: SequenceState = bincode::deserialize(&bytes)
        .map_err(|e| format!("Could not deserialize sequence state: {e}"))?;
    if state.format_version != 1 {
        return Err(format!("Unsupported sequence state version: {}", state.format_version));
    }
    Ok(Some(state))
}

#[derive(Deserialize, Clone)]
struct SequenceExportInput {
    name: String,
    frames: Vec<SequenceExportFrameInput>,
}

#[derive(Deserialize, Clone)]
struct SequenceExportFrameInput {
    frame: String,
}

fn safe_folder_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.is_empty() { fallback.to_string() } else { cleaned }
}

#[derive(Serialize)]
struct WriteRequestBinResult {
    path: String,
    target_ecu: Option<String>,
    source_pdx: Option<String>,
}

#[tauri::command]
async fn write_uds_request_bin(
    app: tauri::AppHandle,
    frame: String,
    target_ecu: Option<String>,
    source_pdx: Option<String>,
) -> Result<Option<WriteRequestBinResult>, String> {
    let request = parse_hex_frame_text(&frame)?;
    let clean_target = target_ecu.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let suggested = clean_target
        .map(|ecu| format!("{}_uds_request.bin", safe_folder_name(ecu, "ecu")))
        .unwrap_or_else(|| "uds_request.bin".to_string());
    let file = app
        .dialog()
        .file()
        .add_filter("UDS binary frame", &["bin"])
        .set_file_name(&suggested)
        .blocking_save_file();

    let Some(file) = file else { return Ok(None); };
    let path = PathBuf::from(file.to_string());
    let mut writer = SingleBinFrameWriter::new(path);
    let written = write_single_uds_frame(&mut writer, &request, clean_target)?;
    Ok(Some(WriteRequestBinResult {
        path: written.to_string_lossy().to_string(),
        target_ecu: clean_target.map(str::to_string),
        source_pdx,
    }))
}


// ======================================================
// Live UDS over DoIP transport — doip_rw_tokio 0.1.1
// ======================================================
//
// Important separation of responsibilities:
//   * uds_mdd_lib / Builder creates the raw UDS payload (example: 22 F1 88).
//   * doip_rw_tokio owns the DoIP/TCP transport details.
//   * This command only translates UI values into the library API and returns
//     the UDS response bytes to the frontend.
//
// We intentionally do NOT build 0x8001 / ACK / NACK frames by hand anymore.

fn parse_logical_address(raw: &str, label: &str) -> Result<u16, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(format!("{label} is empty."));
    }
    let parsed = if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16)
    } else {
        value.parse::<u16>()
    };
    parsed.map_err(|_| format!("Invalid {label} '{value}'. Use for example 0x0E80."))
}

fn format_hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}


fn append_uds_transport_log(app: &tauri::AppHandle, message: &str) {
    let Ok(log_path) = project_log_path(app, "uds_transport.log") else { return; };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis())
        .unwrap_or(0);
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(file, "[{stamp}] {message}");
    }
}

#[tauri::command]
fn write_uds_transport_log(app: tauri::AppHandle, message: String) -> Result<String, String> {
    append_uds_transport_log(&app, &message);
    Ok(project_log_path(&app, "uds_transport.log")?.to_string_lossy().to_string())
}

#[tauri::command]
fn get_uds_transport_log_path(app: tauri::AppHandle) -> Result<String, String> {
    Ok(project_log_path(&app, "uds_transport.log")?.to_string_lossy().to_string())
}

#[derive(Serialize)]
struct SendUdsDoipResult {
    response: String,
    response_bytes: Vec<u8>,
    source_address: String,
    target_address: String,
    transport: String,
    acknowledgement: String,
}

#[tauri::command]
async fn send_uds_request_doip(
    app: tauri::AppHandle,
    frame: String,
    host: String,
    port: u16,
    source_address: String,
    target_address: String,
    timeout_ms: Option<u64>,
    log_context: Option<String>,
) -> Result<SendUdsDoipResult, String> {
    let context = log_context
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "UDS send".to_string());
    append_uds_transport_log(
        &app,
        &format!(
            "BEGIN | {context} | request={} | server={}:{} | tester={} | target={}",
            frame.trim(),
            host.trim(),
            port,
            source_address.trim(),
            target_address.trim()
        ),
    );

    let result = send_uds_request_doip_inner(
        frame,
        host,
        port,
        source_address,
        target_address,
        timeout_ms,
    )
    .await;

    match &result {
        Ok(reply) => append_uds_transport_log(
            &app,
            &format!(
                "END OK | {context} | response={} | ack={} | transport={}",
                reply.response, reply.acknowledgement, reply.transport
            ),
        ),
        Err(error) => append_uds_transport_log(
            &app,
            &format!("END ERROR | {context} | {error}"),
        ),
    }

    result
}

async fn send_uds_request_doip_inner(
    frame: String,
    host: String,
    port: u16,
    source_address: String,
    target_address: String,
    timeout_ms: Option<u64>,
) -> Result<SendUdsDoipResult, String> {
    let uds = parse_hex_frame_text(&frame)?;
    if uds.is_empty() {
        return Err("The generated UDS request is empty.".to_string());
    }

    let source = parse_logical_address(&source_address, "tester logical address")?;
    let target = parse_logical_address(&target_address, "ECU logical address")?;
    let timeout_ms = timeout_ms.unwrap_or(3000).clamp(100, 60_000);
    let host = host.trim();
    if host.is_empty() {
        return Err("DoIP server IP/host is empty.".to_string());
    }

    // doip_rw represents a DoIP logical address as a u16.
    let tester_la = source;
    let ecu_la = target;

    // Resolve both IPv4 and IPv6 host names. The library expects SocketAddr.
    let mut resolved = lookup_host((host, port))
        .await
        .map_err(|e| format!("Could not resolve DoIP server {host}:{port}: {e}"))?;
    let remote_addr = resolved
        .next()
        .ok_or_else(|| format!("No IP address found for DoIP server {host}:{port}."))?;
    let local_addr = if remote_addr.is_ipv4() {
        "0.0.0.0:0".parse().expect("valid IPv4 wildcard")
    } else {
        "[::]:0".parse().expect("valid IPv6 wildcard")
    };

    // Create the external-tester DoIP/TCP connection through doip_rw_tokio.
    // The crate owns the asynchronous TCP/DoIP connection state and timings.
    let connect_future = DoIpTcpConnection::connect_doip_tcp(
        local_addr,
        remote_addr,
        tester_la,
        Timings::default(),
    );
    let mut connection = timeout(Duration::from_millis(timeout_ms), connect_future)
        .await
        .map_err(|_| format!("Timed out connecting to DoIP server {remote_addr}."))?
        .map_err(|e| format!("doip_rw_tokio could not connect to {remote_addr}: {e:?}"))?;

    // Send the UDS payload. The library creates the DoIP DiagnosticMessage,
    // waits for the DoIP diagnostic ACK/NACK and automatically handles an
    // AliveCheck request while waiting when `true` is passed below.
    let ack = timeout(
        Duration::from_millis(timeout_ms),
        send_uds(
            &mut connection,
            ecu_la,
            UdsBuffer::Owned(uds.clone()),
            |_payload_type, size| vec![0u8; size],
            Duration::from_millis(timeout_ms),
            true,
        ),
    )
    .await
    .map_err(|_| format!("Timed out waiting for the DoIP acknowledgement after {timeout_ms} ms."))?
    .map_err(|e| format!("DoIP diagnostic send failed: {e:?}"))?;

    let acknowledgement = match &ack {
        DoIpTcpMessage::DiagnosticMessagePositiveAck(_) => "positive ACK".to_string(),
        DoIpTcpMessage::DiagnosticMessageNegativeAck(nack) => {
            return Err(format!("DoIP server returned a diagnostic NACK: {nack:?}"));
        }
        other => format!("{other:?}"),
    };

    // send_uds() returns the transport ACK. The actual UDS response is the
    // next DiagnosticMessage on the same DoIP connection.
    for _ in 0..8 {
        let received = timeout(
            Duration::from_millis(timeout_ms),
            connection.receive_message(|_payload_type, size| vec![0u8; size]),
        )
        .await
        .map_err(|_| format!("Timed out after {timeout_ms} ms waiting for the UDS response."))?
        .map_err(|e| format!("Could not receive DoIP response: {e:?}"))?;

        match received {
            DoIpTcpMessage::DiagnosticMessage(message) => {
                let response = message.user_data.get_ref().to_vec();
                return Ok(SendUdsDoipResult {
                    response: format_hex_bytes(&response),
                    response_bytes: response,
                    source_address: format!("0x{source:04X}"),
                    target_address: format!("0x{target:04X}"),
                    transport: "doip_rw_tokio 0.1.1".to_string(),
                    acknowledgement,
                });
            }
            DoIpTcpMessage::DiagnosticMessageNegativeAck(nack) => {
                return Err(format!("DoIP server returned a diagnostic NACK while waiting for the response: {nack:?}"));
            }
            // Alive checks and other legal DoIP control messages are handled
            // or ignored here while we wait for the actual diagnostic reply.
            _ => continue,
        }
    }

    Err("No UDS DiagnosticMessage was received from the DoIP server.".to_string())
}

#[tauri::command]
async fn export_uds_sequence(
    app: tauri::AppHandle,
    sequence: SequenceExportInput,
) -> Result<Option<String>, String> {
    if sequence.frames.is_empty() {
        return Err("The selected sequence does not contain any frames.".to_string());
    }

    let folder = app.dialog().file().blocking_pick_folder();
    let Some(folder) = folder else { return Ok(None); };
    let root = PathBuf::from(folder.to_string());
    let sequence_dir = root.join(safe_folder_name(&sequence.name, "Sequence"));
    let frames = sequence
        .frames
        .iter()
        .map(|frame| parse_hex_frame_text(&frame.frame))
        .collect::<Result<Vec<_>, _>>()?;

    let mut writer = BinSequenceFrameWriter::new(sequence_dir.clone());
    write_uds_sequence(&mut writer, &frames)?;

    Ok(Some(sequence_dir.to_string_lossy().to_string()))
}

// UDS Builder cache. Parsing an MDD can create a large diagnostic tree.
// Keep that tree in the Rust process and send only small, requested slices
// to the WebView. This avoids serializing every service/parameter at once.
static UDS_MDD_SERVICE_CACHE: OnceLock<Mutex<HashMap<String, Arc<Vec<UdsMddService>>>>> = OnceLock::new();

fn uds_mdd_service_cache() -> &'static Mutex<HashMap<String, Arc<Vec<UdsMddService>>>> {
    UDS_MDD_SERVICE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse_uds_services_from_mdd(path: &str) -> Result<Vec<UdsMddService>, String> {
    if path.is_empty() { return Err("No MDD file was selected.".to_string()); }
    if !Path::new(path).exists() { return Err(format!("MDD file not found: {path}")); }

    let data = extract_flatbuffer_from_mdd(path)?;
    let root = read_u32_le(&data, 0)? as usize;
    if root >= data.len() { return Err("Invalid FlatBuffer root".to_string()); }

    let mut services = Vec::new();
    for (variant_index, variant) in fb_vector_tables(&data, root, 5)?.into_iter().enumerate() {
        let source_ecu = if let Some(layer) = fb_table_field(&data, variant, 0)? {
            fb_string_field(&data, layer, 0)?
                .unwrap_or_else(|| format!("variant/{variant_index}"))
        } else {
            format!("variant/{variant_index}")
        };

        // Every service collected while walking this variant is tagged with the
        // child ECU/variant that owns the effective diagnostic view. This
        // includes inherited parent/shared services. The original layer name is
        // still kept separately in source_layer for traceability.
        if let Some(layer) = fb_table_field(&data, variant, 0)? {
            collect_layer_services(
                &data,
                layer,
                &format!("variant/{variant_index}"),
                &source_ecu,
                &mut services,
            )?;
        }
        for (parent_index, parent) in fb_vector_tables(&data, variant, 3)?.into_iter().enumerate() {
            collect_parent_services(
                &data,
                parent,
                &format!("variant/{variant_index}/parent/{parent_index}"),
                &source_ecu,
                &mut services,
            )?;
        }
    }

    let mut seen = std::collections::HashSet::new();
    services.retain(|svc| seen.insert((svc.source_ecu.clone(), svc.source_layer.clone(), svc.name.clone(), svc.sid_hex.clone())));

    if services.is_empty() {
        return Err("The MDD was decoded successfully, but no diagnostic services were found in its variants.".to_string());
    }
    Ok(services)
}

fn cached_uds_services(path: &str) -> Result<Arc<Vec<UdsMddService>>, String> {
    let clean = clean_path(path);
    {
        let cache = uds_mdd_service_cache()
            .lock()
            .map_err(|_| "UDS MDD cache lock was poisoned.".to_string())?;
        if let Some(services) = cache.get(&clean) {
            return Ok(Arc::clone(services));
        }
    }

    let parsed = Arc::new(parse_uds_services_from_mdd(&clean)?);
    let mut cache = uds_mdd_service_cache()
        .lock()
        .map_err(|_| "UDS MDD cache lock was poisoned.".to_string())?;
    cache.insert(clean, Arc::clone(&parsed));
    Ok(parsed)
}

fn write_builder_log(app: &tauri::AppHandle, message: &str) {
    let Ok(log_path) = project_log_path(app, "uds_builder.log") else { return; };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or(0);
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(file, "[{stamp}] {message}");
    }
}

#[derive(Serialize, Clone)]
struct UdsBuilderFamilySummary {
    sid_hex: String,
    name: String,
    option_count: usize,
}

#[derive(Serialize, Clone)]
struct UdsBuilderMddSummary {
    service_count: usize,
    family_count: usize,
    families: Vec<UdsBuilderFamilySummary>,
    ecu_names: Vec<String>,
}

fn uds_ecu_names_for_services(services: &[UdsMddService]) -> Vec<String> {
    services
        .iter()
        .map(|service| service.source_ecu.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn persist_mdd_ecu_names(
    app: &tauri::AppHandle,
    mdd_path: &str,
    ecu_names: &[String],
) -> Result<(), String> {
    if ecu_names.is_empty() {
        return Ok(());
    }

    let clean_mdd = clean_path(mdd_path);
    let mut library = load_diagnostic_library(app)?;
    let mut changed = false;

    for source in &mut library.sources {
        let stored_matches = clean_path(&source.stored_path) == clean_mdd;
        let generated_matches = source
            .generated_mdd_path
            .as_deref()
            .map(clean_path)
            .as_deref()
            == Some(clean_mdd.as_str());

        if (stored_matches || generated_matches) && source.ecu_names != ecu_names {
            source.ecu_names = ecu_names.to_vec();
            changed = true;
        }
    }

    if changed {
        save_diagnostic_library(app, &library)?;
    }
    Ok(())
}

fn uds_family_name_rust(sid: &str) -> &'static str {
    match sid {
        "10" => "DiagnosticSessionControl",
        "11" => "ECUReset",
        "14" => "ClearDiagnosticInformation",
        "19" => "ReadDTCInformation",
        "22" => "ReadDataByIdentifier",
        "23" => "ReadMemoryByAddress",
        "27" => "SecurityAccess",
        "28" => "CommunicationControl",
        "2E" => "WriteDataByIdentifier",
        "2F" => "InputOutputControlByIdentifier",
        "31" => "RoutineControl",
        "34" => "RequestDownload",
        "35" => "RequestUpload",
        "36" => "TransferData",
        "37" => "RequestTransferExit",
        "3D" => "WriteMemoryByAddress",
        "3E" => "TesterPresent",
        "85" => "ControlDTCSetting",
        _ => "UDS service",
    }
}

#[tauri::command]
fn load_uds_builder_mdd(app: tauri::AppHandle, path: String) -> Result<UdsBuilderMddSummary, String> {
    let path = clean_path(&path);
    write_builder_log(&app, &format!("Builder load requested: {path}"));
    if let Ok(meta) = fs::metadata(&path) {
        write_builder_log(&app, &format!("MDD file size: {} bytes", meta.len()));
    }

    let services = match cached_uds_services(&path) {
        Ok(value) => value,
        Err(error) => {
            write_builder_log(&app, &format!("Builder MDD parse failed: {error}"));
            return Err(error);
        }
    };
    write_builder_log(&app, &format!("Builder MDD parsed: {} definitions", services.len()));

    let ecu_names = uds_ecu_names_for_services(services.as_ref());
    write_builder_log(
        &app,
        &format!(
            "Builder MDD ECU variants: {}",
            if ecu_names.is_empty() { "none".to_string() } else { ecu_names.join(", ") }
        ),
    );
    if let Err(error) = persist_mdd_ecu_names(&app, &path, &ecu_names) {
        write_builder_log(&app, &format!("Could not persist MDD ECU names: {error}"));
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for service in services.iter() {
        if let Some(sid) = service.sid_hex.as_ref() {
            *counts.entry(sid.clone()).or_insert(0) += 1;
        }
    }

    let mut families = counts
        .into_iter()
        .map(|(sid_hex, option_count)| UdsBuilderFamilySummary {
            name: uds_family_name_rust(&sid_hex).to_string(),
            sid_hex,
            option_count,
        })
        .collect::<Vec<_>>();
    families.sort_by_key(|family| u8::from_str_radix(&family.sid_hex, 16).unwrap_or(0xFF));

    write_builder_log(&app, &format!("Returning lightweight Builder summary: {} families", families.len()));
    Ok(UdsBuilderMddSummary {
        service_count: services.len(),
        family_count: families.len(),
        families,
        ecu_names,
    })
}

fn uds_builder_summary_for_services(services: &[UdsMddService]) -> UdsBuilderMddSummary {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for service in services {
        if let Some(sid) = service.sid_hex.as_ref() {
            *counts.entry(sid.clone()).or_insert(0) += 1;
        }
    }

    let mut families = counts
        .into_iter()
        .map(|(sid_hex, option_count)| UdsBuilderFamilySummary {
            name: uds_family_name_rust(&sid_hex).to_string(),
            sid_hex,
            option_count,
        })
        .collect::<Vec<_>>();
    families.sort_by_key(|family| u8::from_str_radix(&family.sid_hex, 16).unwrap_or(0xFF));

    UdsBuilderMddSummary {
        service_count: services.len(),
        family_count: families.len(),
        families,
        ecu_names: uds_ecu_names_for_services(services),
    }
}

#[tauri::command]
fn get_uds_builder_ecu_summary(
    app: tauri::AppHandle,
    path: String,
    ecu_name: String,
) -> Result<UdsBuilderMddSummary, String> {
    let path = clean_path(&path);
    let ecu = ecu_name.trim();
    if ecu.is_empty() {
        return Err("Choose an ECU before loading its diagnostic services.".to_string());
    }
    let services = cached_uds_services(&path)?;
    let filtered = services
        .iter()
        .filter(|service| service.source_ecu.eq_ignore_ascii_case(ecu))
        .cloned()
        .collect::<Vec<_>>();

    write_builder_log(
        &app,
        &format!("ECU service summary requested: {ecu} -> {} definition(s)", filtered.len()),
    );

    if filtered.is_empty() {
        let known = services
            .iter()
            .map(|service| service.source_ecu.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        return Err(format!(
            "No diagnostic service was found in the MDD for ECU '{ecu}'. MDD variants: {}",
            if known.is_empty() { "none".to_string() } else { known.join(", ") }
        ));
    }

    Ok(uds_builder_summary_for_services(&filtered))
}

#[tauri::command]
fn get_uds_builder_family_options(
    app: tauri::AppHandle,
    path: String,
    sid_hex: String,
    ecu_name: String,
) -> Result<Vec<UdsMddService>, String> {
    let path = clean_path(&path);
    let sid = sid_hex.trim().trim_start_matches("0x").to_uppercase();
    let ecu = ecu_name.trim();
    write_builder_log(&app, &format!("Builder family requested: ECU {ecu}, SID 0x{sid}"));
    let services = cached_uds_services(&path)?;
    let result = services
        .iter()
        .filter(|service| service.source_ecu.eq_ignore_ascii_case(ecu))
        .filter(|service| service.sid_hex.as_deref() == Some(sid.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    write_builder_log(&app, &format!("Returning {} option(s) for ECU {ecu}, SID 0x{sid}", result.len()));
    Ok(result)
}

#[tauri::command]
fn get_uds_builder_log_path(app: tauri::AppHandle) -> Result<String, String> {
    Ok(project_log_path(&app, "uds_builder.log")?.to_string_lossy().to_string())
}

#[tauri::command]
fn get_uds_services_from_mdd(path: String) -> Result<Vec<UdsMddService>, String> {
    let path = clean_path(&path);
    Ok(cached_uds_services(&path)?.as_ref().clone())
}


#[tauri::command]
fn get_uds_services_for_ecu(path: String, ecu_name: String) -> Result<Vec<UdsMddService>, String> {
    let path = clean_path(&path);
    let ecu = ecu_name.trim();
    if ecu.is_empty() {
        return Err("Choose an ECU first.".to_string());
    }
    let services = cached_uds_services(&path)?;
    Ok(services
        .iter()
        .filter(|service| service.source_ecu.eq_ignore_ascii_case(ecu))
        .cloned()
        .collect())
}


#[derive(Debug, Clone, Serialize)]
struct DtcDefinition {
    trouble_code: u32,
    code_hex: String,
    display_trouble_code: Option<String>,
    short_name: Option<String>,
    description: Option<String>,
    level: Option<u32>,
    is_temporary: Option<bool>,
}

fn json_to_u32(value: &serde_json::Value) -> Option<u32> {
    if let Some(n) = value.as_u64() {
        return (n <= u32::MAX as u64).then_some(n as u32);
    }
    let text = value.as_str()?.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        text.parse::<u32>().ok()
    }
}

fn object_text_value(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    let value = map.get(key)?;
    if let Some(text) = value.as_str() {
        let text = text.trim();
        return (!text.is_empty()).then(|| text.to_string());
    }
    if let Some(obj) = value.as_object() {
        if let Some(text) = obj.get("value").and_then(|v| v.as_str()) {
            let text = text.trim();
            return (!text.is_empty()).then(|| text.to_string());
        }
    }
    None
}

fn collect_dtc_definitions(value: &serde_json::Value, out: &mut Vec<DtcDefinition>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(code) = map.get("trouble_code").and_then(json_to_u32) {
                let code_hex = format!("{code:06X}");
                out.push(DtcDefinition {
                    trouble_code: code,
                    code_hex,
                    display_trouble_code: object_text_value(map, "display_trouble_code"),
                    short_name: object_text_value(map, "short_name"),
                    description: object_text_value(map, "text")
                        .or_else(|| object_text_value(map, "description"))
                        .or_else(|| object_text_value(map, "long_name")),
                    level: map.get("level").and_then(json_to_u32),
                    is_temporary: map.get("is_temporary").and_then(|v| v.as_bool()),
                });
            }
            for child in map.values() {
                collect_dtc_definitions(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_dtc_definitions(item, out);
            }
        }
        _ => {}
    }
}

#[tauri::command]
async fn get_dtc_definitions_from_mdd(path: String) -> Result<Vec<DtcDefinition>, String> {
    let path = clean_path(&path);
    if path.is_empty() {
        return Err("No MDD file was selected.".to_string());
    }
    let input = PathBuf::from(&path);
    if !input.exists() {
        return Err(format!("MDD file not found: {}", input.display()));
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Could not create DTC parser timestamp: {e}"))?
        .as_millis();
    let temp_json = std::env::temp_dir().join(format!(
        "odx_desktop_dtc_{}_{}.json",
        std::process::id(),
        stamp
    ));

    let input_for_task = input.clone();
    let json_for_task = temp_json.clone();
    let parse_result = tauri::async_runtime::spawn_blocking(move || {
        odx_converter::parser::export_mdd_to_json(&input_for_task, &json_for_task)
    })
    .await
    .map_err(|e| format!("DTC MDD parser task failed: {e}"))?;

    if let Err(e) = parse_result {
        let _ = fs::remove_file(&temp_json);
        return Err(format!("Failed to parse MDD for DTC definitions: {e:#}"));
    }

    let raw = fs::read_to_string(&temp_json)
        .map_err(|e| format!("Failed to read temporary MDD JSON: {e}"))?;
    let _ = fs::remove_file(&temp_json);
    let json: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("Failed to decode MDD JSON for DTC definitions: {e}"))?;

    let mut definitions = Vec::new();
    collect_dtc_definitions(&json, &mut definitions);

    let mut seen = std::collections::HashSet::new();
    definitions.retain(|dtc| {
        seen.insert((
            dtc.trouble_code,
            dtc.short_name.clone(),
            dtc.display_trouble_code.clone(),
        ))
    });
    definitions.sort_by_key(|dtc| dtc.trouble_code);
    Ok(definitions)
}

#[tauri::command]
async fn save_json_as(app: tauri::AppHandle, contents: String) -> Result<Option<String>, String> {
    let file = app
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .set_file_name("output.json")
        .blocking_save_file();

    match file {
        Some(path) => {
            let path_buf = PathBuf::from(path.to_string());
            fs::write(&path_buf, contents).map_err(|e| e.to_string())?;
            Ok(Some(path_buf.to_string_lossy().to_string()))
        }
        None => Ok(None),
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            convert_pdx,
            parse_mdd,
            pick_pdx_file,
            pick_mdd_file,
            get_project_output_folder,
            pick_project_output_folder,
            reset_project_output_folder,
            import_diagnostic_sources,
            list_diagnostic_sources,
            remove_diagnostic_source,
            convert_saved_sources,
            load_uds_builder_mdd,
            get_uds_builder_ecu_summary,
            get_uds_builder_family_options,
            get_uds_builder_log_path,
            get_uds_services_from_mdd,
            get_uds_services_for_ecu,
            get_dtc_definitions_from_mdd,
            save_uds_sequence_state,
            load_uds_sequence_state,
            write_uds_request_bin,
            send_uds_request_doip,
            write_uds_transport_log,
            get_uds_transport_log_path,
            export_uds_sequence,
            save_json_as,
            validate_pdx_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
