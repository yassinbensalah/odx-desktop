#!/usr/bin/env python3
"""Extract the raw FlatBuffer payload from an MDD file.

Usage:
    python3 flb.py vehicle.mdd
    python3 flb.py vehicle.mdd -o vehicle.bin
"""

import argparse
import lzma
from pathlib import Path

import blackboxprotobuf


def extract_flatbuffer(input_path: Path) -> bytes:
    data = input_path.read_bytes()

    # Remove MDD header.
    if data[:16] == b"MDD version 0   ":
        payload = data[20:]
    else:
        payload = data[4:]

    # Decode protobuf container and extract the compressed FlatBuffer chunk.
    message, _ = blackboxprotobuf.decode_message(payload)
    chunk = message["6"]
    compressed = chunk["8"]

    return lzma.decompress(compressed, format=lzma.FORMAT_ALONE)


def main() -> None:
    parser = argparse.ArgumentParser(description="Extract FlatBuffer .bin from an MDD file")
    parser.add_argument("input", type=Path, help="Input .mdd file")
    parser.add_argument("-o", "--output", type=Path, help="Output .bin file")
    args = parser.parse_args()

    input_path = args.input.expanduser().resolve()
    output_path = args.output.expanduser().resolve() if args.output else input_path.with_suffix(".bin")

    flatbuffer_data = extract_flatbuffer(input_path)
    output_path.write_bytes(flatbuffer_data)

    print(f"Saved: {output_path}")
    print(f"FlatBuffer size: {len(flatbuffer_data)} bytes")


if __name__ == "__main__":
    main()
