//! INTERNALS §43 in the CLI: `brain map` with the reverse graph, filters and pages; `brain card`; `brain split`
//! (refused unless both files pass check); the impact record of `brain run` after a save.
#![cfg(unix)]
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn brain(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr))
}

fn app(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let tmp = std::env::temp_dir().join(format!("bc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).0);
    let dir = tmp.join("app");
    (tmp, dir)
}

const SHOP: &str = r#"<meta seo title="Shop" description="A small shop page used to try splitting a component out of a view.">
<script>
  let items = sig([{ id: 1, name: 'Tea' }, { id: 2, name: 'Coffee' }])
  function remove(id) { items = items.filter((i) => i.id !== id) }
</script>

<main class="page">
  <h1>Shop</h1>
  <ul>
    {#each items as it (it.id)}
      <li class="row">
        <span>{it.name}</span>
        <button on:click={() => remove(it.id)}>Remove</button>
      </li>
    {/each}
  </ul>
</main>
"#;

#[test]
fn map_reverse_graph_filters_and_pages() {
    let (tmp, dir) = app("map");
    let (_, all) = brain(&dir, &["map", "--json"]);
    assert!(all.contains("\"reverse\":{\"reads\":{") && all.contains("\"session.user\":[\"views/_layout.brain\"]"), "{all}");
    let (_, r) = brain(&dir, &["map", "--reads", "session"]);
    assert!(r.contains("views/login.brain") && r.contains("views/account/index.brain") && !r.contains("\"file\":\"comp/counter.brain\""), "{r}");
    let (_, v) = brain(&dir, &["map", "--view", "/account"]);
    assert!(v.contains("\"path\":\"/account\"") && !v.contains("\"path\":\"/about\""), "{v}");
    let (_, p) = brain(&dir, &["map", "--limit", "2"]);
    assert!(p.contains("\"nextCursor\":2") && p.matches("\"level\":").count() == 2, "{p}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn card_returns_the_contract_only() {
    let (tmp, dir) = app("card");
    let (ok, c) = brain(&dir, &["card", "counter"]);
    assert!(ok && c.contains("\"name\":\"Counter\"") && c.contains("\"name\":\"start\",\"type\":\"num\",\"default\":\"0\"") && c.contains("\"status\":{\"errors\":0"), "{c}");
    assert!(!c.contains("<button"), "no body in a card: {c}");
    let (ok, e) = brain(&dir, &["card", "nope"]);
    assert!(!ok && e.contains("no component `nope`"), "{e}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn split_extracts_and_the_app_still_checks() {
    let (tmp, dir) = app("split");
    std::fs::write(dir.join("views/shop.brain"), SHOP).unwrap();
    let (ok, out) = brain(&dir, &["split", "views/shop.brain", "li.row", "ItemRow"]);
    assert!(ok && out.contains("\"created\":\"comp/item-row.brain\"") && out.contains("\"emits\":[\"click\"]"), "{out}");
    let view = std::fs::read_to_string(dir.join("views/shop.brain")).unwrap();
    assert!(view.contains("import ItemRow from '../comp/item-row.brain'") && view.contains("<ItemRow"), "{view}");
    let (ok, chk) = brain(&dir, &["check", "views/shop.brain"]);
    assert!(ok, "{chk}");
    // a selector that matches nothing is refused and changes nothing
    let before = std::fs::read_to_string(dir.join("views/shop.brain")).unwrap();
    let (ok, _) = brain(&dir, &["split", "views/shop.brain", "table.x", "Nope"]);
    assert!(!ok && std::fs::read_to_string(dir.join("views/shop.brain")).unwrap() == before && !dir.join("comp/nope.brain").exists());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn run_prints_an_impact_record_per_save() {
    let (tmp, dir) = app("impact");
    let mut c = Command::new(env!("CARGO_BIN_EXE_brain"))
        .args(["run", "--port", "0"])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let port = {
        let err = c.stderr.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx.send(l);
            }
        });
        loop {
            let l = rx.recv_timeout(Duration::from_secs(120)).expect("brain run printed no address");
            if let Some(p) = l.split("127.0.0.1:").nth(1).and_then(|r| r.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|p| p.parse::<u16>().ok()) {
                break p;
            }
        }
    };
    let get = |p: &str| {
        let _ = std::net::TcpStream::connect(("127.0.0.1", port)).map(|mut s| {
            use std::io::{Read, Write};
            let _ = s.write_all(format!("GET {p} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes());
            let mut b = String::new();
            let _ = s.read_to_string(&mut b);
        });
    };
    get("/views/about.brain");
    let (tx, rx) = std::sync::mpsc::channel();
    let out = c.stdout.take().unwrap();
    std::thread::spawn(move || {
        for l in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(l);
        }
    });
    // edit once the baseline measurement is announced (a condition, not a fixed wait)
    let t = Instant::now();
    loop {
        assert!(t.elapsed() < Duration::from_secs(120), "no impact baseline");
        if rx.recv_timeout(Duration::from_millis(200)).is_ok_and(|l| l.starts_with("impact {\"baseline\"")) {
            break;
        }
    }
    let f = dir.join("views/about.brain");
    let src = std::fs::read_to_string(&f).unwrap();
    std::fs::write(&f, src.replacen("</h1>", "</h1>\n  <p>More words on this page, so the view ships more bytes than before.</p>", 1)).unwrap();
    let t = Instant::now();
    let rec = loop {
        assert!(t.elapsed() < Duration::from_secs(120), "no impact record");
        if let Ok(l) = rx.recv_timeout(Duration::from_millis(200))
            && l.starts_with("impact ")
        {
            break l;
        }
    };
    let _ = c.kill();
    let _ = c.wait();
    assert!(rec.contains("\"file\":\"views/about.brain\"") && rec.contains("\"route\":\"/about\"") && rec.contains("\"delta\":") && rec.contains("\"seo\":["), "{rec}");
    let _ = std::fs::remove_dir_all(&tmp);
}
