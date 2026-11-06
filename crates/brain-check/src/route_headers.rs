//! Literal native-route header contracts, read without executing app modules or importing backend code.
use crate::{RouteHeaders, infer};
use brain_parse::ast::{Expr, Stmt};
use std::path::{Path, PathBuf};

pub(crate) fn load(disk: &Path, root: &Path, deps: &mut Vec<PathBuf>) -> Option<RouteHeaders> {
    let relative = disk.strip_prefix(root).ok()?;
    let parts: Vec<_> = relative.iter().map(|p| p.to_str()).collect::<Option<_>>()?;
    if parts.len() != 4 || parts[0] != "server" || parts[1] != "routes" || !matches!(parts[3],"handler.jsbrain"|"verify.jsbrain") { return None; }
    let path = disk.parent()?.join("route.jsbrain");
    deps.push(path.clone()); // Absence matters too: creating a contract changes the available evidence.
    let module = infer::read_module(Some(root), &path)?;
    let expr = module.script.iter().find_map(|stmt| if let Stmt::ExportDefault { expr, .. } = stmt { Some(expr) } else { None })?;
    let fields = infer::object(expr)?;
    let mut declared = std::collections::HashSet::new();
    if let Some(value) = fields.get("headers") {
        let Expr::Array { elems, .. } = value.unparen() else { return None };
        if elems.len() > 16 { return None; }
        for item in elems {
            let Expr::Str { value, .. } = item.as_ref()?.unparen() else { return None };
            if value.len() > 128 { return None; }
            declared.insert(value.to_ascii_lowercase());
        }
    }
    Some(RouteHeaders { declared, file: path.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/") })
}
