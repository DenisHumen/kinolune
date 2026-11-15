#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
#
# List the DLLs a Windows PE file imports (PKG-020), without dumpbin/objdump.
#
#   pe_imports.py <file.dll|file.exe> ...
#
# Prints "<file>\t<imported dll>" per import (normal and delay-load import
# tables). Exits 2 when a file is not a PE image. Used by package.sh for the
# GPL/nonfree gate and the "every import resolves" check.
import struct
import sys


def _rva_to_offset(sections, rva):
    for va, vsize, raw, rsize in sections:
        if va <= rva < va + max(vsize, rsize):
            return rva - va + raw
    return None


def _cstr(data, off):
    end = data.index(b"\0", off)
    return data[off:end].decode("ascii", "replace")


def imports(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:2] != b"MZ":
        raise ValueError("not a PE file")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe : pe + 4] != b"PE\0\0":
        raise ValueError("not a PE file")
    nsec, = struct.unpack_from("<H", data, pe + 6)
    opt_size, = struct.unpack_from("<H", data, pe + 20)
    opt = pe + 24
    magic, = struct.unpack_from("<H", data, opt)
    dd = opt + (112 if magic == 0x20B else 96)
    sec = opt + opt_size
    sections = []
    for i in range(nsec):
        s = sec + 40 * i
        vsize, va, rsize, raw = struct.unpack_from("<IIII", data, s + 8)
        sections.append((va, vsize, raw, rsize))
    names = []
    # Data directory 1: import table (20-byte descriptors, name RVA at +12).
    # Data directory 13: delay-load import table (32-byte, name RVA at +4).
    for index, size, name_at in ((1, 20, 12), (13, 32, 4)):
        rva, _ = struct.unpack_from("<II", data, dd + 8 * index)
        if not rva:
            continue
        off = _rva_to_offset(sections, rva)
        while off is not None and off + size <= len(data):
            entry = data[off : off + size]
            if entry == b"\0" * size:
                break
            name_rva, = struct.unpack_from("<I", entry, name_at)
            name_off = _rva_to_offset(sections, name_rva)
            if name_off is not None:
                names.append(_cstr(data, name_off))
            off += size
    return names


def main(argv):
    status = 0
    for path in argv:
        try:
            for name in imports(path):
                print(f"{path}\t{name}")
        except (ValueError, OSError, struct.error) as e:
            print(f"pe_imports.py: {path}: {e}", file=sys.stderr)
            status = 2
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
