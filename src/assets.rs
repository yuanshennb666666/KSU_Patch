//! Extraction of the assets that KernelSU embeds inside its own native library.
//!
//! The release APK has no loose `.ko` files. The modules and `ksuinit` live *inside*
//! `lib/<abi>/libksud.so`, stored by `rust-embed` with its `compression` feature, which
//! uses `include-flate`: plain DEFLATE with no gzip/zlib wrapper. Because such a stream
//! carries no magic bytes, header-based scanning cannot find it. What does work is
//! brute force -- attempt a raw-DEFLATE inflate at every offset and keep the results
//! that come out as well-formed ELF objects.

use crate::elf::{self, ElfInfo};
use miniz_oxide::inflate::core::inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
use miniz_oxide::inflate::core::{decompress, DecompressorOxide};
use miniz_oxide::inflate::TINFLStatus;

/// Upper bound for one embedded asset. The largest known asset (aarch64 busybox) is
/// about 1.7 MB; 8 MB leaves room for future growth without letting a bogus candidate
/// inflate without bound.
pub const MAX_ASSET: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// ET_REL object whose module name is `kernelsu`: the payload this tool installs.
    Module,
    /// Any other ET_REL object bundled by a fork (e.g. KernelSU-Next's
    /// `susfs_guard_lkm`). It shares the ramdisk with the payload but is not the module
    /// that a KMI selects, so it must not take part in KMI resolution.
    AuxModule,
    KsuInit,
    BusyBox,
    Bootctl,
    /// `waitsys`, the binder-wait helper added in KernelSU 3.3.1.
    Waitsys,
    /// A fork-specific ELF helper bundled next to the payload (mkbootfs, xecinit, ...).
    /// It is carried through untouched and is not part of the KMI selection.
    Helper,
    Unknown,
}

pub struct Asset {
    /// Offset of the DEFLATE stream inside the native library.
    pub offset: usize,
    /// Length of the DEFLATE stream (compressed size).
    pub csize: usize,
    /// Inflated content.
    pub data: Vec<u8>,
    pub elf: Option<ElfInfo>,
    pub kind: Kind,
    /// `name=` from the module's `.modinfo`, for ET_REL objects.
    pub module_name: Option<String>,
    pub vermagic: Option<String>,
    pub ddk_kmi: Option<String>,
    pub clang: Option<String>,
    /// KMI this module was matched to, if it could be resolved.
    pub kmi: Option<String>,
    /// Why that KMI was chosen.
    pub evidence: Vec<String>,
}

impl Asset {
    pub fn identity(&self) -> String {
        match self.kind {
            Kind::Module => "kernelsu.ko".to_string(),
            Kind::AuxModule => match self.module_name.as_deref() {
                Some(name) if !name.is_empty() => format!("{name}.ko"),
                _ => "auxiliary .ko".to_string(),
            },
            Kind::KsuInit => "ksuinit".to_string(),
            Kind::BusyBox => "busybox".to_string(),
            Kind::Bootctl => "bootctl".to_string(),
            Kind::Waitsys => "waitsys".to_string(),
            Kind::Helper => {
                if elf::contains(&self.data, b"mkbootfs") {
                    "mkbootfs".to_string()
                } else if elf::contains(&self.data, b"xecinit") {
                    "xecinit".to_string()
                } else {
                    "helper binary".to_string()
                }
            }
            Kind::Unknown => "(unrecognised)".to_string(),
        }
    }

    pub fn arch(&self) -> &'static str {
        self.elf.as_ref().map(|e| e.arch()).unwrap_or("-")
    }

    pub fn type_name(&self) -> &'static str {
        self.elf.as_ref().map(|e| e.type_name()).unwrap_or("-")
    }
}

/// Walk `lib` and return every raw-DEFLATE stream that inflates to an ELF object.
///
/// The asset data is one contiguous run, so after a hit the scan resumes at the end of
/// that stream instead of restarting from the following byte.
pub fn scan(lib: &[u8], from: usize, to: usize) -> Vec<Asset> {
    let mut d = DecompressorOxide::new();
    let mut out = vec![0u8; MAX_ASSET];
    let mut raw: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    let mut start = from.min(lib.len());
    let to = to.min(lib.len());
    while let Some((at, csize, data)) = next_stream(lib, start, to, &mut d, &mut out) {
        start = (at + csize).max(at + 1);
        raw.push((at, csize, data));
    }
    raw.into_iter().map(|(o, c, d)| build(o, c, d)).collect()
}

