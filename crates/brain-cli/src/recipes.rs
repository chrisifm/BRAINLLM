//! Recipes prepare reviewable, schema-derived files; applying them belongs to changes.rs.
use serde_json::{Value, json};
use std::path::Path;

pub fn list() -> Value {
    json!({"schemaVersion":1,"recipes":[
        {"name":"resource-ui","params":{"resource":"existing resource name","view":"optional view filename"},"states":["loading","error","empty","success","disabled"],"experimental":true},
        {"name":"auth-ui","params":{"view":"optional view filename"},"states":["checking","anonymous","authenticated","error","busy"],"experimental":true},
        {"name":"password-reset","params":{"view":"fixed reset-password callback"},"states":["request","sent","confirm","complete","error","busy"],"experimental":true},
        {"name":"email-verification","params":{"view":"fixed verify-email callback"},"states":["anonymous","request","sent","verify","complete","error","busy"],"experimental":true},
        {"name":"google-signin","params":{"view":"optional view filename"},"states":["checking","anonymous","authenticated","error","busy"],"experimental":true}
    ]})
}

pub fn generate(root: &Path, name: &str, params: &Value, dry_run: bool) -> Result<Value, String> {
    let p = params
        .as_object()
        .ok_or("Recipe params must be an object")?;
    for key in p.keys() {
        if key != "view" && !(name == "resource-ui" && key == "resource") {
            return Err(format!("Unknown recipe parameter: {key}"));
        }
    }
    let (default_view, content, contract) = match name {
        "resource-ui" => {
            let resource = p
                .get("resource")
                .and_then(Value::as_str)
                .ok_or("resource-ui requires resource")?;
            let resource = crate::backend::contracts(root, "resources")?
                .into_iter()
                .find(|r| r.name == resource)
                .ok_or("Resource contract not found")?;
            (
                format!("{}-manage", resource.name),
                resource_view(&resource)?,
                Some(serde_json::to_value(resource).map_err(|e| e.to_string())?),
            )
        }
        "auth-ui" => ("account-access".into(), auth_view().into(), None),
        "google-signin" => ("google-access".into(), String::new(), None),
        "password-reset" => (
            "reset-password".into(),
            include_str!("templates/auth/reset-password.brain").into(),
            None,
        ),
        "email-verification" => (
            "verify-email".into(),
            include_str!("templates/auth/verify-email.brain").into(),
            None,
        ),
        _ => return Err(format!("Unknown recipe: {name}")),
    };
    let view = p
        .get("view")
        .map(|v| v.as_str().ok_or("view must be a string"))
        .transpose()?
        .unwrap_or(&default_view);
    if view.is_empty()
        || view.len() > 64
        || !view
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err(
            "view must contain lowercase letters, digits, hyphens or underscores (1-64 characters)"
                .into(),
        );
    }
    if matches!(name, "password-reset" | "email-verification") && view != default_view {
        return Err("Account mail recipes preserve their stable callback paths; configure PUBLIC_SITE_URL for a hosting prefix".into());
    }
    let path = format!("views/{view}.brain");
    if root.join(&path).symlink_metadata().is_ok() {
        return Err(format!(
            "Recipe refuses to overwrite {path}; choose another view"
        ));
    }
    let mut operations = vec![json!({"path":path,"expectedRevision":null,"content":content})];
    let required: &[&str] = match name {
        "resource-ui" => &["button", "field", "input", "checkbox", "error-state"],
        "auth-ui" | "password-reset" => &["button", "field", "input", "error-state"],
        "email-verification" => &["button", "error-state"],
        "google-signin" => &["button"],
        _ => &[],
    };
    let mut basics: std::collections::BTreeMap<String, String> = crate::design::new_basics()
        .into_iter()
        .map(|(path, source)| (path.to_owned(), source))
        .collect();
    basics.insert(
        "ui/error-state.brain".into(),
        include_str!("../../../runtime/ui/error-state.brain").into(),
    );
    for control in required {
        let path = format!("ui/{control}.brain");
        if crate::changes::read(root, &path)?["revision"].is_null() {
            operations.push(json!({"path":path,"expectedRevision":null,"content":basics.get(&path).ok_or("Native recipe control source is unavailable")?}));
        }
    }
    if name == "google-signin" {
        let module = format!("lib/{view}-google.jsbrain");
        let adapter = format!("public/{view}-google.js");
        for candidate in [&module, &adapter] {
            if root.join(candidate).symlink_metadata().is_ok() {
                return Err(format!("Recipe refuses to overwrite {candidate}"));
            }
        }
        operations[0]["content"] = json!(google_view(&module));
        operations.push(json!({"path":module,"expectedRevision":null,"content":format!("import adapter from '../{adapter}'\nexport default adapter\n")}));
        operations.push(json!({"path":adapter,"expectedRevision":null,"content":include_str!("../../../runtime/templates/google-signin.js")}));
    }
    Ok(
        json!({"schemaVersion":1,"recipe":name,"dryRun":dry_run,"previewOnly":true,
        "operations":operations,
        "contract":contract,"requirements":["Experimental Rust backend must be running and migrated","Expose /api and /_brain/auth on the frontend origin; a separate backend port alone is insufficient","Configure fixed loopback BRAIN_BACKEND_URL for native API proxy and SSR session verification; enforce policy on the backend","Native account endpoints stay same-origin under the app base; resource API paths require an appropriate HTTP policy base","auth-ui and google-signin require configured BRAIN_JWT_SECRET/ISSUER/AUDIENCE and GOOGLE_CLIENT_ID, HTTPS, and the documented Google GIS CSP allowlist"],
        "nativeAccountFlows":{"passwordReset":"brain recipe password-reset","emailVerification":"brain recipe email-verification","administrator":"brain backend bootstrap-admin --user <existing-uuid>","mail":"Configure private BRAIN_MAIL_RESEND_KEY/BRAIN_MAIL_FROM and PUBLIC_SITE_URL; no email is sent by this preview"},"deployment":"brain deploy config pm2|nginx prepares optional guarded native topology configuration"}),
    )
}

