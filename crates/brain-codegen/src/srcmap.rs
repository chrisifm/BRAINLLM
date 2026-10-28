//! INTERNALS §18: errors point at `.brain` lines. While generating, the codegen prefixes statements with
//! `/*@L<line>*/` markers (source line of what follows); [`finish`] strips them and builds a Source Map v3 that
//! maps every generated line to the `.brain` line of the nearest marker above it.

/// Marker for a source line (stripped by [`finish`]).
pub(crate) fn mark(line: u32) -> String {
    if line == 0 { String::new() } else { format!("/*@L{line}*/") }
}

fn vlq(v: i64, out: &mut String) {
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut n = if v < 0 { ((-v) << 1) | 1 } else { v << 1 };
    loop {
        let mut digit = n & 31;
        n >>= 5;
        if n > 0 {
            digit |= 32;
        }
        out.push(B64[digit as usize] as char);
        if n == 0 {
            break;
        }
    }
}

fn base64(data: &[u8]) -> String {
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Strips the markers from `js` and returns it with its source map (JSON). With `inline`, the map is also appended
/// as a `sourceMappingURL` data URL (dev).
pub(crate) fn finish(js: &str, file: &str, src: &str, inline: bool) -> (String, String) {
    let mut out = String::with_capacity(js.len());
    let mut mappings = String::new();
    let mut current: Option<i64> = None;
    let mut prev_src_line: i64 = 0;
    for (i, line) in js.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
            mappings.push(';');
        }
        let mut rest = line;
        let mut clean = String::with_capacity(line.len());
        while let Some(at) = rest.find("/*@L") {
            clean.push_str(&rest[..at]);
            let after = &rest[at + 4..];
            match after.find("*/") {
                Some(end) => {
                    if let Ok(n) = after[..end].parse::<i64>() {
                        // The first marker of a line wins: it is where the line's code starts.
                        if clean.trim().is_empty() || current.is_none() {
                            current = Some(n);
                        }
                    }
                    rest = &after[end + 2..];
                }
                None => {
                    clean.push_str(&rest[at..]);
                    rest = "";
                }
            }
        }
        clean.push_str(rest);
        out.push_str(&clean);
        if let Some(n) = current
            && !clean.trim().is_empty()
        {
            // [generated column, source index, source line, source column], relative except the column.
            let src_line = n - 1;
            vlq(0, &mut mappings);
            vlq(0, &mut mappings);
            vlq(src_line - prev_src_line, &mut mappings);
            vlq(0, &mut mappings);
            prev_src_line = src_line;
        }
    }
    let name = file.rsplit('/').next().unwrap_or(file);
    let mut map = String::from("{\"version\":3,\"file\":");
    brain_parse::push_json_str(&mut map, &format!("{name}.js"));
    map.push_str(",\"sources\":[");
    brain_parse::push_json_str(&mut map, name);
    map.push_str("],\"sourcesContent\":[");
    brain_parse::push_json_str(&mut map, src);
    map.push_str("],\"names\":[],\"mappings\":");
    brain_parse::push_json_str(&mut map, &mappings);
    map.push('}');
    if inline {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("//# sourceMappingURL=data:application/json;base64,");
        out.push_str(&base64(map.as_bytes()));
        out.push('\n');
    }
    (out, map)
}
