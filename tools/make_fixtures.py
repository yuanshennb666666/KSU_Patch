#!/usr/bin/env python3
"""Generate boot-image test fixtures for ksupatch.

Builds synthetic but structurally valid GKI boot images so the patcher can be exercised
end to end without a real device dump:

  boot_v4.img         header v4, arm64 flat kernel + gzip ramdisk (init is a symlink)
  init_boot_v4.img    header v4, no kernel, gzip ramdisk (init is an aarch64 ELF)
  boot_v3.img         header v3 (no signature field) variant
  boot_magisk.img     ramdisk carrying Magisk markers, must be refused
  boot_old.img        header v2, must be refused as legacy

Run:  python tools/make_fixtures.py [outdir]
"""

import gzip
import os
import struct
import sys

PAGE = 4096
HEADER_V3_SIZE = 1580
HEADER_V4_SIZE = 1584
MAGIC = b"ANDROID!"

# cpio newc modes
S_IFREG = 0o100000
S_IFDIR = 0o040000
S_IFLNK = 0o120000


def cpio_newc(entries):
    """entries: list of (name, mode, payload bytes or None)."""

    def field(value):
        return b"%08X" % value

    out = bytearray()
    ino = 100

    def add(name, mode, data):
        nonlocal ino, out
        name_bytes = name.encode() + b"\0"
        out += (
            b"070701"
            + field(ino)
            + field(mode)
            + field(0)  # uid
            + field(0)  # gid
            + field(1)  # nlink
            + field(0)  # mtime
            + field(len(data))
            + field(0)  # devmajor
            + field(0)  # devminor
            + field(0)  # rdevmajor
            + field(0)  # rdevminor
            + field(len(name_bytes))
            + field(0)  # check
        )
        out += name_bytes
        while len(out) % 4:
            out += b"\0"
        out += data
        while len(out) % 4:
            out += b"\0"
        ino += 1

    for name, mode, data in entries:
        add(name, mode, data)
    add("TRAILER!!!", S_IFREG, b"")
    while len(out) % 512:
        out += b"\0"
    return bytes(out)


def tiny_elf(machine):
    """Minimal 64-bit little-endian ELF header, enough to identify the architecture."""
    e = bytearray(128)
    e[0:4] = b"\x7fELF"
    e[4] = 2  # 64-bit
    e[5] = 1  # little endian
    e[6] = 1  # version
    struct.pack_into("<H", e, 16, 2)  # e_type = ET_EXEC
    struct.pack_into("<H", e, 18, machine)
    struct.pack_into("<I", e, 20, 1)  # e_version
    struct.pack_into("<H", e, 52, 64)  # e_ehsize
    return bytes(e)


def fake_arm64_kernel(release=b"5.10.198-android13-8-g1a2b3c4d5e6f"):
    """arm64 flat `Image`: magic at 0x38 plus a version banner, like a real GKI kernel."""
    k = bytearray(1 << 20)
    struct.pack_into("<I", k, 0x38, 0x644D5241)  # ARM\x64
    banner = b"Linux version " + release + b" (build@localhost) (Android clang) #1 SMP PREEMPT"
    k[0x1000:0x1000 + len(banner)] = banner
    return bytes(k)


def leaky_ramdisk(with_magisk=False):
    entries = [
        (".", S_IFDIR | 0o755, b""),
        ("init", S_IFLNK | 0o777, b"/system/bin/init"),
        ("system", S_IFDIR | 0o755, b""),
        ("first_stage_ramdisk", S_IFDIR | 0o755, b""),
        ("first_stage_ramdisk/fstab.gki", S_IFREG | 0o644, b"# fstab\n"),
        ("lib", S_IFDIR | 0o755, b""),
        ("lib/modules", S_IFDIR | 0o755, b""),
        ("prop.default", S_IFREG | 0o644, b"ro.build.type=user\n"),
    ]
    if with_magisk:
        entries.append(("overlay/init.magisk.rc", S_IFREG | 0o644, b"# magisk\n"))
        entries.append((".backup/.magisk", S_IFREG | 0o644, b"config\n"))
    return cpio_newc(entries)


