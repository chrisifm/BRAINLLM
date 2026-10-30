//! `.brain_env` for the commands (INTERNALS §36): which environment a command uses, the resolved variables, the
//! `PUBLIC_*` constants for the compiler, and the doctor checks. Values never reach a diagnostic, a log or MCP.

use std::path::Path;
use std::sync::RwLock;

pub const FILE: &str = ".brain_env";
pub const EXAMPLE: &str = ".brain_env.example";
/// Keys the build reads on its own (so B0722 "nothing reads it" never applies to them).
pub const TOOLCHAIN_KEYS: &[&str] = brain_check::env::TOOLING_KEYS;

/// `PUBLIC_*` values of the environment the running command selected; the compiler turns
/// `process.env.PUBLIC_X` / `import.meta.env.PUBLIC_X` into these constants.
static PUBLIC: RwLock<Vec<(String, String)>> = RwLock::new(Vec::new());

pub fn public() -> Vec<(String, String)> {
    PUBLIC.read().map(|p| p.clone()).unwrap_or_default()
}

/// `--env <name>` or the command's default (`run` → development; `build`/`start` → production).
pub fn name(args: &[String], default: &str) -> String {
    args.iter().position(|a| a == "--env").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| default.to_string())
}

fn process(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// Selects `env` for this process: resolves the file (process > section > shared) and publishes its `PUBLIC_*`.
/// Returns every resolved variable (for the load sidecar and the server child) — never print it.
pub fn select(root: &Path, env: &str) -> Vec<(String, String)> {
    let Ok(src) = std::fs::read_to_string(root.join(FILE)) else {
        *PUBLIC.write().unwrap() = Vec::new();
        return Vec::new();
    };
    let f = brain_check::env::parse(&src);
    *PUBLIC.write().unwrap() = f.public(env, &process);
    f.resolve(env, &process).into_iter().collect()
}

/// Dev/test loader for brain-serve: selects `env` (refreshing the PUBLIC_* constants) each time it is called.
pub fn loader(root: &Path, env: &str) -> brain_serve::EnvLoader {
    let (root, env) = (root.to_path_buf(), env.to_string());
    std::sync::Arc::new(move || select(&root, &env))
}

/// The resolved value of one key (e.g. `PUBLIC_SITE_URL` for canonical URLs).
pub fn value(root: &Path, env: &str, key: &str) -> Option<String> {
    if let Some(value)=process(key).filter(|v|!v.is_empty()) {return Some(value);}
    let src = std::fs::read_to_string(root.join(FILE)).ok()?;
    brain_check::env::parse(&src).resolve(env, &process).get(key).cloned().filter(|v| !v.is_empty())
}

/// B0724/B0723 of the file itself (`brain check` of the app).
pub fn check(root: &Path) -> Vec<crate::compiler::Diag> {
    match std::fs::read_to_string(root.join(FILE)) {
        Ok(src) => brain_check::env::check_file(&src, FILE),
        Err(_) => Vec::new(),
    }
}

/// B0720 for `env`: a key with no value fails `brain build`/`brain start`.
pub fn empty(root: &Path, env: &str) -> Vec<crate::compiler::Diag> {
    match std::fs::read_to_string(root.join(FILE)) {
        Ok(src) => brain_check::env::empty_keys(&src, FILE, env, &process),
        Err(_) => Vec::new(),
    }
}

/// `brain map --json`: key names per environment (never values).
pub fn map(root: &Path) -> Option<brain_mcp::Json> {
    use brain_mcp::Json;
    let src = std::fs::read_to_string(root.join(FILE)).ok()?;
    let f = brain_check::env::parse(&src);
    let none = |_: &str| None;
    let mut envs: Vec<(String, Json)> = Vec::new();
    for e in std::iter::once("shared".to_string()).chain(f.environments()).collect::<std::collections::BTreeSet<_>>() {
        let mut keys: Vec<String> = f.resolve(&e, &none).into_keys().collect();
        keys.sort();
        envs.push((e, Json::Arr(keys.into_iter().map(Json::Str).collect())));
    }
    Some(Json::Obj(envs))
}

/// Doctor findings about the env files, as (code, level, message, fix); plus the example kept in sync (written here).
pub fn doctor(root: &Path) -> Vec<(String, &'static str, String, String)> {
    let mut out = Vec::new();
    let tracked = std::process::Command::new("git")
        .args(["ls-files", "--error-unmatch", FILE])
        .current_dir(root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if tracked {
        out.push(("B0725".into(), "error", ".brain_env is tracked by git: its values may be secrets".into(), "run `git rm --cached .brain_env` and keep `.brain_env` in .gitignore (commit .brain_env.example instead)".into()));
    }
    if root.join(".env").is_file() {
        out.push(("B0726".into(), "warning", "a .env file is ignored: variables live in .brain_env".into(), "move its lines into the shared block (or a [section]) of .brain_env and delete .env".into()));
    }
    if let Ok(src) = std::fs::read_to_string(root.join(FILE)) {
        let want = example_of(&src);
        if std::fs::read_to_string(root.join(EXAMPLE)).ok().as_deref() != Some(want.as_str()) && std::fs::write(root.join(EXAMPLE), &want).is_ok() {
            out.push(("B0727".into(), "info", ".brain_env.example was out of sync with .brain_env".into(), "updated it with the current keys (no values); commit it".into()));
        }
    }
    // keys the toolchain itself reads: PUBLIC_SITE_URL (canonical, sitemap, og:url), PUBLIC_SITE_NAME (site name
    // when brain.json has none) — never "unused"
    let webhook_keys = crate::webhook_config::references(root).unwrap_or_default();
    for d in brain_check::env::unused_keys(root).into_iter().filter(|d| !TOOLCHAIN_KEYS.iter().any(|k| d.msg.contains(&format!("`{k}`"))) && !webhook_keys.iter().any(|key|d.msg.contains(&format!("`{key}`")))) {
        out.push((d.code.to_string(), "info", d.msg.clone(), d.fix.clone()));
    }
    out
}

/// The example file for `src`: the same comments, keys and sections, with every value removed.
pub fn example_of(src: &str) -> String {
    let mut o = String::from("# Keys of .brain_env (values live only in .brain_env, which git ignores). `brain doctor` keeps this list in sync.\n");
    for line in src.lines() {
        let t = line.trim();
        if t.starts_with('#') && o.len() > 0 && line.starts_with("# Variables") {
            continue;
        }
        if t.is_empty() || t.starts_with('[') {
            o.push_str(t);
            o.push('\n');
        } else if let Some((k, _)) = t.split_once('=') {
            o.push_str(&format!("{}=\n", k.trim()));
        }
    }
    let mut o = o.replace("\n\n\n", "\n\n");
    while o.ends_with("\n\n") {
        o.pop();
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_keeps_keys_and_sections_without_values() {
        let src = "# shared\nPUBLIC_SITE_NAME=Acme\nSECRET=abc # x\n\n[production]\nDATABASE_URL=postgres://u:p@h/db\n";
        let ex = example_of(src);
        assert!(ex.contains("PUBLIC_SITE_NAME=\nSECRET=\n\n[production]\nDATABASE_URL=\n"), "{ex}");
        assert!(!ex.contains("Acme") && !ex.contains("postgres") && !ex.contains("abc"), "no values: {ex}");
        assert_eq!(name(&["build".into(), "--env".into(), "staging".into()], "production"), "staging");
        assert_eq!(name(&["build".into()], "production"), "production");
    }
}

/// Sets `key` to `value` in `[section]` of `.brain_env` (`section` empty = the shared lines before any section),
/// creating the file or the section when missing, and keeps `.brain_env.example` in sync. The value is never echoed:
/// errors name the key and the section only. A result the file checks reject (B0723 secret-shaped `PUBLIC_*`, B0724)
/// is not written. This is how an agent fills environments over MCP (`brain_env_set`) without reading any value.
pub fn set(root: &Path, key: &str, section: &str, value: &str) -> Result<(), String> {
    let key_ok = key.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !key_ok {
        return Err(format!("invalid key `{key}`; fix: UPPER_SNAKE_CASE, e.g. PUBLIC_API_URL or STRIPE_KEY"));
    }
    let sec_ok = section.is_empty() || (section.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && section.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'));
    if !sec_ok {
        return Err(format!("invalid section `{section}`; fix: development, staging, production (or another lowercase name; empty = shared)"));
    }
    if value.contains(['\n', '\r', '"']) {
        return Err(format!("the value of {key} has a newline or a double quote; fix: one line, without `\"`"));
    }
    let line = if value.contains([' ', '#', '\t', '\'']) { format!("{key}=\"{value}\"") } else { format!("{key}={value}") };
    let path = root.join(FILE);
    let src = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
    // the block of the section: [start, end) — the shared block runs from the top to the first header
    let header = |l: &str| l.trim().starts_with('[') && l.trim().ends_with(']');
    let (start, end) = if section.is_empty() {
        (0, lines.iter().position(|l| header(l)).unwrap_or(lines.len()))
    } else {
        match lines.iter().position(|l| l.trim() == format!("[{section}]")) {
            Some(h) => (h + 1, lines.iter().skip(h + 1).position(|l| header(l)).map_or(lines.len(), |i| h + 1 + i)),
            None => {
                while lines.last().is_some_and(|l| l.trim().is_empty()) {
                    lines.pop();
                }
                lines.push(String::new());
                lines.push(format!("[{section}]"));
                (lines.len(), lines.len())
            }
        }
    };
    let is_key = |l: &str| l.trim().split_once('=').is_some_and(|(k, _)| k.trim() == key);
    match (start..end).find(|&i| is_key(&lines[i])) {
        Some(i) => lines[i] = line,
        None => {
            // after the block's last non-empty line
            let mut at = end;
            while at > start && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, line);
        }
    }
    let out = lines.join("\n") + "\n";
    let bad: Vec<String> = brain_check::env::check_file(&out, FILE).iter().filter(|d| !d.is_warning()).map(|d| format!("{} {}", d.code, d.msg)).collect();
    if !bad.is_empty() {
        return Err(format!("{key} was not set in [{section}]: {}", bad.join("; ")));
    }
    std::fs::write(&path, out).map_err(|e| format!("{FILE}: {e}"))?;
    let _ = std::fs::write(root.join(EXAMPLE), example_of(&std::fs::read_to_string(&path).unwrap_or_default()) + "\n");
    Ok(())
}

#[cfg(test)]
mod set_tests {
    use super::*;

    #[test]
    fn set_writes_the_section_without_echoing_values() {
        let d = std::env::temp_dir().join(format!("brain-env-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(FILE), "PUBLIC_SITE_NAME=x\n\n[development]\nDEMO=1\n\n[production]\nPUBLIC_SITE_URL=https://a.example\n").unwrap();
        set(&d, "PUBLIC_API_URL", "production", "https://api.example").unwrap();
        set(&d, "PUBLIC_API_URL", "development", "http://localhost:9000").unwrap();
        set(&d, "DEMO", "development", "2").unwrap();
        set(&d, "STATUS_TOKEN", "staging", "tok en#1").unwrap();
        set(&d, "SHARED", "", "s").unwrap();
        let src = std::fs::read_to_string(d.join(FILE)).unwrap();
        let f = brain_check::env::parse(&src);
        let none = |_: &str| None;
        let prod = f.resolve("production", &none);
        assert_eq!(prod.get("PUBLIC_API_URL").map(String::as_str), Some("https://api.example"));
        assert_eq!(prod.get("SHARED").map(String::as_str), Some("s"));
        let dev = f.resolve("development", &none);
        assert_eq!(dev.get("DEMO").map(String::as_str), Some("2"));
        assert_eq!(dev.get("PUBLIC_API_URL").map(String::as_str), Some("http://localhost:9000"));
        assert_eq!(f.resolve("staging", &none).get("STATUS_TOKEN").map(String::as_str), Some("tok en#1"));
        assert_eq!(src.matches("DEMO=").count(), 1, "replaced, not duplicated: {src}");
        // the example lists keys only
        let ex = std::fs::read_to_string(d.join(EXAMPLE)).unwrap();
        assert!(ex.contains("PUBLIC_API_URL=\n") && !ex.contains("api.example") && !ex.contains("tok en"), "{ex}");
        // errors never carry the value
        let e = set(&d, "bad-key", "production", "secret-value").unwrap_err();
        assert!(!e.contains("secret-value"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
