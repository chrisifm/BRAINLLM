//! Server-only dependency discovery seeds pure runtime modules without making them client import edges.
use crate::{Compile, Node, resolve, specifiers};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

#[derive(Default)]
pub(crate) struct Uses {
    pub exports: HashMap<String, HashSet<String>>,
    scanned: HashSet<String>,
    bytes: usize,
}
impl Uses {
    pub fn collect(
        &mut self,
        root: &Path,
        from: &str,
        code: &str,
        compile: Compile<'_>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 64 {
            return Err("server runtime dependency chain exceeds 64 modules".into());
        }
        if !self.scanned.insert(from.into()) {
            return Ok(());
        }
        self.bytes = self
            .bytes
            .checked_add(code.len())
            .ok_or("server dependency size overflow")?;
        if self.scanned.len() > 512 || code.len() > 4 * 1024 * 1024 || self.bytes > 32 * 1024 * 1024
        {
            return Err(
                "server runtime discovery exceeds 512 modules, 4 MiB per module or 32 MiB total"
                    .into(),
            );
        }
        for (_, _, spec) in specifiers(code)? {
            if let Some(module) = spec.strip_prefix("/@brain/") {
                let names = brain_min::imported_names(code, &|candidate| candidate == spec)?;
                let imports = self.exports.entry(module.into()).or_default();
                if names.is_empty()
                    || crate::dynamic_import_of(code, &|candidate| candidate == spec)
                {
                    imports.insert("*".into());
                } else {
                    imports.extend(names);
                }
            } else if spec.starts_with("./") || spec.starts_with("../") {
                let Node::File(path) = resolve(from, &spec)? else {
                    continue;
                };
                if self.scanned.contains(&path) {
                    continue;
                }
                let absolute = root.join(&path);
                if std::fs::metadata(&absolute)
                    .map_err(|error| format!("{path}: {error}"))?
                    .len()
                    > 4 * 1024 * 1024
                {
                    return Err("server dependency exceeds 4 MiB".into());
                }
                let source = std::fs::read_to_string(&absolute)
                    .map_err(|error| format!("{path}: {error}"))?;
                let server = if path.ends_with(".js") || path.ends_with(".mjs") {
                    source
                } else {
                    let unit = compile(&source, Path::new(&path))?;
                    if unit.server_js.trim().is_empty() {
                        unit.js
                    } else {
                        unit.server_js
                    }
                };
                self.collect(root, &path, &server, compile, depth + 1)?;
            }
        }
        Ok(())
    }
}
