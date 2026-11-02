//! `brain check` and `brain doctor` on the declared origins: B0744 (shape), B0745 (socket origin outside the HTTP
//! origins) and B0746 (a page calling an undeclared origin). Each one fails or misleads only at runtime otherwise.
use brain_backend::origins::{self, BRAIN_JSON_KEY, PLATFORM_KEY, SOCKET_KEY};
use brain_parse::{Diag, Severity, Span};
use serde_json::Value;
use std::path::Path;

const BRAIN_JSON: &str = "brain.json";
const PLATFORM: &str = "server/platform.jsbrain";

/// Where a literal is written, so the diagnostic lands on it; line 1 when the source spells it differently.
fn span_of(source: &str, literal: &str) -> Span {
    for (index, line) in source.lines().enumerate() {
        if let Some(col) = line.find(literal) {
            return Span { line: index as u32 + 1, col: col as u32 + 1, ..Default::default() };
        }
    }
    Span { line: 1, col: 1, ..Default::default() }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value.and_then(Value::as_array).map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn declaration(diags: &mut Vec<Diag>, file: &str, source: &str, raw: &str, key: &str) {
    let Some(problem) = origins::problem(raw) else { return };
    let message = origins::declaration_message(raw, key);
    let (msg, fix) = message.split_once("; fix: ").unwrap_or((message.as_str(), "list each exact origin"));
    let mut diag = Diag::new("B0744", file, span_of(source, raw), msg.trim_start_matches("B0744: "), fix);
    // brain.json entries stop the backend from starting; a platform entry that only never matches keeps working as before.
    diag.level = if problem.fatal || file == BRAIN_JSON { Severity::Error } else { Severity::Warning };
    diags.push(diag);
}

pub fn check(app: &Path) -> Vec<Diag> {
    let mut diags = Vec::new();
    let source = std::fs::read_to_string(app.join(BRAIN_JSON)).unwrap_or_default();
    // Invalid JSON and invalid platform contracts have their own diagnostics; nothing can be said about origins then.
    let json: Value = serde_json::from_str(&source).unwrap_or(Value::Null);
    let backend = json.get("backend");
    let http = strings(backend.and_then(|b| b.get("origins")));
    let sockets = strings(backend.and_then(|b| b.get("sockets")).and_then(|s| s.get("allowedOrigins")));
    for raw in &http {
        declaration(&mut diags, BRAIN_JSON, &source, raw, BRAIN_JSON_KEY);
    }
    for raw in &sockets {
        declaration(&mut diags, BRAIN_JSON, &source, raw, SOCKET_KEY);
    }
    let path = app.join(PLATFORM);
    let platform_source = std::fs::read_to_string(&path).unwrap_or_default();
    let cors = if path.is_file() {
        crate::backend::read_contract(app, &path).map(|c| strings(c.get("cors").and_then(|c| c.get("origins")))).unwrap_or_default()
    } else {
        Vec::new()
    };
    for raw in &cors {
        declaration(&mut diags, PLATFORM, &platform_source, raw, PLATFORM_KEY);
    }
    let sites: Vec<String> = ["production", "development"]
        .iter()
        .filter_map(|env| crate::env::value(app, env, "PUBLIC_SITE_URL"))
        .filter_map(|site| brain_backend::links::origin(&site))
        .collect();
    for raw in sockets.iter().filter(|raw| origins::problem(raw).is_none()) {
        let (known, level, against) = if http.is_empty() {
            (sites.contains(raw) || cors.contains(raw), Severity::Warning, "PUBLIC_SITE_URL")
        } else {
            (http.contains(raw), Severity::Error, BRAIN_JSON_KEY)
        };
        if !known {
            let mut diag = Diag::new(
                "B0745",
                BRAIN_JSON,
                span_of(&source, raw),
                format!("{SOCKET_KEY} has {raw:?}, which is outside the HTTP origins from {against}, so the page could open a socket it cannot call over HTTP"),
                format!("add {raw:?} to {BRAIN_JSON_KEY}, or remove it from the socket list; never a wildcard. Run brain explain B0745"),
            );
            diag.level = level;
            diags.push(diag);
        }
    }
    let declared: Vec<String> = http.iter().chain(&cors).chain(&sites).filter_map(|o| brain_check::origins::absolute_origin(o)).collect();
    let (connect, mut problems) = brain_pack::read_connect_src(app);
    let (frames, frame_problems) = brain_pack::read_frame_src(app);
    problems.extend(frame_problems);
    let (scripts, script_problems) = brain_pack::read_script_src(app);
    problems.extend(script_problems);
    let at = span_of(&source, "\"csp\"");
    for problem in problems {
        let (msg, fix) = problem.split_once("; fix: ").unwrap_or((problem.as_str(), "list exact origins"));
        diags.push(Diag::new("B0796", BRAIN_JSON, at, msg, fix));
    }
    // A brain.json that cannot be read has its own diagnostic and says nothing about the policy.
    if connect.is_none() && json.is_object() {
        let mut diag = Diag::new(
            "B0795",
            BRAIN_JSON,
            Span { line: 1, col: 1, ..Default::default() },
            "pages may call any HTTPS origin, because brain.json declares no `csp.connectSrc`; a script injected into a page could send data anywhere",
            "list the origins pages call: \"csp\": { \"connectSrc\": [\"https://api.example.com\"] }, or [] for the site's own origin only. Run brain explain B0795",
        );
        diag.level = Severity::Warning;
        diags.push(diag);
    }
    diags.extend(brain_check::origins::check(app, &declared, connect.as_deref()));
    diags.extend(brain_check::origins::check_embeds(app, &frames, &scripts));
    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn app(files: &[(&str, &str)]) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("brain-origin-check-{}-{n}", std::process::id()));
        for (rel, src) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, src).unwrap();
        }
        root
    }
    fn run(files: &[(&str, &str)]) -> Vec<Diag> {
        let root = app(files);
        let mut found = check(&root);
        fs::remove_dir_all(&root).ok();
        found.retain(|d| d.code != "B0795"); // its own test below; these cases are about origins
        found
    }

    #[test]
    fn only_an_app_that_declares_no_list_gets_b0795() {
        let root = app(&[("brain.json", "{}")]);
        assert!(check(&root).iter().any(|d| d.code == "B0795" && d.level == Severity::Warning));
        fs::write(root.join("brain.json"), r#"{"csp":{"connectSrc":[]}}"#).unwrap();
        assert!(check(&root).iter().all(|d| d.code != "B0795"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bad_shapes_name_the_value_the_key_and_the_replacement() {
        let found = run(&[("brain.json", "{\n  \"backend\": {\n    \"origins\": [\"https://shop.example.test/\", \"shop.example.test\", \"https://*.example.test\", \"https://shop.example.test/app\"]\n  }\n}\n")]);
        assert_eq!(found.iter().filter(|d| d.code == "B0744").count(), 4, "{found:?}");
        let slash = &found[0];
        assert_eq!((slash.file.as_str(), slash.line, slash.level), ("brain.json", 3, Severity::Error));
        assert!(slash.msg.contains("https://shop.example.test/") && slash.msg.contains("backend.origins"), "{slash:?}");
        assert!(slash.fix.contains("\"https://shop.example.test\"") && slash.fix.contains("brain explain B0744"), "{slash:?}");
        assert!(found[2].msg.contains("wildcard") && !found[2].fix.contains('*'), "{:?}", found[2]);
    }

    #[test]
    fn a_trailing_slash_in_platform_warns_because_it_only_never_matches() {
        let found = run(&[("server/platform.jsbrain", "<meta server>\nexport default { cors: { origins: ['https://shop.example.test/'] } }\n")]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].code, found[0].level, found[0].line), ("B0744", Severity::Warning, 2));
    }

    #[test]
    fn socket_origin_outside_http_origins_is_an_error_with_both_values() {
        let found = run(&[("brain.json", r#"{"backend":{"origins":["https://shop.example.test"],"sockets":{"allowedOrigins":["https://blog.example.test"]}}}"#)]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].code, found[0].level), ("B0745", Severity::Error));
        assert!(found[0].msg.contains("https://blog.example.test") && found[0].fix.contains("backend.origins"), "{:?}", found[0]);
    }

    #[test]
    fn valid_declarations_and_declared_calls_are_clean() {
        let found = run(&[
            ("brain.json", r#"{"backend":{"origins":["https://shop.example.test","https://api.example.test"],"sockets":{"allowedOrigins":["https://shop.example.test"]}}}"#),
            ("views/a.brain", "<script>\nasync function a() { return fetch('https://api.example.test/items') }\n</script>\n<p>A</p>\n"),
        ]);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn an_app_without_declarations_is_clean() {
        assert!(run(&[("brain.json", "{}"), ("views/a.brain", "<p>A</p>\n")]).is_empty());
    }
}
