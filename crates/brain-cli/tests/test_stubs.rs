//! `stub()` and `reply()` in `*.test.brain`: a `server:` function and an endpoint answer without a server, and form()
//! puts focus back after the server rejects a submit, in real Chrome.
#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn run(dir: &Path, args: &[&str]) -> Output {
    // the test page is the same for both engines; hide Node so the owned headless Chrome lane runs
    let path: Vec<_> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).filter(|d| !d.join("node").exists()).collect();
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).env("PATH", std::env::join_paths(path).unwrap()).current_dir(dir).stdin(Stdio::null()).output().unwrap()
}

fn app(name: &str) -> PathBuf {
    let tmp = std::env::temp_dir().join(format!("brain-stubs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(Command::new(env!("CARGO_BIN_EXE_brain")).args(["new", "app"]).current_dir(&tmp).stdin(Stdio::null()).output().unwrap().status.success());
    tmp.join("app")
}

const FORM: &str = r#"<script>
  server: async function save(ctx, values) { return { saved: true } }
  let done = sig('')
  const f = form({ email: 'email!', title: 'str!' }, { submit: async v => { await save(v); done = 'saved' } })
</script>
<form on:submit={f.submit}>
  <label>Email <input id="email" name="email" bind:value={f.v.email} disabled={f.busy}></label>
  <label>Title <input id="title" name="title" bind:value={f.v.title} disabled={f.busy}></label>
  <button id="go" disabled={f.busy}>Save</button>
  {#if f.error}<p role="alert" id="error">{f.error}</p>{/if}
  <output id="done">{done}</output>
</form>
"#;

const SIGN_IN: &str = r#"<script>
  const f = form({ email: 'email!', password: 'str!' }, { submit: async () => {} })
</script>
<form on:submit={f.submit}>
  <label>Email <input id="who" name="email" bind:value={f.v.email}></label>
  <label>Password <input name="password" type="password" bind:value={f.v.password}></label>
  <button>Sign in</button>
</form>
"#;

const LIST: &str = r#"<script>
  import items from '../api/items.jsbrain'
  let list = resource(items.get)
</script>
{#await list}<p>Loading</p>{:then rows}<ul>{#each rows as item (item.id)}<li>{item.title}</li>{:empty}<li>None</li>{/each}</ul>{:catch e}<p id="failed">failed</p>{/await}
"#;

const API: &str = "export default api('/items', { get: { out: '[Item]' } })\n";
const TYPES: &str = "export const Item = { id: 'num!', title: 'str!' }\n";

const TESTS: &str = r#"<script>
  import Editor from './editor.brain'
  import Items from './items.brain'
  import SignIn from './signin.brain'

  const fill = async () => {
    await type('#email', 'a@b.co')
    await type('#title', 'Hello')
  }

  const leaving = () => [...window.__brainDirty].filter(x => x.dirty).length

  test('leaving a half-typed form asks; leaving a half-typed sign in does not', async () => {
    await mount(Editor)
    await fill()
    expect(leaving()).toBe(1)
    await mount(SignIn)
    await type('#who', 'a@b.co')
    expect(leaving()).toBe(0)
  })

  test('a server function answers from its stub and receives the arguments', async () => {
    const save = stub('save', values => ({ saved: values.title === 'Hello' }))
    await mount(Editor)
    await fill()
    await click('#go')
    expect(text('#done')).toBe('saved')
    expect(save.calls[0][0].email).toBe('a@b.co')
  })

  test('focus goes to the first field the server rejected', async () => {
    stub('save', async () => { await new Promise(r => setTimeout(r, 120)); return reply(422, { errors: { title: 'Taken' } }) })
    await mount(Editor)
    await fill()
    document.querySelector('form button').focus()
    await click('#go')
    expect(document.activeElement.name).toBe('title')
  })

  test('a rejection with no field gives focus back to the control that held it', async () => {
    stub('save', async () => { await new Promise(r => setTimeout(r, 120)); return reply(500, { error: 'nope' }) })
    await mount(Editor)
    await fill()
    document.querySelector('form button').focus()
    await click('#go')
    expect(text('#error').length > 0).toBe(true)
    expect(document.activeElement.tagName).toBe('BUTTON')
  })

  test('a resource reads its endpoint from a stub', async () => {
    stub('GET /items', [{ id: 1, title: 'One' }])
    await mount(Items)
    expect(text('li')).toBe('One')
  })

  test('the next test sees its own answer, not a cached one', async () => {
    stub('GET /items', reply(500, { error: 'down' }))
    await mount(Items)
    expect(count('#failed')).toBe(1)
  })
</script>
"#;

#[test]
fn stubs_answer_server_functions_and_resources_and_focus_survives_a_rejection() {
    let dir = app("ok");
    std::fs::write(dir.join("comp/editor.brain"), FORM).unwrap();
    std::fs::write(dir.join("comp/items.brain"), LIST).unwrap();
    std::fs::write(dir.join("comp/signin.brain"), SIGN_IN).unwrap();
    std::fs::write(dir.join("api/items.jsbrain"), API).unwrap();
    std::fs::write(dir.join("types.jsbrain"), TYPES).unwrap();
    std::fs::write(dir.join("comp/stubbed.test.brain"), TESTS).unwrap();
    let out = run(&dir, &["test", "comp/stubbed.test.brain"]);
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    assert!(text.contains("6 ok"), "{text}");
}

#[test]
fn a_stub_nobody_calls_fails_its_test() {
    let dir = app("idle");
    std::fs::write(dir.join("comp/idle.test.brain"), "<script>\n  import Counter from './counter.brain'\n\n  test('misspelled', async () => {\n    stub('sav', { ok: true })\n    await mount(Counter)\n  })\n</script>\n").unwrap();
    let out = run(&dir, &["test", "comp/idle.test.brain"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("stub('sav') was never called"), "{text}");
}
