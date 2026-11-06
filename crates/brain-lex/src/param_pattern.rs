//! Bounded, deterministic ASCII segment constraints shared by frontend routes and redirects.
//! Author patterns are parsed here; emitted regular expressions contain only compiler-owned hex escapes/counts.
use crate::Json;
use std::collections::BTreeMap;

pub const MAX_INPUT: usize = 256;
#[derive(Debug, Clone, PartialEq, Eq)]
struct Atom {
    set: u128,
    min: usize,
    max: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    atoms: Vec<Atom>,
}
pub type Params = BTreeMap<String, Pattern>;
pub type RouteParams = BTreeMap<String, Params>;

fn problem(msg: &str) -> String {
    format!(
        "{msg}; fix: use nonempty ASCII literals/classes with fixed or deterministic bounded counts, e.g. [2-9a-z]{{7}}"
    )
}
fn allowed(b: u8) -> bool {
    (33..=126).contains(&b) && b != b'/' && b != b'\\'
}
fn character(bytes: &[u8], at: &mut usize, class: bool) -> Result<u8, String> {
    let b = *bytes
        .get(*at)
        .ok_or_else(|| problem("incomplete pattern"))?;
    *at += 1;
    let escaped = b == b'\\';
    let b = if escaped {
        let b = *bytes.get(*at).ok_or_else(|| problem("incomplete escape"))?;
        *at += 1;
        b
    } else {
        b
    };
    if !allowed(b)
        || (!escaped
            && if class {
                b == b']' || b == b'^'
            } else {
                b".[]{}()*+?|^$".contains(&b)
            })
    {
        return Err(problem("unsupported pattern character"));
    }
    if escaped && b.is_ascii_alphanumeric() {
        return Err(problem(
            "character shortcuts and backreferences are not supported",
        ));
    }
    Ok(b)
}
fn count(bytes: &[u8], at: &mut usize) -> Result<usize, String> {
    let start = *at;
    let mut n = 0usize;
    while let Some(b'0'..=b'9') = bytes.get(*at) {
        n = n * 10 + usize::from(bytes[*at] - b'0');
        *at += 1;
        if n > MAX_INPUT {
            return Err(problem("pattern count exceeds 256"));
        }
    }
    if *at == start {
        Err(problem("a count needs an integer"))
    } else {
        Ok(n)
    }
}
impl Pattern {
    pub fn parse(src: &str) -> Result<Self, String> {
        if src.is_empty() || src.len() > 256 || !src.is_ascii() {
            return Err(problem("pattern must contain 1..256 ASCII bytes"));
        }
        let b = src.as_bytes();
        let mut at = 0;
        let mut atoms = Vec::new();
        let (mut lo, mut hi) = (0, 0);
        while at < b.len() {
            if atoms.len() == 32 {
                return Err(problem("pattern exceeds 32 atoms"));
            }
            let set = if b[at] == b'[' {
                at += 1;
                let mut set = 0u128;
                let mut entries = 0;
                while at < b.len() && b[at] != b']' {
                    let start = character(b, &mut at, true)?;
                    let end =
                        if b.get(at) == Some(&b'-') && b.get(at + 1).is_some_and(|c| *c != b']') {
                            at += 1;
                            character(b, &mut at, true)?
                        } else {
                            start
                        };
                    if end < start {
                        return Err(problem("class range is reversed"));
                    }
                    for c in start..=end {
                        if !allowed(c) {
                            return Err(problem("class includes a separator or control"));
                        }
                        set |= 1u128 << c;
                    }
                    entries += 1;
                }
                if entries == 0 || b.get(at) != Some(&b']') {
                    return Err(problem("class must be nonempty and closed"));
                }
                at += 1;
                set
            } else {
                1u128 << character(b, &mut at, false)?
            };
            let (mut min, mut max) = (1, 1);
            if b.get(at) == Some(&b'{') {
                at += 1;
                min = count(b, &mut at)?;
                max = min;
                if b.get(at) == Some(&b',') {
                    at += 1;
                    max = count(b, &mut at)?;
                }
                if b.get(at) != Some(&b'}') || min > max {
                    return Err(problem("invalid bounded count"));
                }
                at += 1;
            }
            lo += min;
            hi += max;
            if hi > MAX_INPUT {
                return Err(problem("pattern maximum exceeds 256 characters"));
            }
            atoms.push(Atom { set, min, max });
        }
        if lo == 0 {
            return Err(problem("pattern must require at least one character"));
        }
        for (i, a) in atoms.iter().enumerate().filter(|(_, a)| a.min != a.max) {
            for next in &atoms[i + 1..] {
                if next.max > 0 && a.set & next.set != 0 {
                    return Err(problem("variable count overlaps its continuation"));
                }
                if next.min > 0 {
                    break;
                }
            }
        }
        Ok(Self { atoms })
    }
    /// Greedy matching is exact because every variable atom is disjoint from its nullable FOLLOW set.
    /// No recursion, retry, regex engine or input-sized allocation: O(atoms + input), input <=256 bytes.
    pub fn matches(&self, input: &str) -> bool {
        if input.is_empty() || input.len() > MAX_INPUT || !input.is_ascii() {
            return false;
        }
        let b = input.as_bytes();
        let mut at = 0;
        for a in &self.atoms {
            let start = at;
            while at < b.len() && at - start < a.max && a.set & (1u128 << b[at]) != 0 {
                at += 1;
            }
            if at - start < a.min {
                return false;
            }
        }
        at == b.len()
    }
    /// A single raw segment, with an early length guard. Every token is emitted from validated atoms.
    pub fn regex_source(&self) -> String {
        let mut out =
            String::from("(?=[\\x21-\\x2e\\x30-\\x5b\\x5d-\\x7e]{1,256}(?:/|(?![\\s\\S])))");
        for a in &self.atoms {
            out.push('[');
            let mut c = 0usize;
            while c < 128 {
                if a.set & (1u128 << c) == 0 {
                    c += 1;
                    continue;
                }
                let first = c;
                while c + 1 < 128 && a.set & (1u128 << (c + 1)) != 0 {
                    c += 1;
                }
                out.push_str(&format!("\\x{first:02x}"));
                if c > first {
                    out.push_str(&format!("-\\x{c:02x}"));
                }
                c += 1;
            }
            out.push(']');
            if a.min == a.max {
                out.push_str(&format!("{{{}}}", a.min));
            } else {
                out.push_str(&format!("{{{},{}}}", a.min, a.max));
            }
        }
        out
    }
}