/// Scan several disjoint byte ranges (the sections that can hold the asset blob),
/// returning every asset found ordered by offset.
pub fn scan_ranges(lib: &[u8], ranges: &[(usize, usize)]) -> Vec<Asset> {
    let mut out = Vec::new();
    for &(a, b) in ranges {
        out.extend(scan(lib, a, b));
    }
    out.sort_by_key(|a| a.offset);
    out
}

fn next_stream(
    lib: &[u8],
    from: usize,
    to: usize,
    d: &mut DecompressorOxide,
    out: &mut [u8],
) -> Option<(usize, usize, Vec<u8>)> {
    let mut i = from;
    while i + 4 < to {
        d.init();
        let (status, in_consumed, out_consumed) = decompress(
            d,
            &lib[i..to],
            out,
            0,
            TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
        );
        if status == TINFLStatus::Done
            && in_consumed > 0
            && out_consumed >= 4
            && out[..4] == elf::ELF_MAGIC
        {
            return Some((i, in_consumed, out[..out_consumed].to_vec()));
        }
        i += 1;
    }
    None
}

fn build(offset: usize, csize: usize, data: Vec<u8>) -> Asset {
    let elf = ElfInfo::parse(&data);
    let mut asset = Asset {
        offset,
        csize,
        data,
        elf,
        kind: Kind::Unknown,
        module_name: None,
        vermagic: None,
        ddk_kmi: None,
        clang: None,
        kmi: None,
        evidence: Vec::new(),
    };
    if let Some(i) = elf {
        if i.etype == elf::ET_REL {
            let modinfo = elf::modinfo(&asset.data, &i);
            asset.module_name = modinfo
                .iter()
                .find(|(key, _)| key == "name")
                .map(|(_, value)| value.clone());
            asset.vermagic = elf::vermagic(&asset.data, &i);
            asset.ddk_kmi = elf::ddk_kmi(&asset.data);
            asset.clang = elf::clang_version(&asset.data);
            // Forks bundle extra modules next to the payload (KernelSU-Next ships a
            // `susfs_guard_lkm` for every KMI). Only `kernelsu` is what a KMI selects;
            // anything else is carried through untouched and reported separately. A
            // module with no `name=` at all predates the field, so treat it as payload.
            asset.kind = match asset.module_name.as_deref() {
                Some("kernelsu") | None => Kind::Module,
                Some(_) => Kind::AuxModule,
            };
        } else {
            asset.kind = if elf::contains(&asset.data, b"BusyBox")
                && elf::contains(&asset.data, b"multi-call")
            {
                Kind::BusyBox
            } else if elf::contains(&asset.data, b"set-active-boot-slot")
                || elf::contains(&asset.data, b"hal-info")
            {
                Kind::Bootctl
            } else if elf::contains(&asset.data, b"ksuinit")
                && elf::contains(&asset.data, b"init.real")
            {
                Kind::KsuInit
            } else if elf::contains(&asset.data, b"ksud-waitsys") {
                Kind::Waitsys
            } else {
                // ET_EXEC/ET_DYN objects that are none of the known tools are fork
                // helpers (ReSukiSU ships mkbootfs, XECKernelPro its xecinit). They are
                // reported but never installed or matched to a KMI.
                Kind::Helper
            };
        }
    } else {
        asset.kind = Kind::Unknown;
    }
    asset
}

/// `android<digits>-<digits>.<digits>`, the exact shape of a KMI name.
fn is_kmi_name(b: &[u8]) -> bool {
    let Some(rest) = b.strip_prefix(b"android") else {
        return false;
    };
    let Some(dash) = rest.iter().position(|&c| c == b'-') else {
        return false;
    };
    let (android, rest) = rest.split_at(dash);
    if android.is_empty() || !android.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let rest = &rest[1..];
    let Some(dot) = rest.iter().position(|&c| c == b'.') else {
        return false;
    };
    let (major, minor) = rest.split_at(dot);
    let minor = &minor[1..];
    !major.is_empty()
        && major.iter().all(u8::is_ascii_digit)
        && !minor.is_empty()
        && minor.iter().all(u8::is_ascii_digit)
}

