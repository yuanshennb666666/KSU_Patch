//! Boot-image handling: inspection, KMI/arch detection, ramdisk cpio injection, repacking
//! and verification.
//!
//! The structural work (header layouts v0-v4, block alignment, AVB footer, ramdisk
//! re-compression) is delegated to the `android-bootimg` crate -- the same implementation
//! the official tooling uses -- so this module stays focused on the KernelSU-specific
//! parts and on producing an auditable report.

use android_bootimg::cpio::{Cpio, CpioEntry};
use android_bootimg::parser::{BootImage, BootImageVersion};
use android_bootimg::patcher::BootImagePatchOption;
use std::io::Cursor;

use crate::err::{fail, Res, ASSET, IMAGE, INJECT, IO};

/// Mode the init shim is installed with.
pub const INIT_MODE: u32 = 0o755;
/// Mode the kernel module is installed with.
pub const MODULE_MODE: u32 = 0o755;
/// Mode of the KernelSU runtime config file.
pub const CONFIG_MODE: u32 = 0o644;

/// Name KernelSU's init shim is installed under.
pub const INIT_NAME: &str = "init";
/// The original `init` is moved here so the shim can chain to it.
pub const INIT_REAL_NAME: &str = "init.real";
/// Loadable kernel module installed into the ramdisk.
pub const MODULE_NAME: &str = "kernelsu.ko";
/// KernelSU runtime configuration file.
pub const CONFIG_NAME: &str = "ksu_config";

const BOOT_MAGIC: &[u8] = b"ANDROID!";
const VENDOR_BOOT_MAGIC: &[u8] = b"VNDRBOOT";

pub struct BlockInfo {
    /// Size of the block as stored in the image (still compressed).
    pub stored_len: usize,
    /// Decompressed size, when it could be determined.
    pub plain_len: Option<usize>,
    pub format: String,
}