fn object<'a>(j: &'a Json, label: &str) -> Result<&'a [(String, Json)], String> {
    let Json::Obj(kv) = j else {
        return Err(format!(
            "{label} must be an object; fix: map parameter names to pattern strings"
        ));
    };
    for (i, (key, _)) in kv.iter().enumerate() {
        if kv[..i].iter().any(|(k, _)| k == key) {
            return Err(format!(
                "{label} has a duplicate key; fix: keep each key once"
            ));
        }
    }
    Ok(kv)
}
/// Parse constraints on the named, single-segment parameters of one canonical route.
pub fn parse_params(j: &Json, route: &str, label: &str) -> Result<Params, String> {
    let kv = object(j, label)?;
    if kv.len() > 32 {
        return Err(format!("{label} exceeds 32 parameters"));
    }
    let mut out = Params::new();
    for (name, value) in kv {
        if !route
            .split('/')
            .any(|s| s.strip_prefix(':') == Some(name.as_str()))
        {
            return Err(format!(
                "{label} names an unknown single-segment parameter; fix: use a name declared by the route"
            ));
        }
        let src = value
            .as_str()
            .ok_or_else(|| format!("{label} pattern must be a string"))?;
        let p = Pattern::parse(src).map_err(|e| format!("{label}: {e}"))?;
        out.insert(name.clone(), p);
    }
    if !out.is_empty() {
        let mut names = Vec::new();
        for name in route.split('/').filter_map(|s| s.strip_prefix(':')) {
            if names.contains(&name) {
                return Err(format!(
                    "{label} route repeats a parameter; fix: use unique parameter names"
                ));
            }
            names.push(name);
        }
    }
    Ok(out)
}
/// Read only this opt-in key, rejecting duplicate selectors before get-first can change its meaning.
pub fn from_config(j: &Json) -> Result<RouteParams, String> {
    let entries = match j {
        Json::Obj(kv) => kv
            .iter()
            .filter(|(k, _)| k == "routeParams")
            .collect::<Vec<_>>(),
        _ => vec![],
    };
    if entries.len() > 1 {
        return Err(
            "brain.json routeParams is duplicated; fix: keep one routeParams object".into(),
        );
    }
    let Some((_, value)) = entries.first() else {
        return Ok(RouteParams::new());
    };
    let mut out = RouteParams::new();
    let kv = object(value, "brain.json routeParams")?;
    if kv.len() > 1024 {
        return Err("brain.json routeParams exceeds 1024 routes".into());
    }
    for (route, j) in kv {
        if !route.starts_with('/')
            || route.ends_with('/')
            || route.contains("//")
            || !route.split('/').any(|s| s.starts_with(':'))
        {
            return Err(
                "brain.json routeParams needs exact parameterized brain map paths, e.g. /items/:id"
                    .into(),
            );
        }
        out.insert(
            route.clone(),
            parse_params(j, route, "brain.json routeParams")?,
        );
    }
    Ok(out)
}
/// Applies only opt-in restrictions; the route table still owns structural matching and precedence.
pub fn allows(route: &str, path: &str, all: &RouteParams) -> bool {
    let Some(params) = all.get(route) else {
        return true;
    };
    let mut values = Vec::new();
    let mut got = path.split('/').filter(|s| !s.is_empty());
    for segment in route.split('/').filter(|s| !s.is_empty()) {
        let Some(value) = got.next() else {
            return false;
        };
        if let Some(name) = segment.strip_prefix(':') {
            values.push((name, value));
        }
    }
    params.iter().all(|(name, p)| {
        values
            .iter()
            .find(|(n, _)| n == name)
            .is_some_and(|(_, value)| p.matches(value))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grammar_and_linear_matching() {
        let p = Pattern::parse("[2-9a-z]{7}").unwrap();
        for s in ["2abcdef", "abc2345", "zzzzzzz"] {
            assert!(p.matches(s));
        }
        for s in [
            "zzz",
            "0abcdef",
            "Abc2345",
            "%32abcd",
            "2abcdefg",
            "a/bcdef",
            &"a".repeat(257),
        ] {
            assert!(!p.matches(s));
        }
        let p = Pattern::parse("[a-z]{1,8}-[0-9]{2,4}").unwrap();
        assert!(p.matches("a-23"));
        assert!(!p.matches("abcdefghi-23"));
        assert!(Pattern::parse("[a-z]{1,3}[0-9]{0,2}x").is_err());
        assert!(Pattern::parse("[a-z]{1,3}[0-9]{0,2}-").is_ok());
        assert!(Pattern::parse("a{2}a{2}").unwrap().matches("aaaa"));
    }
    #[test]
    fn unsupported_and_bounded() {
        for s in [
            "",
            "a*",
            "a+",
            "a?",
            "(a)",
            "a|b",
            "[^a]",
            "[z-a]",
            "[a-z]{1,3}a",
            "a{1,}",
            "a{257}",
            "a{0}",
            "\\d",
            "\\1",
            "a/b",
            "[a-z]{0,2}[a-z]{0,2}b",
            "[ -z]",
        ] {
            assert!(Pattern::parse(s).is_err(), "{s}");
        }
        assert!(Pattern::parse(&"a".repeat(33)).is_err());
        assert!(Pattern::parse("[a-z]{256}").is_ok());
    }
    #[test]
    fn config_is_strict() {
        for s in [
            r#"{"routeParams":{},"routeParams":{}}"#,
            r#"{"routeParams":{"/:id":{"id":"a","id":"b"}}}"#,
            r#"{"routeParams":{"/:id":{"missing":"a"}}}"#,
            r#"{"routeParams":{"/static":{"id":"a"}}}"#,
        ] {
            assert!(from_config(&Json::parse(s).unwrap()).is_err(), "{s}");
        }
    }
}
