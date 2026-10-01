#!/usr/bin/env python3
"""Inspect a GKI boot / init_boot image and list its ramdisk, with no external deps.

Decompresses the ramdisk (gzip or LZ4 legacy, the two formats GKI uses), parses the cpio
archive, lists every entry, and greps the payload for kernel/KMI hints. Useful for working
out the target KMI of an init_boot image, which carries no kernel to read it from.

Usage:  python tools/inspect_ramdisk.py <boot.img|init_boot.img> [--grep PATTERN]
"""

import gzip
import re
import struct
import sys

PAGE = 4096


def lz4_legacy_decompress(data):
    """LZ4 legacy frame: 4-byte magic, then [u32 size][raw LZ4 block] ..."""
    magic = struct.unpack_from("<I", data, 0)[0]
    if magic != 0x184C2102:
        raise ValueError("not LZ4 legacy (magic %#x), expected 0x184C2102" % magic)
    out = bytearray()
    pos = 4
    while pos + 4 <= len(data):
        size = struct.unpack_from("<I", data, pos)[0]
        pos += 4
        if size == 0:
            break
        if size == 0x184C2103:  # end mark written by some tools
            break
        if pos + size > len(data):
            raise ValueError("truncated block at %d (size %d)" % (pos - 4, size))
        out += lz4_block_decode(data[pos:pos + size])
        pos += size
    return bytes(out)


def lz4_block_decode(block):
    """Raw LZ4 block: token sequences of literals + back-references."""
    out = bytearray()
    pos = 0
    n = len(block)
    while pos < n:
        token = block[pos]
        pos += 1

        lit_len = token >> 4
        if lit_len == 15:
            while pos < n:
                extra = block[pos]
                pos += 1
                lit_len += extra
                if extra != 255:
                    break
        out += block[pos:pos + lit_len]
        pos += lit_len
        if pos >= n:
            break

        offset = block[pos] | (block[pos + 1] << 8)
        pos += 2
        if offset == 0:
            raise ValueError("zero match offset")
        match_len = token & 0x0F
        if match_len == 15:
            while pos < n:
                extra = block[pos]
                pos += 1
                match_len += extra
                if extra != 255:
                    break
        match_len += 4

        start = len(out) - offset
        if start < 0:
            raise ValueError("match offset before start of output")
        for i in range(match_len):
            out.append(out[start + i])
    return bytes(out)


def split_image(image):
    """Return (version, kernel, ramdisk, kernel_compressed)."""
    if image[:8] != b"ANDROID!":
        raise SystemExit("not a boot image")
    version = struct.unpack_from("<I", image, 40)[0]
    if version < 3:
        raise SystemExit("legacy header v%d" % version)
    page = 4096
    kernel_size, ramdisk_size = struct.unpack_from("<II", image, 8)

    def align(pos):
        return (pos + page - 1) // page * page

    pos = align(1584 if version >= 4 else 1580)
    kernel = image[pos:pos + kernel_size]
    pos += align(kernel_size)
    ramdisk = image[pos:pos + ramdisk_size]
    return version, kernel, ramdisk


def decompress_ramdisk(raw):
    if raw[:2] == b"\x1f\x8b":
        return "gzip", gzip.decompress(raw)
    if raw[:4] == b"\x02\x21\x4c\x18":
        return "lz4-legacy", lz4_legacy_decompress(raw)
    for name, magic in (("xz", b"\xfd7zXZ"), ("lzma", b"\x5d\x00\x00"), ("zstd", b"\x28\xb5\x2f\xfd"), ("bzip2", b"BZh")):
        if raw.startswith(magic):
            return name, None
    return "unknown(%s)" % raw[:8].hex(), None


def parse_cpio(data):
    """newc (070701) / crc (070702) -> list of (name, mode, payload)."""
    out = []
    pos = 0
    while pos + 110 <= len(data):
        magic = data[pos:pos + 6]
        if magic not in (b"070701", b"070702"):
            break
        fields = [int(data[pos + 6 + i * 8:pos + 14 + i * 8], 16) for i in range(13)]
        mode, filesize, namesize = fields[1], fields[6], fields[11]
        name_start = pos + 110
        name = data[name_start:name_start + namesize - 1].decode("latin1")
        pos = name_start + namesize
        pos += (-pos) % 4
        payload = data[pos:pos + filesize]
        pos += filesize
        pos += (-pos) % 4
        if name == "TRAILER!!!":
            break
        out.append((name, mode, payload))
    return out


def kind_of(mode):
    t = mode & 0o170000
    if t == 0o040000:
        return "dir "
    if t == 0o120000:
        return "link"
    if t == 0o100000:
        return "file"
    return "?   "


def main():
    argv = sys.argv[1:]
    pattern = None
    if "--grep" in argv:
        pattern = argv[argv.index("--grep") + 1]
        argv = [a for a in argv if a != "--grep" and a != pattern]
    path = argv[0]

    image = open(path, "rb").read()
    version, kernel, raw_ramdisk = split_image(image)
    fmt, plain = decompress_ramdisk(raw_ramdisk)

    print("image          : %s (%d bytes)" % (path, len(image)))
    print("header         : v%d, page %d" % (version, PAGE))
    print("kernel         : %d bytes" % len(kernel))
    print("ramdisk        : %d bytes stored, format %s" % (len(raw_ramdisk), fmt))
    if plain is None:
        raise SystemExit("unsupported ramdisk compression: %s" % fmt)
    print("ramdisk plain  : %d bytes" % len(plain))
    entries = parse_cpio(plain)
    print("cpio entries   : %d" % len(entries))
    print()

    print("%-6s %-7s %10s  %s" % ("kind", "mode", "size", "name"))
    for name, mode, payload in entries:
        print("%-6s %-7o %10d  %s" % (kind_of(mode), mode & 0o7777, len(payload), name))
    print()

    # Hints for the target KMI, for images that carry no kernel.
    print("hints")
    needles = {
        "vermagic=": b"vermagic=",
        "Linux version": b"Linux version ",
        "androidNN": None,
        "lib/modules": b"lib/modules",
        "kernelsu": b"kernelsu",
        "magisk": b"magisk",
    }
    for name, mode, payload in entries:
        for label, needle in needles.items():
            if needle is None:
                for m in set(re.findall(rb"android\d+", payload)):
                    print("  %-16s in %-32s %s" % (label, name, m.decode()))
            elif needle in payload:
                at = payload.index(needle)
                frag = payload[at:at + 72]
                frag = re.sub(rb"[^\x20-\x7e]", b".", frag)
                print("  %-16s in %-32s %s" % (label, name, frag.decode("latin1")))
    if pattern:
        rx = re.compile(pattern.encode())
        print()
        print("grep %r" % pattern)
        for name, mode, payload in entries:
            for m in rx.finditer(payload):
                frag = re.sub(rb"[^\x20-\x7e]", b".", payload[m.start():m.start() + 80])
                print("  %-32s %s" % (name, frag.decode("latin1")))


if __name__ == "__main__":
    main()
