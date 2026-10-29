//! `brain guide [topic] [--json]`: task playbooks for the LLM that writes an app, embedded in the binary from
//! `crates/brain-cli/guides/*.md`. `brain spec` says what exists; a guide says what to do for one kind of task, in
//! which order, which command verifies it and which diagnostic catches each common mistake. Also served over MCP as
//! `brain_guide`. Every statement must hold for the current code: tests below check that each diagnostic code and each
//! `brain <command>` a guide names exists.

use brain_mcp::Json;

/// `(topic, one-line summary, markdown)`, in the order a new session reads them.
pub const GUIDES: &[(&str, &str, &str)] = &[
    ("start", "the working loop: spec → map → add → fix/check → test → doctor → build, which command when", include_str!("../guides/start.md")),
    ("views", "routes from files, layouts, guards, 404/403, navigation", include_str!("../guides/views.md")),
    ("data", "load, server functions, api() + resource(), channels, caching", include_str!("../guides/data.md")),
    ("forms", "form() schema, submit, server field errors, edit forms", include_str!("../guides/forms.md")),
    ("server", "sessions, server-only code, .brain_env, errors, deploy with brain start", include_str!("../guides/server.md")),
    ("seo", "pick the target query first; answer-first pages for search and AI answers (GEO)", include_str!("../guides/seo.md")),
    ("a11y", "accessibility rules the build enforces and how to pass them first time", include_str!("../guides/a11y.md")),
    ("webmcp", "exposing page actions to browser AI agents: when, names, descriptions", include_str!("../guides/webmcp.md")),
    ("tests", "colocated *.test.brain: mount, interact, assert every state", include_str!("../guides/tests.md")),
    ("i18n", "several languages: keys, plurals, formatting, localized routes", include_str!("../guides/i18n.md")),
    ("performance", "measure, budgets, lazy components, images, long lists, workers", include_str!("../guides/performance.md")),
];

fn find(topic: &str) -> Result<&'static (&'static str, &'static str, &'static str), String> {
    GUIDES.iter().find(|g| g.0 == topic).ok_or_else(|| {
        let all: Vec<_> = GUIDES.iter().map(|g| g.0).collect();
        format!("unknown guide `{topic}`; fix: use one of {}", all.join(" "))
    })
}

/// The list of guides as text (no topic) or the guide's markdown.
pub fn text(topic: Option<&str>) -> Result<String, String> {
    match topic {
        Some(t) => find(t).map(|g| g.2.to_string()),
        None => {
            let w = GUIDES.iter().map(|g| g.0.len()).max().unwrap_or(0);
            let mut s = String::from("BRAINLLM guides: run `brain guide <topic>` before each kind of task (start with `start`).\n\n");
            for (t, sum, _) in GUIDES {
                s.push_str(&format!("  {t:w$}  {sum}\n"));
            }
            Ok(s)
        }
    }
}

