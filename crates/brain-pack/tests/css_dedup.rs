//! Exact generated-CSS interning keeps declaration order, scope proximity and external class contracts.
use brain_pack::{classes, cssdedup::dedupe};

#[test]
fn adjacent_identical_bodies_share_one_rule_and_keep_specificity() {
    let (css, report) =
        dedupe("@scope ([data-b=\"one\"]) to ([data-b]){.a{color:red}#b{color:red}}");
    assert!(css.contains(".a,#b{color:red}"), "{css}");
    assert!(!css.contains(":is(.a,#b)"));
    assert_eq!(report.rules_merged, 1);
    assert_eq!(report.groups_merged, 0);
    assert!(report.output_bytes < report.input_bytes);
    let (anchored, _) = dedupe(
        "@scope ([data-b=\"one\"]) to ([data-b]){:where(:scope):is(.a),.a{color:red}:where(:scope):is(.b),.b{color:red}}",
    );
    assert_eq!(anchored.matches("color:red").count(), 1, "{anchored}");
    assert!(anchored.contains(":where(:scope):is(.a),.a,:where(:scope):is(.b),.b"));
}
#[test]
fn adjacent_equal_contexts_can_be_interned_recursively_but_distinct_contexts_cannot() {
    let (css, report) = dedupe(
        "@layer comp{@media (width>1px){@supports (display:grid){@scope (.root){.a{color:red}}}}}@layer comp{@media (width>1px){@supports (display:grid){@scope (.root){.b{color:red}}}}}",
    );
    assert_eq!(css.matches("color:red").count(), 1, "{css}");
    assert_eq!(report.groups_merged, 4);
    for source in [
        "@media (width>1px){@scope (.x){.a{color:red}}}@media (width>2px){@scope (.x){.a{color:red}}}",
        "@supports (display:grid){@scope (.x){.a{color:red}}}@supports (display:flex){@scope (.x){.a{color:red}}}",
        "@layer first{@scope (.x){.a{color:red}}}@layer second{@scope (.x){.a{color:red}}}",
        "@layer{.a{color:red}}@layer{.b{color:red}}",
        "@scope ([data-b=\"a\"]) to ([data-b]){.row{color:red}}@scope ([data-b=\"b\"]) to (.other){.row{color:red}}",
    ] {
        assert_eq!(dedupe(source).0, source, "context isolation: {source}");
    }
}
#[test]
fn nonadjacent_rules_and_overrides_are_not_equivalent_to_duplicate_deletion() {
    let source = "@scope (.a){.x{color:red}}@scope (.a){.x{color:blue}}@scope (.a){.x{color:red}}";
    let (output, report) = dedupe(source);
    assert_eq!(
        output.matches("color:red").count(),
        2,
        "final override must survive: {output}"
    );
    assert_eq!(output.matches("color:blue").count(), 1);
    assert!(output.find("color:blue").unwrap() < output.rfind("color:red").unwrap());
    assert_eq!(report.rules_merged, 0);
    let source = ".a{color:red}.other{color:blue}.b{color:red}";
    assert_eq!(dedupe(source).0, source);
    let (css, report) = dedupe("@scope (.a){.x{color:red}}@scope (.a){.x{color:red}}");
    assert_eq!(css, "@scope (.a){.x{color:red}}");
    assert_eq!(report.duplicate_rules_removed, 1);
}
#[test]
fn minimal_differences_custom_values_fallback_order_and_comments_stay_separate() {
    for source in [
        ".a{color:red}.b{color:blue}",
        ".a{color:red}.b{color:red!important}",
        ".a{display:block;display:flex}.b{display:flex;display:block}",
        ".a{--value:a  b}.b{--value:a b}",
        ".a{--value:a }.b{--value:a}",
        ".a{--value:var(--one,var(--two))}.b{--value:var(--two,var(--one))}",
        ".a{color:red}/* keep boundary */.b{color:red}",
        ".a{& .x{color:red}}.b{& .x{color:red}}",
        ".a{color:red}.b:future-pseudo{color:red}",
        "@keyframes a{from{opacity:0}to{opacity:1}}@keyframes b{from{opacity:0}to{opacity:1}}",
    ] {
        let (css, report) = dedupe(source);
        assert_eq!(css, source);
        assert_eq!(report.rules_merged, 0);
    }
    let (css, _) = dedupe(
        ".a{--value:'a  b';content:'}';color:var(--value)}.b{--value:'a  b';content:'}';color:var(--value)}",
    );
    assert!(
        css.contains(".a,.b{--value:'a  b';content:'}';color:var(--value)}"),
        "{css}"
    );
}
#[test]
fn feedback_is_generated_coordinates_and_contains_no_css_literals() {
    let (_, report) = dedupe(".a{content:'PRIVATE_VALUE'}\n.b{content:'PRIVATE_VALUE'}");
    assert_eq!(report.changes[0].line, 2);
    assert_eq!(report.changes[0].column, 1);
    let json = report.to_json();
    assert!(json.contains("\"coordinateSpace\":\"generated-css\""));
    assert!(!json.contains("PRIVATE_VALUE"));
    assert!(!json.contains(".a") && !json.contains(".b"));
    let malformed = ".a{content:'unterminated}";
    assert_eq!(dedupe(malformed).0, malformed);
    let deep = format!("{}{}", "@media all{".repeat(65), "}".repeat(65));
    assert_eq!(dedupe(&deep).1.skipped, Some("css-budget"));
}
#[test]
fn public_css_foreign_strings_and_unstyled_markup_classes_cannot_collide() {
    let styles = [".long-component-name{color:red}"];
    let map = classes::plan(&styles, &[], &[".a{color:blue}"]);
    assert_ne!(map["long-component-name"], "a");
    let raw = "element.classList.add('a');element.querySelector('.b')";
    let map = classes::plan(&styles, &[raw], &[]);
    assert!(!["a", "b"].contains(&map["long-component-name"].as_str()));
    assert!(
        classes::plan(&styles, &[r"element.classList.add('\x61')"], &[]).is_empty(),
        "opaque escaped external contracts disable unsafe renaming"
    );
    let markup = "const t=tpl(\"<div class=\\\"a long-component-name\\\"></div>\")";
    let map = classes::plan(&styles, &[markup], &[]);
    assert_ne!(map["long-component-name"], "a");
    let output = classes::rename_js(markup, &map);
    assert!(output.contains(&format!("a {}", map["long-component-name"])));
    assert!(
        classes::plan(
            &styles,
            &["const t=tpl(\"<div class=\\\"&#97;\\\"></div>\")"],
            &[]
        )
        .is_empty(),
        "entity-encoded contracts cannot be guessed"
    );
    assert!(
        classes::plan(&styles, &[], &[r".\61{color:blue}"]).is_empty(),
        "escaped public selectors cannot collide with new short names"
    );
    let both = [".component-one{color:red}.component-two{color:red}"];
    let a = classes::plan(&both, &[raw, markup], &[".a{}"]);
    let b = classes::plan(&both, &[markup, raw], &[".a{}"]);
    assert_eq!(a, b);
    assert_ne!(a["component-one"], a["component-two"]);
}

