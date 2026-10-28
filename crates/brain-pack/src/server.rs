//! Server modules for `dist/_server/` (#51): the server code of a view or component (`load`, `server:` functions)
//! with every relative import rewritten to the server module of the imported file, and runtime imports
//! (`/@brain/x.js`) pointing at their `/_b/` chunk, which server.mjs resolves with its module hook.
//! Names come from the source path (`_server/<hash(path)>.mjs`), so import cycles need no special handling.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::Compile;

/// Where `brain build` writes the `brain:content` module (brain-seo's `module_js`) before bundling (TOOLCHAIN #136).
pub const CONTENT: &str = ".brain/build/server/_rt/content.mjs";

/// Writes the server module of `rel` (and, recursively, of what it imports) into `files`; returns its dist path.
pub fn emit(
    root: &Path,
    rel: &str,
    compile: Compile,
    runtime_url: &dyn Fn(&str) -> Option<String>,
    written: &mut BTreeMap<String, String>,
    files: &mut BTreeMap<String, String>,
    depth: u32,
) -> Result<String, String> {
    if let Some(p) = written.get(rel) {
        return Ok(p.clone());
    }
    if depth > 64 {
        return Err(format!("{rel}: server imports nested too deep"));
    }
    let out = format!("_server/{}.mjs", crate::content_hash(rel));
    written.insert(rel.to_string(), out.clone());
    let src = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    let code = if rel.ends_with(".js") || rel.ends_with(".mjs") {
        src
    } else {
        let u = compile(&src, Path::new(rel))?;
        if u.server_js.trim().is_empty() { u.js } else { u.server_js }
    };
    let dir = Path::new(rel).parent().unwrap_or(Path::new("")).to_path_buf();
    let res = crate::rewrite_imports(&code, &mut |spec| {
        // `brain:content` (collections, INTERNALS §33): the module the CLI generates for this build, copied once
        if spec == "brain:content" {
            if !files.contains_key("_server/content.mjs") {
                let src = std::fs::read_to_string(root.join(CONTENT)).map_err(|_| format!("{rel}: imports `brain:content`, but {CONTENT} was not generated for this build; fix: build with `brain build`"))?;
                files.insert("_server/content.mjs".into(), src);
            }
            return Ok(Some("./content.mjs".to_string()));
        }
        if let Some(name) = spec.strip_prefix("/@brain/") {
            return runtime_url(name).map(Some).ok_or_else(|| format!("{rel}: imports `{spec}`, which the build did not emit"));
        }
        if spec.starts_with("./") || spec.starts_with("../") {
            let target = normalize(&dir.join(spec)).ok_or_else(|| format!("{rel}: `{spec}` points outside the app"))?;
            let target = target.to_string_lossy().replace('\\', "/");
            let m = emit(root, &target, compile, runtime_url, written, files, depth + 1)?;
            return Ok(Some(format!("./{}", m.trim_start_matches("_server/"))));
        }
        Ok(None) // node: builtins and packages stay as written
    })?;
    files.insert(out.clone(), res);
    Ok(out)
}

/// `views/../lib/x.jsbrain` → `lib/x.jsbrain`; None when it climbs above the app root.
fn normalize(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(x) => out.push(x),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_module_is_copied_into_server_output() {
        let root = std::env::temp_dir().join(format!("brain-pack-content-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".brain/build/server/_rt")).unwrap();
        std::fs::create_dir_all(root.join("views")).unwrap();
        std::fs::write(root.join(CONTENT), "export const page = () => []\n").unwrap();
        std::fs::write(root.join("views/blog.brain"), "x").unwrap();
        let compile = |_: &str, _: &Path| -> Result<crate::Unit, String> {
            Ok(crate::Unit { js: String::new(), css: String::new(), server_js: "import { page } from \"brain:content\";\nexport async function load() { return { posts: page(1) } }\n".into(), server_fns: vec![] })
        };
        let (mut written, mut files) = (BTreeMap::new(), BTreeMap::new());
        let m = emit(&root, "views/blog.brain", &compile, &|_| None, &mut written, &mut files, 0).unwrap();
        assert!(files[&m].contains("from \"./content.mjs\"") && files["_server/content.mjs"].contains("export const page"), "{files:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rewrites_imports_between_server_modules() {
        let root = std::env::temp_dir().join(format!("brain-server-mod-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::create_dir_all(root.join("views")).unwrap();
        std::fs::write(root.join("views/a.brain"), "A").unwrap();
        std::fs::write(root.join("lib/db.js"), "import fs from 'node:fs'\nexport default { fs }\n").unwrap();
        let compile = |_: &str, rel: &Path| -> Result<crate::Unit, String> {
            assert_eq!(rel, Path::new("views/a.brain"));
            Ok(crate::Unit { js: String::new(), css: String::new(), server_js: "import db from \"../lib/db.js\";\nimport { sig } from \"/@brain/brain.js\";\nexport async function load() { return db }\n".into(), server_fns: vec![] })
        };
        let rt = |n: &str| (n == "brain.js").then(|| "/_b/core.js".to_string());
        let (mut written, mut files) = (BTreeMap::new(), BTreeMap::new());
        let a = emit(&root, "views/a.brain", &compile, &rt, &mut written, &mut files, 0).unwrap();
        let db = written["lib/db.js"].clone();
        let code = &files[&a];
        assert!(code.contains(&format!("import db from \"./{}\"", db.trim_start_matches("_server/"))), "{code}");
        assert!(code.contains("from \"/_b/core.js\""), "{code}");
        assert!(files[&db].contains("from 'node:fs'"), "builtins untouched");
        assert!(normalize(Path::new("views/../../x")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
