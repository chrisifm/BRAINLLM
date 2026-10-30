//! brain-serve: dev HTTP/HMR plus an opt-in opaque native socket tunnel.
//! Native tunnels use one owned Tokio I/O worker; compiler and ordinary HTTP lifecycle remain separate.

pub(crate) mod http;
mod native_socket_proxy;
pub use native_socket_proxy::{NativeSocketProxy, SocketProxyDiagnostic, SocketProxyStats};
pub use server::native_proxy::NativeProxy;
pub mod load;
pub mod prod;
mod route;
mod server;
mod sha1;
mod ws;
pub mod feedback;

pub use server::{start_dev_with_native_proxy, serve, set_quiet, start, start_dev, CompileFn, Compiled, DiagnoseFn, Dev, EnvLoader, EventFn, Options, ProblemsFn};

/// Escapes a string as a JSON literal (quotes included).
pub fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                o.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// A flat JSON object (what `seo(props)` returns) → (key, value) pairs: strings unquoted, numbers and booleans as
/// written, objects/arrays kept as raw JSON (`jsonld`), nulls dropped. None if `s` is not an object.
pub fn json_flat(s: &str) -> Option<Vec<(String, String)>> {
    let b = s.trim().as_bytes();
    if b.first() != Some(&b'{') {
        return None;
    }
    // end of the JSON value starting at i (strings, nested objects/arrays, scalars)
    fn end(b: &[u8], mut i: usize) -> usize {
        let (mut depth, mut in_str) = (0i32, false);
        while i < b.len() {
            let c = b[i];
            if in_str {
                if c == b'\\' {
                    i += 1;
                } else if c == b'"' {
                    in_str = false;
                    if depth == 0 {
                        return i + 1;
                    }
                }
            } else {
                match c {
                    b'"' => in_str = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' if depth == 0 => return i,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return i + 1;
                        }
                    }
                    b',' if depth == 0 => return i,
                    _ => {}
                }
            }
            i += 1;
        }
        i
    }
    let s = std::str::from_utf8(b).ok()?;
    let mut out = Vec::new();
    let mut i = 1;
    loop {
        while i < b.len() && (b[i] as char).is_whitespace() || b.get(i) == Some(&b',') {
            i += 1;
        }
        if i >= b.len() || b[i] == b'}' {
            break;
        }
        let ke = end(b, i);
        let key = load::unquote(&s[i..ke]);
        i = ke;
        while i < b.len() && (b[i] == b':' || (b[i] as char).is_whitespace()) {
            i += 1;
        }
        let ve = end(b, i);
        let raw = s[i..ve].trim();
        i = ve;
        match raw.as_bytes().first() {
            Some(b'"') => out.push((key, load::unquote(raw))),
            Some(b'n') | None => {}
            _ => out.push((key, raw.to_string())),
        }
    }
    Some(out)
}

#[cfg(test)]
mod flat_tests {
    #[test]
    fn flat_object() {
        let v = super::json_flat(r#"{"title":"A \"b\"","n":3,"jsonld":{"@type":"Article","x":[1,{"y":"}"}]},"z":null,"og:image":"/i.png"}"#).unwrap();
        assert_eq!(v, vec![("title".into(), "A \"b\"".into()), ("n".into(), "3".into()), ("jsonld".into(), r#"{"@type":"Article","x":[1,{"y":"}"}]}"#.into()), ("og:image".into(), "/i.png".into())]);
    }
}