#[test]
fn custom_property_units_and_token_whitespace_survive_the_entire_minifier() {
    let css = brain_pack::minify_css(
        ".a { --gap:0px; --tokens: a  b ; --color:#FFFFFF; width:calc(var(--gap) + 1px); color:#FFFFFF; margin:0px }",
    );
    assert!(
        css.contains("--gap:0px"),
        "custom length cannot become a number inside calc: {css}"
    );
    assert!(
        css.contains("--tokens: a  b "),
        "custom token whitespace is observable: {css}"
    );
    assert!(
        css.contains("--color:#FFFFFF"),
        "custom tokens cannot be algebraically normalized: {css}"
    );
    assert!(css.contains("width:calc(var(--gap) + 1px)"));
    assert!(
        css.contains("color:#fff") && css.contains("margin:0"),
        "ordinary safe shortening remains enabled: {css}"
    );
    let css = brain_pack::minify_css(".a{--empty: ;--last:a }");
    assert!(
        css.contains("--empty: ") && css.contains("--last:a "),
        "empty and trailing custom tokens must survive: {css}"
    );
    let css = brain_pack::minify_css(".a{--note:/*keep*/ a  b;--quoted:'a  b'}");
    assert!(css.contains("--note:/*keep*/ a  b") && css.contains("--quoted:'a  b'"));
    let opaque = ".a { --theme: { color: red; nested: 0px }; color: #FFFFFF }";
    assert_eq!(
        brain_pack::minify_css(opaque),
        opaque,
        "opaque custom blocks fail closed"
    );
}

