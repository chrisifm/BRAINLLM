//! Starter images for `brain new` (`public/`): the app icon at 192 and 512 px (web manifest) and a 1200×630 Open
//! Graph image. Drawn with plain geometry and written as PNG by hand (zlib over brain-pack's deflate, CRC-32 per
//! chunk): no image library, and the bytes are deterministic.

/// Accent of the starter theme (`--accent` in tokens.cssbrain).
const ACCENT: [u8; 3] = [0x00, 0x62, 0xc4];
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const BG: [u8; 3] = [0xf5, 0xf5, 0xf7];
const FG: [u8; 3] = [0x1d, 0x1d, 0x1f];

/// PNG (RGB, 8 bit) of `w`×`h` pixels from a pixel function.
pub fn png(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0); // filter: none
        for x in 0..w {
            raw.extend_from_slice(&px(x, y));
        }
    }
    let mut z = vec![0x78, 0x01];
    z.extend(brain_pack::gzip::deflate(&raw, 0));
    z.extend(adler32(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]); // depth 8, RGB, deflate, filter 0, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let mut c = kind.to_vec();
    c.extend_from_slice(data);
    out.extend(&c);
    out.extend(brain_pack::gzip::crc32(&c).to_be_bytes());
}

fn adler32(d: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in d {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

/// Coverage (0..1) of a `w`×`h` rectangle with corner radius `r` at pixel (x, y), antialiased over one pixel.
fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (cx, cy) = (x.clamp(r, w - r), y.clamp(r, h - r));
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r;
    let outside = (-x).max(x - w).max(-y).max(y - h);
    (0.5 - d.max(outside)).clamp(0.0, 1.0)
}

/// Coverage of the "b" mark (a stem and a ring) in a unit square, sampled at (u, v) ∈ [0, 1].
fn b_mark(u: f32, v: f32, px: f32) -> f32 {
    let stem = if (0.30..=0.42).contains(&u) && (0.22..=0.78).contains(&v) { 1.0 } else { 0.0 };
    let d = ((u - 0.555).powi(2) + (v - 0.6).powi(2)).sqrt();
    let ring = (1.0 - ((d - 0.155).abs() - 0.06) / px).clamp(0.0, 1.0);
    f32::max(stem, if u >= 0.36 { ring } else { 0.0 })
}

/// App icon: accent rounded square with the white mark.
pub fn icon(size: u32) -> Vec<u8> {
    let s = size as f32;
    png(size, size, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let cov = rounded(fx, fy, s, s, s * 0.22);
        let mark = b_mark(fx / s, fy / s, 1.0 / s);
        mix(BG, mix(ACCENT, WHITE, mark), cov)
    })
}

/// Open Graph image (1200×630): light background with a soft accent glow, the icon on the left and three lines that
/// stand for a title and its text (real text arrives with the generated OG images of brain-seo).
pub fn og() -> Vec<u8> {
    let (w, h) = (1200u32, 630u32);
    png(w, h, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let glow = (1.0 - (((fx - 1050.0).powi(2) + (fy + 80.0).powi(2)).sqrt() / 900.0)).clamp(0.0, 1.0) * 0.18;
        let mut c = mix(BG, ACCENT, glow);
        // icon 240 px at (120, 195)
        let (ix, iy, is) = (fx - 120.0, fy - 195.0, 240.0);
        if (-1.0..=is + 1.0).contains(&ix) && (-1.0..=is + 1.0).contains(&iy) {
            let cov = rounded(ix, iy, is, is, is * 0.22);
            c = mix(c, mix(ACCENT, WHITE, b_mark(ix / is, iy / is, 1.0 / is)), cov);
        }
        // title bar + two text bars
        for (bx, by, bw, bh, col) in [(430.0, 235.0, 560.0, 54.0, FG), (430.0, 325.0, 640.0, 22.0, mix(BG, FG, 0.45)), (430.0, 365.0, 470.0, 22.0, mix(BG, FG, 0.45))] {
            let cov = rounded(fx - bx, fy - by, bw, bh, bh / 2.0);
            c = mix(c, col, cov);
        }
        c
    })
}

/// `public/favicon.svg`: the same mark as vector.
pub const FAVICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#0062c4"/><rect x="19.2" y="14" width="7.7" height="36" fill="#fff"/><circle cx="35.5" cy="38.4" r="9.9" fill="none" stroke="#fff" stroke-width="7.7"/></svg>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_is_valid_and_deterministic() {
        let a = icon(32);
        assert_eq!(&a[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&a[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(a[16..20].try_into().unwrap()), 32);
        assert_eq!(a, icon(32), "same bytes every time");
        // the IDAT inflates back to (1 + 32*3) * 32 bytes: check with Node's zlib when available
        let o = og();
        assert_eq!(u32::from_be_bytes(o[16..20].try_into().unwrap()), 1200);
        assert_eq!(u32::from_be_bytes(o[20..24].try_into().unwrap()), 630);
        assert!(o.len() < 200_000, "flat colors compress: {} bytes", o.len());
    }
}
