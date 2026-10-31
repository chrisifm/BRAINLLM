//! Source-map scanning speedups must not change a byte of emitted JS, server JS, CSS or maps.
//! Snapshots in tests/fixtures/srcmap/*.out were produced by the scanner before those speedups
//! (`UPDATE_SNAPSHOTS=1` rewrites them; only do that for an intended output change).

use std::path::Path;

fn rows(count: usize) -> String {
    let mut source = String::from("<script>\n");
    for n in 0..count { source.push_str(&format!("const v{n} = 'row {n} naïve /*@L{n}*/'\n")); }
    source.push_str("</script><main>");
    for n in 0..count { source.push_str(&format!("<p>{{v{n}}}</p>")); }
    source.push_str("</main>");
    source
}

fn cases() -> Vec<(&'static str, String, &'static str)> {
    vec![
        ("simple", "<script>let n = sig(0)</script><button on:click={n++}>{n}</button>".into(), "comp/simple.brain"),
        ("rows", rows(40), "comp/rows.brain"),
        ("markers", "<script>\nlet value = sig('naïve /*@L71*/')\nconst re = /[/*@L73*/]\\/x/g\nconst change = () => {\nvalue = `π ${re.source} /*@L72*/`\n}\n</script>\n<main><p>html /*@L75*/ {value}</p><button on:click={change}>Go</button></main>".into(), "comp/markers.brain"),
        ("server", "<meta seo title=\"Source map identity\" description=\"Client and server generation share the source-map pass.\"><script>export async function load() {return {value:'server /*@L9*/ value'}}</script><main><h1>{props.value}</h1></main>".into(), "views/server.brain"),
    ]
}

#[test]
fn source_map_outputs_match_pre_speedup_snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/srcmap");
    for (name, source, file) in cases() {
        for dev in [false, true] {
            let output = brain_codegen::compile_with(&source, Path::new(file), &brain_codegen::Options { dev, ..Default::default() })
                .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:#?}"));
            let got = format!("--js--\n{}\n--map--\n{}\n--server--\n{}\n--css--\n{}\n", output.js, output.map, output.server_js, output.css);
            let snapshot = dir.join(format!("{name}.{}.out", if dev { "dev" } else { "prod" }));
            if std::env::var("UPDATE_SNAPSHOTS").is_ok() || !snapshot.exists() {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(&snapshot, &got).unwrap();
                continue;
            }
            assert_eq!(got, std::fs::read_to_string(&snapshot).unwrap(), "{name} dev={dev} differs from {}", snapshot.display());
        }
    }
}