/// `--json`: `{schemaVersion, guides: [{topic, summary}]}` or `{schemaVersion, topic, summary, guide}`.
pub fn json(topic: Option<&str>) -> Result<Json, String> {
    let v = ("schemaVersion", Json::Num(1.0));
    match topic {
        Some(t) => find(t).map(|(t, sum, body)| Json::obj([v, ("topic", Json::str(*t)), ("summary", Json::str(*sum)), ("guide", Json::str(*body))])),
        None => Ok(Json::obj([
            v,
            ("guides", Json::Arr(GUIDES.iter().map(|(t, sum, _)| Json::obj([("topic", Json::str(*t)), ("summary", Json::str(*sum))])).collect())),
        ])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The topics the owner asked for (2026-10-29); removing one is a decision, not a refactor.
    #[test]
    fn every_required_topic_exists_and_is_a_dense_playbook() {
        for t in ["start", "views", "data", "forms", "server", "seo", "a11y", "webmcp", "tests", "i18n", "performance"] {
            let (_, sum, body) = find(t).unwrap_or_else(|e| panic!("{e}"));
            assert!(!sum.trim().is_empty(), "{t}: empty summary");
            assert!(body.starts_with(&format!("# guide: {t} ")), "{t}: must open with `# guide: {t} — …`");
            let lines = body.lines().count();
            assert!((30..=130).contains(&lines), "{t}: {lines} lines; keep a playbook between 30 and ~120 lines");
            for section in ["## When", "## Verify", "## Common mistakes", "## Minimal correct example"] {
                assert!(body.contains(section), "{t}: missing `{section}`");
            }
            assert!(!body.contains("<!--planned"), "{t}: guides mark planned features in prose, not with SPEC markers");
        }
        assert!(text(Some("nope")).unwrap_err().contains("start views data"));
    }

    #[test]
    fn listing_and_json_cover_every_guide() {
        let list = text(None).unwrap();
        let j = json(None).unwrap().stringify();
        for (t, sum, _) in GUIDES {
            assert!(list.contains(sum) && j.contains(&format!("\"topic\":\"{t}\"")), "{t}");
        }
        let seo = json(Some("seo")).unwrap();
        assert!(seo.get("guide").and_then(Json::as_str).is_some_and(|g| g.contains("target query")));
    }

    /// Every diagnostic code a guide cites resolves in `brain explain` (no invented codes).
    #[test]
    fn every_cited_code_exists() {
        for (t, _, body) in GUIDES {
            let b = body.as_bytes();
            for i in 0..b.len().saturating_sub(4) {
                let boundary = i == 0 || !b[i - 1].is_ascii_alphanumeric();
                if boundary && b[i] == b'B' && b[i + 1..i + 5].iter().all(u8::is_ascii_digit) && b.get(i + 5).is_none_or(|c| !c.is_ascii_alphanumeric()) {
                    let code = &body[i..i + 5];
                    assert!(crate::explain_json(code).is_ok(), "guide `{t}` cites {code}, which `brain explain` does not know");
                }
            }
        }
    }

    /// Every `brain <command>` a guide names is a real command listed in `brain --help`.
    #[test]
    fn every_cited_command_exists() {
        let usage = crate::USAGE;
        let mut known: Vec<String> = vec![];
        for l in usage.lines() {
            let l = l.trim_start();
            if let Some(rest) = l.strip_prefix("brain ") {
                let words: Vec<&str> = rest.split_whitespace().collect();
                if let Some(w) = words.first() {
                    known.push(w.trim_start_matches("--").to_string());
                }
                // `brain stop | reload | status`
                for (k, w) in words.iter().enumerate() {
                    if *w == "|" {
                        if let Some(n) = words.get(k + 1) {
                            known.push(n.to_string());
                        }
                    }
                }
            }
        }
        for (t, _, body) in GUIDES {
            for (i, _) in body.match_indices("brain ") {
                let before = body[..i].chars().next_back();
                if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '/') {
                    continue;
                }
                let word: String = body[i + 6..].chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-').collect();
                if word.is_empty() {
                    continue;
                }
                assert!(known.iter().any(|k| *k == word), "guide `{t}` names `brain {word}`, which `brain --help` does not list");
            }
        }
    }

    /// Same rule as scripts/check-english.sh (which also scans these files): no accented letters, no Spanish words.
    #[test]
    fn guides_are_english() {
        let script = include_str!("../../../scripts/check-english.sh");
        let words_line = script.lines().find(|l| l.starts_with("words='")).expect("check-english.sh words list");
        let words: Vec<&str> = words_line.trim_start_matches("words='").trim_end_matches('\'').split('|').collect();
        for (t, _, body) in GUIDES {
            // the script's character class, written as escapes so this file passes it too
            let spanish = "\u{e1}\u{e9}\u{ed}\u{f3}\u{fa}\u{f1}\u{c1}\u{c9}\u{cd}\u{d3}\u{da}\u{d1}\u{bf}\u{a1}";
            assert!(!body.chars().any(|c| spanish.contains(c)), "guide `{t}` has a Spanish character");
            for w in body.split(|c: char| !c.is_alphanumeric()) {
                assert!(!words.contains(&w), "guide `{t}` has the Spanish word `{w}`");
            }
        }
    }
}
