//! RUNTIME #38: the embedded brain/ui kit (runtime/ui, served from `.brain/kit/ui/`) compiles without diagnostics,
//! and `bind:value` on a `~` prop writes back to the parent's binding.

use std::path::Path;

#[test]
fn every_kit_component_compiles_clean() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/ui");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut n = 0;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "brain") {
            continue;
        }
        let src = std::fs::read_to_string(&p).unwrap();
        let rel = format!(".brain/kit/ui/{}", p.file_name().unwrap().to_string_lossy());
        let o = brain_codegen::compile(&src, Path::new(&rel)).unwrap_or_else(|d| panic!("{rel}: {d:#?}"));
        let blocking: Vec<_> = o.warnings.iter().filter(|d| !d.is_info()).map(|d| format!("{} {}:{} {}", d.code, d.line, d.col, d.msg)).collect();
        assert!(blocking.is_empty(), "{rel}: {blocking:#?}");
        n += 1;
    }
    eprintln!("{n} kit components compiled");
}

#[test]
fn bind_on_a_model_prop_writes_to_the_parent() {
    let src = r#"<meta props="value:str~ label:str">
<label>{label} <input bind:value={value} /></label>"#;
    let o = brain_codegen::compile(src, Path::new("comp/field.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("$p[\"bind:value\"]?.(e.target.value)"), "{}", o.js);
}

#[test]
fn form_reads_are_reactive() {
    let src = r#"<script>
const f = form({ email: '' })
</script>
<form on:submit={f.submit(() => {})}>
  <input aria-label="Email" bind:value={f.v.email} />
  <p class:bad={f.err.email}>{f.err.email}</p>
  <button disabled={f.busy}>Send</button>
</form>"#;
    let o = brain_codegen::compile(src, Path::new("comp/signup.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("insert(null, () => f.err.email"), "{}", o.js);
    assert!(o.js.contains("attr($e") && o.js.contains("() => f.busy"), "{}", o.js);
}

#[test]
fn form_reads_reach_kit_components_as_getters() {
    // RUNTIME #38 repro (views/form.brain): `let f = form(…)` in a view, its fields passed to kit components.
    let src = r#"<meta seo title="Form" description="Kit form components with form()." canonical="/form">
<script>
  let saved = sig('')
  let f = form({ email: 'email!', age: 'num>=18' }, {
    submit: async (v) => { saved = JSON.stringify(v) },
  })
</script>
<main>
  <h1>Form</h1>
  <form on:submit={f.submit} novalidate>
    <Input label="Email" name="email" type="email" bind:value={f.v.email} error={f.err.email} />
    <Button type="submit" busy={f.busy}>Save</Button>
  </form>
  <p>{saved}</p>
</main>"#;
    for dev in [false, true] {
        let opts = brain_codegen::Options { dev, ..Default::default() };
        let o = brain_codegen::compile_with(src, Path::new("views/form.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
        assert!(o.js.contains("get \"error\"() { return f.err.email; }"), "dev={dev}\n{}", o.js);
        assert!(o.js.contains("get \"busy\"() { return f.busy; }"), "dev={dev}\n{}", o.js);
    }
}

#[test]
fn await_on_a_resource_reads_its_value() {
    let src = r#"<script>
const usersApi = { get: () => fetch('/u').then((r) => r.json()) }
let users = resource(usersApi.get)
let id = sig(1)
let user = resource(() => fetch(`/u/${id}`).then((r) => r.json()))
</script>
{#await users}<p>Loading…</p>{:then list}<p>{list.length}</p>{:catch e}<p>{e.message}</p>{/await}
{#await user}<p>…</p>{:then u}<p>{u.name}</p>{:catch e}<p role="alert">{e.message}</p>{/await}
<button on:click={id++}>Next</button>"#;
    for dev in [false, true] {
        let opts = brain_codegen::Options { dev, ..Default::default() };
        let o = brain_codegen::compile_with(src, Path::new("comp/users.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
        assert!(o.js.contains("users.v") && o.js.contains("user.v"), "dev={dev}\n{}", o.js);
        assert!(o.js.contains("resource(usersApi.get)") && o.js.contains("resource(() =>"), "dev={dev}\n{}", o.js);
    }
}