/// The KMI names this APK was built with. They appear verbatim as
/// `<androidNN>-<major>.<minor>_kernelsu.ko` (or `_sevenk.ko` in 7kimisu) in the embedded
/// asset name table, and that table is one concatenated run, so the name has to be recovered
/// by looking strictly backwards from the suffix and validating the shape -- walking back over
/// "name" bytes alone would run straight through the neighbouring names.
pub fn supported_kmi(lib: &[u8]) -> Vec<String> {
    const SUFFIXES: [&[u8]; 2] = [b"_kernelsu.ko", b"_sevenk.ko"];
    /// Longest plausible KMI name (`android17-6.18`); a bound keeps the window small.
    const WINDOW: usize = 32;
    let mut out: Vec<String> = Vec::new();
    for suffix in SUFFIXES {
        let mut search = 0usize;
        while let Some(rel) = elf::find(&lib[search..], suffix) {
            let at = search + rel;
            search = at + suffix.len();
            let win_from = at.saturating_sub(WINDOW);
            let win = &lib[win_from..at];
            // Longest suffix of the window that is a valid KMI name and ends right here.
            for len in (1..=win.len()).rev() {
                let cand = &win[win.len() - len..];
                if is_kmi_name(cand) {
                    let kmi = String::from_utf8_lossy(cand).into_owned();
                    if !out.contains(&kmi) {
                        out.push(kmi);
                    }
                    break;
                }
            }
        }
    }
    out.sort();
    out
}

/// Filename the manager's init expects for the module inside the ramdisk.
///
/// KernelSU and most forks install `kernelsu.ko`; 7kimisu installs `sevenk.ko` and its
/// ksuinit looks for exactly that name.
pub fn module_file_name(lib: &[u8]) -> &'static str {
    if elf::find(lib, b"_sevenk.ko").is_some() {
        "sevenk.ko"
    } else {
        crate::boot::MODULE_NAME
    }
}

/// (Android release, clang version) pairs a module states about itself: the DDK build path
/// names the KMI, and the module records the toolchain it was compiled with. Pooling these
/// across every native library in the APK matters, because one ABI may carry the DDK path
/// that another ABI needs to break a tie.
pub fn ddk_anchors(found: &[Asset]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for a in found.iter().filter(|a| a.kind == Kind::Module) {
        if let (Some(kmi), Some(cl)) = (a.ddk_kmi.as_deref(), a.clang.as_deref()) {
            if let Some(android) = kmi.split('-').next() {
                let pair = (android.to_string(), cl.to_string());
                if !out.contains(&pair) {
                    out.push(pair);
                }
            }
        }
    }
    out
}

pub struct Judge {
    /// (asset index, KMI) for every module that could be resolved.
    pub assigned: Vec<(usize, String)>,
    pub module_count: usize,
    /// Modules that could not be pinned to a unique KMI.
    pub unresolved: Vec<usize>,
}

struct Mod {
    idx: usize,
    xy: Option<String>,
    ddk: Option<String>,
    clang: Option<String>,
}

