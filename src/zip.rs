//! Minimal ZIP (APK) container reader.
//!
//! Only what this tool needs: read the central directory, then extract entries that are
//! either STORED or DEFLATE-compressed. No full-featured zip crate is pulled in, which
//! keeps the binary small.

use std::path::Path;

const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_EOCD64: u32 = 0x0606_4b50;
const SIG_EOCD64_LOCATOR: u32 = 0x0706_4b50;
const SIG_CENTRAL: u32 = 0x0201_4b50;
const SIG_LOCAL: u32 = 0x0403_4b50;

const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u64le(b: &[u8], o: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(v)
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    /// Compression method as stored in the central directory.
    pub method: u16,
    /// Compressed size.
    pub csize: u64,
    /// Uncompressed size.
    pub usize_: u64,
    /// Offset of the local file header.
    pub local_off: u64,
}

pub struct Zip {
    data: Vec<u8>,
    pub entries: Vec<Entry>,
}

impl Zip {
    /// Open an APK/ZIP from disk.
    pub fn open(path: &Path) -> Result<Zip, String> {
        let data = std::fs::read(path)
            .map_err(|e| format!("cannot read file `{}`: {}", path.display(), e))?;
        Self::from_bytes(data)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Zip, String> {
        if data.len() < 22 {
            return Err(format!(
                "file is {} bytes, too small to contain a ZIP end-of-central-directory record",
                data.len()
            ));
        }
        if u32le(&data, 0) != SIG_LOCAL && u32le(&data, 0) != SIG_EOCD {
            return Err(format!(
                "not a ZIP/APK container: first 4 bytes are {:02X?}, expected a local file \
                 header (50 4B 03 04) or an empty-archive EOCD (50 4B 05 06)",
                &data[..4]
            ));
        }
        let eocd = find_eocd(&data)?;
        // NOTE: these are 16-bit fields, not 32-bit. Reading them as u32 swallows the
        // low half of the central-directory size and yields a huge bogus count.
        let mut entry_count = u16le(&data, eocd + 10) as u64;
        let mut cd_off = u32le(&data, eocd + 16) as u64;

        // ZIP64: the 32-bit fields are saturated, so the real values live in the ZIP64
        // end-of-central-directory record which the locator points at.
        if entry_count == 0xFFFF || cd_off == 0xFFFF_FFFF {
            let loc = eocd
                .checked_sub(20)
                .filter(|&o| u32le(&data, o) == SIG_EOCD64_LOCATOR)
                .ok_or_else(|| {
                    "ZIP64 archive but the ZIP64 end-of-central-directory locator is missing"
                        .to_string()
                })?;
            let z64 = usize::try_from(u64le(&data, loc + 8))
                .map_err(|_| "ZIP64 end-of-central-directory offset is too large".to_string())?;
            let z64_end = z64
                .checked_add(56)
                .ok_or_else(|| "ZIP64 end-of-central-directory offset overflows".to_string())?;
            if z64_end > data.len() || u32le(&data, z64) != SIG_EOCD64 {
                return Err("ZIP64 end-of-central-directory record is out of range".to_string());
            }
            entry_count = u64le(&data, z64 + 32);
            cd_off = u64le(&data, z64 + 48);
        }

        let max_entries = data.len() / 46 + 1;
        if entry_count > max_entries as u64 {
            return Err(format!(
                "archive declares {entry_count} entries, more than the file can hold"
            ));
        }
        let mut entries = Vec::with_capacity(entry_count as usize);
        let mut off = usize::try_from(cd_off)
            .map_err(|_| "central directory offset is too large".to_string())?;
        for i in 0..entry_count {
            let entry_end = off
                .checked_add(46)
                .ok_or_else(|| "central directory offset overflows".to_string())?;
            if entry_end > data.len() {
                return Err(format!(
                    "central directory entry #{} runs past end of file (offset {:#X}, file size {})",
                    i,
                    off,
                    data.len()
                ));
            }
            if u32le(&data, off) != SIG_CENTRAL {
                return Err(format!(
                    "central directory entry #{} at {:#X} has bad signature {:#010X}",
                    i,
                    off,
                    u32le(&data, off)
                ));
            }
            let method = u16le(&data, off + 10);
            let mut csize = u32le(&data, off + 20) as u64;
            let mut usize_ = u32le(&data, off + 24) as u64;
            let name_len = u16le(&data, off + 28) as usize;
            let extra_len = u16le(&data, off + 30) as usize;
            let comment_len = u16le(&data, off + 32) as usize;
            let mut local_off = u32le(&data, off + 42) as u64;

            let name_end = off
                .checked_add(46)
                .and_then(|v| v.checked_add(name_len))
                .ok_or_else(|| format!("central directory entry #{} name offset overflows", i))?;
            if name_end > data.len() {
                return Err(format!(
                    "central directory entry #{} has a truncated name",
                    i
                ));
            }
            let name = String::from_utf8_lossy(&data[off + 46..name_end]).into_owned();

            // ZIP64 extended information extra field (id 0x0001) carries the real 64-bit
            // values for whichever 32-bit fields were saturated, in a fixed order.
            let extra_end = name_end
                .checked_add(extra_len)
                .ok_or_else(|| format!("central directory entry #{} extra offset overflows", i))?;
            if extra_end <= data.len() {
                let extra = &data[name_end..extra_end];
                let mut x = 0usize;
                while x + 4 <= extra.len() {
                    let id = u16le(extra, x);
                    let sz = u16le(extra, x + 2) as usize;
                    if x + 4 + sz > extra.len() {
                        break;
                    }
                    if id == 0x0001 {
                        let f = &extra[x + 4..x + 4 + sz];
                        let mut p = 0usize;
                        if usize_ == 0xFFFF_FFFF && p + 8 <= f.len() {
                            usize_ = u64le(f, p);
                            p += 8;
                        }
                        if csize == 0xFFFF_FFFF && p + 8 <= f.len() {
                            csize = u64le(f, p);
                            p += 8;
                        }
                        if local_off == 0xFFFF_FFFF && p + 8 <= f.len() {
                            local_off = u64le(f, p);
                        }
                    }
                    x += 4 + sz;
                }
            }

            entries.push(Entry {
                name,
                method,
                csize,
                usize_,
                local_off,
            });
            off = extra_end
                .checked_add(comment_len)
                .ok_or_else(|| format!("central directory entry #{} offset overflows", i))?;
        }

        Ok(Zip { data, entries })
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Extract one entry, inflating it when necessary.
    pub fn read(&self, e: &Entry) -> Result<Vec<u8>, String> {
        let o = usize::try_from(e.local_off)
            .map_err(|_| format!("local header for `{}` offset is too large", e.name))?;
        let local_end = o
            .checked_add(30)
            .ok_or_else(|| format!("local header for `{}` offset overflows", e.name))?;
        if local_end > self.data.len() {
            return Err(format!(
                "local header for `{}` at {:#X} is out of range",
                e.name, o
            ));
        }
        if u32le(&self.data, o) != SIG_LOCAL {
            return Err(format!(
                "local header for `{}` at {:#X} has bad signature {:#010X}",
                e.name,
                o,
                u32le(&self.data, o)
            ));
        }
        let name_len = u16le(&self.data, o + 26) as usize;
        let extra_len = u16le(&self.data, o + 28) as usize;
        let start = o
            .checked_add(30)
            .and_then(|v| v.checked_add(name_len))
            .and_then(|v| v.checked_add(extra_len))
            .ok_or_else(|| format!("entry `{}` payload offset overflows", e.name))?;
        let size = usize::try_from(e.csize)
            .map_err(|_| format!("entry `{}` compressed size is too large", e.name))?;
        let end = start
            .checked_add(size)
            .ok_or_else(|| format!("entry `{}` payload offset overflows", e.name))?;
        if end > self.data.len() {
            return Err(format!(
                "entry `{}` payload is truncated: needs bytes {}..{} but the file is {} bytes",
                e.name,
                start,
                end,
                self.data.len()
            ));
        }
        let raw = &self.data[start..end];
        match e.method {
            METHOD_STORED => Ok(raw.to_vec()),
            METHOD_DEFLATE => miniz_oxide::inflate::decompress_to_vec(raw).map_err(|err| {
                format!("entry `{}` failed to inflate (method 8): {:?}", e.name, err)
            }),
            other => Err(format!(
                "entry `{}` uses compression method {} which this tool does not implement \
                 (only 0=stored and 8=deflate are supported)",
                e.name, other
            )),
        }
    }
}

fn find_eocd(data: &[u8]) -> Result<usize, String> {
    let n = data.len();
    let lowest = n.saturating_sub(22 + 0xFFFF);
    let mut i = n - 22;
    loop {
        if u32le(data, i) == SIG_EOCD {
            let clen = u16le(data, i + 20) as usize;
            if i.checked_add(22).and_then(|v| v.checked_add(clen)) == Some(n) {
                return Ok(i);
            }
        }
        if i == lowest {
            break;
        }
        i -= 1;
    }
    Err("no ZIP end-of-central-directory record found (file is not a valid APK/ZIP)".to_string())
}
