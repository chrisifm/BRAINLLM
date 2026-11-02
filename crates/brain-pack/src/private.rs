//! Files in dist/ that no host may serve (INTERNALS §31): server code (`server.mjs`, `_server/`), the build's host
//! configs (`_brain/`, `_headers`, `_redirects`) and dot files other than `.well-known`.
//!
//! This is the one list. The Rust static handler (`brain start`), the generated host configs (`_brain/nginx.conf`,
//! `_brain/Caddyfile`, `brain deploy config nginx`) and the build's own filters read it; the Node server
//! (`runtime/server/server.mjs`, `publicStaticPath`) keeps the same list, and a test compares them.

/// A request path (relative to the base, without the leading `/`) that starts with one of these is private.
/// Raw prefixes, not path segments: `server.mjs` also covers `server.mjs.br`, `server.mjs.gz` and `server.mjs.map`.
pub const PREFIXES: &[&str] = &["_server", "_brain", "server.mjs"];
/// Private only as the exact path.
pub const FILES: &[&str] = &["_headers", "_redirects"];
/// The one dot directory a host serves; every other path segment that starts with `.` is private.
pub const PUBLIC_DOT_DIR: &str = ".well-known";
/// Prefixes that also name live routes of the app server (`/_brain/auth/…`, `/_brain/socket`): a reverse proxy in front
/// of `brain start` forwards them, and the server refuses the files under them itself. File-serving hosts block them.
pub const ROUTED: &[&str] = &["_brain"];

/// True when `rel` (a dist/-relative path, or a request path without its base and leading `/`) must never be served.
/// ASCII case-insensitive: on a case-insensitive file system `/_SERVER/x.mjs` opens `_server/x.mjs`.
pub fn is_private(rel: &str) -> bool {
    let rel = rel.trim_start_matches('/');
    let lower = rel.to_ascii_lowercase();
    PREFIXES.iter().any(|p| lower.starts_with(p))
        || FILES.contains(&lower.as_str())
        || rel.split('/').any(|part| part.starts_with('.') && !part.eq_ignore_ascii_case(PUBLIC_DOT_DIR))
}

fn alternatives(routed: bool) -> String {
    let prefixes: Vec<String> = PREFIXES.iter().filter(|p| !routed || !ROUTED.contains(p)).map(|p| p.replace('.', "\\.")).collect();
    let files: Vec<String> = FILES.iter().map(|f| format!("{}$", f.replace('.', "\\."))).collect();
    prefixes.into_iter().chain(files).collect::<Vec<_>>().join("|")
}

/// nginx `location` blocks that answer 404 for every private path under the base `pre` (`/app` or empty).
/// `routed`: the server block proxies to `brain start`, so prefixes in [`ROUTED`] stay reachable. `indent` prefixes
/// every line. Regex locations: put them before any other regex location of the server block.
pub fn nginx(pre: &str, routed: bool, indent: &str) -> String {
    let well = PUBLIC_DOT_DIR.trim_start_matches('.');
    format!(
        "{indent}location ~* ^{pre}/(?:{}) {{ return 404; }}\n{indent}location ~ (?:^|/)\\.(?!{well}(?:/|$)) {{ return 404; }}\n",
        alternatives(routed)
    )
}

/// Caddy matchers and responses that answer 404 for every private path under the base `pre`.
pub fn caddy(pre: &str) -> String {
    let well = PUBLIC_DOT_DIR.replace('.', "\\.");
    format!(
        "@private path_regexp (?i)^{pre}/(?:{})\nrespond @private 404\n@dotfiles {{\n    path_regexp /\\.\n    not path_regexp ^(?:/(?:[^/.][^/]*|{well})?)*$\n}}\nrespond @dotfiles 404\n",
        alternatives(false)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_paths_match_what_the_node_server_refuses() {
        for p in ["_server/rpc/a.mjs", "_server/manifest.json", "server.mjs", "server.mjs.br", "_brain/nginx.conf", "_headers", "_redirects", ".env", "a/.git/config", "_SERVER/a.mjs", "Server.mjs", ".well-known/.secret"] {
            assert!(is_private(p), "{p} is private");
        }
        for p in ["", "index.html", "about/", "_b/app.js", ".well-known/security.txt", "headers", "_headers.html", "servers/x"] {
            assert!(!is_private(p), "{p} is public");
        }
    }

    #[test]
    fn nginx_blocks_every_entry_and_a_proxy_keeps_routed_prefixes() {
        let files = nginx("", false, "");
        assert!(files.contains("location ~* ^/(?:_server|_brain|server\\.mjs|_headers$|_redirects$) { return 404; }"), "{files}");
        assert!(files.contains("location ~ (?:^|/)\\.(?!well-known(?:/|$)) { return 404; }"), "{files}");
        let proxy = nginx("/app", true, "    ");
        assert!(proxy.contains("    location ~* ^/app/(?:_server|server\\.mjs|_headers$|_redirects$) { return 404; }"), "{proxy}");
    }

    #[test]
    fn caddy_blocks_every_entry() {
        let c = caddy("/app");
        assert!(c.contains("@private path_regexp (?i)^/app/(?:_server|_brain|server\\.mjs|_headers$|_redirects$)\nrespond @private 404\n"), "{c}");
        assert!(c.contains("not path_regexp ^(?:/(?:[^/.][^/]*|\\.well-known)?)*$"), "{c}");
    }

    /// The Node server cannot import this module: its guard must name the same entries.
    #[test]
    fn node_server_guard_names_the_same_list() {
        let src = include_str!("../../../runtime/server/server.mjs");
        let line = src.lines().find(|l| l.contains("const PRIVATE_PREFIXES")).expect("server.mjs declares PRIVATE_PREFIXES");
        let quoted = |names: &[&str]| format!("[{}]", names.iter().map(|n| format!("'{n}'")).collect::<Vec<_>>().join(", "));
        assert!(line.contains(&format!("PRIVATE_PREFIXES = {}", quoted(PREFIXES))), "{line}");
        let line = src.lines().find(|l| l.contains("const PRIVATE_FILES")).expect("server.mjs declares PRIVATE_FILES");
        assert!(line.contains(&format!("PRIVATE_FILES = {}", quoted(FILES))), "{line}");
        assert!(src.contains(&format!("!== '{PUBLIC_DOT_DIR}'")));
    }
}
