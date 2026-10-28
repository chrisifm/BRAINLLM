//! gzip (RFC 1952) over DEFLATE (RFC 1951), written by hand: LZ77 with hash chains, one dynamic-Huffman block with
//! length-limited codes (package-merge), and an optional Zopfli-style optimal parse that iterates a shortest-path
//! search over the match graph with costs taken from the previous pass's Huffman code lengths.
//! Build-time only: speed matters less than bytes.

const WINDOW: usize = 32 * 1024;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;

// ---------- bit writer ----------

struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn new() -> Bits {
        Bits { out: Vec::new(), acc: 0, n: 0 }
    }
    /// Writes `len` bits of `v`, least significant first (DEFLATE order for headers and extra bits).
    fn put(&mut self, v: u32, len: u32) {
        self.acc |= (v as u64) << self.n;
        self.n += len;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    /// Writes a Huffman code (codes are defined MSB first, so they are bit-reversed).
    fn code(&mut self, code: u32, len: u32) {
        let mut r = 0;
        for i in 0..len {
            r |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.put(r, len);
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

// ---------- symbols ----------

#[derive(Clone, Copy)]
enum Sym {
    Lit(u8),
    /// (length 3..=258, distance 1..=32768)
    Match(u16, u16),
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn len_code(len: u16) -> usize {
    LEN_BASE.iter().rposition(|&b| b <= len).unwrap()
}
fn dist_code(d: u16) -> usize {
    DIST_BASE.iter().rposition(|&b| b <= d).unwrap()
}

// ---------- LZ77 match finder ----------

struct Matcher<'a> {
    data: &'a [u8],
    head: Vec<i32>,
    prev: Vec<i32>,
    max_chain: usize,
}

impl<'a> Matcher<'a> {
    fn new(data: &'a [u8], max_chain: usize) -> Matcher<'a> {
        Matcher { data, head: vec![-1; 1 << 15], prev: vec![-1; data.len()], max_chain }
    }
    fn hash(&self, i: usize) -> usize {
        let d = self.data;
        ((d[i] as usize) << 10 ^ (d[i + 1] as usize) << 5 ^ d[i + 2] as usize) & 0x7fff
    }
    fn insert(&mut self, i: usize) {
        if i + MIN_MATCH <= self.data.len() {
            let h = self.hash(i);
            self.prev[i] = self.head[h];
            self.head[h] = i as i32;
        }
    }
    /// For position `i` (already inserted callers excluded), the best length found for each distance class:
    /// returns `best[len] = shortest distance` for every length reachable (3..=max).
    fn matches(&self, i: usize, out: &mut Vec<(u16, u16)>) {
        out.clear();
        let d = self.data;
        if i + MIN_MATCH > d.len() {
            return;
        }
        let max = MAX_MATCH.min(d.len() - i);
        let mut cand = self.head[self.hash(i)];
        let mut chain = 0;
        let mut best = MIN_MATCH - 1;
        while cand >= 0 && chain < self.max_chain {
            let c = cand as usize;
            if c >= i {
                cand = self.prev[c];
                continue;
            }
            let dist = i - c;
            if dist > WINDOW {
                break;
            }
            if d[c + best] == d[i + best] {
                let mut l = 0;
                while l < max && d[c + l] == d[i + l] {
                    l += 1;
                }
                if l > best {
                    // every length between the previous best and this one is reachable at this (shorter) distance
                    out.push((l as u16, dist as u16));
                    best = l;
                    if l == max {
                        break;
                    }
                }
            }
            cand = self.prev[c];
            chain += 1;
        }
    }
}

/// Greedy-with-lazy-evaluation parse (zlib level 9 style).
fn lazy_parse(data: &[u8]) -> Vec<Sym> {
    let mut m = Matcher::new(data, 4096);
    let mut syms = Vec::with_capacity(data.len() / 2);
    let mut buf = Vec::new();
    let mut i = 0;
    let best = |m: &Matcher, i: usize, buf: &mut Vec<(u16, u16)>| -> Option<(u16, u16)> {
        m.matches(i, buf);
        buf.last().copied()
    };
    while i < data.len() {
        let cur = best(&m, i, &mut buf);
        match cur {
            Some((l, d)) => {
                // lazy: if the next position has a longer match, emit a literal now
                m.insert(i);
                let next = if i + 1 < data.len() { best(&m, i + 1, &mut buf) } else { None };
                if next.is_some_and(|(nl, _)| nl > l + 1) {
                    syms.push(Sym::Lit(data[i]));
                    i += 1;
                    continue;
                }
                syms.push(Sym::Match(l, d));
                for k in 1..l as usize {
                    m.insert(i + k);
                }
                i += l as usize;
            }
            None => {
                syms.push(Sym::Lit(data[i]));
                m.insert(i);
                i += 1;
            }
        }
    }
    syms
}

/// Shortest-path parse: cost of every literal/length/distance symbol in bits, from a previous code.
fn optimal_parse(data: &[u8], lit_len: &[u8; 286], dist_len: &[u8; 30]) -> Vec<Sym> {
    let n = data.len();
    let cost_lit = |b: u8| if lit_len[b as usize] == 0 { 15.0 } else { lit_len[b as usize] as f64 };
    let cost_match = |l: u16, d: u16| {
        let lc = len_code(l);
        let dc = dist_code(d);
        let ll = if lit_len[257 + lc] == 0 { 15.0 } else { lit_len[257 + lc] as f64 };
        let dl = if dist_len[dc] == 0 { 15.0 } else { dist_len[dc] as f64 };
        ll + LEN_EXTRA[lc] as f64 + dl + DIST_EXTRA[dc] as f64
    };
    let mut m = Matcher::new(data, 8192);
    let mut cost = vec![f64::INFINITY; n + 1];
    let mut from: Vec<(u32, Sym)> = vec![(0, Sym::Lit(0)); n + 1];
    cost[0] = 0.0;
    let mut buf = Vec::new();
    for i in 0..n {
        let c = cost[i];
        let lc = c + cost_lit(data[i]);
        if lc < cost[i + 1] {
            cost[i + 1] = lc;
            from[i + 1] = (i as u32, Sym::Lit(data[i]));
        }
        m.matches(i, &mut buf);
        let mut prev_len = MIN_MATCH as u16 - 1;
        for &(l, d) in buf.iter() {
            // lengths (prev_len, l] are all available at distance d
            for len in (prev_len + 1)..=l {
                let j = i + len as usize;
                let mc = c + cost_match(len, d);
                if mc < cost[j] {
                    cost[j] = mc;
                    from[j] = (i as u32, Sym::Match(len, d));
                }
            }
            prev_len = l;
        }
        m.insert(i);
    }
    let mut syms = Vec::new();
    let mut j = n;
    while j > 0 {
        let (i, s) = from[j];
        syms.push(s);
        j = i as usize;
    }
    syms.reverse();
    syms
}

// ---------- Huffman ----------

/// Length-limited Huffman code lengths (package-merge). Symbols with frequency 0 get length 0.
fn code_lengths(freq: &[u32], max_bits: u32) -> Vec<u8> {
    let mut lens = vec![0u8; freq.len()];
    let mut syms: Vec<(u32, usize)> = freq.iter().enumerate().filter(|(_, f)| **f > 0).map(|(i, f)| (*f, i)).collect();
    match syms.len() {
        0 => return lens,
        1 => {
            lens[syms[0].1] = 1;
            return lens;
        }
        _ => {}
    }
    syms.sort();
    // Each item: (weight, list of symbols it contains)
    let leaves: Vec<(u64, Vec<usize>)> = syms.iter().map(|(f, s)| (*f as u64, vec![*s])).collect();
    let mut cur = leaves.clone();
    for _ in 1..max_bits {
        let mut packaged: Vec<(u64, Vec<usize>)> = Vec::with_capacity(cur.len() / 2);
        for pair in cur.chunks_exact(2) {
            let mut v = pair[0].1.clone();
            v.extend_from_slice(&pair[1].1);
            packaged.push((pair[0].0 + pair[1].0, v));
        }
        // merge leaves + packaged by weight
        let mut merged = Vec::with_capacity(leaves.len() + packaged.len());
        let (mut a, mut b) = (0, 0);
        while a < leaves.len() || b < packaged.len() {
            if b >= packaged.len() || (a < leaves.len() && leaves[a].0 <= packaged[b].0) {
                merged.push(leaves[a].clone());
                a += 1;
            } else {
                merged.push(packaged[b].clone());
                b += 1;
            }
        }
        cur = merged;
    }
    for item in cur.iter().take(2 * syms.len() - 2) {
        for &s in &item.1 {
            lens[s] += 1;
        }
    }
    lens
}

/// Canonical codes from lengths (RFC 1951 §3.2.2).
fn canonical(lens: &[u8]) -> Vec<u32> {
    let mut bl_count = [0u32; 16];
    for &l in lens {
        if l > 0 {
            bl_count[l as usize] += 1;
        }
    }
    let mut next = [0u32; 16];
    let mut code = 0;
    for bits in 1..16 {
        code = (code + bl_count[bits - 1]) << 1;
        next[bits] = code;
    }
    lens.iter()
        .map(|&l| {
            if l == 0 {
                0
            } else {
                let c = next[l as usize];
                next[l as usize] += 1;
                c
            }
        })
        .collect()
}

fn stats(syms: &[Sym]) -> ([u32; 286], [u32; 30]) {
    let mut lf = [0u32; 286];
    let mut df = [0u32; 30];
    for s in syms {
        match *s {
            Sym::Lit(b) => lf[b as usize] += 1,
            Sym::Match(l, d) => {
                lf[257 + len_code(l)] += 1;
                df[dist_code(d)] += 1;
            }
        }
    }
    lf[256] = 1;
    (lf, df)
}

fn lengths_for(syms: &[Sym]) -> ([u8; 286], [u8; 30]) {
    let (lf, mut df) = stats(syms);
    if df.iter().all(|&f| f == 0) {
        df[0] = 1; // a block must declare at least one distance code
    }
    let ll: [u8; 286] = code_lengths(&lf, 15).try_into().unwrap();
    let dl: [u8; 30] = code_lengths(&df, 15).try_into().unwrap();
    (ll, dl)
}

/// Run-length encoding of the concatenated code lengths with symbols 16/17/18.
fn rle(lens: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lens.len() {
        let v = lens[i];
        let mut run = 1;
        while i + run < lens.len() && lens[i + run] == v {
            run += 1;
        }
        let mut left = run;
        if v == 0 {
            while left >= 11 {
                let k = left.min(138);
                out.push((18, (k - 11) as u8));
                left -= k;
            }
            if left >= 3 {
                out.push((17, (left - 3) as u8));
                left = 0;
            }
        } else {
            out.push((v, 0));
            left -= 1;
            while left >= 3 {
                let k = left.min(6);
                out.push((16, (k - 3) as u8));
                left -= k;
            }
        }
        for _ in 0..left {
            out.push((v, 0));
        }
        i += run;
    }
    out
}

/// Size in bits of a block written with `syms` (used to pick the best parse).
fn block_bits(syms: &[Sym]) -> usize {
    let (ll, dl) = lengths_for(syms);
    let mut bits = 0usize;
    for s in syms {
        match *s {
            Sym::Lit(b) => bits += ll[b as usize] as usize,
            Sym::Match(l, d) => {
                let lc = len_code(l);
                let dc = dist_code(d);
                bits += ll[257 + lc] as usize + LEN_EXTRA[lc] as usize + dl[dc] as usize + DIST_EXTRA[dc] as usize;
            }
        }
    }
    bits + ll[256] as usize + 600 // header estimate, same for every candidate
}

fn write_block(syms: &[Sym]) -> Vec<u8> {
    let (ll, dl) = lengths_for(syms);
    let hlit = (257..=286).rev().find(|&n| ll[n - 1] != 0).unwrap_or(257).max(257);
    let hdist = (1..=30).rev().find(|&n| dl[n - 1] != 0).unwrap_or(1);
    let mut all: Vec<u8> = ll[..hlit].to_vec();
    all.extend_from_slice(&dl[..hdist]);
    let runs = rle(&all);
    let mut cf = [0u32; 19];
    for (s, _) in &runs {
        cf[*s as usize] += 1;
    }
    let cl: [u8; 19] = code_lengths(&cf, 7).try_into().unwrap();
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let hclen = (4..=19).rev().find(|&n| cl[ORDER[n - 1]] != 0).unwrap_or(4);
    let (lc, dc, cc) = (canonical(&ll), canonical(&dl), canonical(&cl));

    let mut w = Bits::new();
    w.put(1, 1); // BFINAL
    w.put(2, 2); // dynamic Huffman
    w.put((hlit - 257) as u32, 5);
    w.put((hdist - 1) as u32, 5);
    w.put((hclen - 4) as u32, 4);
    for &o in ORDER.iter().take(hclen) {
        w.put(cl[o] as u32, 3);
    }
    for (s, extra) in &runs {
        w.code(cc[*s as usize], cl[*s as usize] as u32);
        match s {
            16 => w.put(*extra as u32, 2),
            17 => w.put(*extra as u32, 3),
            18 => w.put(*extra as u32, 7),
            _ => {}
        }
    }
    for s in syms {
        match *s {
            Sym::Lit(b) => w.code(lc[b as usize], ll[b as usize] as u32),
            Sym::Match(l, d) => {
                let i = len_code(l);
                w.code(lc[257 + i], ll[257 + i] as u32);
                w.put((l - LEN_BASE[i]) as u32, LEN_EXTRA[i] as u32);
                let j = dist_code(d);
                w.code(dc[j], dl[j] as u32);
                w.put((d - DIST_BASE[j]) as u32, DIST_EXTRA[j] as u32);
            }
        }
    }
    w.code(lc[256], ll[256] as u32);
    w.finish()
}

/// Raw DEFLATE stream. `iterations` > 0 runs the optimal parser that many times (Zopfli-style); 0 = lazy only.
pub fn deflate(data: &[u8], iterations: u32) -> Vec<u8> {
    if data.is_empty() {
        // fixed-Huffman block with only end-of-block
        return vec![0x03, 0x00];
    }
    let mut best = lazy_parse(data);
    let mut best_bits = block_bits(&best);
    for _ in 0..iterations {
        let (ll, dl) = lengths_for(&best);
        let cand = optimal_parse(data, &ll, &dl);
        let bits = block_bits(&cand);
        if bits < best_bits {
            best = cand;
            best_bits = bits;
        } else {
            break;
        }
    }
    write_block(&best)
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// gzip member with no file name and mtime 0 (reproducible builds).
pub fn gzip(data: &[u8], iterations: u32) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 0xff];
    out.extend(deflate(data, iterations));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// Decompresses with the system `gzip -dc` (test only); `None` if gzip is not installed.
    fn gunzip(z: &[u8]) -> Option<Vec<u8>> {
        let mut p = Command::new("gzip").arg("-dc").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
        p.stdin.take().unwrap().write_all(z).unwrap();
        let o = p.wait_with_output().ok()?;
        assert!(o.status.success(), "gzip -dc failed: {}", String::from_utf8_lossy(&o.stderr));
        Some(o.stdout)
    }

    fn samples() -> Vec<Vec<u8>> {
        let js = include_str!("lib.rs").as_bytes().to_vec();
        let mut v = vec![b"".to_vec(), b"a".to_vec(), b"abcabcabcabcabcabc".to_vec(), vec![0u8; 100_000], js];
        let mut x: u32 = 12345;
        v.push((0..70_000).map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x % 7) as u8 + b'a'
        }).collect());
        v
    }

    #[test]
    fn roundtrips_through_system_gzip() {
        for s in samples() {
            for it in [0, 3] {
                let z = gzip(&s, it);
                if let Some(back) = gunzip(&z) {
                    assert_eq!(back, s, "len {} iterations {it}", s.len());
                }
            }
        }
    }

    #[test]
    fn crc_and_lengths() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let l = code_lengths(&[1, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536], 7);
        assert!(l.iter().all(|&b| b <= 7 && b > 0));
        // Kraft equality for a complete code
        let sum: f64 = l.iter().map(|&b| 2f64.powi(-(b as i32))).sum();
        assert!((sum - 1.0).abs() < 1e-9, "{sum}");
    }

    #[test]
    fn optimal_parse_is_not_worse() {
        let s = include_str!("lib.rs").as_bytes();
        assert!(deflate(s, 5).len() <= deflate(s, 0).len());
    }
}