pub struct ImageInfo {
    /// `boot` or `init_boot`.
    pub kind: &'static str,
    pub version: u32,
    pub page_size: usize,
    pub image_size: usize,
    pub kernel: Option<BlockInfo>,
    pub ramdisk: Option<BlockInfo>,
    pub extra_blocks: Vec<(&'static str, usize)>,
    /// Architecture implied by the kernel image, when it can be told.
    pub arch: Option<&'static str>,
    /// Kernel release string from the version banner.
    pub kernel_release: Option<String>,
    /// KMI derived from the kernel bytes.
    pub kmi: Option<String>,
}

fn describe_magic(data: &[u8]) -> String {
    if data.len() < 8 {
        format!("only {} bytes long", data.len())
    } else if data.starts_with(BOOT_MAGIC) {
        "boot image (`ANDROID!` magic)".to_string()
    } else if data.starts_with(VENDOR_BOOT_MAGIC) {
        "vendor_boot image (`VNDRBOOT` magic)".to_string()
    } else {
        format!(
            "unrecognised: the first 8 bytes are {:02X?}, expected `ANDROID!` or `VNDRBOOT`",
            &data[..8]
        )
    }
}

/// Parse and describe a boot / init_boot image.
///
/// Only GKI-era headers (v3/v4) are accepted. Anything else is rejected with an explicit
/// reason rather than silently mis-patched.
pub fn inspect(data: &[u8], label: &str) -> Res<ImageInfo> {
    if data.len() < 8 {
        return Err(fail(
            IMAGE,
            "image parsing",
            label,
            format!("file is {} bytes, too small to be a boot image", data.len()),
            "point this at the boot.img / init_boot.img you extracted from the device firmware",
        ));
    }

    // Peek `header_version` before handing the image to the parser. Every Android boot
    // header keeps this field at offset 40, and the parser would otherwise fail on the
    // block table of a legacy image first, burying the real reason.
    if data.starts_with(BOOT_MAGIC) && data.len() >= 44 {
        let ver = u32::from_le_bytes([data[40], data[41], data[42], data[43]]);
        if ver < 3 {
            return Err(fail(
                IMAGE,
                "image parsing",
                label,
                format!("boot header version {ver} is a legacy format (v0/v1/v2)"),
                "KernelSU's module loading relies on the GKI ramdisk layout, which only \
                 exists from boot header v3 upward (Android 11+ / kernel 5.10+). This \
                 device is not supported by the module-based install",
            ));
        }
        if ver > 4 {
            return Err(fail(
                IMAGE,
                "image parsing",
                label,
                format!("boot header version {ver} is newer than this build understands"),
                "update the tool",
            ));
        }
    }

    let image = BootImage::parse(data).map_err(|e| {
        fail(
            IMAGE,
            "image parsing",
            label,
            format!("cannot parse header: {e} ({})", describe_magic(data)),
            "this tool handles GKI boot images only; a raw or repacked partition dump may need to be re-extracted",
        )
    })?;

    let version = match image.get_header().get_version() {
        BootImageVersion::Android(v) => v,
        BootImageVersion::Vendor(v) => {
            return Err(fail(
                IMAGE,
                "image parsing",
                label,
                format!("this is a vendor_boot image (version {v})"),
                "vendor_boot is out of scope: patch boot.img or init_boot.img instead",
            ));
        }
        BootImageVersion::RawRamdisk => {
            return Err(fail(
                IMAGE,
                "image parsing",
                label,
                "this is a bare ramdisk, not a boot image",
                "patch boot.img or init_boot.img instead",
            ));
        }
    };

    if version < 3 {
        return Err(fail(
            IMAGE,
            "image parsing",
            label,
            format!("boot header version {version} is a legacy format (v0/v1/v2)"),
            "KernelSU's module loading relies on the GKI ramdisk layout, which only exists \
             from boot header v3 upward (Android 11+ / kernel 5.10+). This device is not \
             supported by the module-based install",
        ));
    }
    if version > 4 {
        return Err(fail(
            IMAGE,
            "image parsing",
            label,
            format!("boot header version {version} is newer than this build understands"),
            "update the tool",
        ));
    }

    let header = image.get_header();
    if image.get_blocks().get_ramdisk().is_none() {
        return Err(fail(
            IMAGE,
            "image parsing",
            label,
            "the image has no ramdisk section, so there is nowhere to install the module",
            "on Android 13+ the ramdisk lives in init_boot.img -- patch that instead of boot.img",
        ));
    }

    let mut kernel_plain = Vec::new();
    let kernel = match image.get_blocks().get_kernel() {
        Some(k) => {
            k.dump(&mut kernel_plain, false).map_err(|e| {
                fail(
                    IMAGE,
                    "kernel decompression",
                    label,
                    format!("cannot decompress the kernel section: {e}"),
                    "the image may be truncated or corrupt",
                )
            })?;
            Some(BlockInfo {
                stored_len: k.get_data().len(),
                plain_len: Some(kernel_plain.len()),
                format: format!("{:?}", k.get_compress_format()),
            })
        }
        None => None,
    };

    let ramdisk = image.get_blocks().get_ramdisk().map(|r| {
        let mut plain = Vec::new();
        let plain_len = r.dump(&mut plain, false).ok().map(|_| plain.len());
        BlockInfo {
            stored_len: r.get_data().len(),
            plain_len,
            format: format!("{:?}", r.get_compress_format()),
        }
    });

    // BootImageBlocks only exposes the kernel and the ramdisk, so the remaining block sizes
    // come from the header fields.
    let mut extra_blocks: Vec<(&'static str, usize)> = Vec::new();
    for (name, has, size) in [
        ("second", header.has_second_size(), header.get_second_size()),
        ("recovery_dtbo", header.has_recovery_dtbo_size(), header.get_recovery_dtbo_size()),
        ("dtb", header.has_dtb_size(), header.get_dtb_size()),
        ("signature", header.has_signature_size(), header.get_signature_size()),
        ("bootconfig", header.has_bootconfig_size(), header.get_bootconfig_size()),
    ] {
        if has && size > 0 {
            extra_blocks.push((name, size as usize));
        }
    }

    // init_boot has no kernel section at all: it exists precisely to carry the ramdisk.
    let kind = if kernel.is_some() { "boot" } else { "init_boot" };

    Ok(ImageInfo {
        kind,
        version,
        page_size: header.page_size(),
        image_size: image.get_size(),
        kernel,
        ramdisk,
        extra_blocks,
        arch: detect_arch(&kernel_plain),
        kernel_release: detect_release(&kernel_plain),
        kmi: detect_kmi(&kernel_plain),
    })
}

/// Architecture of the payload the ramdisk expects, read from its `init` binary when that
/// binary is a real ELF object rather than a symlink.
pub fn arch_from_init(cpio: &Cpio) -> Option<&'static str> {
    let d = cpio.entry_by_name(INIT_NAME)?.data()?;
    if d.len() < 20 || d[..4] != [0x7F, b'E', b'L', b'F'] {
        return None;
    }
    match u16::from_le_bytes([d[18], d[19]]) {
        183 => Some("aarch64"),
        62 => Some("x86_64"),
        40 => Some("arm"),
        243 => Some("riscv64"),
        _ => None,
    }
}

/// Architecture, from the kernel image.
///
/// A decompressed kernel is not an ELF object: arm64 ships a flat `Image` and x86 ships a
/// bzImage, so each is recognised by its own header.
pub fn detect_arch(kernel: &[u8]) -> Option<&'static str> {
    if kernel.len() >= 64 && kernel[..4] == [0x7F, b'E', b'L', b'F'] {
        let machine = u16::from_le_bytes([kernel[18], kernel[19]]);
        return match machine {
            183 => Some("aarch64"),
            62 => Some("x86_64"),
            40 => Some("arm"),
            243 => Some("riscv64"),
            _ => None,
        };
    }
    // arm64 flat Image: magic at offset 0x38.
    if kernel.len() > 0x3C
        && u32::from_le_bytes([kernel[0x38], kernel[0x39], kernel[0x3A], kernel[0x3B]])
            == 0x644D_5241
    {
        return Some("aarch64");
    }
    // x86 bzImage: setup signature.
    if kernel.len() > 0x206 && &kernel[0x202..0x206] == b"HdrS" {
        return Some("x86_64");
    }
    None
}

