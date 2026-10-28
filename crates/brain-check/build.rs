//! The exports of every runtime module (`runtime/*.js`), so the checker only accepts primitives that really exist
//! (B0128 for a planned one). Without the runtime folder (crate used on its own) the list is empty and the check is off.

use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut known = false;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        known = true;
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "js")).collect();
        files.sort();
        for p in files {
            println!("cargo:rerun-if-changed={}", p.display());
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(&p).unwrap_or_default();
            for line in src.lines() {
                let l = line.trim_start();
                let Some(rest) = l.strip_prefix("export ") else { continue };
                let rest = rest.trim_start_matches("async ");
                if let Some(r) = rest.strip_prefix("function ").or_else(|| rest.strip_prefix("const ")).or_else(|| rest.strip_prefix("let ")).or_else(|| rest.strip_prefix("class ")) {
                    let id: String = r.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect();
                    if !id.is_empty() {
                        pairs.push((name.clone(), id));
                    }
                } else if let Some(r) = rest.strip_prefix('{') {
                    let body = r.split('}').next().unwrap_or("");
                    for part in body.split(',') {
                        let part = part.trim();
                        let id = part.rsplit(" as ").next().unwrap_or(part).trim();
                        if !id.is_empty() {
                            pairs.push((name.clone(), id.to_string()));
                        }
                    }
                }
            }
        }
    }
    // The framework's own `ui/` kit (runtime/ui/*.brain), by content hash: its widgets may keep state (B0116 is for the
    // author's `ui/`), recognized by origin and not by path.
    let ui = dir.join("ui");
    println!("cargo:rerun-if-changed={}", ui.display());
    let mut kit: Vec<u64> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&ui) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "brain") {
                println!("cargo:rerun-if-changed={}", p.display());
                kit.push(fnv(&std::fs::read_to_string(&p).unwrap_or_default()));
            }
        }
    }
    kit.sort();
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("runtime_exports.rs");
    let items: Vec<String> = pairs.iter().map(|(m, n)| format!("({m:?}, {n:?})")).collect();
    let hashes: Vec<String> = kit.iter().map(|h| format!("{h}")).collect();
    std::fs::write(out, format!("pub const RUNTIME_KNOWN: bool = {known};\npub const RUNTIME_EXPORTS: &[(&str, &str)] = &[{}];\npub const KIT_HASHES: &[u64] = &[{}];\n", items.join(", "), hashes.join(", "))).unwrap();
}

/// FNV-1a 64 of the text with `\r\n` folded to `\n` (same function as `brain_check::scope::kit_source`).
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.replace("\r\n", "\n").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
