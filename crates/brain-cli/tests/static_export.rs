//! Static export must be portable and must never silently remove request-time security or data.
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, process::{Command, Output, Stdio}};
struct App(PathBuf);
impl Drop for App { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(root).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}
fn ok(root: &Path, args: &[&str]) -> serde_json::Value {
    let o = run(root, args);
    assert!(o.status.success(), "{:?}: {} {}", args, String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(at).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() { walk(root, &p, out) } else { out.insert(p.strip_prefix(root).unwrap().to_string_lossy().into_owned(), fs::read(p).unwrap()); }
        }
    }
    let mut out = BTreeMap::new(); walk(root, root, &mut out); out
}
#[test]
fn static_starter_exports_complete_pages_and_fails_closed_before_changing_dist() {
    let tmp = App(std::env::temp_dir().join(format!("brain-static-{}", std::process::id())));
    fs::create_dir_all(&tmp.0).unwrap();
    let incompatible = run(&tmp.0, &["new", "bad", "--static", "--backend", "--json"]);
    assert!(!incompatible.status.success()); assert!(!tmp.0.join("bad").exists());
    ok(&tmp.0, &["new", "site", "--static", "--json"]);
    let app = tmp.0.join("site");
    assert!(!app.join("server").exists());
    assert!(!app.join("views/account/_guard.jsbrain").exists());
    assert!(!fs::read_to_string(app.join(".brain_env")).unwrap().contains("BRAIN_SECRET"));
    // Even an old SSR deployment must become a clean portable directory, without the old-chunk grace.
    fs::create_dir_all(app.join("dist/_server")).unwrap();
    fs::create_dir_all(app.join("dist/_b")).unwrap();
    for path in ["server.mjs", "_server/manifest.json", "_b/old.js"] { fs::write(app.join("dist").join(path), "stale").unwrap(); }
    #[cfg(unix)]
    {
        let outside = tmp.0.join("outside"); fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "owned by another directory").unwrap();
        std::os::unix::fs::symlink(&outside, app.join("dist/stale-link")).unwrap();
    }
    let report = ok(&app, &["build", "--site", "https://example.com", "--json"]);
    assert_eq!(report["outputMode"], "static");
    #[cfg(unix)]
    {
        assert_eq!(fs::read_to_string(tmp.0.join("outside/keep.txt")).unwrap(), "owned by another directory");
        assert!(!app.join("dist/stale-link").exists());
    }
    let output = files(&app.join("dist"));
    for path in ["index.html", "about/index.html", "404.html", "sitemap.xml", "robots.txt", "llms.txt"] { assert!(output.contains_key(path), "missing {path}"); }
    for path in output.keys() { assert!(path != "server.mjs" && !path.starts_with("_server/") && !path.ends_with(".brain") && !path.ends_with(".jsbrain"), "{path}"); }
    assert!(!output.contains_key("_b/old.js"));
    let home = String::from_utf8_lossy(&output["index.html"]);
    assert!(home.contains("Your static website") && home.contains("canonical") && home.contains("application/ld+json"));
    assert!(!home.contains("<script type=\"module\""), "pure HTML needs no client runtime");
    // A nested static page may have browser interaction, without a backend.
    fs::create_dir_all(app.join("views/docs")).unwrap();
    fs::write(app.join("views/docs/index.brain"), "<meta seo title=\"Documentation\" description=\"Documentation with a browser counter that works entirely in the browser without a server-side runtime or native backend.\">\n<script>let count = sig(0)</script>\n<main><h1>Documentation</h1><button on:click={() => count++}>Count {count}</button></main>").unwrap();
    ok(&app, &["build", "--json"]);
    assert!(app.join("dist/docs/index.html").is_file());
    let clean = files(&app.join("dist"));
    let home_source = fs::read_to_string(app.join("views/home.brain")).unwrap();
    for (path, source, message) in [
        ("views/_guard.jsbrain", "<meta server>\nexport default function guard(ctx) { return true }", "guards and middleware"),
        ("views/_middleware.jsbrain", "<meta server>\nexport default function middleware(ctx) { return true }", "guards and middleware"),
        ("views/home.brain", "<meta seo title=\"Request page\" description=\"A request-time page used to verify static export refuses server dependencies before changing any published files.\">\n<script>export async function load() { return { value: 1 } }</script>\n<main><h1>Request page</h1><p>{props.value}</p></main>", "request-time load()"),
        ("views/home.brain", "<meta seo title=\"Server function\" description=\"A server function cannot be deployed to static file hosting because its RPC endpoint needs a running server.\">\n<script>server: async function save(ctx) { return true }</script>\n<main><h1>Server function</h1><button on:click={() => save()}>Save</button></main>", "server: functions"),
        ("server/routes/endpoint.jsbrain", "<meta server>\nexport default { path: '/endpoint', get: { out: 'str' } }", "backend contracts"),
        ("views/[slug].brain", "<meta seo title=\"Dynamic page\" description=\"A dynamic page used to verify that static export requires a complete list of build-time route parameters.\">\n<main><h1>Dynamic page</h1></main>", "needs paths()"),
    ] {
        let p = app.join(path); fs::create_dir_all(p.parent().unwrap()).unwrap(); fs::write(&p, source).unwrap();
        let o = run(&app, &["build", "--json"]);
        let error = format!("{} {}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        assert!(!o.status.success() && error.contains(message), "{path}: {error}");
        assert_eq!(files(&app.join("dist")), clean, "failed export must preserve the previous artifact");
        if path == "views/home.brain" { fs::write(p, &home_source).unwrap(); } else { fs::remove_file(p).unwrap(); }
    }
    // Different route spellings must not overwrite one HTML destination.
    let config = fs::read_to_string(app.join("brain.json")).unwrap();
    let mut localized: serde_json::Value = serde_json::from_str(&config).unwrap();
    fs::create_dir_all(app.join("i18n")).unwrap();
    for locale in ["en", "es"] { fs::write(app.join(format!("i18n/{locale}.json")), "{}").unwrap(); }
    localized["i18n"] = serde_json::json!({"default":"en","locales":["en","es"]});
    fs::write(app.join("brain.json"), serde_json::to_vec(&localized).unwrap()).unwrap();
    let about = fs::read_to_string(app.join("views/about.brain")).unwrap();
    fs::write(app.join("views/about.brain"), format!("<meta route es=\"/about/\">\n{about}")).unwrap();
    let o = run(&app, &["build", "--json"]);
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stdout).contains("duplicate static destination"), "{}", String::from_utf8_lossy(&o.stdout));
    assert_eq!(files(&app.join("dist")), clean);
    fs::write(app.join("views/about.brain"), about).unwrap();
    fs::write(app.join("brain.json"), config).unwrap();
    // Rendering failures cannot fall back to client-only HTML under the portable contract.
    let broken = home_source.replace("<main", "<script>const broken = JSON.parse('invalid-json')</script>\n<main");
    fs::write(app.join("views/home.brain"), broken).unwrap();
    let o = run(&app, &["build", "--json"]);
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stdout).contains("prerender"), "{}", String::from_utf8_lossy(&o.stdout));
    assert_eq!(files(&app.join("dist")), clean);
    fs::write(app.join("views/home.brain"), &home_source).unwrap();
    let o = run(&app, &["build", "--ext", "brain", "--json"]); assert!(!o.status.success());
    // Explicit paths + build-time load are supported, not mistaken for SSR.
    fs::write(app.join("views/[slug].brain"), "<meta seo title={props.title} description=\"A fully generated page from a build-time route list and local data without any request-time server dependency.\">\n<script>export async function paths() { return [{ slug: 'first' }] }\nexport async function load(ctx) { return { title: ctx.params.slug } }</script>\n<main><h1>{props.title}</h1></main>").unwrap();
    ok(&app, &["build", "--static", "--json"]);
    let page = fs::read_to_string(app.join("dist/first/index.html")).unwrap(); assert!(page.contains("first"));
    let valid_dynamic = fs::read_to_string(app.join("views/[slug].brain")).unwrap();
    fs::write(app.join("views/[slug].brain"), valid_dynamic.replace("[{ slug: 'first' }]", "[{ slug: 'a' }, { slug: 'A' }]")).unwrap();
    let before_collision = files(&app.join("dist"));
    let o = run(&app, &["build", "--json"]);
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stdout).contains("duplicate static destination"), "{}", String::from_utf8_lossy(&o.stdout));
    assert_eq!(files(&app.join("dist")), before_collision);
    fs::write(app.join("views/[slug].brain"), "<meta seo title=\"Invalid paths\" description=\"A route list that does not supply the required parameter must fail before static files can be published.\">\n<script>export async function paths() { return [{ wrong: 'first' }] }</script>\n<main><h1>Invalid paths</h1></main>").unwrap();
    let o = run(&app, &["build", "--json"]);
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stdout).contains("does not fill every param"), "{}", String::from_utf8_lossy(&o.stdout));
    fs::remove_file(app.join("views/[slug].brain")).unwrap();
    fs::create_dir_all(app.join("views/nested")).unwrap();
    fs::write(app.join("views/nested/[...rest].brain"), "<meta seo title=\"Unsafe route\" description=\"Unsafe route parameters must never create an output file outside the static publication directory.\">\n<script>export async function paths() { return [{ rest: '../../escaped' }] }</script>\n<main><h1>Unsafe route</h1></main>").unwrap();
    let before = files(&app.join("dist"));
    let o = run(&app, &["build", "--json"]);
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stdout).contains("safely"), "{}", String::from_utf8_lossy(&o.stdout));
    assert_eq!(files(&app.join("dist")), before);
    assert!(!app.join("escaped/index.html").exists());

}