#[test]
fn authored_custom_tokens_survive_parse_scope_and_production_minification() {
    let source = ".a{--gap:0px;--tokens: a  b ;--note:/*keep*/ c  d;--last:e }";
    let sheet = brain_css::parse_str(source, "comp/card.brain").unwrap();
    let css = brain_pack::minify_css(&brain_css::scoped(&sheet, "one", "comp"));
    assert!(css.contains("--gap:0px"));
    assert!(css.contains("--tokens: a  b "), "{css}");
    assert!(css.contains("--note:/*keep*/ c  d"), "{css}");
    assert!(css.contains("--last:e "), "{css}");
}

#[test]
fn adjacent_private_component_scopes_share_exact_bodies_without_erasing_roots() {
    let body = ":where(:scope):is(.row),.row{--tone:red;color:var(--tone);padding:1px}";
    let source = format!(
        "@layer comp{{@scope ([data-b=\"a\"]) to ([data-b]){{{body}}}@scope ([data-b=\"b\"]) to ([data-b]){{{body}}}}}"
    );
    let (css, report) = dedupe(&source);
    assert!(
        css.contains("@scope ([data-b=\"a\"],[data-b=\"b\"]) to ([data-b])"),
        "{css}"
    );
    assert_eq!(css.matches("padding:1px").count(), 1);
    assert_eq!(report.scope_bodies_interned, 1);
    for source in [
        "@scope ([data-b=\"a\"]) to ([data-b]){.x{color:red}}.override{color:blue}@scope ([data-b=\"b\"]) to ([data-b]){.x{color:red}}",
        "@scope ([data-b=\"a\"]) to ([data-b]){.x{color:red}}@scope ([data-b=\"b\"]) to ([data-b]){.x{color:blue}}",
        "@scope ([data-b=\"a\"]) to ([data-b]){& .x{color:red}}@scope ([data-b=\"b\"]) to ([data-b]){& .x{color:red}}",
        "@scope (.outer){@scope ([data-b=\"a\"]) to ([data-b]){.x{color:red}}@scope ([data-b=\"b\"]) to ([data-b]){.x{color:red}}}",
    ] {
        let (css, report) = dedupe(source);
        assert_eq!(report.scope_bodies_interned, 0, "{css}");
        assert_eq!(css, source);
    }
}
#[test]
fn selector_ids_and_url_fragments_are_not_color_values() {
    let css = brain_pack::minify_css(
        ":is(#aabbcc) { color:#aabbcc; filter: url(#aabbcc); background-image:URL(\"#aabbcc\") }",
    );
    assert!(css.contains(":is(#aabbcc)"), "{css}");
    assert!(css.contains("color:#abc"));
    assert!(css.contains("url(#aabbcc)"));
    assert!(css.contains("URL(\"#aabbcc\")"));
}

