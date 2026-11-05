use std::path::Path;
#[test]
fn native_helpers_import_by_use_and_preserve_local_bindings() {
    let output=brain_codegen::compile("<script>const stamp=dateParse('2024-01-01')</script><p>{stamp}</p>",Path::new("comp/date.brain")).unwrap();
    assert!(output.js.contains("import { dateParse } from \"/@brain/brain-time.js\""),"{}",output.js);
    assert!(!output.js.contains("brain-hash.js"));
    let output=brain_codegen::compile("<script>const dateParse = (value) => value</script><p>{dateParse('Local')}</p>",Path::new("comp/local.brain")).unwrap();
    assert!(!output.js.contains("brain-time.js"));
}
#[test]
fn pure_helpers_are_available_to_server_modules() {
    let output=brain_codegen::compile("<meta server>\nexport async function digest(value) { return await hashText(value) }",Path::new("server/digest.jsbrain")).unwrap();
    let code=output.js+&output.server_js;
    assert!(code.contains("brain-hash.js"),"{code}");
    assert!(code.contains("hashText"),"{code}");
}

#[test]
fn pure_helpers_autoimport_in_component_server_functions_and_map_private_dev_output() {
    let src="<meta seo title=\"Native helpers\" description=\"Native helpers remain private to server functions.\">\n<script>server: async function digest(ctx) { return await hashText('server') }</script><p>Ready</p>";
    let output=brain_codegen::compile_with(src,Path::new("views/native.brain"),&brain_codegen::Options{dev:true,..Default::default()}).unwrap();
    assert!(output.server_js.contains("import { hashText } from \"/@brain/brain-hash.js\""),"{}",output.server_js);
    assert!(output.server_js.contains("sourceMappingURL=data:application/json;base64,"));
    assert!(!brain_codegen::compile(src,Path::new("views/native.brain")).unwrap().server_js.contains("sourcesContent"));
}

#[test]
fn outbound_calls_auto_import_in_server_modules() {
    let output=brain_codegen::compile("<meta server>\nexport async function quote(order) { const out = await httpCall('https://rates.example.test/v1/quote', { method: 'POST', json: order, idempotencyKey: true }); return out.json }",Path::new("server/quote.jsbrain")).unwrap();
    let code=output.js+&output.server_js;
    assert!(code.contains("brain-outbound.js"),"{code}");
    let output=brain_codegen::compile("<script>const httpCall = (url) => url</script><p>{httpCall('local')}</p>",Path::new("comp/local.brain")).unwrap();
    assert!(!output.js.contains("brain-outbound.js"));
}

#[test]
fn bounded_list_and_text_helpers_import_for_client_and_server_without_capturing_locals() {
    let body = "const values = range(1, 4); const name = text.fold('LOUD')";
    let output = brain_codegen::compile(&format!("<script>{body}</script><p>{{name}}:{{values.join(',')}}</p>"), Path::new("comp/helpers.brain")).unwrap();
    assert!(output.js.contains("from \"/@brain/brain-list.js\""), "{}", output.js);
    assert!(output.js.contains("from \"/@brain/brain-text.js\""), "{}", output.js);
    let output = brain_codegen::compile("<meta server>\nexport async function values() { return { values: range(1, 4), name: text.fold('LOUD') } }", Path::new("server/helpers.jsbrain")).unwrap();
    let code = output.js + &output.server_js;
    assert!(code.contains("from \"/@brain/brain-list.js\""), "{code}");
    assert!(code.contains("from \"/@brain/brain-text.js\""), "{code}");
    let output = brain_codegen::compile("<script>const range = (a,b) => [a,b]; const text = { fold: (value) => value }</script><p>{text.fold(range(1,2).join(','))}</p>", Path::new("comp/local-helpers.brain")).unwrap();
    assert!(!output.js.contains("brain-list.js") && !output.js.contains("brain-text.js"), "{}", output.js);
}
