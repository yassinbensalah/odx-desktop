# ODX Desktop — Windows Quick Guide

This guide explains how to **build ODX Desktop**, run the **PDX → MDD converter alone**, and manually parse an **MDD → JSON** file.

## 1. Requirements

Use **Windows 10/11 64-bit**.

Install:

- Visual Studio 2022 Build Tools
  - **Desktop development with C++**
  - Windows 10/11 SDK
- Rust: https://rustup.rs/
- Tauri CLI 2
- Microsoft Edge WebView2 Runtime

For the manual MDD parser only, also install:

- Python 3
- FlatBuffers `flatc.exe`

Install the Python dependency:

```powershell
pip install blackboxprotobuf
```

Check Rust and Tauri:

```powershell
rustc --version
cargo --version
cargo tauri --version
```

If Tauri CLI is missing:

```powershell
cargo install tauri-cli --version "^2" --locked
```

---

## 2. Build ODX Desktop

Open PowerShell in the project root:

```powershell
cd "C:\path\to\ODX-Desktop"
```

Check the Rust backend:

```powershell
cd src-tauri
cargo check
cd ..
```

Run the application for testing:

```powershell
cargo tauri dev
```

Build the Windows release:

```powershell
cargo tauri build
```

The main executable is generated under:

```text
src-tauri\target\release\odx-desktop.exe
```

The Windows installer is generated under:

```text
src-tauri\target\release\bundle\nsis\
```

---

## 3. Run the PDX → MDD converter alone

The converter can run without the desktop application.

Go to:

```powershell
cd ".\src-tauri\vendor\odx-converter-rs"
```

Build it:

```powershell
cargo build --release
```

Convert a PDX:

```powershell
.\target\release\odx-converter.exe "C:\Diagnostic\vehicle.pdx"
```

Choose an output folder:

```powershell
.\target\release\odx-converter.exe "C:\Diagnostic\vehicle.pdx" -O "C:\Diagnostic\output"
```

Generate JSON directly if needed:

```powershell
.\target\release\odx-converter.exe "C:\Diagnostic\vehicle.pdx" --json
```

Parse an existing MDD directly:

```powershell
.\target\release\odx-converter.exe --decode "C:\Diagnostic\vehicle.mdd"
```

---

## 4. Manual MDD → JSON with `flb.py` and `flatc`

The manual flow is:

```text
vehicle.mdd
   ↓ flb.py
vehicle.bin
   ↓ flatc
vehicle.json
```

Run `flb.py`:

```powershell
python flb.py "C:\Diagnostic\vehicle.mdd"
```

This extracts the FlatBuffer payload as:

```text
vehicle.bin
```

Then parse the `.bin` file with `flatc` and the FlatBuffers schema:

```powershell
flatc --json --raw-binary .\diagnostic_description.fbs -- "C:\Diagnostic\vehicle.bin"
```

This creates:

```text
vehicle.json
```

If `flatc.exe` is not in `PATH`, use its full path:

```powershell
"C:\Tools\flatc.exe" --json --raw-binary .\diagnostic_description.fbs -- "C:\Diagnostic\vehicle.bin"
```

> Do not run `flatc` directly on the `.mdd` file. First extract the `.bin` payload with `flb.py`.

---

## 5. Quick summary

Build the desktop:

```powershell
cargo tauri build
```

Build and run only the converter:

```powershell
cd ".\src-tauri\vendor\odx-converter-rs"
cargo build --release
.\target\release\odx-converter.exe "C:\Diagnostic\vehicle.pdx"
```

Manual MDD parsing:

```powershell
python flb.py "C:\Diagnostic\vehicle.mdd"
flatc --json --raw-binary .\diagnostic_description.fbs -- "C:\Diagnostic\vehicle.bin"
```
Logs: The application log files (uds_transport.log and uds_builder.log) are saved directly in the selected project output folder.
