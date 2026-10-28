//! `brain spec [--topic X]`: the dense spec for the LLM (SPEC.md), whole or a single section.

/// Full SPEC.md, including what is planned (for `--all`).
pub const SPEC_ALL: &str = include_str!("../../../SPEC.md");

/// What is served to the LLM: without the `<!--planned:#N-->` … `<!--/planned-->` blocks, so the spec only
/// promises what the tool already does. SPEC.md is validated at compile time (build.rs), so it cannot fail here.
pub fn served() -> String {
    strip_planned(SPEC_ALL).expect("SPEC.md validated in build.rs")
}

/// Removes the planned blocks. Strict rule: each marker goes ALONE on its line. A marker that shares its
/// line with text, an unclosed block, an orphan close or a nested one is an error with a line number: a filter
/// that fails silently leaves the LLM without half the spec (2026-10-13: Sonnet 0/20 because of that).
pub fn strip_planned(src: &str) -> Result<String, String> {
    let mut out = String::with_capacity(src.len());
    let mut open: Option<usize> = None;
    for (n, line) in src.split_inclusive('\n').enumerate() {
        let n = n + 1;
        let t = line.trim();
        let has_open = t.contains("<!--planned");
        let has_close = t.contains("<!--/planned-->");
        if has_open || has_close {
            let alone = (has_open && !has_close && t.starts_with("<!--planned") && t.ends_with("-->") && t.matches("-->").count() == 1)
                || (has_close && !has_open && t == "<!--/planned-->");
            if !alone {
                return Err(format!("SPEC.md:{n}: the planned marker must be alone on its line: `{t}`"));
            }
            if has_open {
                if let Some(o) = open {
                    return Err(format!("SPEC.md:{n}: nested planned block (the previous one opens on line {o})"));
                }
                open = Some(n);
            } else if open.take().is_none() {
                return Err(format!("SPEC.md:{n}: `<!--/planned-->` without an opening"));
            }
            continue;
        }
        if open.is_none() {
            out.push_str(line);
        }
    }
    if let Some(o) = open {
        return Err(format!("SPEC.md:{o}: planned block without `<!--/planned-->`"));
    }
    Ok(out)
}

/// Topic → prefixes of the SPEC.md `## ` headings that cover it.
pub const TOPICS: &[(&str, &[&str])] = &[
    ("files", &["Files"]),
    ("architecture", &["Architecture"]),
    ("errors", &["Errors"]),
    ("component", &["Component"]),
    ("script", &["Component", "BrainScript"]),
    ("markup", &["Markup"]),
    ("channels", &["Communication"]),
    ("css", &["CSS"]),
    ("a11y", &["Accessibility"]),
    ("seo", &["SEO"]),
    ("server", &["Server"]),
    ("tests", &["Tests"]),
    ("cli", &["CLI", "Loop"]),
    ("example", &["Reference component"]),
];

pub fn topic(name: &str, all: bool) -> Result<String, String> {
    let spec = if all { SPEC_ALL.to_string() } else { served() };
    let Some((_, heads)) = TOPICS.iter().find(|(t, _)| *t == name) else {
        let all: Vec<_> = TOPICS.iter().map(|(t, _)| *t).collect();
        return Err(format!("unknown topic `{name}`; fix: use one of {}", all.join(" ")));
    };
    let mut out = String::new();
    let mut take = false;
    for line in spec.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            take = heads.iter().any(|p| h.starts_with(p));
        } else if line.starts_with("# ") {
            take = false;
        }
        if take {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.is_empty() {
        return Err(format!("SPEC.md has no `{name}` section; fix: run the full `brain spec`"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planned_blocks_are_removed() {
        let src = "a\n<!--planned:#34-->\nfuture\n<!--/planned-->\nb\n  <!--planned:#1-->  \nx\n<!--/planned-->\nc\n";
        assert_eq!(strip_planned(src).unwrap(), "a\nb\nc\n");
    }

    #[test]
    fn broken_markers_fail_loudly() {
        let e = |s: &str| strip_planned(s).unwrap_err();
        assert!(e("a\n<!--planned:#1-->\nb\n").contains("without `<!--/planned-->`"));
        assert!(e("a\n<!--/planned-->\n").contains("without an opening"));
        assert!(e("<!--planned:#1-->\n<!--planned:#2-->\nx\n<!--/planned-->\n").contains("nested"));
        // the real case from SPEC.md:61: close tag followed by text
        assert!(e("<!--planned:#66-->\nlazy\n<!--/planned--> Text interpolation is escaped.\n").contains("SPEC.md:3: the planned marker must be alone"));
        assert!(e("text <!--planned:#2-->\nx\n<!--/planned-->\n").contains("alone on its line"));
    }

    /// The real SPEC.md: it validates and keeps every `## ` section that is not inside a planned block.
    #[test]
    fn real_spec_keeps_every_public_section() {
        let served = strip_planned(SPEC_ALL).unwrap_or_else(|e| panic!("SPEC.md: {e}"));
        let mut inside = false;
        let mut expected = vec![];
        for line in SPEC_ALL.lines() {
            if line.contains("<!--planned") {
                inside = true;
            }
            if !inside && line.starts_with("## ") {
                expected.push(line);
            }
            if line.contains("<!--/planned-->") {
                inside = false;
            }
        }
        for h in &expected {
            assert!(served.contains(h), "the served spec lost the `{h}` section");
        }
        assert!(expected.len() >= 8, "SPEC.md should have at least 8 public sections, it has {}", expected.len());
        assert!(!served.contains("planned"));
    }

    #[test]
    fn every_topic_resolves_and_is_smaller() {
        for (t, _) in TOPICS {
            let s = topic(t, true).unwrap_or_else(|e| panic!("{e}"));
            assert!(s.starts_with("## ") && s.len() < SPEC_ALL.len(), "{t}");
        }
        assert!(topic("nope", false).unwrap_err().contains("files"));
    }
}