/// The kernel release string (`5.10.198-android13-8-g...`), read from the version banner.
pub fn detect_release(kernel: &[u8]) -> Option<String> {
    const NEEDLE: &[u8] = b"Linux version ";
    let at = find(kernel, NEEDLE)? + NEEDLE.len();
    let end = kernel[at..]
        .iter()
        .position(|&c| c == b' ' || c == 0)
        .unwrap_or(0);
    if end == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&kernel[at..at + end]).into_owned())
}

/// KMI (`android13-5.10`) from the kernel bytes.
///
/// Reimplements the upstream scan: look for a `major.minor` candidate, then require an
/// `androidNN` token in the same whitespace-delimited run. This mirrors how the official
/// installer derives the KMI from an image, which is what makes `auto` trustworthy.
pub fn detect_kmi(kernel: &[u8]) -> Option<String> {
    let mut i = 0usize;
    while i + 4 < kernel.len() {
        let (x0, x1, x2, x3) = (kernel[i], kernel[i + 1], kernel[i + 2], kernel[i + 3]);
        let plausible = x1 == b'.'
            && x2.is_ascii_digit()
            && match x0 {
                b'5' => x3.is_ascii_digit(),
                b'6'..=b'9' => true,
                _ => false,
            };
        if plausible {
            let end = (i + 100).min(kernel.len());
            let window = &kernel[i..end];
            let stop = window.iter().position(|&c| c == 0).unwrap_or(window.len());
            if let Ok(s) = std::str::from_utf8(&window[..stop]) {
                if let Some(kmi) = kmi_from_str(s) {
                    return Some(kmi);
                }
            }
        }
        i += 1;
    }
    None
}

/// `(\d+\.\d+)` followed, in the same run, by `android\d+` -> `androidNN-X.Y`.
fn kmi_from_str(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i >= b.len() || b[i] != b'.' {
            continue;
        }
        let dot = i;
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == dot + 1 {
            continue;
        }
        let version = &s[start..i];
        // The androidNN token has to appear before any whitespace.
        let mut j = i;
        while j < b.len() && !b[j].is_ascii_whitespace() {
            j += 1;
        }
        if let Some(android) = find_android(&s[i..j]) {
            return Some(format!("{android}-{version}"));
        }
    }
    None
}