fn google_view(module: &str) -> String {
    format!(
        r#"<meta seo title="Account access" description="Sign in with Google or use email sign-in." robots="noindex">
<meta client height="220px">
<script>
  import Button from '../ui/button.brain'
  import adapter from '../{module}'
  let host = ref()
  let busy = true
  let signedIn = false
  let message = 'Preparing secure sign-in.'
  function state(value) {{ busy = value.busy; signedIn = value.signedIn; message = value.message }}
  const widget = interop(() => host, adapter, {{ onState: state }}, {{ onError: () => {{ busy = false; message = 'Sign-in is unavailable. Retry or use email sign-in.' }} }})
  function retry() {{ widget.update({{ onState: state }}) }}
  function logout() {{ widget.update({{ onState: state, action: 'logout' }}) }}
</script>
<head><meta name="brain-google-signin" content="1"></head>
<section aria-labelledby="google-heading">
  <h1 id="google-heading">Account access</h1>
  <p role="status" aria-live="polite">{{message}}</p>
  <div bind:ref={{host}} aria-label="Google sign-in" aria-busy={{busy}}></div>
  {{#if signedIn}}
    <Button on:click={{logout}} disabled={{busy}}>Sign out</Button>
  {{:else}}
    <Button on:click={{retry}} disabled={{busy}}>Retry sign-in</Button>
    <a href="/account-access">Use email sign-in</a>
  {{/if}}
</section>
<style>
  section {{ max-width: 36rem; padding: var(--space-4); margin: auto; }}
  div {{ min-height: 44px; }}
</style>
"#
    )
}

pub(crate) fn resource_view(r: &brain_backend::Resource) -> Result<String, String> {
    if !brain_backend::contract::identifier(&r.name)
        || r.fields
            .keys()
            .any(|key| !brain_backend::contract::identifier(key))
    {
        return Err("Resource and field names must be valid backend identifiers".into());
    }
    let mut schema = serde_json::Map::new();
    let mut controls = String::new();
    let mut decode = String::new();
    let mut encode = String::new();
    for (key, field) in &r.fields {
        let kind = if field.kind == "obj" {
            "str"
        } else {
            &field.kind
        };
        schema.insert(
            key.clone(),
            json!(format!(
                "{kind}{}",
                if field.required && kind != "bool" {
                    "!"
                } else {
                    ""
                }
            )),
        );
        match field.kind.as_str() {
            "bool" => controls.push_str(&format!("      <Checkbox label=\"{key}\" name=\"{key}\" bind:checked={{f.v.{key}}} error={{f.err.{key}}} disabled={{f.busy || deleting !== ''}} />\n")),
            "obj" => {
                controls.push_str(&format!("      <label>{key} (JSON object)<textarea name=\"{key}\" bind:value={{f.v.{key}}} disabled={{f.busy || deleting !== ''}}></textarea></label>\n      {{#if f.err.{key}}}<p role=\"alert\">{{f.err.{key}}}</p>{{/if}}\n"));
                decode.push_str(&format!("      values.{key} = v.{key} === '' ? {{}} : JSON.parse(v.{key})\n      if (!values.{key} || Array.isArray(values.{key}) || typeof values.{key} !== 'object') throw Error('{key} must be a JSON object')\n"));
                encode.push_str(&format!("    values.{key} = row.data.{key} == null ? '' : JSON.stringify(row.data.{key})\n"));
            },
            "str" | "email" | "num" => controls.push_str(&format!("      <Input label=\"{key}\" name=\"{key}\" type=\"{}\" bind:value={{f.v.{key}}} error={{f.err.{key}}} disabled={{f.busy || deleting !== ''}} />\n", if field.kind == "num" { "number" } else if field.kind == "email" { "email" } else { "text" })),
            other => return Err(format!("Recipe does not support field kind {other}")),
        }
    }
    let template = r#"<meta seo title="Manage __NAME__" description="Search and manage __NAME__ records." robots="noindex">
<script>
  import Button from '../ui/button.brain'
  import Input from '../ui/input.brain'
  import Checkbox from '../ui/checkbox.brain'
  import ErrorState from '../ui/error-state.brain'
  const collection = api('/api/__NAME__', { get: {}, post: {} })
  const record = api('/api/__NAME__/:id', { put: {}, delete: {} })
  let search = sig('')
  let query = sig('')
  let offset = sig(0)
  let editing = sig('')
  let deleting = sig('')
  let notice = sig('')
  let failure = sig('')
  const limit = 25
  let rows = resource(() => collection.get({ search: query, limit, offset }))
  let f = form(__SCHEMA__, { submit: async (v) => {
      let values = { ...v }
__DECODE__      if (editing) await record.put({ ...values, id: editing })
      else await collection.post(values)
      editing = ''
      f.reset()
      notice = 'Record saved.'
      failure = ''
  } })
  const edit = (row) => {
    let values = { ...row.data }
__ENCODE__    f.reset(values)
    editing = row.id
    notice = ''
  }
  const remove = async (id) => {
    deleting = id
    failure = ''
    notice = ''
    try {
      await record.delete({ id })
      if (editing === id) { editing = ''; f.reset() }
      notice = 'Record deleted.'
      if (offset > 0) offset = Math.max(0, offset - limit)
    } catch (error) { failure = 'Could not delete this record. Retry or check your permissions.' }
    finally { deleting = '' }
  }
  const find = () => { offset = 0; query = search; notice = '' }
</script>
<main class="page">
  <h1>Manage __NAME__</h1>
  <form on:submit={find}>
    <Input label="Search records" bind:value={search} />
    <Button type="submit" disabled={f.busy || deleting !== ''}>Search</Button>
  </form>
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if failure}<p role="alert">{failure}</p>{/if}
  {#await rows}
    <p role="status">Loading records...</p>
  {:then page}
    <ul aria-label="Records">
      {#each page.items as row (row.id)}
        <li>
          <p>{JSON.stringify(row.data)}</p>
          <Button variant="ghost" disabled={f.busy || deleting !== ''} on:click={() => edit(row)}>Edit {row.id}</Button>
          <Button variant="danger" busy={deleting === row.id} disabled={f.busy || deleting !== ''} on:click={() => remove(row.id)}>Delete {row.id}</Button>
        </li>
      {:empty}
        <li>No records match this search.</li>
      {/each}
    </ul>
    <nav aria-label="Record pages">
      <Button variant="ghost" disabled={offset === 0 || f.busy || deleting !== ''} on:click={() => offset = Math.max(0, offset - limit)}>Previous</Button>
      <span>Page {Math.floor(offset / limit) + 1}</span>
      <Button variant="ghost" disabled={page.items.length < limit || f.busy || deleting !== ''} on:click={() => offset += limit}>Next</Button>
    </nav>
  {:catch error}
    <ErrorState error={error} retry={rows.refetch} login="/account-access" />
  {/await}
  <section aria-label="Record editor">
    <h2>{editing ? 'Edit record' : 'Create record'}</h2>
    <form on:submit={f.submit}>
__CONTROLS__      {#if f.error}<p role="alert">{f.error}</p>{/if}
      <Button type="submit" busy={f.busy} disabled={deleting !== ''}>Save record</Button>
      <Button variant="ghost" disabled={f.busy || deleting !== ''} on:click={() => { editing = ''; f.reset(); notice = '' }}>Clear editor</Button>
    </form>
  </section>
</main>
<style>
  .page { max-width: 80ch; margin: auto; padding: var(--space-4); display: grid; gap: var(--space-4); }
  form, section, li, label { display: grid; gap: var(--space-2); }
  ul { padding: 0; list-style: none; display: grid; gap: var(--space-3); }
  nav { display: flex; align-items: center; gap: var(--space-2); }
  textarea { min-height: 120px; font: inherit; color: var(--fg); background: var(--surface); border: 1px solid var(--line); }
</style>
"#;
    Ok(template
        .replace("__NAME__", &r.name)
        .replace("__SCHEMA__", &Value::Object(schema).to_string())
        .replace("__CONTROLS__", &controls)
        .replace("__DECODE__", &decode)
        .replace("__ENCODE__", &encode))
}

fn auth_view() -> &'static str {
    r#"<meta seo title="Account access" description="Create an account, sign in and sign out." robots="noindex">
<script>
  import Button from '../ui/button.brain'
  import Input from '../ui/input.brain'
  import ErrorState from '../ui/error-state.brain'
  const refresh = api('/_brain/auth/refresh', { post: {} })
  const login = api('/_brain/auth/token', { post: {} })
  const signup = api('/_brain/auth/signup', { post: {} })
  const logout = api('/_brain/auth/logout', { post: {} })
  const me = api('/_brain/auth/me', { get: {} })
  let signedUp = sig(false)
  let registering = sig(false)
  let leaving = sig(false)
  let failure = sig('')
  let account = resource(async () => {
    try { return await me.get() }
    catch (error) {
      if (error.status !== 401) throw error
      try { await refresh.post({}); return await me.get() }
      catch (failure) { if (failure.status === 401) return { user: null }; throw failure }
    }
  })
  let f = form({ email: 'email!', password: 'str!>=12' }, { submit: async (v) => {
    if (registering) { await signup.post(v); signedUp = true; registering = false }
    else { await login.post(v); f.reset(); account.refetch() }
  } })
  const leave = async () => {
    leaving = true
    failure = ''
    try { await logout.post({}); f.reset(); account.refetch() }
    catch (error) { failure = 'Could not sign out. Retry when the account service is available.' }
    finally { leaving = false }
  }
</script>
<main class="page">
  <h1>Account access</h1>
  {#if failure}<p role="alert">{failure}</p>{/if}
  {#await account}
    <p role="status">Checking your session...</p>
  {:then session}
    {#if session.user}
      <p role="status">Signed in as {session.user.email}.</p>
      <Button busy={leaving} on:click={leave}>Sign out</Button>
    {:else}
      {#if signedUp}<p role="status">Account created. You can sign in now.</p>{/if}
      <h2>{registering ? 'Create account' : 'Sign in'}</h2>
      <form on:submit={f.submit}>
        <Input label="Email" name="email" type="email" autocomplete="email" bind:value={f.v.email} error={f.err.email} disabled={f.busy} />
        <Input label="Password" name="password" type="password" autocomplete={registering ? 'new-password' : 'current-password'} bind:value={f.v.password} error={f.err.password} disabled={f.busy} />
        {#if f.error}<p role="alert">{f.error}</p>{/if}
        <Button type="submit" busy={f.busy}>{registering ? 'Create account' : 'Sign in'}</Button>
        <Button variant="ghost" disabled={f.busy} on:click={() => { registering = !registering; f.reset() }}>{registering ? 'Use an existing account' : 'Create an account'}</Button>
      </form>
    {/if}
  {:catch error}
    <ErrorState error={error} retry={account.refetch} />
  {/await}
</main>
<style>
  .page { max-width: 48ch; margin: auto; padding: var(--space-4); display: grid; gap: var(--space-3); }
  form { display: grid; gap: var(--space-3); }
</style>
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepares_auth_without_writing_and_rejects_traversal() {
        let scratch =
            Scratch(std::env::temp_dir().join(format!("brain-recipe-{}", std::process::id())));
        std::fs::create_dir_all(&scratch.0).unwrap();
        let root = &scratch.0;
        let plan = generate(&root, "auth-ui", &json!({}), true).unwrap();
        assert_eq!(plan["operations"][0]["path"], "views/account-access.brain");
        assert!(
            plan["operations"][0]["content"]
                .as_str()
                .unwrap()
                .contains("/_brain/auth/me")
        );
        assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
        assert!(generate(&root, "auth-ui", &json!({"view":"../../escape"}), true).is_err());
        assert!(generate(&root, "auth-ui", &json!({"unknown":true}), true).is_err());
    }
    #[test]
    fn resource_controls_follow_contract_not_handwritten_shapes() {
        let r: brain_backend::Resource = serde_json::from_value(json!({"name":"notes","fields":{"title":{"kind":"str","required":true},"done":{"kind":"bool","required":false},"details":{"kind":"obj","required":false}},"policy":{"read":["owner"],"create":["candidate"],"update":["owner"],"delete":["owner"]}})).unwrap();
        let view = resource_view(&r).unwrap();
        assert!(view.contains("\"title\":\"str!\""));
        assert!(view.contains("bind:checked={f.v.done}"));
        assert!(view.contains("JSON.parse(v.details)"));
        assert!(view.contains("values.details = row.data.details"));
        assert!(view.contains("record.put({ ...values, id: editing })"));
        assert!(view.contains("page.items as row (row.id)"));
        assert!(!view.contains("__SCHEMA__"));
    }
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn scaffold_fixture(name: &str) -> Scratch {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "brain-recipe-{name}-{}-{stamp}",
            std::process::id()
        ));
        crate::scaffold::new_app(&root).unwrap();
        Scratch(root)
    }
    fn write_plan(root: &Path, plan: &Value) {
        for operation in plan["operations"].as_array().unwrap() {
            assert!(operation["expectedRevision"].is_null());
            let path = root.join(operation["path"].as_str().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, operation["content"].as_str().unwrap()).unwrap();
        }
    }
    fn assert_compiles(root: &Path, generated: &str) {
        let (count, diags) = crate::run_check(Some(root.to_path_buf())).unwrap();
        assert!(
            count > 1,
            "must check the generated view in its complete scaffold"
        );
        assert!(
            !diags.iter().any(|diag| !diag.is_warning()),
            "{generated}: {}",
            crate::diags_json(&diags)
        );
    }
    #[test]
    fn generated_google_view_has_owned_adapter_and_compiles_in_real_scaffold() {
        let scratch = scaffold_fixture("google");
        let plan = generate(&scratch.0, "google-signin", &json!({}), true).unwrap();
        assert_eq!(plan["operations"].as_array().unwrap().len(), 3);
        write_plan(&scratch.0, &plan);
        assert_compiles(&scratch.0, "google-signin");
        let source = std::fs::read_to_string(scratch.0.join("views/google-access.brain")).unwrap();
        brain_codegen::compile_with(
            &source,
            Path::new("views/google-access.brain"),
            &brain_codegen::Options {
                root: Some(scratch.0.clone()),
                ..Default::default()
            },
        )
        .unwrap_or_else(|diags| panic!("{}", crate::diags_json(&diags)));
        assert!(generate(&scratch.0, "google-signin", &json!({}), true).is_err());
    }
    #[test]
    fn generated_auth_view_compiles_in_real_scaffold() {
        let scratch = scaffold_fixture("auth");
        let plan = generate(&scratch.0, "auth-ui", &json!({}), true).unwrap();
        write_plan(&scratch.0, &plan);
        assert_compiles(&scratch.0, "auth-ui");
    }
    #[test]
    fn generated_resource_view_all_field_kinds_compiles_in_real_scaffold() {
        let scratch = scaffold_fixture("resource");
        crate::backend::add(
            &scratch.0,
            "resource",
            "notes",
            "title:str! address:email! quantity:num done:bool details:obj",
        )
        .unwrap();
        let plan = generate(
            &scratch.0,
            "resource-ui",
            &json!({"resource":"notes"}),
            true,
        )
        .unwrap();
        write_plan(&scratch.0, &plan);
        assert_compiles(&scratch.0, "resource-ui with all field kinds");
        assert!(
            generate(
                &scratch.0,
                "resource-ui",
                &json!({"resource":"notes"}),
                true
            )
            .is_err(),
            "regeneration must not overwrite an applied view"
        );
    }
}
