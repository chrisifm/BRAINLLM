//! Versioned change manifests: one JSON file per change under `crates/brain-cli/releases/<version>/`, embedded by
//! `build.rs` and shipped inside the binary of every npm package. They tell an LLM author what each version added
//! (capability + smallest adoption snippet + guide), what it deprecated or broke (manual steps, file probes, codes) and
//! which codemod applies, so `brain upgrade`, `brain guide whats-new`/`migrate`, `brain doctor` and MCP read one source.
//! Adding an entry is one new file; a test fails when a CHANGELOG bullet has no entry (see CONTRIBUTING "Release").
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/releases.rs"));

/// Codemods `brain upgrade --apply` can run; a manifest naming another id fails the schema test.
pub const CODEMODS: &[(&str, &str)] = &[
    ("package-pin", "set the @christianirack/brainllm pin in package.json to the target version (revision-checked)"),
    ("record-version", "record the target version as \"brainVersion\" in brain.json"),
    ("brain-fix", "apply every exact edit `brain fix` offers for the new diagnostics"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// something new an app can start using
    Capability,
    /// different behavior or a new diagnostic; usually nothing to do, sometimes a step
    Change,
    /// still works, will be removed: migrate before the next minor
    Deprecation,
    /// code or configuration that worked before no longer does: manual steps required
    Breaking,
    /// a defect fixed; workarounds can be removed
    Fix,
    /// framework internals with no author-visible effect
    Internal,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    /// app-relative glob (`*`, `**`, `?`, `{a,b}`)
    pub glob: String,
    /// substring a line must contain; empty = the file existing is the match
    #[serde(default)]
    pub contains: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Adopt {
    /// file the snippet goes in (app-relative)
    pub file: String,
    /// command that starts the adoption, when one exists (`brain add …`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// the smallest correct code that starts using the capability
    pub snippet: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    /// why it matters to an app author
    pub why: String,
    /// distinctive phrases of the CHANGELOG bullets this entry covers
    pub changelog: Vec<String>,
    /// `brain guide <topic>` or a repository doc path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apis: Vec<String>,
    /// app files this change can affect (globs)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopt: Option<Adopt>,
    /// any match = the app already uses the capability
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<Probe>,
    /// each match is a place to review by hand (breaking changes and deprecations)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finds: Vec<Probe>,
    /// manual steps, in order
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codemod: Option<String>,
}

/// One version's entries.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub entries: Vec<Entry>,
}

/// Every embedded release, oldest first; a malformed file is an error naming it.
pub fn all() -> Result<Vec<Release>, String> {
    let mut out: Vec<Release> = vec![];
    for (version, file, body) in RELEASE_FILES {
        crate::upgrade::parse_version(version).map_err(|e| format!("releases/{version}: {e}"))?;
        let entry: Entry = serde_json::from_str(body).map_err(|e| format!("releases/{version}/{file}: {e}"))?;
        if format!("{}.json", entry.id) != *file {
            return Err(format!("releases/{version}/{file}: the file name must be `<id>.json` (id `{}`)", entry.id));
        }
        match out.iter_mut().find(|r| r.version == *version) {
            Some(r) => r.entries.push(entry),
            None => out.push(Release { version: version.to_string(), entries: vec![entry] }),
        }
    }
    out.sort_by(|a, b| cmp(&a.version, &b.version));
    Ok(out)
}

pub fn cmp(a: &str, b: &str) -> Ordering {
    match (crate::upgrade::parse_version(a), crate::upgrade::parse_version(b)) {
        (Ok(x), Ok(y)) => crate::upgrade::compare_versions(&x, &y),
        _ => a.cmp(b),
    }
}

/// Releases in `(from, to]`.
pub fn between(from: &str, to: &str) -> Result<Vec<Release>, String> {
    Ok(all()?.into_iter().filter(|r| cmp(&r.version, from) == Ordering::Greater && cmp(&r.version, to) != Ordering::Greater).collect())
}

/// The newest version with a manifest.
#[allow(dead_code)]
pub fn latest() -> Option<String> {
    all().ok().and_then(|r| r.last().map(|r| r.version.clone()))
}

// ---- app file probes ---------------------------------------------------------------------------------------------

const SKIP_DIRS: &[&str] = &["node_modules", "dist", ".brain", ".git", "target", ".cache"];
const MAX_FILES: usize = 5000;
const MAX_FILE: u64 = 1024 * 1024;

/// App-relative paths (with `/`) of the source files a probe may read, sorted.
#[allow(dead_code)]
pub fn app_files(root: &Path) -> Vec<String> {
    let mut out = vec![];
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(dir) = fs::read_dir(root.join(&rel)) else { continue };
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(meta) = fs::symlink_metadata(e.path()) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            let child = rel.join(&name);
            if meta.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(child);
                }
            } else if meta.is_file() && meta.len() <= MAX_FILE && out.len() < MAX_FILES {
                out.push(child.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

/// `(file, line)` of every match; line 0 = the file itself matched (empty `contains`).
#[allow(dead_code)]
pub fn probe(root: &Path, files: &[String], p: &Probe) -> Vec<(String, usize)> {
    let mut hits = vec![];
    for f in files.iter().filter(|f| glob(&p.glob, f)) {
        if p.contains.is_empty() {
            hits.push((f.clone(), 0));
            continue;
        }
        let Ok(src) = fs::read_to_string(root.join(f)) else { continue };
        for (i, line) in src.lines().enumerate() {
            if line.contains(&p.contains) {
                hits.push((f.clone(), i + 1));
            }
        }
    }
    hits
}

/// Glob over `/`-separated paths: `**` spans directories, `*` and `?` stay inside one, `{a,b}` alternates.
pub fn glob(pattern: &str, path: &str) -> bool {
    if let Some(open) = pattern.find('{')
        && let Some(close) = pattern[open..].find('}').map(|c| open + c)
    {
        return pattern[open + 1..close].split(',').any(|alt| glob(&format!("{}{alt}{}", &pattern[..open], &pattern[close + 1..]), path));
    }
    fn segs(p: &[&str], s: &[&str]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some(&"**") => (0..=s.len()).any(|i| segs(&p[1..], &s[i..])),
            Some(seg) => !s.is_empty() && one(seg.as_bytes(), s[0].as_bytes()) && segs(&p[1..], &s[1..]),
        }
    }
    fn one(p: &[u8], s: &[u8]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some(b'*') => (0..=s.len()).any(|i| one(&p[1..], &s[i..])),
            Some(b'?') => !s.is_empty() && one(&p[1..], &s[1..]),
            Some(c) => s.first() == Some(c) && one(&p[1..], &s[1..]),
        }
    }
    let p: Vec<&str> = pattern.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    segs(&p, &s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn glob_matches_directories_alternatives_and_single_segments() {
        assert!(glob("**/*.brain", "views/a/b.brain") && glob("**/*.brain", "app.brain"));
        assert!(glob("views/_layout.brain", "views/_layout.brain") && !glob("views/*.brain", "views/a/b.brain"));
        assert!(glob("server/**/*.{jsbrain,brain}", "server/x/y.jsbrain") && !glob("server/**/*.{jsbrain,brain}", "lib/y.jsbrain"));
        assert!(glob("?.json", "a.json") && !glob("?.json", "ab.json"));
    }

    /// The schema every entry must follow: what makes it useful to an LLM author, checked at test time.
    #[test]
    fn every_manifest_entry_follows_the_schema() {
        let releases = all().unwrap_or_else(|e| panic!("{e}"));
        assert!(releases.iter().any(|r| r.version == "0.5.0") && releases.iter().any(|r| r.version == "0.6.0"));
        let mut ids = HashSet::new();
        let guides: Vec<&str> = crate::guide::GUIDES.iter().map(|g| g.0).collect();
        for r in &releases {
            for e in &r.entries {
                let at = format!("releases/{}/{}.json", r.version, e.id);
                assert!(ids.insert(e.id.clone()), "{at}: duplicate id");
                assert!(!e.id.is_empty() && e.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'), "{at}: id is kebab-case");
                assert!(!e.title.trim().is_empty() && !e.title.contains('\n') && e.title.len() <= 120, "{at}: one-line title ≤ 120");
                assert!(e.why.trim().len() >= 20, "{at}: `why` explains what it changes for an app");
                assert!(!e.changelog.is_empty() && e.changelog.iter().all(|n| n.trim().len() >= 8), "{at}: changelog needles of ≥ 8 chars");
                for c in &e.codes {
                    assert!(crate::explain_json(c).is_ok(), "{at}: code {c} is not in the catalog");
                }
                if let Some(id) = &e.codemod {
                    assert!(CODEMODS.iter().any(|c| c.0 == id), "{at}: unknown codemod `{id}`");
                }
                if let Some(g) = &e.guide {
                    match g.strip_prefix("brain guide ") {
                        Some(t) => assert!(guides.contains(&t), "{at}: no guide `{t}`"),
                        None => assert!(repo().join(g).is_file(), "{at}: guide path {g} does not exist"),
                    }
                }
                for p in e.uses.iter().chain(&e.finds) {
                    assert!(!p.glob.is_empty() && !p.glob.starts_with('/') && !p.glob.contains(".."), "{at}: app-relative glob");
                }
                match e.kind {
                    Kind::Capability => {
                        let a = e.adopt.as_ref().unwrap_or_else(|| panic!("{at}: a capability needs `adopt`"));
                        assert!(!a.file.is_empty() && !a.snippet.trim().is_empty(), "{at}: adopt needs file and snippet");
                        assert!(!e.uses.is_empty(), "{at}: a capability needs `uses` probes to tell if the app adopted it");
                        assert!(e.guide.is_some(), "{at}: a capability names the guide or doc to read");
                    }
                    Kind::Breaking | Kind::Deprecation => {
                        assert!(!e.steps.is_empty(), "{at}: manual steps required");
                        assert!(!e.finds.is_empty() || !e.codes.is_empty(), "{at}: needs `finds` probes or `codes` to locate file:line");
                    }
                    _ => {}
                }
            }
        }
    }

    /// `(version or "Unreleased", bullet text)` for every bullet an entry must cover.
    fn changelog_bullets() -> Vec<(String, String)> {
        let src = fs::read_to_string(repo().join("CHANGELOG.md")).unwrap();
        let (mut version, mut exempt) = (String::new(), true);
        let mut out: Vec<(String, String)> = vec![];
        for line in src.lines() {
            if let Some(h) = line.strip_prefix("## [") {
                version = h.split(']').next().unwrap_or("").to_string();
                exempt = true;
            } else if let Some(h) = line.strip_prefix("### ") {
                exempt = matches!(h.trim(), "Tests" | "Internal" | "Upgrading");
            } else if exempt || version.is_empty() {
                continue;
            } else if let Some(b) = line.strip_prefix("- ") {
                out.push((version.clone(), b.trim().to_string()));
            } else if line.starts_with("  ") && !line.trim().is_empty() {
                if let Some(last) = out.last_mut() {
                    last.1.push(' ');
                    last.1.push_str(line.trim());
                }
            }
        }
        out
    }

    /// Release recipe guard: every CHANGELOG bullet of a version with manifests (and of [Unreleased], which belongs to
    /// the newest manifest version) is covered by an entry, and every needle still matches a bullet.
    #[test]
    fn every_changelog_bullet_has_a_manifest_entry() {
        let releases = all().unwrap();
        let newest = releases.last().unwrap().version.clone();
        let mut missing = vec![];
        let mut used: HashSet<(String, String)> = HashSet::new();
        for (section, bullet) in changelog_bullets() {
            let version = if section == "Unreleased" { newest.clone() } else { section.clone() };
            let Some(r) = releases.iter().find(|r| r.version == version) else { continue };
            let hit = r.entries.iter().flat_map(|e| e.changelog.iter().map(move |n| (e, n))).find(|(_, n)| bullet.contains(n.as_str()));
            match hit {
                Some((e, n)) => {
                    used.insert((e.id.clone(), n.clone()));
                }
                None => missing.push(format!("[{section}] {}", &bullet[..bullet.len().min(90)])),
            }
        }
        assert!(
            missing.is_empty(),
            "CHANGELOG bullets without a manifest entry; add `crates/brain-cli/releases/{newest}/<id>.json` (or a needle to an existing entry), see CONTRIBUTING \"Release\":\n{}",
            missing.join("\n")
        );
        let bullets = changelog_bullets();
        for r in &releases {
            for e in &r.entries {
                for n in &e.changelog {
                    let found = bullets.iter().any(|(s, b)| (s == &r.version || (s == "Unreleased" && r.version == newest)) && b.contains(n.as_str()));
                    assert!(found, "releases/{}/{}.json: needle `{n}` matches no CHANGELOG bullet of that version", r.version, e.id);
                }
            }
        }
    }

    #[test]
    fn between_is_exclusive_of_the_start_and_inclusive_of_the_end() {
        let v: Vec<String> = between("0.4.0", "0.6.0").unwrap().into_iter().map(|r| r.version).collect();
        assert_eq!(v, ["0.5.0", "0.6.0"]);
        assert!(between("0.6.0", "0.6.0").unwrap().is_empty());
        assert_eq!(between("0.5.0", "0.6.0-rc.1").unwrap().len(), 0);
    }
}
