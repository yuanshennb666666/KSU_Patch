# Third-party components and licensing

`ksupatch` is licensed under the **GNU General Public License v3.0** (see `LICENSE`). It
depends on, bundles, or derives from the work below.

## Statically linked dependency: `android-bootimg`

- Crate: `android-bootimg`, pinned in `Cargo.toml` to
  `github.com/5ec1cff/android_bootimg` @ `150425b0`
- License: **GPL-3.0** (declared only by the `LICENSE` file in that repository; its
  `Cargo.toml` carries no `license` field)
- Use: boot-header parsing, ramdisk handling, cpio, and repacking

It is statically linked into `ksupatch.exe`, which makes the binary a combined work. This is
the main reason the project is GPL-3.0 rather than something more permissive: reusing the
same library the official implementation uses is what keeps the repack byte-equivalent to
it, and that library is GPL-3.0.

Replacing it with a self-contained implementation of the same parsing and repacking logic
would lift that constraint; until then, GPL-3.0 is the accurate license for this work.

## Bundled binary: UPX — GPL-2.0-or-later

- File: `tools/upx.exe`
- Version: 5.1.1
- Upstream: <https://github.com/upx/upx>

Called by `build.bat` to compress the built executable. UPX's license text ships with the
upstream project and is not duplicated here.

UPX's exception for a program's *output* applies, so the compressed `ksupatch.exe` is not
covered by UPX's license.

`build.bat` detects the file's absence and skips compression, so `tools/upx.exe` can be
removed from a distribution without breaking the build.

## Bundled asset: KernelSU launcher icon — GPL-3.0

- File: `assets/icon.ico`
- Derived from: the launcher icon inside the KernelSU Manager APK
- KernelSU upstream: <https://github.com/tiann/KernelSU> — GPL-3.0

Used as the executable's icon. `tools/make_icon.py` regenerates it from a KernelSU APK, so
the file can be dropped and rebuilt, or left out entirely — the build then produces an
executable with the default icon.

## KernelSU itself

`ksupatch` reads a KernelSU release APK that the user supplies and extracts the kernel
module and `ksuinit` from it. No part of KernelSU is redistributed in this repository: the
APK is never committed (see `.gitignore`), and the extracted payload only ever ends up in
the patched image the user builds for their own device.

## Rust dependencies

The direct dependencies are listed in `Cargo.toml`; `Cargo.lock` records the full
transitive set with exact versions. Run `cargo tree` for the whole graph.

| Crate | License |
| --- | --- |
| `android-bootimg` (git) | GPL-3.0 |
| `miniz_oxide` | MIT OR Zlib OR Apache-2.0 |

Everything else arrives through `android-bootimg`.
