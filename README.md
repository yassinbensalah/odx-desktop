# ODX Desktop — Linux Quick Guide

This guide explains how to **build ODX Desktop**, run the **PDX → MDD converter alone**, and manually parse an **MDD → JSON** file.

## 1. Requirements

Recommended: **Ubuntu / Debian 64-bit**.

Install all Linux build dependencies:

```bash
sudo apt update

sudo apt install -y \
  build-essential \
  curl \
  git \
  pkg-config \
  libssl-dev \
  libglib2.0-dev \
  libdbus-1-dev \
  libgtk-3-dev \
  libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  patchelf
```

What they are used for:

- `build-essential` — GCC, linker, `make`, and basic native build tools.
- `curl` — used to install Rust with `rustup`.
- `git` — source-control support and some Cargo dependencies.
- `pkg-config` — lets Rust/native crates find installed Linux libraries.
- `libssl-dev` — OpenSSL headers and libraries.
- `libglib2.0-dev` — GLib development files required by GTK/WebKit components.
- `libdbus-1-dev` — D-Bus development files used by Linux desktop integration.
- `libgtk-3-dev` — GTK 3 development files for the desktop UI stack.
- `libwebkit2gtk-4.1-dev` — WebKitGTK used by Tauri to display the application frontend.
- `libayatana-appindicator3-dev` — Linux tray/app-indicator support used by Tauri.
- `librsvg2-dev` — SVG/icon rendering support.
- `patchelf` — used when packaging Linux binaries/AppImage.

Install Rust if needed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Install Tauri CLI 2:

```bash
cargo install tauri-cli --version "^2" --locked
```

Check:

```bash
rustc --version
cargo --version
cargo tauri --version
```

For the **manual MDD parser only**, also install Python and FlatBuffers:

```bash
sudo apt install -y python3 python3-venv flatbuffers-compiler

python3 -m venv .venv
source .venv/bin/activate
pip install blackboxprotobuf
```

> Node.js/npm are not required because the frontend files are already included in the project.

## 2. Build ODX Desktop

From the project root:

```bash
chmod +x BUILD.sh
./BUILD.sh
```

Or test first:

```bash
cd src-tauri
cargo check
cd ..
cargo tauri dev
```

Generated Linux files are under:

```text
src-tauri/target/release/odx-desktop
src-tauri/target/release/bundle/deb/
src-tauri/target/release/bundle/appimage/
```

## 3. Run the PDX → MDD Converter Alone

The converter can run without the desktop application:

```bash
cd src-tauri/vendor/odx-converter-rs
cargo build --release
```

Convert a PDX:

```bash
./target/release/odx-converter "/path/to/vehicle.pdx"
```

Choose an output folder:

```bash
./target/release/odx-converter "/path/to/vehicle.pdx" -O "/path/to/output"
```

Generate MDD + JSON:

```bash
./target/release/odx-converter "/path/to/vehicle.pdx" --json
```

Decode an existing MDD directly:

```bash
./target/release/odx-converter --decode "/path/to/vehicle.mdd"
```

## 4. Manual MDD → JSON with `flb.py` and `flatc`

Flow:

```text
vehicle.mdd
   ↓ flb.py
vehicle.bin
   ↓ flatc
vehicle.json
```

Go to the converter folder:

```bash
cd src-tauri/vendor/odx-converter-rs
```

Extract the FlatBuffer:

```bash
python3 src/writer/flb.py "/path/to/vehicle.mdd"
```

This creates:

```text
/path/to/vehicle.bin
```

Parse the `.bin` file:

```bash
flatc --json --raw-binary diagnostic_description.fbs -- "/path/to/vehicle.bin"
```

This creates `vehicle.json`.

> Do not give the `.mdd` directly to `flatc`. First extract the `.bin` with `flb.py`.

## 5. PDX Validator on Linux

PDX conversion works natively on Linux.

The **Validate PDX** button additionally requires a native Linux validator:

```text
src-tauri/pdx_validator
```

A Windows `pdx_validator.exe` cannot run natively on Linux.

## 6. Quick Commands

Build desktop:

```bash
cargo tauri build
```

Build converter only:

```bash
cd src-tauri/vendor/odx-converter-rs
cargo build --release
```

Convert PDX:

```bash
./target/release/odx-converter "/path/to/vehicle.pdx"
```

Manual MDD parsing:

```bash
python3 src/writer/flb.py "/path/to/vehicle.mdd"
flatc --json --raw-binary diagnostic_description.fbs -- "/path/to/vehicle.bin"
```
Logs: The application log files (uds_transport.log and uds_builder.log) are saved directly in the selected project output folder.
