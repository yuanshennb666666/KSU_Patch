//! Minimal Android binary XML (AXML) reader.
//!
//! Just enough to read the `<manifest>` element's `package`, `versionName` and
//! `versionCode` out of an APK's `AndroidManifest.xml`. The manifest is what anchors a
//! patch to a specific KernelSU release, so it is read rather than guessed.

#[derive(Debug, Default, Clone)]
pub struct Manifest {
    pub package: Option<String>,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
}

const CHUNK_STRING_POOL: u16 = 0x0001;
const CHUNK_XML_START_TAG: u16 = 0x0102;

const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;
const TYPE_INT_BOOLEAN: u8 = 0x12;

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

struct StringPool<'a> {
    buf: &'a [u8],
    utf8: bool,
    data_start: usize,
    offsets: Vec<u32>,
}

impl<'a> StringPool<'a> {
    fn parse(buf: &'a [u8], base: usize) -> Result<StringPool<'a>, String> {
        if base + 28 > buf.len() {
            return Err("string pool chunk header is out of range".to_string());
        }
        let count = u32le(buf, base + 8) as usize;
        let flags = u32le(buf, base + 16);
        let strings_start = u32le(buf, base + 20) as usize;
        if base + 28 + count * 4 > buf.len() {
            return Err(format!(
                "string pool declares {} strings but the offset table is out of range",
                count
            ));
        }
        let mut offsets = Vec::with_capacity(count);
        for i in 0..count {
            offsets.push(u32le(buf, base + 28 + i * 4));
        }
        Ok(StringPool {
            buf,
            utf8: flags & 0x100 != 0,
            data_start: base + strings_start,
            offsets,
        })
    }

    fn get(&self, idx: u32) -> Option<String> {
        if idx == 0xFFFF_FFFF {
            return None;
        }
        let off = *self.offsets.get(idx as usize)? as usize;
        let mut p = self.data_start.checked_add(off)?;
        if p >= self.buf.len() {
            return None;
        }
        if self.utf8 {
            // Length prefix is 1 or 2 bytes counting UTF-16 code units.
            let b0 = self.buf[p];
            p += if b0 & 0x80 != 0 { 2 } else { 1 };
            if p > self.buf.len() {
                return None;
            }
            let end = p + self.buf[p..].iter().position(|&c| c == 0)?;
            Some(String::from_utf8_lossy(&self.buf[p..end]).into_owned())
        } else {
            if p + 2 > self.buf.len() {
                return None;
            }
            let b0 = u16le(self.buf, p);
            p += if b0 & 0x8000 != 0 { 4 } else { 2 };
            let mut s = String::new();
            while p + 2 <= self.buf.len() {
                let u = u16le(self.buf, p);
                if u == 0 {
                    break;
                }
                p += 2;
                if (0xD800..0xDC00).contains(&u) && p + 2 <= self.buf.len() {
                    let lo = u16le(self.buf, p);
                    if (0xDC00..0xE000).contains(&lo) {
                        p += 2;
                        let c = 0x1_0000
                            + (((u as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                        if let Some(ch) = char::from_u32(c) {
                            s.push(ch);
                            continue;
                        }
                    }
                }
                if let Some(ch) = char::from_u32(u as u32) {
                    s.push(ch);
                }
            }
            Some(s)
        }
    }
}

pub fn parse(buf: &[u8]) -> Result<Manifest, String> {
    if buf.len() < 8 {
        return Err(format!(
            "AndroidManifest.xml is {} bytes, shorter than the binary XML chunk header",
            buf.len()
        ));
    }
    let ctype = u16le(buf, 0);
    if ctype != 0x0003 {
        return Err(format!(
            "AndroidManifest.xml is not binary XML: chunk type is {:#06X}, expected 0x0003",
            ctype
        ));
    }
    let header_size = u16le(buf, 2) as usize;
    let declared = u32le(buf, 4) as usize;
    let end = declared.min(buf.len()).max(8);

    let mut pool: Option<StringPool> = None;
    let mut out = Manifest::default();
    let mut off = if header_size >= 8 { header_size } else { 8 };

    while off + 8 <= end {
        let ctype = u16le(buf, off);
        let csize = u32le(buf, off + 4) as usize;
        if csize < 8 || off + csize > end {
            break;
        }
        match ctype {
            CHUNK_STRING_POOL => {
                pool = Some(StringPool::parse(buf, off)?);
            }
            CHUNK_XML_START_TAG => {
                if let Some(sp) = &pool {
                    let tag = sp.get(u32le(buf, off + 20)).unwrap_or_default();
                    if tag == "manifest" {
                        read_manifest_attrs(buf, off, end, sp, &mut out);
                        return Ok(out);
                    }
                }
            }
            _ => {}
        }
        off += csize;
    }
    Ok(out)
}

fn read_manifest_attrs(
    buf: &[u8],
    off: usize,
    end: usize,
    sp: &StringPool,
    out: &mut Manifest,
) {
    // ResXMLTree_node is 16 bytes; ResXMLTree_attrExt starts right after it and
    // `attributeStart` is relative to that.
    let attr_start = u16le(buf, off + 24) as usize;
    let attr_size = u16le(buf, off + 26) as usize;
    let attr_count = u16le(buf, off + 28) as usize;
    if attr_size < 20 {
        return;
    }
    let base = off + 16 + attr_start;
    for i in 0..attr_count {
        let a = base + i * attr_size;
        if a + 20 > end || a + 20 > buf.len() {
            break;
        }
        let name = sp.get(u32le(buf, a + 4)).unwrap_or_default();
        let data_type = buf[a + 15];
        let data = u32le(buf, a + 16);
        match name.as_str() {
            "package" => out.package = sp.get(data),
            "versionName" => out.version_name = sp.get(data),
            "versionCode" => {
                out.version_code = match data_type {
                    TYPE_STRING => sp.get(data).and_then(|s| s.trim().parse().ok()),
                    TYPE_INT_DEC | TYPE_INT_HEX | TYPE_INT_BOOLEAN => Some(data as u64),
                    _ => Some(data as u64),
                }
            }
            _ => {}
        }
    }
}
