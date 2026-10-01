#!/usr/bin/env python3
"""Differential comparison of two patched boot images, entry by entry.

Compares the ramdisk cpio of two images: which entries differ, which exist only in one, and
where the first raw byte difference is. Used to line this tool up against the official
implementation, and to check a phone-produced image against ours.

Usage:  python tools/diff_images.py <a.img> <b.img>
"""

import hashlib
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import inspect_ramdisk as ir  # noqa: E402


def load(path):
    image = open(path, "rb").read()
    _, kernel, raw = ir.split_image(image)
    fmt, plain = ir.decompress_ramdisk(raw)
    entries = ir.parse_cpio(plain) if plain else []
    return image, kernel, raw, fmt, plain, entries


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    pa, pb = sys.argv[1], sys.argv[2]
    ia, ka, ra, fa, plain_a, entries_a = load(pa)
    ib, kb, rb, fb, plain_b, entries_b = load(pb)

    print("A %s" % pa)
    print("B %s" % pb)
    print()
    print("image size     : A=%d B=%d  %s" % (len(ia), len(ib), "same" if len(ia) == len(ib) else "DIFFERENT"))
    print("kernel         : %d vs %d  %s" % (len(ka), len(kb), "same" if ka == kb else "DIFFERENT"))
    print("ramdisk stored : %d vs %d  %s" % (len(ra), len(rb), "same" if ra == rb else "DIFFERENT"))
    print("ramdisk format : %s vs %s" % (fa, fb))
    print("ramdisk plain  : %d vs %d  %s" % (len(plain_a), len(plain_b), "same" if plain_a == plain_b else "DIFFERENT"))
    print()

    if ia == ib:
        print("WHOLE FILE IDENTICAL")
        print("sha256         : %s" % hashlib.sha256(ia).hexdigest())
        return 0

    n = min(len(ia), len(ib))
    first = next((i for i in range(n) if ia[i] != ib[i]), n)
    print("first differing byte at 0x%X (%d)" % (first, first))
    print("sha256         : A=%s" % hashlib.sha256(ia).hexdigest())
    print("                 B=%s" % hashlib.sha256(ib).hexdigest())
    print()

    am = {name: (mode, payload) for name, mode, payload in entries_a}
    bm = {name: (mode, payload) for name, mode, payload in entries_b}
    print("entries        : A=%d B=%d" % (len(am), len(bm)))
    print("only in A      : %s" % (sorted(set(am) - set(bm)) or "(none)"))
    print("only in B      : %s" % (sorted(set(bm) - set(am)) or "(none)"))
    print()

    print("%-38s %-6s %-6s %s" % ("name", "mode", "size", "verdict"))
    for name in sorted(set(am) & set(bm)):
        ma, da = am[name]
        mb, db = bm[name]
        if ma == mb and da == db:
            continue
        if da == db:
            verdict = "MODE DIFFERS (%o vs %o)" % (ma & 0o7777, mb & 0o7777)
        elif ma == mb:
            verdict = "CONTENT DIFFERS (%d vs %d bytes)" % (len(da), len(db))
        else:
            verdict = "MODE+CONTENT DIFFER"
        print("%-38s %-6o %-6d %s" % (name, ma & 0o7777, len(da), verdict))
    print()
    same = sum(1 for k in set(am) & set(bm) if am[k] == bm[k])
    print("identical entries: %d of %d shared" % (same, len(set(am) & set(bm))))
    return 1


if __name__ == "__main__":
    sys.exit(main())
