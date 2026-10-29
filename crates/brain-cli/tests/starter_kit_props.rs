//! The templates that `brain new` and `brain add` write use the ui/ kit (`<Button>`, `<Field>`…) with no import, and
//! `brain check` does not yet check kit props (B0206 covers imported components only). This test keeps the templates
//! honest: every plain attribute on a kit component must be a prop in that component's `<meta props>`. It caught
//! `loading={f.busy}` on `<Button>` in the starter login (the prop is `busy`).

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn brain_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            brain_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "brain") {
            out.push(p);
        }
    }
}

fn kebab(name: &str) -> String {
    let mut s = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                s.push('-');
            }
            s.push(ch.to_ascii_lowercase());
        } else {
            s.push(ch);
        }
    }
    s
}

/// Names declared in `<meta props="a:str b:bool=false c:str?">`.
fn kit_props(src: &str) -> Vec<String> {
    let Some(i) = src.find("<meta props=\"") else { return vec![] };
    let rest = &src[i + "<meta props=\"".len()..];
    let body = &rest[..rest.find('"').unwrap()];
    // a default may hold spaces only inside quotes ('…'), so split on spaces outside them
    let (mut out, mut cur, mut q) = (vec![], String::new(), false);
    for ch in body.chars() {
        match ch {
            '\'' => {
                q = !q;
                cur.push(ch)
            }
            ' ' if !q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur))
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur)
    }
    out.into_iter().map(|p| p.split([':', '=', '?', '~']).next().unwrap().to_string()).collect()
}

/// `(component, attribute names)` for every `<Upper …>` opening tag.
fn tags(src: &str) -> Vec<(String, Vec<String>)> {
    let b = src.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'<' && b[i + 1].is_ascii_uppercase() {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric()) {
                j += 1;
            }
            let name = src[i + 1..j].to_string();
            let (mut attrs, mut depth, mut quote) = (vec![], 0i32, 0u8);
            let mut cur = String::new();
            while j < b.len() {
                let c = b[j];
                if quote != 0 {
                    if c == quote {
                        quote = 0
                    }
                } else if depth > 0 {
                    if c == b'{' {
                        depth += 1
                    } else if c == b'}' {
                        depth -= 1
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c
                } else if c == b'{' {
                    depth = 1
                } else if c == b'>' {
                    break;
                } else if c.is_ascii_alphanumeric() || c == b':' || c == b'-' || c == b'_' {
                    cur.push(c as char);
                } else {
                    if !cur.is_empty() {
                        attrs.push(std::mem::take(&mut cur))
                    }
                }
                if (quote != 0 || depth > 0 || c == b'=') && !cur.is_empty() {
                    attrs.push(std::mem::take(&mut cur));
                }
                j += 1;
            }
            if !cur.is_empty() {
                attrs.push(cur)
            }
            out.push((name, attrs));
            i = j;
        }
        i += 1;
    }
    out
}

#[test]
fn templates_use_only_declared_kit_props() {
    let mut files = vec![];
    brain_files(&root().join("crates/brain-cli/src/templates"), &mut files);
    let mut checked = 0;
    let mut bad = vec![];
    for f in &files {
        let src = fs::read_to_string(f).unwrap();
        for (comp, attrs) in tags(&src) {
            let kit = root().join("runtime/ui").join(format!("{}.brain", kebab(&comp)));
            let Ok(ksrc) = fs::read_to_string(&kit) else { continue };
            let props = kit_props(&ksrc);
            for a in attrs.iter().filter(|a| !a.contains(':') && a.as_str() != "lazy") {
                checked += 1;
                if !props.contains(a) {
                    bad.push(format!("{}: <{comp} {a}> (props: {})", f.strip_prefix(root()).unwrap_or(f).display(), props.join(", ")));
                }
            }
        }
    }
    assert!(checked > 0, "no kit component found in the templates");
    assert!(bad.is_empty(), "kit props that do not exist:\n{}", bad.join("\n"));
}

#[test]
fn parser_sees_the_old_starter_bug() {
    let t = tags(r#"<Button type="submit" loading={f.busy} on:click={go}>Sign in</Button>"#);
    assert_eq!(t[0].0, "Button");
    assert_eq!(t[0].1, ["type", "loading", "on:click"]);
    assert_eq!(kit_props("<meta props=\"variant:str='primary' busy:bool=false label:str?\">"), ["variant", "busy", "label"]);
}
