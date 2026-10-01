#!/usr/bin/env python3
"""Prove the patched image contains the official KernelSU payload, byte for byte.

Pulls `kernelsu.ko`, `init` (ksuinit) and `ksu_config` back out of a patched image's
ramdisk and compares their SHA-256 against the release assets the caller names. Naming the
expected assets explicitly keeps this an independent check: the script has no idea which
KMI or architecture was supposed to be installed, so it cannot agree with a wrong choice.

Ramdisk decompression and cpio parsing are shared with tools/inspect_ramdisk.py.

Usage:
  python tools/verify_payload.py <image> <module-asset> <ksuinit-asset> [--release v3.3.0]

Example:
  python tools/verify_payload.py out/a64/*.img \
      lkm-aarch64-android17-6.18_kernelsu.ko ksuinit-aarch64
"""

import hashlib
import os
import sys
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import inspect_ramdisk as ir  # noqa: E402

UA = {"User-Agent": "ksupatch-verify"}


def sha256(b):
    return hashlib.sha256(b).hexdigest()


def fetch(release, asset):
    cache_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "fixtures", "release")
    os.makedirs(cache_dir, exist_ok=True)
    path = os.path.join(cache_dir, asset)
    if not os.path.exists(path):
        url = "https://github.com/tiann/KernelSU/releases/download/%s/%s" % (release, asset)
        print("downloading    : %s" % url)
        urllib.request.urlretrieve(url, path)
    return open(path, "rb").read()


def main():
    positional = [a for a in sys.argv[1:] if not a.startswith("--")]
    release = "v3.3.0"
    if "--release" in sys.argv:
        release = sys.argv[sys.argv.index("--release") + 1]
    if len(positional) != 3:
        raise SystemExit(__doc__)
    image_path, module_asset, init_asset = positional

    image = open(image_path, "rb").read()
    print("image          : %s (%d bytes)" % (image_path, len(image)))
    _, _, raw_ramdisk = ir.split_image(image)
    fmt, plain = ir.decompress_ramdisk(raw_ramdisk)
    if plain is None:
        raise SystemExit("unsupported ramdisk compression: %s" % fmt)
    entries = ir.parse_cpio(plain)
    files = {n: p for n, m, p in entries if m & 0o170000 == 0o100000}
    links = {n: p.decode("latin1") for n, m, p in entries if m & 0o170000 == 0o120000}
    print("ramdisk        : %d bytes stored -> %d bytes plain, format %s, %d entries"
          % (len(raw_ramdisk), len(plain), fmt, len(entries)))
    print("regular files  : %s" % ", ".join(sorted(files)))
    print("symlinks       : %s"
          % (", ".join("%s -> %s" % (k, v) for k, v in sorted(links.items())) or "(none)"))
    print()

    ok = True
    for local_name, asset_name in (("kernelsu.ko", module_asset), ("init", init_asset)):
        if local_name not in files:
            print("MISSING        : %s is not a regular file in the ramdisk" % local_name)
            ok = False
            continue
        mine = files[local_name]
        theirs = fetch(release, asset_name)
        same = sha256(mine) == sha256(theirs)
        ok &= same
        print("%-14s : %s" % (local_name, "IDENTICAL to %s" % asset_name if same else "DIFFERS from %s" % asset_name))
        print("                 in image : %7d bytes  sha256 %s" % (len(mine), sha256(mine)))
        print("                 asset    : %7d bytes  sha256 %s" % (len(theirs), sha256(theirs)))

    if "ksu_config" in files:
        print("%-14s : %r" % ("ksu_config", files["ksu_config"].decode()))
    print("%-14s : %s" % ("init.real", links.get("init.real") or ("regular file" if "init.real" in files else "ABSENT")))
    print()
    print("result         : %s" % ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
