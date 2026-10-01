//! SHA-1, in lowercase hex.
//!
//! Needed because an install that backs up the stock image records `stock_image.sha1` in
//! the ramdisk, and reproducing that byte for byte means computing the same digest. Small
//! enough to keep in-tree rather than pull another crate.

pub fn hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 80];
    for chunk in msg.chunks(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::hex;

    #[test]
    fn known_vectors() {
        assert_eq!(hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // Padding-boundary lengths: 55/56 and 63/64/65 straddle the block edges.
        assert_eq!(hex(&[b'a'; 55]), "c1c8bbdc22796e28c0e15163d20899b65621d65a");
        assert_eq!(hex(&[b'a'; 56]), "c2db330f6083854c99d4b5bfb6e8f29f201be699");
        assert_eq!(hex(&[b'a'; 63]), "03f09f5b158a7a8cdad920bddc29b81c18a551f5");
        assert_eq!(hex(&[b'a'; 64]), "0098ba824b5c16427bd7a1122a5a442a25ec644d");
        assert_eq!(hex(&[b'a'; 65]), "11655326c708d70319be2610e8a57d9a5b959d3b");
        assert_eq!(hex(&[b'a'; 120]), "f34c1488385346a55709ba056ddd08280dd4c6d6");
    }
}