#[test]
fn static_external_forms_keep_native_markup_and_share_csp_validation_with_auto_output() {
    let tmp = App(std::env::temp_dir().join(format!("brain-static-native-form-{}", std::process::id())));
    fs::create_dir_all(&tmp.0).unwrap();
    ok(&tmp.0, &["new", "site", "--static", "--json"]);
    let app = tmp.0.join("site");
    let mut cfg: serde_json::Value = serde_json::from_str(&fs::read_to_string(app.join("brain.json")).unwrap()).unwrap();
    cfg["csp"] = serde_json::json!({"connectSrc":[],"formAction":["https://forms.example.com"]});
    cfg["seo"]["siteName"] = serde_json::json!("Public display name");
    cfg["seo"]["markdown"] = serde_json::json!(false);
    fs::write(app.join("brain.json"), serde_json::to_vec(&cfg).unwrap()).unwrap();
    fs::write(app.join("views/home.brain"), r#"<meta seo title="Contact" description="Contact the team using a native external form with browser validation and no requirement for a JavaScript or application server.">
<main><h1>Contact</h1><form action="https://forms.example.com/contact" method="post"><label>Email <input name="email" type="email" required></label><button type="submit">Send</button></form></main>"#).unwrap();
    ok(&app, &["build", "--site", "https://example.com", "--json"]);
    let html = fs::read_to_string(app.join("dist/index.html")).unwrap();
    assert!(html.contains("action=\"https://forms.example.com/contact\"") && html.contains("name=\"email\""), "{html}");
    assert!(!html.contains("type=\"module\"") && !html.contains("text/markdown"));
    let llms = fs::read_to_string(app.join("dist/llms.txt")).unwrap();
    assert!(llms.starts_with("# Public display name\n"));
    assert!(!llms.contains(".md)"));
    assert!(!app.join("dist/index.md").exists());
    for file in ["_headers", "_brain/nginx.conf", "_brain/Caddyfile"] {
        let policy = fs::read_to_string(app.join("dist").join(file)).unwrap();
        assert!(policy.contains("form-action 'self' https://forms.example.com"), "{file}: {policy}");
    }
    for mode in ["static", "auto"] {
        cfg["output"] = serde_json::json!(mode);
        cfg["csp"]["formAction"] = serde_json::json!(["https://forms.example.com; script-src *"]);
        fs::write(app.join("brain.json"), serde_json::to_vec(&cfg).unwrap()).unwrap();
        let out = run(&app, &["check", "--json"]);
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(!out.status.success() && text.contains("B0796") && text.contains("formAction"), "{mode}: {text}");
    }
}

#[cfg(unix)]
#[test]
fn public_input_aliases_fail_before_discovery_or_output_changes_in_static_and_ordinary_modes() {
    let tmp = App(std::env::temp_dir().join(format!("brain-public-alias-input-{}", std::process::id())));
    fs::create_dir_all(&tmp.0).unwrap();
    ok(&tmp.0, &["new", "site", "--static", "--json"]);
    let app = tmp.0.join("site");
    ok(&app, &["build", "--site", "https://example.com", "--json"]);
    let previous = files(&app.join("dist"));
    let outside = tmp.0.join("outside.txt"); fs::write(&outside, "synthetic outside marker").unwrap();
    let mut cfg: serde_json::Value = serde_json::from_str(&fs::read_to_string(app.join("brain.json")).unwrap()).unwrap();
    for mode in ["static", "auto"] {
        cfg["output"] = serde_json::json!(mode);
        fs::write(app.join("brain.json"), serde_json::to_vec(&cfg).unwrap()).unwrap();
        for (source, name) in [(&outside, "alias.bin"), (&app.join("public"), "cycle")] {
            let link = app.join("public").join(name);
            std::os::unix::fs::symlink(source, &link).unwrap();
            for command in ["check", "build", "doctor"] {
                let result = run(&app, &[command, "--json"]);
                let text = format!("{}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
                assert!(!result.status.success() && text.contains("symlink"), "{mode}/{command}: {text}");
                assert!(!text.contains("synthetic outside marker"));
                assert_eq!(files(&app.join("dist")), previous);
            }
            fs::remove_file(link).unwrap();
        }
    }
}
