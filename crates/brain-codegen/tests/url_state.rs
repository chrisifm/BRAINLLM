use std::path::Path;
fn compile(source: &str) -> String {
    brain_codegen::compile_with(&format!("<meta seo title=\"Catalog\" description=\"Browse the product catalog with public filters.\" canonical=\"/catalog\">{source}"), Path::new("views/catalog.brain"), &brain_codegen::Options { dev: true, ..Default::default() })
        .unwrap_or_else(|d| panic!("{d:?}")).js
}
#[test]
fn public_select_signal_is_url_bound_once_and_form_values_stay_private() {
    let js = compile("<script>let category = sig('all')</script><select aria-label=\"Category\" bind:value={category}><option value=\"all\">All</option></select><select aria-label=\"Other\" bind:value={category}><option value=\"all\">All</option></select>");
    assert!(js.contains("urlBind as $urlBind"), "{js}");
    // Dev HMR also retains a non-executing wiring string; count executable statement lines only.
    assert_eq!(js.lines().filter(|line|line.trim_start().starts_with("$urlBind(category,")).count(), 1);
    assert!(js.contains(&format!("ui.{}.category",brain_codegen::component_hash_len("views/catalog.brain",12))));
    let private = compile("<script>let email = sig('')</script><label>Email<input type=\"email\" bind:value={email}/></label>");
    assert!(!private.contains("brain-url.js"));
}
#[test]
fn explicit_url_state_and_select_opt_out_do_not_get_another_key() {
    let js = compile("<script>let category = urlState('all', {key:'category'})</script><select aria-label=\"Category\" bind:value={category}><option value=\"all\">All</option></select>");
    assert!(js.contains("urlState"));
    assert!(js.contains("category.v"));
    assert!(!js.contains("$urlBind(category,"));
    let js = compile("<script>let category = sig('all')</script><select aria-label=\"Category\" data-url-state=\"off\" bind:value={category}><option value=\"all\">All</option></select>");
    assert!(!js.contains("brain-url.js"));
}

#[test]
fn modal_navigation_keys_survive_formatting_and_css_hash_length_changes() {
    let source="<Dialog title=\"Public dialog\"><p>Details</p></Dialog>";
    let first=compile(source);
    let changed=compile(&format!("<!-- Formatting change -->\n{source}"));
    let key=format!("ui.dialog.{}.0",brain_codegen::component_hash_len("views/catalog.brain",12));
    assert!(first.contains(&key));assert!(changed.contains(&key));
    let production=brain_codegen::compile_with(&format!("<meta seo title=\"Catalog\" description=\"Browse public details in this catalog.\" canonical=\"/catalog\">{source}"),Path::new("views/catalog.brain"),&brain_codegen::Options{dev:false,hash_len:4,..Default::default()}).unwrap();
    assert!(production.js.contains(&key));
    let explicit=compile("<Dialog title=\"Public dialog\" urlKey=\"details\"><p>Details</p></Dialog>");
    assert!(!explicit.contains("ui.dialog."));
}
