//! Minimal hand-written JSON (RFC 8259), shared by every crate (diagnostics, MCP, SEO, build). No serde, by contract.
//! Strings are escaped by the same `push_json_str` the diagnostics use: one serializer in the workspace.

use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Insertion order is preserved (MCP clients appreciate it when reading).
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Json::Str(s) = self { Some(s) } else { None }
    }
    pub fn obj<const N: usize>(kv: [(&str, Json); N]) -> Json {
        Json::Obj(kv.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }

    pub fn parse(src: &str) -> Result<Json, String> {
        let mut p = Parser { b: src.as_bytes(), i: 0, depth: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("trailing garbage after JSON at byte {}", p.i));
        }
        Ok(v)
    }

    pub fn stringify(&self) -> String {
        let mut o = String::new();
        self.write(&mut o);
        o
    }

    fn write(&self, o: &mut String) {
        match self {
            Json::Null => o.push_str("null"),
            Json::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.0e15 => {
                let _ = write!(o, "{}", *n as i64);
            }
            Json::Num(n) if n.is_finite() => {
                let _ = write!(o, "{n}");
            }
            Json::Num(_) => o.push_str("null"),
            Json::Str(s) => write_str(o, s),
            Json::Arr(a) => {
                o.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    v.write(o);
                }
                o.push(']');
            }
            Json::Obj(kv) => {
                o.push('{');
                for (i, (k, v)) in kv.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    write_str(o, k);
                    o.push(':');
                    v.write(o);
                }
                o.push('}');
            }
        }
    }
}

fn write_str(o: &mut String, s: &str) {
    crate::diag::push_json_str(o, s);
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: u32,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.i))
    }
    fn lit(&mut self, s: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(s.as_bytes()) {
            self.i += s.len();
            Ok(v)
        } else {
            self.err("invalid literal")
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        let Some(&c) = self.b.get(self.i) else { return self.err("unexpected end of input") };
        match c {
            b'n' => self.lit("null", Json::Null),
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'"' => self.string().map(Json::Str),
            b'[' | b'{' => {
                self.depth += 1;
                if self.depth > 128 {
                    return self.err("nesting too deep");
                }
                let v = if c == b'[' { self.array() } else { self.object() };
                self.depth -= 1;
                v
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => self.err("unexpected character"),
        }
    }
    fn array(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut a = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Json::Arr(a));
        }
        loop {
            a.push(self.value()?);
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                _ => return self.err("expected `,` or `]`"),
            }
        }
    }
    fn object(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut kv = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Json::Obj(kv));
        }
        loop {
            self.ws();
            if self.b.get(self.i) != Some(&b'"') {
                return self.err("expected a string key");
            }
            let k = self.string()?;
            self.ws();
            if self.b.get(self.i) != Some(&b':') {
                return self.err("expected `:`");
            }
            self.i += 1;
            kv.push((k, self.value()?));
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Obj(kv));
                }
                _ => return self.err("expected `,` or `}`"),
            }
        }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.b.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok());
        match h.and_then(|h| u32::from_str_radix(h, 16).ok()) {
            Some(v) => {
                self.i += 4;
                Ok(v)
            }
            None => self.err("invalid \\u escape"),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.b.len() && !matches!(self.b[self.i], b'"' | b'\\') && self.b[self.i] >= 0x20 {
                self.i += 1;
            }
            // Cuts land on ASCII bytes, so the slice is valid UTF-8 (the input is &str).
            out.push_str(std::str::from_utf8(&self.b[start..self.i]).unwrap());
            match self.b.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let Some(&e) = self.b.get(self.i) else { return self.err("incomplete escape") };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.b[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return self.err("invalid surrogate pair");
                                }
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                            }
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => return self.err("unknown escape"),
                    }
                }
                Some(_) => return self.err("unescaped control character"),
                None => return self.err("unterminated string"),
            }
        }
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).unwrap();
        s.parse::<f64>().map(Json::Num).or_else(|_| self.err("invalid number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let src = r#"{"a":[1,-2.5,true,null],"s":"x\"y\\n\u00fc\ud83d\ude00","o":{}}"#;
        let v = Json::parse(src).unwrap();
        assert_eq!(v.get("s").unwrap().as_str().unwrap(), "x\"y\\nü😀");
        assert_eq!(v.stringify(), r#"{"a":[1,-2.5,true,null],"s":"x\"y\\nü😀","o":{}}"#);
        assert_eq!(Json::parse(&v.stringify()).unwrap(), v);
    }

    #[test]
    fn rejects_bad_input() {
        for bad in ["", "{", "[1,]", "{\"a\" 1}", "\"\u{1}\"", "tru", "1 2", "\"\\x\""] {
            assert!(Json::parse(bad).is_err(), "should have failed: {bad:?}");
        }
        assert!(Json::parse(&"[".repeat(200)).is_err());
    }
}