/// Match every embedded module to a KMI name.
///
/// `vermagic` alone is not enough: it only carries the kernel `major.minor`, and several
/// KMI names share one (android12-5.10 / android13-5.10). The layers below add the
/// evidence needed to break those ties. Anything still ambiguous is reported rather than
/// guessed.
pub fn judge(assets: &mut [Asset], supported: &[String], seed: &[(String, String)]) -> Judge {
    let mods: Vec<Mod> = assets
        .iter()
        .enumerate()
        .filter(|(_, a)| a.kind == Kind::Module)
        .map(|(idx, a)| Mod {
            idx,
            xy: a.vermagic.as_deref().and_then(elf::kernel_xy),
            ddk: a.ddk_kmi.clone(),
            clang: a.clang.clone(),
        })
        .collect();

    let mut owner: Vec<Option<usize>> = vec![None; supported.len()];
    let mut reason: Vec<Option<String>> = vec![None; assets.len()];
    let at = |mods: &Vec<Mod>, idx: usize| mods.iter().position(|m| m.idx == idx);
    let taken = |owner: &Vec<Option<usize>>, idx: usize| owner.iter().any(|o| *o == Some(idx));

    // Layer 1 -- authoritative: the module carries the DDK source path it was built
    // against, which names its own KMI.
    for m in &mods {
        if let Some(k) = m.ddk.as_deref() {
            if let Some(si) = supported.iter().position(|s| s == k) {
                if owner[si].is_none() {
                    owner[si] = Some(m.idx);
                    reason[m.idx] = Some(format!(
                        "module embeds DDK source path `/opt/ddk/src/{k}/`, which names this KMI"
                    ));
                }
            }
        }
    }

    // Layer 2 -- constraint propagation. Vermagic fixes the kernel major.minor; if only
    // one remaining KMI name shares it, the match is forced.
    propagate(&mut owner, &mods, supported, &mut reason);

    // Layer 3 -- toolchain anchor. An Android release pins a clang version, so a clang
    // version observed on a module whose KMI is already known identifies the Android
    // release of an unresolved one. `seed` carries anchors learned elsewhere (another
    // ABI, or a DDK path seen in any library).
    let mut anchors: Vec<(String, String)> = seed.to_vec();
    for (si, o) in owner.iter().enumerate() {
        if let Some(idx) = o {
            let Some(p) = at(&mods, *idx) else { continue };
            if let (Some(android), Some(cl)) =
                (supported[si].split('-').next(), mods[p].clang.as_deref())
            {
                if !anchors.iter().any(|(a, _)| a == android) {
                    anchors.push((android.to_string(), cl.to_string()));
                }
            }
        }
    }
    for m in &mods {
        if taken(&owner, m.idx) {
            continue;
        }
        let (Some(cl), Some(xy)) = (m.clang.as_deref(), m.xy.as_deref()) else {
            continue;
        };
        let mut androids: Vec<String> = anchors
            .iter()
            .filter(|(_, a)| a == cl)
            .map(|(x, _)| x.clone())
            .collect();
        androids.sort();
        androids.dedup();
        if androids.len() == 1 {
            let want = format!("{}-{}", androids[0], xy);
            if let Some(si) = supported.iter().position(|s| *s == want) {
                if owner[si].is_none() {
                    owner[si] = Some(m.idx);
                    reason[m.idx] = Some(format!(
                        "clang {cl} anchors to Android release {} (from already-resolved modules), and kernel {xy} gives {want}",
                        androids[0]
                    ));
                }
            }
        }
    }

    // Layer 4 -- re-run propagation. Layer 3 typically resolves one member of a
    // shared-kernel pair, which then forces the other.
    propagate(&mut owner, &mods, supported, &mut reason);

    let mut assigned = Vec::new();
    let mut unresolved = Vec::new();
    for (si, o) in owner.iter().enumerate() {
        if let Some(idx) = o {
            assigned.push((*idx, supported[si].clone()));
            if let Some(r) = reason[*idx].take() {
                assets[*idx].evidence.push(r);
            }
            assets[*idx].kmi = Some(supported[si].clone());
        }
    }
    for m in &mods {
        if !taken(&owner, m.idx) {
            unresolved.push(m.idx);
        }
    }
    assigned.sort();
    unresolved.sort();

    Judge {
        assigned,
        module_count: mods.len(),
        unresolved,
    }
}

/// Forced-assignment pass. A module's vermagic fixes the kernel `major.minor`; if only
/// one remaining KMI name shares it -- or only one remaining module has it -- the pair is
/// forced. Applied in both directions until nothing more can be deduced.
fn propagate(
    owner: &mut Vec<Option<usize>>,
    mods: &[Mod],
    supported: &[String],
    reason: &mut Vec<Option<String>>,
) {
    let taken = |owner: &Vec<Option<usize>>, idx: usize| owner.iter().any(|o| *o == Some(idx));
    loop {
        let mut progress = false;
        for m in mods {
            if taken(owner, m.idx) {
                continue;
            }
            let Some(xy) = m.xy.as_deref() else { continue };
            let cands: Vec<usize> = (0..supported.len())
                .filter(|&si| owner[si].is_none() && supported[si].ends_with(&format!("-{xy}")))
                .collect();
            if cands.len() == 1 {
                owner[cands[0]] = Some(m.idx);
                reason[m.idx] = Some(format!(
                    "vermagic kernel {xy} leaves exactly one candidate ({})",
                    supported[cands[0]]
                ));
                progress = true;
            }
        }
        for si in 0..supported.len() {
            if owner[si].is_some() {
                continue;
            }
            let xy = supported[si].rsplit('-').next().unwrap_or("").to_string();
            let cands: Vec<usize> = mods
                .iter()
                .filter(|m| !taken(owner, m.idx) && m.xy.as_deref() == Some(xy.as_str()))
                .map(|m| m.idx)
                .collect();
            if cands.len() == 1 {
                owner[si] = Some(cands[0]);
                reason[cands[0]] = Some(format!(
                    "only one unassigned module has kernel {xy}, which {} requires",
                    supported[si]
                ));
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
}
