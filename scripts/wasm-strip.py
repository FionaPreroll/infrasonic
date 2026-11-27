#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Remove custom sections (names, producers, ...) from WebAssembly files, in place.

wasm-bindgen needs the `target_features` section, so the build keeps custom sections and
this script removes them from the final output."""
import sys


def leb(data, i):
    result = shift = 0
    while True:
        b = data[i]
        i += 1
        result |= (b & 0x7F) << shift
        shift += 7
        if b < 0x80:
            return result, i


for path in sys.argv[1:]:
    data = open(path, "rb").read()
    if data[:4] != b"\0asm":
        sys.exit(f"{path}: not WebAssembly")
    out = bytearray(data[:8])
    i = 8
    while i < len(data):
        start = i
        sid = data[i]
        size, body = leb(data, i + 1)
        i = body + size
        if sid != 0:
            out += data[start:i]
    open(path, "wb").write(out)
