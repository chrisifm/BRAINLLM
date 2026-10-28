//! Agents need an exact compiler identity to distinguish cache generations within one package version.
use std::process::Command;

#[test]
fn version_json_identifies_the_compiler_generation() {
    let output = Command::new(env!("CARGO_BIN_EXE_brain")).args(["--version", "--json"]).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let value = brain_mcp::Json::parse(&text).unwrap();
    assert_eq!(value.get("version").and_then(brain_mcp::Json::as_str), Some(env!("CARGO_PKG_VERSION")));
    let compiler = value.get("compilerVersion").and_then(brain_mcp::Json::as_str).unwrap();
    let (version, fingerprint) = compiler.split_once('+').unwrap();
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    assert_eq!(fingerprint.len(), 16);
    assert!(fingerprint.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(value.get("schemaVersion"), Some(&brain_mcp::Json::Num(1.0)));
}
