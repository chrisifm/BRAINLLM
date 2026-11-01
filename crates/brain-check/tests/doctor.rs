//! #36 (INTERNALS §18): app-wide overengineering findings on QA's doctor-app fixture.

use std::path::Path;

#[test]
fn doctor_app_matches_expected() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/fixtures-pending");
    let root = base.join("doctor-app");
    let Ok(expected) = std::fs::read_to_string(root.join("doctor.expected.json")) else { return };
    let got = brain_check::doctor::doctor(&root);
    let got: Vec<String> = got.iter().map(|d| format!("{} {} doctor-app/{} {}:{}", d.code, d.level.as_str(), d.file, d.line, d.col)).collect();
    // Expected entries, read without a JSON library: code, level, file, line, col in that order per object.
    let field = |obj: &str, k: &str| -> String {
        let i = obj.find(&format!("\"{k}\":")).unwrap_or(0) + k.len() + 3;
        let rest = obj[i..].trim_start();
        let end = if rest.starts_with('"') { rest[1..].find('"').map(|e| e + 2).unwrap_or(rest.len()) } else { rest.find([',', '\n', '}']).unwrap_or(rest.len()) };
        rest[..end].trim().trim_matches('"').to_string()
    };
    let want: Vec<String> = expected
        .split("\"code\"")
        .skip(1)
        .map(|o| format!("\"code\"{o}"))
        .map(|o| format!("{} {} {} {}:{}", field(&o, "code"), field(&o, "level"), field(&o, "file"), field(&o, "line"), field(&o, "col")))
        .collect();
    let mut g = got.clone();
    let mut w = want.clone();
    g.sort();
    w.sort();
    assert_eq!(g, w);
}

#[test]
fn a_store_shared_by_every_mount_of_one_component_is_not_overengineering() {
    let root = std::env::temp_dir().join(format!("brain-doctor-b1002-{}", std::process::id()));
    let write = |rel: &str, src: &str| {
        let f = root.join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, src).unwrap();
    };
    write("channels/menu.jsbrain", "export default store({ open: false })\n");
    write("comp/menu.brain", "<script>\n  import menu from '../channels/menu.jsbrain'\n</script>\n<button type=\"button\" on:click={menu.open = !menu.open}>Menu</button>\n");
    write("views/index.brain", "<script>\n  import Menu from '../comp/menu.brain'\n</script>\n<main><Menu /></main>\n");
    let b1002 = || brain_check::doctor::doctor(&root).iter().filter(|d| d.code == "B1002").count();
    assert_eq!(b1002(), 1, "one mount: a local sig is enough");
    write("views/index.brain", "<script>\n  import Menu from '../comp/menu.brain'\n</script>\n<main><Menu /><Menu /></main>\n");
    assert_eq!(b1002(), 0, "two mounts share the store");
    write("views/index.brain", "<script>\n  import Menu from '../comp/menu.brain'\n  let items = sig([1])\n</script>\n<main>{#each items as item}<Menu />{/each}</main>\n");
    assert_eq!(b1002(), 0, "a mount inside a loop shares it too");
    let _ = std::fs::remove_dir_all(&root);
}
