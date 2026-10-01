#!/usr/bin/env python3
"""Build assets/icon.ico from the icon inside the KernelSU APK.

The APK ships the launcher icon at every density (36/48/72/96/144/192), which is a standard
ladder, so the icon can be taken straight from the same package the tool already patches
with -- no downloaded asset, no redrawing. The largest density is used as the master and the
remaining Windows sizes are resampled from it.

Run:  python tools/make_icon.py [kernelsu.apk] [out.ico]
"""

import io
import os
import sys
import zipfile

from PIL import Image

# Windows asks for these; 256 covers the "extra large icons" view.
WINDOWS_SIZES = [16, 24, 32, 48, 64, 96, 128, 192, 256]
# The launcher icon ladder inside the APK, by pixel size.
DENSITIES = {36, 48, 72, 96, 144, 192}


def main():
    apk = sys.argv[1] if len(sys.argv) > 1 else "KernelSU.apk"
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join("assets", "icon.ico")

    z = zipfile.ZipFile(apk)

    # Find the density ladder: PNGs whose size is one of the launcher densities and that
    # all show the same picture.
    candidates = {}
    for entry in z.infolist():
        if not entry.filename.lower().endswith(".png"):
            continue
        try:
            im = Image.open(io.BytesIO(z.read(entry.filename))).convert("RGBA")
        except Exception:
            continue
        if im.size[0] == im.size[1] and im.size[0] in DENSITIES:
            candidates[im.size[0]] = (entry.filename, im)

    if len(candidates) < 3:
        raise SystemExit(
            "expected the launcher icon at several densities, found %d (%s)"
            % (len(candidates), ", ".join(sorted(candidates)) or "none")
        )

    master_size = max(candidates)
    master_name, master = candidates[master_size]
    print("master         : %s (%dx%d)" % (master_name, master_size, master_size))
    print(
        "densities      : %s"
        % ", ".join("%d (%s)" % (n, candidates[n][0]) for n in sorted(candidates))
    )

    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    # Pillow resamples every requested size from the master with LANCZOS, which is what the
    # 192px master is for.
    master.save(out, format="ICO", sizes=[(s, s) for s in WINDOWS_SIZES])
    print(
        "wrote          : %s (%d bytes, sizes %s)"
        % (out, os.path.getsize(out), ",".join(str(s) for s in WINDOWS_SIZES))
    )


if __name__ == "__main__":
    main()
