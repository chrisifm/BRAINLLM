//! Generated checkpoints execute against the real runtime, including async rejection and loop control flow.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn options(dev: bool) -> brain_codegen::Options {
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/brain.js")
        .canonicalize()
        .unwrap();
    brain_codegen::Options {
        dev,
        runtime: runtime.to_string_lossy().into_owned(),
        ..Default::default()
    }
}
fn compile(source: &str, dev: bool) -> String {
    brain_codegen::compile_with(source, Path::new("lib/budget.jsbrain"), &options(dev))
        .unwrap_or_else(|e| panic!("{e:?}"))
        .js
}
fn node(code: &str) {
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "brain-budget-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir_all(&scratch.0).unwrap();
    let path = scratch.0.join("test.mjs");
    std::fs::write(&path, code).unwrap();
    let mut child = std::process::Command::new("node")
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned Node budget regression exceeded 10s deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn loops_recursion_returns_and_await_rejection_execute_in_dev_and_production() {
    let source = r#"
function recurse(n) { return recurse(n + 1) }
function finite(xs) { let sum = 0; for (const x of xs) { if (x === 2) continue; if (x === 4) break; sum += x }; return sum }
function originalThrow(error) { try { throw error } finally { log('finished') } }
async function delay(promise) { try { return await promise } catch (error) { throw error } finally { log('settled') } }
const object = () => ({ ok: true })

export default { recurse, finite, originalThrow, delay, object }
"#;
    for dev in [false, true] {
        let code = compile(source, dev);
        assert!(code.contains("budgetSuspend"));
        assert!(code.contains("line: 2"));
        node(&format!(
            "import assert from 'node:assert/strict';\n{code}\nassert.equal(finite([1,2,3,4,5]),4); assert.equal(object().ok,true); assert.throws(()=>recurse(0),e=>e.code==='B0910'&&e.budget==='depth'&&e.file==='lib/budget.jsbrain'); const e={{}};assert.throws(()=>originalThrow(e),x=>x===e); await assert.rejects(delay(Promise.reject(e)),x=>x===e);assert.equal(await delay(Promise.resolve(9)),9); assert.equal(finite([1]),1);"
        ));
    }
}
#[test]
fn unbounded_owned_iterator_stops_before_the_native_stack_or_page_hangs() {
    let code = compile(
        "export function iterate(xs) { for (const x of xs) { if (x === -1) break }; return 1 }",
        false,
    );
    node(&format!(
        "import assert from 'node:assert/strict';\n{code}\nconst forever={{ [Symbol.iterator]:function*(){{for(;;)yield 1}} }}; assert.throws(()=>iterate(forever),e=>e.code==='B0910'&&['steps','ms'].includes(e.budget)); assert.equal(iterate([1]),1);"
    ));
}
#[test]
fn module_and_factory_helpers_are_internal_and_cannot_collide_with_author_names() {
    let code = compile(
        "const $brainBudget = 2; export const value = () => $brainBudget",
        false,
    );
    node(&format!(
        "import assert from 'node:assert/strict';\n{code}\nassert.equal(value(),2);"
    ));
    let result = brain_codegen::compile_with(
        "<script>const click = () => 1</script><button on:click={click}>Run</button>",
        Path::new("comp/button.brain"),
        &options(false),
    )
    .unwrap();
    assert_eq!(result.js.matches("budgetOwner();").count(), 1);
    assert!(result.js.contains("finally"));
}

#[test]
fn static_constants_and_private_server_functions_do_not_import_client_budget_helpers() {
    for source in [
        "<p>Static</p>",
        "<script>const value = 1</script><p>{value}</p>",
        "<script>server: async function private_work(ctx) { return 1 }</script><p>Static</p>",
    ] {
        let output =
            brain_codegen::compile_with(source, Path::new("comp/static.brain"), &options(false))
                .unwrap();
        assert!(!output.js.contains("brain-budget.js"), "{}", output.js);
        assert!(!output.js.contains("budgetOwner"));
        if !output.server_js.is_empty() {
            assert!(output.server_js.contains("brain-budget.js"));
        }
    }
}