fn find_android(run: &str) -> Option<String> {
    const NEEDLE: &str = "android";
    let b = run.as_bytes();
    let mut from = 0usize;
    while let Some(p) = run[from..].find(NEEDLE) {
        let at = from + p + NEEDLE.len();
        let mut j = at;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > at {
            return Some(run[from + p..j].to_string());
        }
        from += p + 1;
        if from >= run.len() {
            break;
        }
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The Android platform release declared by the ramdisk itself (`17`), read from the
/// `build.prop` a GKI ramdisk carries.
///
/// An init_boot image has no kernel, so this is the only KMI-relevant fact it contains --
/// enough to narrow the choice down to the KMIs the APK offers for that release.
pub fn android_release(cpio: &Cpio) -> Option<String> {
    const KEYS: [&[u8]; 3] = [
        b"ro.bootimage.build.version.release=",
        b"ro.build.version.release=",
        b"ro.system.build.version.release=",
    ];
    for (name, entry) in cpio.entries() {
        if !name.ends_with(".prop") {
            continue;
        }
        let Some(data) = entry.data() else { continue };
        for key in KEYS {
            if let Some(at) = find(data, key) {
                let rest = &data[at + key.len()..];
                let end = rest
                    .iter()
                    .position(|&c| c == b'\n' || c == b'\r' || c == 0)
                    .unwrap_or(rest.len());
                let value = String::from_utf8_lossy(&rest[..end]).trim().to_string();
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }
    None
}

/// Kernel `major.minor` read from the `vermagic` of a kernel module sitting in the ramdisk,
/// plus the name it was found under.
///
/// An init_boot image carries no kernel, but if its ramdisk already holds a kernel module
/// -- KernelSU's own from an earlier install, or `/lib/modules/*.ko` from a vendor ramdisk
/// -- that module records the exact kernel it was built for. That is the same fact
/// `uname -r` would give, and it is available entirely offline.
pub fn kernel_xy_from_ramdisk(cpio: &Cpio) -> Option<(String, String)> {
    for (name, entry) in cpio.entries() {
        if !name.ends_with(".ko") {
            continue;
        }
        let Some(data) = entry.data() else { continue };
        let Some(info) = crate::elf::ElfInfo::parse(data) else {
            continue;
        };
        let Some(vm) = crate::elf::vermagic(data, &info) else { continue };
        if let Some(xy) = crate::elf::kernel_xy(&vm) {
            return Some((xy, name.clone()));
        }
    }
    None
}

/// Architecture and KMI of the kernel inside another image.
///
/// The architecture matters: a firmware folder can hold images for more than one device,
/// and picking up the wrong one would install a module built for a different kernel.
pub fn kernel_facts_from_image(data: &[u8]) -> Option<(&'static str, Option<String>)> {
    let image = BootImage::parse(data).ok()?;
    let kernel = image.get_blocks().get_kernel()?;
    let mut plain = Vec::new();
    kernel.dump(&mut plain, false).ok()?;
    Some((detect_arch(&plain)?, detect_kmi(&plain)))
}

/// KMI read from the kernel inside another image (a `boot.img`, typically).
///
/// This is the offline answer for a *pristine* init_boot: the kernel is not in that
/// partition, but the sibling boot image from the same firmware has it.
pub fn kmi_from_image(data: &[u8]) -> Option<String> {
    let image = BootImage::parse(data).ok()?;
    let kernel = image.get_blocks().get_kernel()?;
    let mut plain = Vec::new();
    kernel.dump(&mut plain, false).ok()?;
    detect_kmi(&plain)
}

/// Extract the ramdisk as a cpio archive.
pub fn load_ramdisk_cpio(data: &[u8], label: &str) -> Res<Cpio> {
    let image = BootImage::parse(data).map_err(|e| {
        fail(
            IMAGE,
            "image parsing",
            label,
            format!("{e}"),
            "unexpected; re-run the tool",
        )
    })?;
    let ramdisk = image.get_blocks().get_ramdisk().ok_or_else(|| {
        fail(
            IMAGE,
            "ramdisk extraction",
            label,
            "no ramdisk section",
            "patch init_boot.img instead of boot.img",
        )
    })?;
    let format = format!("{:?}", ramdisk.get_compress_format());
    let mut plain = Vec::new();
    ramdisk.dump(&mut plain, false).map_err(|e| {
        fail(
            IMAGE,
            "ramdisk decompression",
            label,
            format!("cannot decompress the ramdisk (stored format {format}): {e}"),
            "the image may be truncated or corrupt",
        )
    })?;
    Cpio::load_from_data(&plain).map_err(|e| {
        fail(
            IMAGE,
            "ramdisk cpio",
            label,
            format!("ramdisk does not contain a readable cpio archive: {e}"),
            "this build handles the cpio formats repacking tools produce; a vendor variant may be unsupported",
        )
    })
}

/// Name the stock-image checksum is recorded under when an install backs it up.
pub const STOCK_SHA1_NAME: &str = "stock_image.sha1";

pub struct InjectOptions<'a> {
    pub module: &'a [u8],
    pub ksuinit: &'a [u8],
    /// Contents of `ksu_config`, or `None` to leave the file out entirely.
    ///
    /// The official installer writes `bundled=1` when it uses its built-in module and writes
    /// nothing at all when a module is supplied from outside, so this is a real byte-level
    /// difference in the output, not a cosmetic one.
    pub ksu_config: Option<&'a str>,
    /// SHA-1 of the *stock* image. When set, `stock_image.sha1` is recorded in the ramdisk,
    /// exactly as an install that protects its backup does.
    pub stock_sha1: Option<&'a str>,
}

pub struct Injection {
    /// The image already carried a KernelSU module, so this is a refresh.
    pub upgrade: bool,
    /// The original `init` was moved to `init.real`.
    pub renamed_init: bool,
    /// No `init` entry existed, so nothing could be chained to.
    pub missing_init: bool,
    pub module_len: usize,
    pub init_len: usize,
    pub config: Option<String>,
    pub stock_sha1: Option<String>,
}

/// Install the KernelSU payload into the ramdisk.
///
/// Mirrors the upstream layout exactly, because the kernel module discovers its payload by
/// these names: `init` becomes the shim, the original moves to `init.real`, and the module
/// sits at the ramdisk root as `kernelsu.ko`.
pub fn inject(cpio: &mut Cpio, opts: &InjectOptions, label: &str) -> Res<Injection> {
    if cpio.is_magisk_patched() {
        return Err(fail(
            INJECT,
            "ramdisk injection",
            label,
            "the ramdisk is already patched by Magisk",
            "start from a stock, unpatched boot/init_boot image -- KernelSU will not install \
             alongside a Magisk-patched ramdisk",
        ));
    }

    let upgrade = cpio.exists(MODULE_NAME);
    let mut renamed_init = false;
    let mut missing_init = false;

    if !upgrade {
        if cpio.exists(INIT_NAME) {
            if !cpio.exists(INIT_REAL_NAME) {
                cpio.mv(INIT_NAME, INIT_REAL_NAME).map_err(|e| {
                    fail(
                        INJECT,
                        "ramdisk injection",
                        label,
                        format!("cannot move `{INIT_NAME}` to `{INIT_REAL_NAME}`: {e}"),
                        "the ramdisk has an unexpected layout; report this image",
                    )
                })?;
                renamed_init = true;
            }
        } else {
            missing_init = true;
        }
    }

    cpio.add(
        INIT_NAME,
        CpioEntry::regular(INIT_MODE, Box::new(opts.ksuinit.to_vec())),
    )
    .map_err(|e| {
        fail(
            INJECT,
            "ramdisk injection",
            label,
            format!("cannot install `{INIT_NAME}`: {e}"),
            "report this image",
        )
    })?;
    cpio.add(
        MODULE_NAME,
        CpioEntry::regular(MODULE_MODE, Box::new(opts.module.to_vec())),
    )
    .map_err(|e| {
        fail(
            INJECT,
            "ramdisk injection",
            label,
            format!("cannot install `{MODULE_NAME}`: {e}"),
            "report this image",
        )
    })?;

    let config = opts.ksu_config.map(|s| s.to_string());
    match &config {
        Some(tokens) => {
            cpio.add(
                CONFIG_NAME,
                CpioEntry::regular(CONFIG_MODE, Box::new(tokens.clone().into_bytes())),
            )
            .map_err(|e| {
                fail(
                    INJECT,
                    "ramdisk injection",
                    label,
                    format!("cannot write `{CONFIG_NAME}`: {e}"),
                    "report this image",
                )
            })?;
        }
        None => cpio.rm(CONFIG_NAME, false),
    }

    if let Some(sha) = opts.stock_sha1 {
        cpio.add(
            STOCK_SHA1_NAME,
            CpioEntry::regular(INIT_MODE, Box::new(sha.as_bytes().to_vec())),
        )
        .map_err(|e| {
            fail(
                INJECT,
                "ramdisk injection",
                label,
                format!("cannot write `{STOCK_SHA1_NAME}`: {e}"),
                "report this image",
            )
        })?;
    }

    Ok(Injection {
        upgrade,
        renamed_init,
        missing_init,
        module_len: opts.module.len(),
        init_len: opts.ksuinit.len(),
        config,
        stock_sha1: opts.stock_sha1.map(|s| s.to_string()),
    })
}

/// Serialise the cpio and rebuild the image, re-compressing the ramdisk with the format it
/// originally used.
pub fn repack(image_data: &[u8], cpio: &Cpio, label: &str) -> Res<Vec<u8>> {
    let mut new_ramdisk = Vec::new();
    cpio.dump(&mut new_ramdisk).map_err(|e| {
        fail(
            IO,
            "repacking",
            label,
            format!("cannot serialise the new ramdisk: {e}"),
            "report this image",
        )
    })?;

    let image = BootImage::parse(image_data).map_err(|e| {
        fail(
            IMAGE,
            "image parsing",
            label,
            format!("{e}"),
            "unexpected; re-run the tool",
        )
    })?;
    let mut patcher = BootImagePatchOption::new(&image);
    // `false` means "this payload is already uncompressed, please re-compress it the way
    // the original ramdisk was" -- which is what keeps the bootloader happy.
    patcher.replace_ramdisk(Box::new(Cursor::new(new_ramdisk)), false);

    let mut out = Cursor::new(Vec::with_capacity(image.get_size() + 64 * 1024));
    patcher.patch(&mut out).map_err(|e| {
        fail(
            IO,
            "repacking",
            label,
            format!("cannot rebuild the image: {e}"),
            "the image may not leave enough room for the larger ramdisk",
        )
    })?;
    Ok(out.into_inner())
}

pub struct Verified {
    pub module_len: usize,
    pub init_len: usize,
    pub init_real: bool,
    pub config: Option<String>,
    pub image_size: usize,
    pub version: u32,
}

/// Re-read the produced image and assert the install is really there.
pub fn verify(data: &[u8], label: &str) -> Res<Verified> {
    let cpio = load_ramdisk_cpio(data, label)?;
    let image = BootImage::parse(data).map_err(|e| {
        fail(
            IMAGE,
            "verification",
            label,
            format!("{e}"),
            "report this defect",
        )
    })?;
    let version = match image.get_header().get_version() {
        BootImageVersion::Android(v) => v,
        _ => 0,
    };

    let module = cpio.entry_by_name(MODULE_NAME).ok_or_else(|| {
        fail(
            ASSET,
            "verification",
            label,
            format!("`{MODULE_NAME}` is missing from the produced image"),
            "report this defect",
        )
    })?;
    let module_len = module.data().map(<[u8]>::len).unwrap_or(0);
    if module_len == 0 {
        return Err(fail(
            ASSET,
            "verification",
            label,
            format!("`{MODULE_NAME}` is present but empty"),
            "report this defect",
        ));
    }
    let init = cpio.entry_by_name(INIT_NAME).ok_or_else(|| {
        fail(
            ASSET,
            "verification",
            label,
            format!("`{INIT_NAME}` is missing"),
            "report this defect",
        )
    })?;
    let init_len = init.data().map(<[u8]>::len).unwrap_or(0);

    Ok(Verified {
        module_len,
        init_len,
        init_real: cpio.exists(INIT_REAL_NAME),
        config: cpio
            .entry_by_name(CONFIG_NAME)
            .and_then(CpioEntry::data)
            .and_then(|v| String::from_utf8(v.to_vec()).ok()),
        image_size: data.len(),
        version,
    })
}
