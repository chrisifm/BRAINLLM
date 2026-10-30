//! INTERNALS §18: errors point at `.brain` lines. While generating, the codegen prefixes statements with
//! `/*@L<line>*/` markers (source line of what follows); [`finish`] strips them and builds a Source Map v3 that
//! maps every generated line to the `.brain` line of the nearest marker above it.
//!
//! Input is the framework emitter's output, never arbitrary or foreign JavaScript: author JavaScript comments
//! were discarded by the BrainScript parser. Marker-looking data survives in strings, template pieces, regexes
//! and quoted HTML/CSS, so only exact marker comments in the own lexer's trivia may act as directives.

struct Marker {
    start: usize,
    end: usize,
    line: u32,
}

fn trivia_markers(js: &str, mut at: usize, end: usize, markers: &mut Vec<Marker>) {
    while at < end {
        let rest = &js[at..end];
        if rest.starts_with("//") {
            at += rest.find('\n').map_or(rest.len(), |n| n + 1);
        } else if rest.starts_with("/*") {
            let Some(close) = rest[2..].find("*/").map(|n| n + 2) else { return };
            if let Some(number) = rest[2..close].strip_prefix("@L")
                && !number.starts_with('0')
                && !number.is_empty()
                && number.bytes().all(|byte| byte.is_ascii_digit())
                && let Ok(line) = number.parse::<u32>()
            {
                markers.push(Marker { start: at, end: at + close + 2, line });
            }
            at += close + 2;
        } else {
            at += rest.chars().next().expect("nonempty trivia").len_utf8();
        }
    }
}

/// Literal/template/regex token ranges are opaque. Template substitutions are normal tokens again, including
/// nested function bodies; their owned statement markers must still be stripped and mapped.
fn emitted_markers(js: &str, file: &str) -> Result<Vec<Marker>, brain_lex::Diag> {
    if !js.contains("/*@L") { return Ok(Vec::new()); }
    let index = brain_lex::LineIndex::new(js);
    let generated = format!("generated:{file}");
    let mut lexer = brain_lex::Lexer::new(js, &generated, &index, 0, js.len());
    let mut previous = 0;
    let mut markers = Vec::new();
    loop {
        let token = lexer.next_token_bytes().map_err(|mut diagnostic| {
            diagnostic.msg = format!("Framework-generated JavaScript could not be scanned for source maps: {}", diagnostic.msg);
            diagnostic.fix = format!("Report a compiler defect with component `{file}` and this generated JavaScript location; it is not an author source location. Do not edit generated output.");
            diagnostic.edits.clear();
            diagnostic
        })?;
        trivia_markers(js, previous, token.span.start as usize, &mut markers);
        if matches!(token.tok, brain_lex::Tok::Eof) { break; }
        previous = token.span.end as usize;
    }
    Ok(markers)
}

/// Canonical compiler-owned code before it becomes quoted metadata (for example HMR wiring).
/// Literal contents remain opaque, just as they do in the final source-map pass.
pub(crate) fn strip(js: &str, file: &str) -> Result<String, brain_lex::Diag> {
    let markers = emitted_markers(js, file)?;
    let mut out = String::with_capacity(js.len());
    let mut previous = 0;
    for marker in markers {
        out.push_str(&js[previous..marker.start]);
        previous = marker.end;
    }
    out.push_str(&js[previous..]);
    Ok(out)
}

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
pub(crate) fn finish(js: &str, file: &str, src: &str, inline: bool) -> Result<(String, String), brain_lex::Diag> {
    let markers = emitted_markers(js, file)?;
    let mut next_marker = 0;
    let mut offset = 0;
    let mut out = String::with_capacity(js.len());
    let mut mappings = String::new();
    let mut current: Option<i64> = None;
    let mut prev_src_line: i64 = 0;
    for (i, line) in js.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
            mappings.push(';');
        }
        let mut clean = String::with_capacity(line.len());
        let mut previous = 0;
        let end = offset + line.len();
        while let Some(marker) = markers.get(next_marker).filter(|marker| marker.start < end) {
            debug_assert!(marker.start >= offset && marker.end <= end);
            clean.push_str(&line[previous..marker.start - offset]);
            // Preserve the existing line-level convention: markers after emitted code do not replace it.
            if clean.trim().is_empty() || current.is_none() {
                current = Some(i64::from(marker.line));
            }
            previous = marker.end - offset;
            next_marker += 1;
        }
        clean.push_str(&line[previous..]);
        offset = end + 1;
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
    Ok((out, map))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_markers_are_trivia_not_literal_text() {
        let js = "/*@L2*/const a = 'naïve /*@L71*/';\n/*@L3*/const b = `text /*@L72*/ ${(() => {\n/*@L4*/return /[/*@L73*/]/.source;\n})()}`;\n// /*@L999*/\n/* ordinary /*@L888*/\n/*@L5*/const c = 1;";
        let (clean, _) = finish(js, "c.brain", "", false).unwrap();
        for data in ["'naïve /*@L71*/'", "`text /*@L72*/", "/[/*@L73*/]/", "// /*@L999*/", "/* ordinary /*@L888*/"] {
            assert!(clean.contains(data), "{data}: {clean}");
        }
        for line in [2, 3, 4, 5] { assert!(!clean.contains(&format!("/*@L{line}*/")), "{clean}"); }
    }

    #[test]
    fn malformed_comments_do_not_become_mapping_directives() {
        let js = "/*@L0*//*@L01*//*@L-1*//*@L4294967296*//*@Lnope*//*@L1:2*//*@L 2*/const n = 1;";
        assert_eq!(finish(js, "c.brain", "", false).unwrap().0, js);
        assert_eq!(finish("/*@L4294967295*/const n = 1;", "c.brain", "", false).unwrap().0, "const n = 1;");
    }

    #[test]
    fn unknown_generated_syntax_reports_a_generated_location() {
        let js = "/*@L3*/const unfinished = 'bad";
        let diagnostic = finish(js, "comp/c.brain", "author source", false).unwrap_err();
        assert_eq!(diagnostic.code, "B0001");
        assert_eq!(diagnostic.file, "generated:comp/c.brain");
        assert_eq!(diagnostic.line, 1);
        assert_eq!(diagnostic.col, 27);
        assert!(diagnostic.msg.starts_with("Framework-generated JavaScript"));
        assert!(diagnostic.fix.contains("not an author source location"));
        assert!(diagnostic.edits.is_empty());
    }
}
