//! Native inference reports only statically proven mismatches, as warnings.
use brain_check::{Ctx, Imported};
use brain_parse::{Diag, Severity};
use std::path::Path;
fn native(src: &str, ctx: &Ctx) -> Vec<Diag> {
    let c = brain_parse::parse(src, Path::new("comp/native.brain")).unwrap();
    brain_check::check(&c, ctx).into_iter().filter(|d| d.code.starts_with("B20")).collect()
}
fn child() -> Ctx {
    let c=brain_parse::parse("<meta props=\"count:num title:str enabled:bool items:list\"><p>Child</p>",Path::new("comp/child.brain")).unwrap();
    let mut ctx=Ctx::default();ctx.imports.insert("./child.brain".into(),Imported::Brain(c.props));ctx
}
#[test]
fn literal_signal_and_derived_props_are_inferred_without_annotations() {
    let src="<script>import Child from './child.brain'\nlet count=sig('wrong')\nlet title=drv(() => 42)\n</script><Child count={count} title={title} enabled={true} items={[]} />";
    let d=native(src,&child());assert_eq!(d.iter().filter(|d|d.code=="B2001").count(),2,"{d:?}");
    assert!(d.iter().all(|d|d.level==Severity::Warning && d.edits.is_empty()));
}
#[test]
fn unique_object_typo_has_an_exact_edit_but_ambiguous_nearest_does_not() {
    let src="<script>let user={name:'Ada', age:42}</script><p>{user.nmae}</p>";
    let d=native(src,&Ctx::default());assert_eq!(d.len(),1,"{d:?}");assert_eq!(d[0].code,"B2002");assert_eq!(d[0].edits.len(),1);
    let fixed=Diag::apply(src,&d[0].edits).unwrap();assert!(fixed.contains("user.name"));assert!(native(&fixed,&Ctx::default()).is_empty());
    let d=native("<script>let user={cat:1,car:2}</script><p>{user.cap}</p>",&Ctx::default());
    assert_eq!(d.len(),1);assert!(d[0].edits.is_empty());
}
#[test]
fn unknown_computed_spread_mutated_escaped_and_shadowed_objects_stay_unknown() {
    for src in [
        "<script>let user=fetchUser()</script><p>{user.missing}</p>",
        "<script>let user={...other,name:'Ada'}</script><p>{user.missing}</p>",
        "<script>let user={name:'Ada'}\nuser.extra=42</script><p>{user.extra}</p>",
        "<script>let user={name:'Ada'}\nlet alias=user\nalias.extra=42</script><p>{user.extra}</p>",
        "<script>let user={name:'Ada'}\npatch(user)</script><p>{user.extra}</p>",
        "<script>let user={name:'Ada'}\nlet holder={user}\nholder.user.extra=42</script><p>{user.extra}</p>",
        "<script>let user={name:'Ada'}\npatch({nested:user})</script><p>{user.extra}</p>",
        "<script>let user={name:'Ada'}\nlet {name}=user</script><p>{user.extra}</p>",
        "<script>let user={name:'Ada'}\nfunction get(){return user}\nlet alias=get()\nalias.extra=42</script><p>{user.extra}</p>",
        "<script>let user={name:'Ada'}\nfunction inspect(user) { return user.extra }</script><p>{user.extra}</p>",
        "<script>let user={name:'Ada'}</script><p>{user['extra']}</p>",
        "<script>let user=null</script><p>{user?.name}</p>",
    ] {assert!(native(src,&Ctx::default()).is_empty(),"{src}");}
}
#[test]
fn closures_and_each_scopes_do_not_borrow_a_conflicting_outer_binding() {
    let src="<script>let user={name:'Ada'}\nlet rows=[]</script>{#each rows as user (user.id)}<p>{user.extra}</p>{/each}<button on:click={(user) => user.extra}>Open</button>";
    assert!(native(src,&Ctx::default()).is_empty());
}
#[test]
fn literal_api_input_contracts_check_known_fields_only_and_allow_runtime_numeric_coercion() {
    let src="<script>const User={age:'num!',name:'str!'}\nlet users=api('/users',{create:{in:'User'}})\nusers.create({age:'not a number',name:42})\nusers.create({age:'42',name:'Ada'})\n</script><p>Users</p>";
    let d=native(src,&Ctx::default());assert_eq!(d.iter().filter(|d|d.code=="B2003").count(),2,"{d:?}");
    let mut ctx=Ctx::default();ctx.type_shapes.insert("Remote".into(),[("age".into(),"num!".into())].into());
    let src="<script>let users=api('/users',{create:{in:'Remote'}})\nusers.create({age:false})</script><p>Users</p>";
    assert_eq!(native(src,&ctx).iter().filter(|d|d.code=="B2003").count(),1);
}
#[test]
fn form_schema_reports_incompatible_init_binding_and_unique_field_typo() {
    let src="<script>const User={age:'num',active:'bool'}\nlet f=form(User,{init:{age:'oops'}})</script><input type=\"checkbox\" bind:checked={f.v.age}><input bind:value={f.v.active}><p>{f.v.acitve}</p>";
    let d=native(src,&Ctx::default());assert_eq!(d.iter().filter(|d|d.code=="B2004").count(),3,"{d:?}");assert_eq!(d.iter().filter(|d|d.code=="B2002").count(),1,"{d:?}");
    let good="<script>const User={age:'num',active:'bool'}\nlet f=form(User,{init:{age:42}})</script><input type=\"number\" bind:value={f.v.age}><input type=\"checkbox\" bind:checked={f.v.active}>";
    assert!(native(good,&Ctx::default()).is_empty());
}
#[test]
fn shadowed_primitives_schema_mutation_and_nullable_guesses_produce_no_findings() {
    for src in [
        "<script>function sig(x){return x}\nlet value=sig('x')</script><p>{value.missing}</p>",
        "<script>const User={age:'num'}\nUser.age='str'\nlet users=api('/users',{create:{in:'User'}})\nusers.create({age:'okay'})</script><p>Users</p>",
        "<script>let user=condition ? null : {name:'Ada'}</script>{#if user}<p>{user.name}</p>{/if}",
    ] {assert!(native(src,&Ctx::default()).is_empty(),"{src}");}
}
#[test]
fn diagnostics_are_bounded_and_do_not_attach_late_edits_to_previous_findings() {
    let mut src=String::from("<script>let user={name:'Ada'}</script><div>");
    src.push_str(&"<p>{user.nmae}</p>".repeat(160));src.push_str("</div>");
    let d=native(&src,&Ctx::default());assert_eq!(d.len(),128);assert!(d.iter().all(|d|d.edits.len()==1));
}

#[test]
fn dynamic_route_parameters_get_queries_and_empty_form_initializers_are_not_request_bodies() {
    for src in [
        "<script>const User={id:'num'}\nlet users=api('/users/:id',{post:{in:'User'}})\nusers.post({id:'route-key'})</script><p>Users</p>",
        "<script>const User={id:'num'}\nlet users=api('/users',{get:{in:'User'}})\nusers.get({id:'query'})</script><p>Users</p>",
        "<script>const User={age:'num'}\nlet f=form(User,{init:{age:''}})</script><input bind:value={f.v.age}>",
    ] {assert!(native(src,&Ctx::default()).is_empty(),"{src}");}
}
