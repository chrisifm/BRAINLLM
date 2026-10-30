use std::collections::HashSet;
fn shake(source: &str, keep: &[&str]) -> String {
    brain_min::tree_shake(
        source,
        &keep.iter().map(|s| s.to_string()).collect::<HashSet<_>>(),
    )
    .unwrap()
}
#[test]
fn removes_unused_local_reexport_roots_and_keeps_module_evaluation() {
    let output = shake(
        "import {rule,check,live} from './schema.mjs';export {rule,check};export function kept(){return live()}",
        &["kept"],
    );
    assert!(
        !output.contains("rule") && !output.contains("check"),
        "{output}"
    );
    assert!(output.contains("live") && output.contains("./schema.mjs"));
    let side = shake("import {unused} from './side.mjs';export {unused};", &[]);
    assert!(side.contains("./side.mjs"));
    assert!(!side.contains("unused"));
}
#[test]
fn aliases_use_external_names_and_local_reads_remain_live() {
    let output = shake(
        "const value=7;export {value as kept,value as omitted};globalThis.seen=value;",
        &["kept"],
    );
    assert!(
        output.contains("as kept") && !output.contains("omitted"),
        "{output}"
    );
    assert!(output.contains("globalThis.seen=value"));
    let used = shake("const value=effect();export {value};consume(value);", &[]);
    assert!(used.contains("effect()") && used.contains("consume(value)"));
}
#[test]
fn never_erases_foreign_reexports_unbound_exports_or_dynamic_eval() {
    let foreign = shake(
        "export {remote as omitted} from './external.mjs';export function kept(){return 1}",
        &["kept"],
    );
    assert!(foreign.contains("remote as omitted") && foreign.contains("./external.mjs"));
    assert!(shake("export {missing};", &[]).contains("missing"));
    let dynamic = "const value=1;eval('value');export {value};";
    assert!(shake(dynamic, &[]).contains("export{value}"));
}
#[test]
fn tdz_and_import_reads_are_preserved_after_export_roots_are_removed() {
    let tdz = shake("consume(later);const later=1;export {later};", &[]);
    assert!(tdz.contains("consume(later)") && tdz.contains("later=1"));
    let import = shake(
        "import {value} from './external.mjs';const alias=value;export {alias};",
        &[],
    );
    assert!(
        import.contains("value") && import.contains("./external.mjs"),
        "{import}"
    );
}

#[test]
fn local_export_pruning_preserves_real_node_values_effects_and_tdz_errors() {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = std::env::temp_dir().join(format!(
        "brain-local-exports-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    std::fs::write(fixture.0.join("schema.mjs"),"globalThis.trace.push('evaluate');export function rule(){return 1}export function check(){return 2}export function live(){return 9}").unwrap();
    let execute = |source: &str| {
        std::fs::write(fixture.0.join("entry.mjs"), source).unwrap();
        let script = "globalThis.trace=[];const {pathToFileURL}=await import('node:url');let value,error=null;try{const m=await import(pathToFileURL(process.argv[1]).href);value=typeof m.kept==='function'?m.kept():m.kept}catch(e){error=e.name}console.log(JSON.stringify({trace,value,error}))";
        let mut child = Command::new("node")
            .args(["--input-type=module", "-e", script])
            .arg(fixture.0.join("entry.mjs"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("owned Node semantic oracle exceeded deadline");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(status.success());
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take(4097)
            .read_to_end(&mut bytes)
            .unwrap();
        assert!(bytes.len() <= 4096);
        String::from_utf8(bytes).unwrap()
    };
    for (source, keep) in [
        (
            "import {rule,check,live} from './schema.mjs';export {rule,check};export function kept(){return live()}",
            vec!["kept"],
        ),
        (
            "const value=7;export {value as kept,value as omitted};",
            vec!["kept"],
        ),
        (
            "globalThis.trace.push(later);const later=1;export {later};",
            vec![],
        ),
        (
            "import {rule} from './schema.mjs';export {rule};globalThis.trace.push(rule());",
            vec![],
        ),
        ("export {missing};", vec![]),
    ] {
        assert_eq!(
            execute(source),
            execute(&shake(source, &keep)),
            "module effects/value/error changed"
        );
    }
}