def init_boot_ramdisk():
    entries = [
        (".", S_IFDIR | 0o755, b""),
        ("init", S_IFREG | 0o755, tiny_elf(183)),  # a real aarch64 ELF, not a symlink
        ("system", S_IFDIR | 0o755, b""),
        ("first_stage_ramdisk", S_IFDIR | 0o755, b""),
    ]
    return cpio_newc(entries)


def build_image(kernel, ramdisk, version=4, cmdline=b"", os_version=0x0B0C1E00, page=PAGE):
    """Genuine GKI (v3/v4) or legacy (v2) header, followed by the aligned payload."""
    if version < 3:
        return build_legacy_v2(kernel, ramdisk, page=2048)

    hdr_size = HEADER_V4_SIZE if version >= 4 else HEADER_V3_SIZE
    h = bytearray(hdr_size)
    h[0:8] = MAGIC
    struct.pack_into("<I", h, 8, len(kernel))
    struct.pack_into("<I", h, 12, len(ramdisk))
    struct.pack_into("<I", h, 16, os_version)
    struct.pack_into("<I", h, 20, hdr_size)
    struct.pack_into("<I", h, 40, version)
    h[44:44 + len(cmdline)] = cmdline
    if version >= 4:
        struct.pack_into("<I", h, 1580, 0)  # signature_size

    out = bytearray(h)
    while len(out) % page:
        out += b"\0"
    out += kernel
    while len(out) % page:
        out += b"\0"
    out += ramdisk
    while len(out) % page:
        out += b"\0"
    return bytes(out)


def build_legacy_v2(kernel, ramdisk, page=2048):
    """A real `boot_img_hdr_v0` with header_version=2, so the refusal is exercised on a
    genuinely legacy image rather than on a malformed one."""
    h = bytearray(1632)  # v0 struct: 8 + 36 fields + name16 + cmdline512 + id32 + extra1024
    h[0:8] = MAGIC
    struct.pack_into("<I", h, 8, len(kernel))        # kernel_size
    struct.pack_into("<I", h, 12, 0x40080000)        # kernel_addr
    struct.pack_into("<I", h, 16, len(ramdisk))      # ramdisk_size
    struct.pack_into("<I", h, 20, 0x41000000)        # ramdisk_addr
    struct.pack_into("<I", h, 24, 0)                 # second_size
    struct.pack_into("<I", h, 28, 0)                 # second_addr
    struct.pack_into("<I", h, 32, 0x40000100)        # tags_addr
    struct.pack_into("<I", h, 36, page)              # page_size
    struct.pack_into("<I", h, 40, 2)                 # header_version
    struct.pack_into("<I", h, 44, 0)                 # os_version
    struct.pack_into("<I", h, 48, 0)                 # id[0] (checksum, unused)

    out = bytearray(h)
    while len(out) % page:
        out += b"\0"
    out += kernel
    while len(out) % page:
        out += b"\0"
    out += ramdisk
    while len(out) % page:
        out += b"\0"
    return bytes(out)


def main():
    outdir = sys.argv[1] if len(sys.argv) > 1 else "fixtures"
    os.makedirs(outdir, exist_ok=True)

    kernel = fake_arm64_kernel()
    ramdisk = gzip.compress(leaky_ramdisk(), 9)

    cases = {
        "boot_v4.img": build_image(kernel, ramdisk, version=4, cmdline=b"androidboot.hardware=gs201"),
        "boot_v3.img": build_image(kernel, ramdisk, version=3),
        "init_boot_v4.img": build_image(b"", gzip.compress(init_boot_ramdisk(), 9), version=4),
        "boot_magisk.img": build_image(kernel, gzip.compress(leaky_ramdisk(with_magisk=True), 9), version=4),
        "boot_old.img": build_image(kernel, ramdisk, version=2),
    }

    for name, data in cases.items():
        path = os.path.join(outdir, name)
        with open(path, "wb") as f:
            f.write(data)
        print("%-20s %10d bytes" % (name, len(data)))


if __name__ == "__main__":
    main()
