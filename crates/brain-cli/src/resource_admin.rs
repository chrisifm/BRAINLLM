//! Project-owned administrative resource route templates. Native backend policy remains authoritative.
use brain_backend::Resource;
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};

/// Server-rendered admin reads; filters and pagination remain real, bookmarkable links.
fn resource_list(name: &str, service: &str) -> String {
    r#"<meta seo title="Manage __NAME__" description="Search private __NAME__ records." robots="noindex">
<script>
  import service from '../../../__SERVICE__'
  import Input from '../../../ui/input.brain'
  import Button from '../../../ui/button.brain'
  export async function load(ctx) {
    let offset = Number(ctx.query.offset || 0)
    if (!Number.isInteger(offset) || offset < 0 || offset > 10000) offset = 0
    const search = String(ctx.query.search || '').slice(0,200)
    const page = await service.list(ctx,{search,limit:25,offset})
    return {page,search,offset}
  }
</script>
<main class="page">
  <h1>Manage __NAME__</h1>
  <a href="/admin/__NAME__/new">Create record</a>
  <form method="get">
    <Input label="Search records" name="search" value={props.search} />
    <Button type="submit">Search</Button>
  </form>
  {#if props.page.items.length}
    <ul aria-label="Records">
      {#each props.page.items as item (item.id)}
        <li><a href={'/admin/__NAME__/' + item.id}>Record {item.id}</a><p>{JSON.stringify(item.data)}</p></li>
      {/each}
    </ul>
  {:else}<p role="status">No records match this search.</p>{/if}
  <nav aria-label="Record pages">
    {#if props.offset > 0}<a href={'/admin/__NAME__?search=' + encodeURIComponent(props.search) + '&offset=' + Math.max(0,props.offset - 25)}>Previous</a>{/if}
    <span>Page {Math.floor(props.offset / 25) + 1}</span>
    {#if props.offset + 25 <= 10000 && props.page.items.length === 25}<a href={'/admin/__NAME__?search=' + encodeURIComponent(props.search) + '&offset=' + (props.offset + 25)}>Next</a>{/if}
  </nav>
</main>
<style>
  .page { max-width:80ch; margin:auto; padding:var(--space-4); display:grid; gap:var(--space-3); overflow-wrap:anywhere; }
  form, label, li { display:grid; gap:var(--space-2); }
  ul { list-style:none; padding:0; display:grid; gap:var(--space-3); }
  nav { display:flex; align-items:center; gap:var(--space-3); }
</style>
"#.replace("__NAME__",name).replace("__SERVICE__",service)
}
fn resource_detail(name: &str, service: &str) -> String {
    r#"<meta seo title="__NAME__ record" description="Inspect one private __NAME__ record." robots="noindex">
<script>
  import service from '../../../__SERVICE__'
  export async function load(ctx) { return {item:await service.get(ctx,ctx.params.id)} }
</script>
<main class="page">
  <h1>__NAME__ record</h1>
  <a href="/admin/__NAME__">Back to records</a>
  <dl><dt>Record ID</dt><dd>{props.item.id}</dd><dt>Values</dt><dd>{JSON.stringify(props.item.data)}</dd></dl>
  <a href={'/admin/__NAME__/' + props.item.id + '/edit'}>Edit record</a>
</main>
<style>.page { max-width:80ch; margin:auto; padding:var(--space-4); display:grid; gap:var(--space-3); overflow-wrap:anywhere; } dd { margin:0; }</style>
"#.replace("__NAME__",name).replace("__SERVICE__",service)
}
fn resource_editor(r: &Resource, service: &str, create: bool) -> Result<String, String> {
    let mut record_key = "__brainRecord".to_string();
    while r.fields.contains_key(&record_key) {
        record_key.push('_');
    }
    let mut rules = serde_json::Map::new();
    rules.insert(record_key.clone(), json!("str"));
    let mut controls = String::new();
    for (key, field) in &r.fields {
        let kind = if field.kind == "obj" {
            "str"
        } else {
            &field.kind
        };
        let rule = format!(
            "{kind}{}",
            if field.required && field.kind != "bool" {
                "!"
            } else {
                ""
            }
        );
        rules.insert(key.clone(), json!(rule));
        let required = if field.required { "true" } else { "false" };
        let control = match field.kind.as_str() {
            "obj" => format!(
                "<Field label=\"{key} (JSON object)\" error={{f.err.{key}}}><textarea name=\"{key}\" aria-label=\"{key} (JSON object)\" bind:value={{f.v.{key}}} disabled={{f.busy || d.busy}}></textarea></Field>"
            ),
            "bool" => format!(
                "<Checkbox label=\"{key}\" name=\"{key}\" bind:checked={{f.v.{key}}} error={{f.err.{key}}} disabled={{f.busy || d.busy}} />"
            ),
            "str" | "email" | "num" => format!(
                "<Input label=\"{key}\" name=\"{key}\" type=\"{}\" required={{{required}}} bind:value={{f.v.{key}}} error={{f.err.{key}}} disabled={{f.busy || d.busy}} />",
                if field.kind == "email" {
                    "email"
                } else if field.kind == "num" {
                    "number"
                } else {
                    "text"
                }
            ),
            _ => return Err("Unsupported generated resource control".into()),
        };
        controls.push_str(&format!("    {control}\n"));
    }
    let delete = if create {
        ""
    } else {
        r#"
  <section aria-label="Delete record">
    <h2>Delete record</h2>
    <form on:submit={d.submit}>
      <input type="hidden" name="__KEY__" bind:value={d.v.__KEY__}>
      <Checkbox label="Confirm deletion" name="confirmation" bind:checked={d.v.confirmation} error={d.err.confirmation} disabled={f.busy || d.busy} />
      {#if d.error}<p role="alert">{d.error}</p>{/if}
      <Button type="submit" busy={d.busy} disabled={f.busy}>Delete</Button>
    </form>
  </section>
"#
    };
    let delete_script = if create {
        "const d = {busy:false}".to_string()
    } else {
        format!(
            r#"server: async function removeRecord(ctx,values) {{
    if (values.confirmation !== true) throw {{errors:{{confirmation:'Confirm deletion'}}}}
    await service.remove(ctx,values.{record_key})
    return {{redirect:'/admin/{name}'}}
  }}
  let d = form({{{record_key}:'str!',confirmation:'bool!'}},{{init:{{{record_key}:props.initial.{record_key},confirmation:false}},submit:removeRecord}})"#,
            name = r.name
        )
    };
    let source = r#"<meta seo title="__TITLE__ __NAME__" description="__TITLE__ __NAME__" robots="noindex">
<script>
  import service from '__PREFIX____SERVICE__'
  import Input from '__PREFIX__ui/input.brain'
  import Checkbox from '__PREFIX__ui/checkbox.brain'
  import Field from '__PREFIX__ui/field.brain'
  import Button from '__PREFIX__ui/button.brain'
  export async function load(ctx) {
    const initial = await service.editor(ctx,__LOAD_ID__)
    initial.__KEY__ = __LOAD_ID__
    return {initial}
  }
  server: async function saveRecord(ctx,values) {
    const input={...values}
    const id=input.__KEY__ || ''
    delete input.__KEY__
    const item=await service.saveForm(ctx,id,input)
    return {...item,redirect:'/admin/__NAME__/' + item.id}
  }
  let f = form(__RULES__,{init:props.initial,submit:saveRecord})
  __DELETE_SCRIPT__
</script>
<main class="page">
  <h1>__TITLE__ __NAME__</h1>
  <a href="/admin/__NAME__">Back to records</a>
  <form on:submit={f.submit}>
    <input type="hidden" name="__KEY__" bind:value={f.v.__KEY__}>
__CONTROLS__    {#if f.error}<p role="alert">{f.error}</p>{/if}
    {#if f.ok}<p role="status">Record saved.</p>{/if}
    <Button type="submit" busy={f.busy} disabled={d.busy}>Save</Button>
  </form>
__DELETE__</main>
<style>
  .page { max-width:60ch; margin:auto; padding:var(--space-4); display:grid; gap:var(--space-3); }
  form, section { display:grid; gap:var(--space-2); }
  textarea { font:inherit; min-height:120px; color:var(--fg); background:var(--surface); border:1px solid var(--line); padding:var(--space-2); }
</style>
"#;
    Ok(source
        .replace("__TITLE__", if create { "Create" } else { "Edit" })
        .replace("__NAME__", &r.name)
        .replace(
            "__PREFIX__",
            if create { "../../../" } else { "../../../../" },
        )
        .replace("__SERVICE__", service)
        .replace("__LOAD_ID__", if create { "''" } else { "ctx.params.id" })
        .replace(
            "__RULES__",
            &serde_json::to_string(&rules).map_err(|_| "Cannot encode admin form rules")?,
        )
        .replace("__DELETE_SCRIPT__", &delete_script)
        .replace("__CONTROLS__", &controls)
        .replace("__DELETE__", delete)
        .replace("__KEY__", &record_key))
}

pub(crate) fn files(root: &Path, r: &Resource) -> Result<Vec<(String, String)>, String> {
    let name = &r.name;
    let base = format!("views/admin/{name}");
    let service = format!("server/services/{name}.jsbrain");
    let guard = format!("{base}/_guard.jsbrain");
    let index = format!("{base}/index.brain");
    let detail = format!("{base}/[id].brain");
    let edit = format!("{base}/[id]/edit.brain");
    let mut files = vec![];
    files.push((guard.clone(),"<meta server>\nexport default function guard(ctx) {\n  if (!ctx.backend || !ctx.user || ctx.user.role !== 'admin') return false\n  return true\n}\n".into()));
    let create = format!("{base}/new.brain");
    files.push((index.clone(), resource_list(name, &service)));
    files.push((detail.clone(), resource_detail(name, &service)));
    files.push((edit.clone(), resource_editor(r, &service, false)?));
    files.push((create.clone(), resource_editor(r, &service, true)?));
    if fs::symlink_metadata(root.join("ui")).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Generator refuses a symlink UI directory".into());
    }
    for control in ["button", "field", "input", "checkbox", "error-state"] {
        if fs::symlink_metadata(root.join(format!("ui/{control}.brain")))
            .is_ok_and(|m| !m.file_type().is_file())
        {
            return Err("Generator requires regular project control files".into());
        }
    }
    let basics: BTreeMap<_, _> = crate::design::new_basics().into_iter().collect();
    for control in ["button", "field", "input", "checkbox"] {
        let path = format!("ui/{control}.brain");
        if fs::symlink_metadata(root.join(&path)).is_err() {
            files.push((
                path.clone(),
                basics
                    .get(path.as_str())
                    .ok_or("Project basic source is unavailable")?
                    .clone(),
            ));
        }
    }
    if fs::symlink_metadata(root.join("ui/error-state.brain")).is_err() {
        files.push((
            "ui/error-state.brain".into(),
            include_str!("../../../runtime/ui/error-state.brain").into(),
        ));
    }
    Ok(files)
}

/// Validate all files together and publish only their leased, compare-and-swap revisions.
/// Existing sources are never operations in this creation-only command.
pub(crate) fn commit(root: &Path, files: &[(String, String)]) -> Result<serde_json::Value, String> {
    let paths: Vec<_> = files.iter().map(|(path, _)| path).collect();
    let owner = "resource-admin-generator";
    let lease =
        crate::changes::lease(root, &json!({"owner":owner,"paths":paths,"ttlSeconds":900}))?;
    let token = lease["token"]
        .as_str()
        .ok_or("Generator lease token is missing")?;
    let result = (|| {
        let operations: Vec<_> = files
            .iter()
            .map(|(path, source)| json!({"path":path,"expectedRevision":null,"content":source}))
            .collect();
        let preview = crate::changes::plan(
            root,
            &json!({"owner":owner,"token":token,"operations":operations}),
        )?;
        let id = preview["planId"]
            .as_str()
            .ok_or("Generator plan identifier is missing")?;
        let applied = crate::changes::apply(root, id, owner, token)?;
        // A committed journal remains available for explicit recovery; its source payload is private.
        Ok(
            json!({"schemaVersion":1,"files":paths,"adminUi":{"status":"generated"},"transaction":applied,"next":"Review policy; run brain backend schema and brain backend migrate; build and doctor the project"}),
        )
    })();
    let released = crate::changes::lease(
        root,
        &json!({"action":"release","owner":owner,"token":token,"paths":paths}),
    );
    match (result, released) {
        (Ok(report), Ok(_)) => Ok(report),
        (Err(error), _) => Err(error),
        (Ok(_), Err(_)) => Err("Administrative files committed, but generator lease cleanup failed; inspect brain changes lease".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> Fixture {
        for _ in 0..128 {
            let root = std::env::temp_dir().join(format!(
                "brain-admin-budget-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => return Fixture(root),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("Cannot reserve admin budget fixture: {error}"),
            }
        }
        panic!("Admin fixture reservation limit exceeded")
    }
    #[test]
    fn administrative_generation_is_opt_in_and_preserves_project_sources() {
        let fixture = fixture();
        let root = fixture.0.join("admin-proof");
        crate::scaffold::new_app(&root).unwrap();
        let tokens = fs::read(root.join("tokens.cssbrain")).unwrap();
        let control_path = root.join("ui/input.brain");
        let original = fs::read_to_string(&control_path).unwrap();
        let customized = format!("<!-- Project-owned control sentinel -->\n{original}");
        fs::write(&control_path, &customized).unwrap();
        crate::backend::add(&root, "resource", "plain", "title:str!").unwrap();
        assert!(!root.join("views/admin/plain").exists());
        crate::backend::add_with_admin(&root, "resource", "posts", "title:str!", true).unwrap();
        assert_eq!(fs::read(&control_path).unwrap(), customized.as_bytes());
        assert_eq!(fs::read(root.join("tokens.cssbrain")).unwrap(), tokens);
        let contract: Value = serde_json::from_slice(
            &fs::read(root.join("server/resources/posts/client-contract.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(contract["adminUi"]["status"], "generated");
        assert_eq!(contract["adminUi"]["routes"].as_array().unwrap().len(), 4);
        let saved = fs::read(root.join("views/admin/posts/new.brain")).unwrap();
        assert!(
            crate::backend::add_with_admin(&root, "resource", "posts", "title:str!", true).is_err()
        );
        assert_eq!(
            fs::read(root.join("views/admin/posts/new.brain")).unwrap(),
            saved
        );
        assert!(
            crate::backend::add_with_admin(&root, "job", "notify", "title:str!", true).is_err()
        );
        assert!(!root.join("server/jobs/notify").exists());
    }
    #[test]
    fn failed_combined_validation_releases_lease_without_publishing_backend_or_routes() {
        let fixture = fixture();
        let root = fixture.0.join("admin-proof");
        crate::scaffold::new_app(&root).unwrap();
        let marker = "<script>let broken = </script><button>Broken project control</button>";
        fs::write(root.join("ui/button.brain"), marker).unwrap();
        assert!(
            crate::backend::add_with_admin(&root, "resource", "posts", "title:str!", true).is_err()
        );
        assert!(!root.join("server/resources/posts/schema.jsbrain").exists());
        assert!(!root.join("views/admin/posts/index.brain").exists());
        assert_eq!(
            fs::read_to_string(root.join("ui/button.brain")).unwrap(),
            marker
        );
        assert!(
            crate::changes::lease(&root, &json!({"action":"list"})).unwrap()["leases"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn conflicting_route_is_never_overwritten_or_partially_generated() {
        let fixture = fixture();
        let root = fixture.0.join("admin-proof");
        crate::scaffold::new_app(&root).unwrap();
        fs::create_dir_all(root.join("views/admin/posts")).unwrap();
        fs::write(
            root.join("views/admin/posts/index.brain"),
            "<h1>Existing project route</h1>",
        )
        .unwrap();
        assert!(
            crate::backend::add_with_admin(&root, "resource", "posts", "title:str!", true).is_err()
        );
        assert!(!root.join("server/resources/posts/schema.jsbrain").exists());
        assert_eq!(
            fs::read_to_string(root.join("views/admin/posts/index.brain")).unwrap(),
            "<h1>Existing project route</h1>"
        );
    }
    #[test]
    fn generated_admin_common_schemas_build_with_the_unchanged_default_budget() {
        let mut failures = Vec::new();
        for (label, fields) in [
            ("text", "title:str! body:str"),
            ("mixed", "title:str! active:bool! score:num meta:obj"),
        ] {
            let fixture = fixture();
            let app = fixture.0.join("admin-proof");
            let root = &app;
            crate::scaffold::new_app(root).unwrap();
            let generated =
                crate::backend::add_with_admin(root, "resource", "posts", fields, true).unwrap();
            assert_eq!(generated["transaction"]["state"], "committed");
            assert!(
                crate::changes::lease(root, &json!({"action":"list"})).unwrap()["leases"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            let config: Value =
                serde_json::from_slice(&fs::read(root.join("brain.json")).unwrap()).unwrap();
            assert_eq!(
                config["budget"]["viewKB"], 30,
                "The fixture must retain the public default"
            );
            crate::env::select(root, "production");
            let mut report = crate::BuildReport {
                files: vec![],
                warnings: vec![],
                ledger: vec![],
                build_cache: brain_mcp::Json::Null,
                optimizations: vec![],
                report: vec![],
                written: 0,
                unchanged: 0,
                signature: String::new(),
                seo: None,
            };
            // Observe emitted bytes before enforcing the same public gate; never alter the budget to inspect it.
            crate::bundle(
                root,
                &root.join("dist"),
                Some("https://example.test"),
                "production",
                &mut report,
                None,
            )
            .unwrap();
            let gate = crate::budget::enforce(root, &report.report);
            let rows: Vec<Value> = report
                .ledger
                .iter()
                .filter_map(|row| serde_json::from_str(&row.stringify()).ok())
                .filter(|row: &Value| {
                    row["route"]
                        .as_str()
                        .is_some_and(|route| route.starts_with("/admin/"))
                })
                .collect();
            eprintln!(
                "{}",
                json!({"schemaVersion":1,"fixture":label,"budgetBytes":30720,"buildGatePassed":gate.is_ok(),"routes":rows,"report":report.report})
            );
            if let Err(error) = gate {
                failures.push(format!("{label}: {error}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
