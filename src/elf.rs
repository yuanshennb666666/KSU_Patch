//! Minimal ELF64 reader plus the byte-level evidence extractors used to identify the
//! embedded kernel modules.

pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

pub const ET_REL: u16 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;

pub const EM_X86_64: u16 = 62;
pub const EM_ARM: u16 = 40;
pub const EM_AARCH64: u16 = 183;
pub const EM_RISCV: u16 = 243;

#[derive(Debug, Clone, Copy)]
pub struct ElfInfo {
    pub etype: u16,
    pub machine: u16,
    pub shoff: u64,
    pub shentsize: u16,
    pub shnum: u16,
    pub shstrndx: u16,
}

impl ElfInfo {
    pub fn parse(b: &[u8]) -> Option<ElfInfo> {
        if b.len() < 64 || b[..4] != ELF_MAGIC {
            return None;
        }
        let class = b[4];
        // Only 64-bit little-endian objects are handled (all Android GKI targets).
        if class != 2 || b[5] != 1 {
            return None;
        }
        let rd16 = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let rd64 = |o: usize| {
            let mut v = [0u8; 8];
            v.copy_from_slice(&b[o..o + 8]);
            u64::from_le_bytes(v)
        };
        Some(ElfInfo {
            etype: rd16(16),
            machine: rd16(18),
            shoff: rd64(0x28),
            shentsize: rd16(0x3A),
            shnum: rd16(0x3C),
            shstrndx: rd16(0x3E),
        })
    }

    pub fn arch(&self) -> &'static str {
        match self.machine {
            EM_AARCH64 => "aarch64",
            EM_X86_64 => "x86_64",
            EM_ARM => "arm",
            EM_RISCV => "riscv64",
            _ => "unknown",
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self.etype {
            ET_REL => "ET_REL",
            ET_EXEC => "ET_EXEC",
            ET_DYN => "ET_DYN",
            _ => "unknown",
        }
    }

    /// Byte range of a named section, resolved through the section header string table.
    pub fn section(&self, b: &[u8], want: &str) -> Option<(usize, usize)> {
        if self.shoff == 0 || self.shnum == 0 || self.shentsize < 64 {
            return None;
        }
        let shoff = self.shoff as usize;
        let total = self.shnum as usize * self.shentsize as usize;
        if shoff.checked_add(total)? > b.len() || self.shstrndx >= self.shnum {
            return None;
        }
        let str_off = shoff + self.shstrndx as usize * self.shentsize as usize;
        let str_sec = str_off + 0x18;
        let so = read_u64(b, str_sec)? as usize;
        let ss = read_u64(b, str_sec + 8)? as usize;
        if so.checked_add(ss)? > b.len() {
            return None;
        }
        let strtab = &b[so..so + ss];
        for i in 0..self.shnum as usize {
            let o = shoff + i * self.shentsize as usize;
            let name_off = read_u32(b, o)? as usize;
            if name_off >= strtab.len() {
                continue;
            }
            let end = strtab[name_off..]
                .iter()
                .position(|&c| c == 0)
                .map(|p| name_off + p)
                .unwrap_or(strtab.len());
            if &strtab[name_off..end] == want.as_bytes() {
                let off = read_u64(b, o + 0x18)? as usize;
                let size = read_u64(b, o + 0x20)? as usize;
                if off.checked_add(size)? <= b.len() {
                    return Some((off, size));
                }
            }
        }
        None
    }
}

fn read_u32(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_u64(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o + 8).map(|s| {
        let mut v = [0u8; 8];
        v.copy_from_slice(s);
        u64::from_le_bytes(v)
    })
}

/// `.modinfo` content of a kernel module, as `key=value` pairs.
pub fn modinfo(b: &[u8], info: &ElfInfo) -> Vec<(String, String)> {
    let Some((off, size)) = info.section(b, ".modinfo") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for field in b[off..off + size].split(|&c| c == 0) {
        if field.is_empty() {
            continue;
        }
        let s = String::from_utf8_lossy(field).into_owned();
        match s.split_once('=') {
            Some((k, v)) => out.push((k.to_string(), v.to_string())),
            None => out.push((s, String::new())),
        }
    }
    out
}

/// `vermagic=` payload. It is the only place a module names the kernel it was built for.
pub fn vermagic(b: &[u8], info: &ElfInfo) -> Option<String> {
    modinfo(b, info)
        .into_iter()
        .find(|(k, _)| k == "vermagic")
        .map(|(_, v)| v)
}

/// Kernel `major.minor` taken from a vermagic string, e.g. `5.10.252-dirty` -> `5.10`.
pub fn kernel_xy(vermagic: &str) -> Option<String> {
    let head = vermagic.split(|c: char| !c.is_ascii_digit() && c != '.').next()?;
    let mut it = head.split('.');
    let major = it.next()?;
    let minor = it.next()?;
    if major.is_empty() || minor.is_empty() {
        return None;
    }
    Some(format!("{major}.{minor}"))
}

/// The Android DDK source path baked into a module by the build, e.g.
/// `/opt/ddk/src/android13-5.15/include/linux/poll.h` -> `android13-5.15`.
/// When present this is authoritative: the module names its own KMI.
pub fn ddk_kmi(b: &[u8]) -> Option<String> {
    const NEEDLE: &[u8] = b"/opt/ddk/src/";
    let at = find(b, NEEDLE)? + NEEDLE.len();
    let rest = &b[at..];
    let end = rest
        .iter()
        .position(|&c| c == b'/')
        .unwrap_or(rest.len());
    let s = String::from_utf8_lossy(&rest[..end.min(32)]).into_owned();
    if s.starts_with("android") && s.contains('-') {
        Some(s)
    } else {
        None
    }
}

/// Toolchain version, e.g. `14.0.7` from
/// `Android (8508608, ...) clang version 14.0.7 (...)`. Used as a cross-check anchor
/// between modules whose kernel version is ambiguous.
pub fn clang_version(b: &[u8]) -> Option<String> {
    const NEEDLE: &[u8] = b"clang version ";
    let at = find(b, NEEDLE)? + NEEDLE.len();
    let rest = &b[at..];
    let end = rest
        .iter()
        .position(|&c| !c.is_ascii_digit() && c != b'.')
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
}

pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
}