#[test]
fn private_scope_interning_matches_real_chrome_computed_styles() {
    use std::{
        path::{Path, PathBuf},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let chrome = std::env::var("BRAIN_CHROME")
        .unwrap_or_else(|_| "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
    if !Path::new(&chrome).is_file() {
        assert_ne!(
            std::env::var("BRAIN_REQUIRE_CHROME").as_deref(),
            Ok("1"),
            "Chrome is required for this proof run"
        );
        eprintln!("skipped real CSS engine proof: set BRAIN_CHROME to an installed browser");
        return;
    }
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(
        std::env::temp_dir().join(format!("brain-css-browser-{}-{stamp}", std::process::id())),
    );
    std::fs::create_dir_all(&scratch.0).unwrap();
    let body = ":where(:scope):is(.row),.row{--tone:red;--gap:0px;--tokens: a  b ;color:var(--tone);padding-left:1px}.leaf{color:var(--tone);margin-left:calc(var(--gap) + 1px)}";
    let scoped =
        |id: &str, body: &str| format!("@scope ([data-b=\"{id}\"]) to ([data-b]){{{body}}}");
    let a = scoped("a", body);
    let b = scoped("b", body);
    let siblings = "<section data-b='a' id='a' class='row'><span id='a-leaf' class='leaf'>A</span></section><section data-b='b' id='b' class='row'><span id='b-leaf' class='leaf'>B</span></section><section data-b='foreign' id='outside' class='row'><span id='outside-leaf' class='leaf'>Outside</span></section>";
    let nested = "<section data-b='a' id='a' class='row'><span id='a-leaf' class='leaf'>A</span><section data-b='b' id='b' class='row'><span id='b-leaf' class='leaf'>B</span><section data-b='foreign' id='outside' class='row'><span id='outside-leaf' class='leaf'>Outside</span></section></section></section>";
    let cases = [
        ("siblings", format!("{a}{b}"), siblings, true),
        ("nested", format!("{a}{b}"), nested, true),
        (
            "override",
            format!("{a}.row{{color:blue;padding-left:3px}}{b}"),
            siblings,
            false,
        ),
        (
            "minimal-difference",
            format!(
                "{a}{}",
                scoped("b", &body.replace("padding-left:1px", "padding-left:2px"))
            ),
            siblings,
            false,
        ),
        (
            "layer-media",
            format!(
                "@layer base,comp;@layer base{{.row{{color:green}}}}@layer comp{{@media (min-width:1px){{{a}{b}}}}}"
            ),
            nested,
            true,
        ),
        (
            "different-conditions",
            format!("@media (min-width:2000px){{{a}}}@media (max-width:1999px){{{a}}}{b}"),
            siblings,
            false,
        ),
        (
            "different-limits",
            format!("{a}@scope ([data-b=\"b\"]) to (.stop){{{body}}}"),
            nested,
            false,
        ),
    ];
    for (name, css, html, merged) in cases {
        let (optimized, report) = dedupe(&css);
        assert_eq!(
            report.scope_bodies_interned > 0,
            merged,
            "{name}: {optimized}"
        );
        let final_css = brain_pack::minify_css(&optimized);
        let dir = scratch.0.join(name);
        std::fs::create_dir(&dir).unwrap();
        for (path, value) in [
            ("original.css", css.as_str()),
            ("optimized.css", final_css.as_str()),
            ("original.html", html),
            ("optimized.html", html),
        ] {
            std::fs::write(dir.join(path), value).unwrap();
        }
    }
    // Public class `a` and a component-local long class must remain distinct after production renaming.
    let dir = scratch.0.join("class-collision");
    std::fs::create_dir(&dir).unwrap();
    let css = "@scope ([data-b=\"a\"]) to ([data-b]){.component-long-name{color:red}}";
    let map = classes::plan(&[css], &[], &[".a{color:blue}"]);
    let html = "<p id='external' class='a'>External</p><p id='owned' data-b='a' class='component-long-name'>Owned</p>";
    std::fs::write(dir.join("original.css"), format!(".a{{color:blue}}{css}")).unwrap();
    std::fs::write(
        dir.join("optimized.css"),
        format!(".a{{color:blue}}{}", classes::rename_css(css, &map)),
    )
    .unwrap();
    std::fs::write(dir.join("original.html"), html).unwrap();
    std::fs::write(
        dir.join("optimized.html"),
        classes::rename_class_attrs(html, &map),
    )
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = Command::new(std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into()));
    command
        .arg("--test")
        .arg(root.join("scripts/test/css-dedup-browser.test.mjs"))
        .env("BRAIN_CSS_FIXTURE_DIR", &scratch.0)
        .env("BRAIN_CHROME", chrome)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().unwrap();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            #[cfg(unix)]
            unsafe {
                unsafe extern "C" {
                    fn kill(pid: i32, signal: i32) -> i32;
                }
                kill(-(child.id() as i32), 9);
            }
            assert!(status.success(), "real CSS computed-style proof failed");
            break;
        }
        if start.elapsed() > Duration::from_secs(90) {
            #[cfg(unix)]
            unsafe {
                unsafe extern "C" {
                    fn kill(pid: i32, signal: i32) -> i32;
                }
                kill(-(child.id() as i32), 15);
            }
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned CSS browser exceeded 90 seconds");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
