//! Typed data is checked before browser execution; no provider or TypeScript process participates.
use brain_check::{ApiContract, Ctx, MethodContract};
use brain_parse::{Diag, Severity};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
fn check(src: &str, ctx: &Ctx) -> Vec<Diag> {
    let c = brain_parse::parse(src, Path::new("comp/flow.brain")).unwrap();
    brain_check::check(&c, ctx)
        .into_iter()
        .filter(|d| d.code.starts_with("B20"))
        .collect()
}
fn typed() -> Ctx {
    let mut ctx = Ctx::default();
    ctx.types = Some(["User".to_string()].into());
    ctx.type_shapes.insert(
        "User".into(),
        [
            ("id".into(), "num!".into()),
            ("name".into(), "str!".into()),
            ("email".into(), "email".into()),
        ]
        .into(),
    );
    ctx
}
#[test]
fn api_resource_await_each_and_derived_values_share_the_declared_shape() {
    let src = "<script>let usersApi=api('/users',{get:{out:'[User]'}})\nlet users=resource(usersApi.get)\nlet derived=drv(()=>users)\n</script>{#await derived}<p>Loading</p>{:then rows}{#each rows as user (user.id)}<p>{user.nmae}</p>{:empty}<p>Empty</p>{/each}{:catch error}<p>Error</p>{/await}";
    let d = check(src, &typed());
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "B2002");
    assert_eq!(d[0].edits[0].text, "name");
    let fixed = Diag::apply(src, &d[0].edits).unwrap();
    assert!(check(&fixed, &typed()).is_empty());
}
#[test]
fn awaited_calls_in_scoped_functions_and_resource_callbacks_keep_fields() {
    for src in [
        "<script>let users=api('/users',{get:{out:'User'}})\nasync function inspect(){const person=await users.get();return person.nmae}</script><p>Users</p>",
        "<script>let users=api('/users',{get:{out:'User'}})\nlet person=resource(async()=>{const result=await users.get();return result})</script>{#await person}<p>Loading</p>{:then user}<p>{user.nmae}</p>{:catch error}<p>Error</p>{/await}",
    ] {
        let d = check(src, &typed());
        assert_eq!(d.iter().filter(|d| d.code == "B2002").count(), 1, "{d:?}");
    }
}
#[test]
fn optional_field_guards_and_optional_chains_avoid_nullable_false_positives() {
    let prefix =
        "<script>let users=api('/users',{get:{out:'User'}})\nlet user=await users.get()</script>";
    for suffix in [
        "{#if user.email}<p>{user.email.length}</p>{/if}",
        "<p>{user.email?.length}</p>",
        "<p>{user.email != null && user.email.length}</p>",
        "<p>{user.email == null ? 'Missing' : user.email.length}</p>",
        "<p>{user.email !== null && user.email !== undefined && user.email.length}</p>",
    ] {
        assert!(
            check(&format!("{prefix}{suffix}"), &typed()).is_empty(),
            "{suffix}"
        );
    }
    let d = check(&format!("{prefix}<p>{{user.email.length}}</p>"), &typed());
    assert_eq!(d.iter().filter(|d| d.code == "B2005").count(), 1, "{d:?}");
    assert!(
        d.iter()
            .all(|d| d.level == Severity::Warning && d.edits.is_empty())
    );
    let d = check(
        &format!("{prefix}<p>{{user.email !== null && user.email.length}}</p>"),
        &typed(),
    );
    assert_eq!(
        d.iter().filter(|d| d.code == "B2005").count(),
        1,
        "undefined remains unchecked: {d:?}"
    );
}
#[test]
fn null_union_narrowing_handles_early_returns_optional_chain_boundaries_and_else() {
    for src in [
        "<script>let user=flag?null:{name:'Ada'}</script>{#if user}<p>{user.name.length}</p>{/if}",
        "<script>let user=flag?null:{name:'Ada'}</script><p>{user?.name.length}</p>",
        "<script>let user=flag?null:{name:'Ada'}\nfunction read(){if(user==null)return;return user.name.length}</script><p>Read</p>",
    ] {
        assert!(check(src, &Ctx::default()).is_empty(), "{src}");
    }
    let src = "<script>let user=flag?null:{name:'Ada'}</script><p>{(user?.name).length}</p>";
    assert_eq!(
        check(src, &Ctx::default())
            .iter()
            .filter(|d| d.code == "B2005")
            .count(),
        1
    );
}
#[test]
fn backend_row_data_and_envelope_come_from_the_native_resource_contract() {
    let mut ctx = Ctx::default();
    ctx.resource_shapes.insert(
        "posts".into(),
        [
            ("title".into(), "str!".into()),
            ("views".into(), "num!".into()),
        ]
        .into(),
    );
    let src = "<script>let posts=api('/api/posts',{get:{}})\nlet page=resource(posts.get)</script>{#await page}<p>Loading</p>{:then result}{#each result.items as row (row.id)}<p>{row.data.titel}</p>{:empty}<p>Empty</p>{/each}{:catch error}<p>Error</p>{/await}";
    let d = check(src, &ctx);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].edits[0].text, "title");
    let src = "<script>let posts=api('/api/posts/:id',{get:{}})\nlet row=await posts.get({id:'one'})</script><p>{row.data.missing}</p>";
    assert_eq!(
        check(src, &ctx)
            .iter()
            .filter(|d| d.code == "B2002")
            .count(),
        1
    );
}
#[test]
fn literal_body_unknown_and_missing_fields_have_safe_noninvented_fixes() {
    let src = "<script>let users=api('/users',{post:{in:'User'}})\nusers.post({id:7,nmae:'Ada'})</script><p>Users</p>";
    let d = check(src, &typed());
    assert_eq!(d.iter().filter(|d| d.code == "B2003").count(), 2, "{d:?}");
    let edits = d.iter().flat_map(|d| d.edits.clone()).collect::<Vec<_>>();
    assert_eq!(edits.len(), 1);
    let fixed = Diag::apply(src, &edits).unwrap();
    assert!(check(&fixed, &typed()).is_empty());
}
#[test]
fn imported_api_contracts_and_shared_type_aliases_are_resolved_without_url_guessing() {
    let mut ctx = typed();
    ctx.api_contracts.insert(
        "../api/users.jsbrain".into(),
        ApiContract {
            path: "/users".into(),
            methods: HashMap::from([(
                "get".into(),
                MethodContract {
                    input: None,
                    output: Some("User".into()),
                },
            )]),
            schemas: HashMap::new(),
        },
    );
    let src = "<script>import usersApi from '../api/users.jsbrain'\nlet person=await usersApi.get()</script><p>{person.nmae}</p>";
    assert_eq!(
        check(src, &ctx)
            .iter()
            .filter(|d| d.code == "B2002")
            .count(),
        1
    );
    ctx.schema_aliases
        .insert("Person".into(), ctx.type_shapes["User"].clone());
    let src = "<script>import {User as Person} from '../types.jsbrain'\nlet users=api('/users',{get:{out:'Person'}})\nlet person=await users.get()</script><p>{person.nmae}</p>";
    assert_eq!(
        check(src, &ctx)
            .iter()
            .filter(|d| d.code == "B2002")
            .count(),
        1
    );
    assert!(check("<script>let users=api('/users',{get:{}})\nlet person=await users.get()</script><p>{person.nmae}</p>",&typed()).is_empty());
}
#[test]
fn guarded_bindings_do_not_leak_to_siblings_or_borrow_outer_slot_values() {
    let src = "<script>let user=flag?null:{name:'Ada'}</script>{#if user}<p>{user.name}</p>{/if}<p>{user.name}</p>";
    assert_eq!(
        check(src, &Ctx::default())
            .iter()
            .filter(|d| d.code == "B2005")
            .count(),
        1
    );
    let src = "<script>let user={name:'Ada'}</script><Widget>{#slot user}<p>{user.nmae}</p>{/slot}</Widget>";
    assert!(check(src, &Ctx::default()).is_empty());
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn scratch() -> Scratch {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "brain-native-types-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(path.join("views")).unwrap();
    Scratch(path)
}
#[test]
fn context_load_tracks_referenced_schema_creation_and_rejects_symlink_modules() {
    let root = scratch();
    std::fs::write(root.0.join("app.brain"), "<main>App</main>").unwrap();
    std::fs::create_dir_all(root.0.join("api")).unwrap();
    std::fs::write(
        root.0.join("types.jsbrain"),
        "export const User={id:'num!',name:'str!'}",
    )
    .unwrap();
    std::fs::write(
        root.0.join("api/users.jsbrain"),
        "export default api('/api/users',{get:{}})",
    )
    .unwrap();
    let path = root.0.join("views/flow.brain");
    let src = "<script>import users from '../api/users.jsbrain'\nlet page=resource(users.get)</script><p>Users</p>";
    let c = brain_parse::parse(src, &path).unwrap();
    let (ctx, deps) = brain_check::load_ctx(&path, &c);
    assert!(ctx.api_contracts.contains_key("../api/users.jsbrain"));
    assert!(ctx.resource_shapes.is_empty());
    let schema = root.0.join("server/resources/users/schema.jsbrain");
    assert!(
        deps.contains(
            &root
                .0
                .canonicalize()
                .unwrap()
                .join("server/resources/users/schema.jsbrain")
        )
    );
    std::fs::create_dir_all(schema.parent().unwrap()).unwrap();
    std::fs::write(&schema, "<meta server>export default {name:'str!'}").unwrap();
    let (ctx, _) = brain_check::load_ctx(&path, &c);
    assert!(ctx.resource_shapes.contains_key("users"));
    #[cfg(unix)]
    {
        std::fs::rename(
            root.0.join("api/users.jsbrain"),
            root.0.join("api/private.jsbrain"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            root.0.join("api/private.jsbrain"),
            root.0.join("api/users.jsbrain"),
        )
        .unwrap();
        let (ctx, _) = brain_check::load_ctx(&path, &c);
        assert!(ctx.api_contracts.is_empty());
        assert!(ctx.resource_shapes.is_empty());
    }
}
#[test]
fn llm_mismatch_fixtures_keep_exact_warning_json_locations_and_repairs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/llm/native-types");
    for name in ["prop", "field", "body", "binding", "nullable"] {
        let src = std::fs::read_to_string(root.join(format!("{name}.brain"))).unwrap();
        let mut ctx = Ctx::default();
        if name == "prop" {
            let child = brain_parse::parse(
                "<meta props=\"count:num\"><p>Count</p>",
                Path::new("comp/counter.brain"),
            )
            .unwrap();
            ctx.imports.insert(
                "./counter.brain".into(),
                brain_check::Imported::Brain(child.props),
            );
        }
        let d = check(&src, &ctx);
        let expected = std::fs::read_to_string(root.join(format!("{name}.expected.json"))).unwrap();
        assert_eq!(Diag::list_to_json(&d), expected.trim(), "{name}");
        let edits = d.iter().flat_map(|d| d.edits.clone()).collect::<Vec<_>>();
        if !edits.is_empty() {
            let fixed = Diag::apply(&src, &edits).unwrap();
            assert!(check(&fixed, &ctx).is_empty(), "{name}");
        }
    }
}
#[test]
fn optional_receiver_guards_narrow_all_proven_ancestors() {
    for expr in [
        "user?.name",
        "user?.name != null",
        "user?.name !== undefined",
    ] {
        let src = format!(
            "<script>let user=flag?null:{{name:'Ada'}}</script>{{#if {expr}}}<p>{{user.name.length}}</p>{{/if}}"
        );
        assert!(check(&src, &Ctx::default()).is_empty(), "{src}");
    }
}
#[test]
fn list_rules_have_nonnullable_items_and_keep_the_container_optional() {
    let ctx = Ctx::default();
    let src = "<script>const Post={tags:'[str]'}\nlet apiClient=api('/posts',{get:{out:'Post'}})\nlet post=await apiClient.get()</script>{#each post.tags as tag (tag)}<p>{tag.length}</p>{:empty}<p>Empty</p>{/each}";
    assert!(check(src, &ctx).is_empty());
    let src = "<script>const Post={tags:'[str!]'}\nlet apiClient=api('/posts',{get:{out:'Post'}})\nlet post=await apiClient.get()</script><p>{post.tags.length}</p>";
    assert!(check(src, &ctx).is_empty());
}
#[test]
fn scoped_api_optional_values_are_checked_and_early_return_guards_are_effective() {
    let prefix = "<script>const User={email:'email'}\nlet users=api('/users',{get:{out:'User'}})\nasync function read(){const person=await users.get();";
    let unsafe_src = format!("{prefix}return person.email.length}}</script><p>Read</p>");
    assert_eq!(
        check(&unsafe_src, &Ctx::default())
            .iter()
            .filter(|d| d.code == "B2005")
            .count(),
        1
    );
    let safe_src = format!(
        "{prefix}if(person.email==null)return;return person.email.length}}</script><p>Read</p>"
    );
    assert!(check(&safe_src, &Ctx::default()).is_empty());
}
#[test]
fn api_and_form_contracts_are_hidden_by_lexical_function_and_each_bindings() {
    let mut ctx = typed();
    ctx.api_contracts.insert(
        "../api/users.jsbrain".into(),
        ApiContract {
            path: "/users".into(),
            methods: HashMap::from([
                (
                    "get".into(),
                    MethodContract {
                        input: None,
                        output: Some("User".into()),
                    },
                ),
                (
                    "post".into(),
                    MethodContract {
                        input: Some("User".into()),
                        output: Some("User".into()),
                    },
                ),
            ]),
            schemas: HashMap::new(),
        },
    );
    for src in [
        "<script>import usersApi from '../api/users.jsbrain'\nasync function read(usersApi){const user=await usersApi.get();usersApi.post({name:42});return user.nmae}</script><p>Users</p>",
        "<script>import usersApi from '../api/users.jsbrain'\nlet rows=[]</script>{#each rows as usersApi (usersApi.id)}{#await usersApi.get()}<p>Loading</p>{:then user}<p>{user.nmae}</p>{:catch error}<p>Error</p>{/await}{:empty}<p>Empty</p>{/each}",
        "<script>const Schema={age:'num'}\nlet f=form(Schema)\nlet rows=[]</script>{#each rows as f (f.id)}<input bind:checked={f.v.age}><p>{f.v.nmae}</p>{:empty}<p>Empty</p>{/each}",
    ] {
        assert!(check(src, &ctx).is_empty(), "{src}: {:?}", check(src, &ctx));
    }
}
#[test]
fn declared_api_methods_have_the_same_unique_field_repairs_as_typed_data() {
    let src = "<script>let users=api('/users',{get:{out:'User'}})</script><p>{users.gte()}</p>";
    let d = check(src, &typed());
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "B2002");
    assert_eq!(d[0].edits[0].text, "get");
}
#[test]
fn optional_api_empty_values_and_defaulted_null_props_are_not_false_mismatches() {
    for body in ["{age:null,active:''}", "{age:'',active:null}"] {
        let src = format!(
            "<script>const User={{age:'num',active:'bool'}}\nlet users=api('/users',{{post:{{in:'User'}}}})\nusers.post({body})</script><p>Users</p>"
        );
        assert!(check(&src, &Ctx::default()).is_empty(), "{src}");
    }
    let child = brain_parse::parse(
        "<meta props=\"count:num=0\"><p>Counter</p>",
        Path::new("comp/counter.brain"),
    )
    .unwrap();
    let mut ctx = Ctx::default();
    ctx.imports.insert(
        "./counter.brain".into(),
        brain_check::Imported::Brain(child.props),
    );
    assert!(
        check(
            "<script>import Counter from './counter.brain'</script><Counter count={null} />",
            &ctx
        )
        .is_empty()
    );
}
#[test]
fn an_application_binding_named_undefined_cannot_narrow_a_nullable_field() {
    let src = "<script>const User={email:'email'}\nlet users=api('/users',{get:{out:'User'}})\nlet user=await users.get()\nlet undefined='sentinel'</script><p>{user.email!==undefined && user.email.length}</p>";
    assert_eq!(
        check(src, &Ctx::default())
            .iter()
            .filter(|d| d.code == "B2005")
            .count(),
        1
    );
}
#[test]
#[cfg(unix)]
fn metadata_loading_rejects_fifo_modules_and_types_without_blocking_or_unsafe_deps() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    };
    let root = scratch();
    std::fs::write(root.0.join("app.brain"), "<main>App</main>").unwrap();
    std::fs::create_dir_all(root.0.join("api")).unwrap();
    for relative in ["api/users.jsbrain", "types.jsbrain"] {
        let path = root.0.join(relative);
        let raw = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(writer_path)
            {
                use std::io::Write;
                let _ = file.write_all(b"export default api('/api/users',{get:{}})");
            }
        });
        let source = "<script>import users from '../api/users.jsbrain'</script><p>Users</p>";
        let file = root.0.join("views/flow.brain");
        let component = brain_parse::parse(source, &file).unwrap();
        let (ctx, deps) = brain_check::load_ctx(&file, &component);
        assert!(ctx.api_contracts.is_empty());
        assert!(!deps.contains(&root.0.canonicalize().unwrap().join(relative)));
        writer.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn template_runtime_writes_invalidate_initializer_nullability_without_erasing_form_contracts() {
    let source="<script>let handle=sig(null);let node=sig(null);let untouched=sig(null);let f=form({active:'bool'})</script><button on:click={()=>{handle.open();node.close();untouched.close()}}>Open</button><Child bind:api={handle}/><dialog bind:ref={node} aria-label=\"Dialog\"></dialog><input type=\"text\" bind:value={f.v.active} aria-label=\"Active\">";
    let diagnostics=check(source,&Ctx::default());
    assert_eq!(diagnostics.iter().filter(|diag|diag.code=="B2005").count(),1,"{diagnostics:?}");
    let nullable=diagnostics.iter().find(|diag|diag.code=="B2005").unwrap();
    assert_eq!(nullable.col,source.find("untouched.close").unwrap() as u32 + 1);
    assert!(nullable.edits.is_empty());
    assert!(diagnostics.iter().any(|diag|diag.code=="B2004"),"form schema/binding diagnostics remain active: {diagnostics:?}");
}
