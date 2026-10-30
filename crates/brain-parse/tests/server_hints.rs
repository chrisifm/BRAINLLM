use std::path::Path;
#[test]
fn malformed_server_modifiers_explain_the_supported_declaration_instead_of_adding_a_semicolon(){
 for statement in ["server const save = async (ctx) => true", "server async function save(ctx) { return true }", "server function save(ctx) { return true }"] {
  let source=format!("<script>\n{statement}\n</script>\n<main>Example</main>");
  let errors=brain_parse::parse(&source,Path::new("views/example.brain")).unwrap_err();
  assert!(errors.iter().any(|error|error.code=="B0704"&&error.fix.contains("server: async function")&&error.fix.contains("do not add a semicolon")),"{errors:?}");
 }
}
#[test]
fn ordinary_server_named_bindings_and_line_breaks_keep_their_previous_meaning(){
 let source="<script>\nconst server=1\nserver\nconst next=2\n</script>\n<main>Example</main>";
 assert!(brain_parse::parse(source,Path::new("views/example.brain")).is_ok());
 assert!(brain_parse::parse("<script>\nserver: async function save(ctx) { return true }\n</script>\n<main>Example</main>",Path::new("views/example.brain")).is_ok());
}
