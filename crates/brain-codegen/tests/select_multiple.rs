//! `<select multiple bind:value={list}>` binds an array both ways: the state selects every listed option, and a
//! change writes the array of selected option values (not the single `e.target.value`). Runs on the real runtime.

use std::path::Path;
use std::process::Command;

const SRC: &str = "<script>
  let picked = sig(['b'])
</script>
<label>Letters
  <select multiple bind:value={picked}>
    <option value=\"a\">A</option>
    <option value=\"b\">B</option>
    <option value=\"c\">C</option>
  </select>
</label>
<output>{picked.join(',')}</output>
";

#[test]
fn a_multiple_select_binds_an_array_both_ways() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("node is not installed: skipping");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let runtime_url = format!("file://{}", root.join("runtime/brain.js").display());
    let opts = brain_codegen::Options { runtime: runtime_url, ..Default::default() };
    let o = brain_codegen::compile_with(SRC, Path::new("comp/letters.brain"), &opts).unwrap_or_else(|d| panic!("{d:?}"));
    assert!(o.js.contains("filter((o) => o.selected)"), "{}", o.js);
    let single = brain_codegen::compile_with(&SRC.replace(" multiple", ""), Path::new("comp/letters.brain"), &opts).unwrap();
    assert!(single.js.contains("e.target.value") && !single.js.contains("o.selected"), "{}", single.js);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("select-multiple");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("letters.js"), &o.js).unwrap();
    let shim = format!("file://{}", root.join("runtime/shim/dom.js").display());
    let script = format!(
        "import {{ install }} from '{shim}'
const doc = install()
const {{ mount }} = await import('{rt}')
const {{ default: C }} = await import('{comp}')
const host = doc.createElement('div'); doc.body.appendChild(host)
mount(C, host)
await new Promise(r => setTimeout(r, 0))
const sel = host.querySelector('select'), opts = sel.querySelectorAll('option')
for (const o of opts) o.value = o.getAttribute('value')
const out = () => host.querySelector('output').textContent
if (opts.map(o => o.selected).join() !== 'false,true,false') throw Error('state did not select: ' + opts.map(o => o.selected))
opts[0].selected = true; opts[2].selected = true; opts[1].selected = false
sel.dispatchEvent(new Event('change', {{ bubbles: true }}))
await new Promise(r => setTimeout(r, 0))
if (out() !== 'a,c') throw Error('change did not write the array: ' + out())
console.log('ok select-multiple')",
        rt = format!("file://{}", root.join("runtime/brain.js").display()),
        comp = format!("file://{}", out.join("letters.js").display()),
    );
    let r = Command::new("node").args(["--input-type=module", "-e", &script]).output().unwrap();
    let so = String::from_utf8_lossy(&r.stdout);
    assert!(r.status.success() && so.contains("ok select-multiple"), "{so}\n{}", String::from_utf8_lossy(&r.stderr));
}
