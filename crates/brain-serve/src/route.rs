//! File-based routes (SPEC «Files»): `views/users.brain` → `/users` · `views/users/[id].brain` → `/users/:id` ·
//! `views/<folder>/_layout.brain` wraps its folder · `views/404.brain` for anything that does not exist.

use std::path::Path;

#[derive(Debug, PartialEq)]
pub struct Route {
    /// Path of the view's `.brain`, relative to the app root (`views/users/[id].brain`).
    pub view: String,
    /// Layouts from outermost to innermost (`views/_layout.brain`, `views/users/_layout.brain`).
    pub layouts: Vec<String>,
    /// Dynamic segments: `[("id", "42")]`.
    pub params: Vec<(String, String)>,
    /// `true` if it fell through to `views/404.brain`.
    pub not_found: bool,
}

/// Resolves `url_path` with the build's single route table (`brain_pack::routes`: same order, `[...rest]`
/// catch-all, `views/404.brain` as `*`), so dev and dist always pick the same view.
pub fn resolve(root: &Path, url_path: &str) -> Option<Route> {
    resolve_checked(root,url_path).ok().flatten()
}

pub fn resolve_checked(root:&Path,url_path:&str)->Result<Option<Route>,String> {
    resolve_request_checked(root,url_path,url_path)
}

pub fn resolve_request(root:&Path,decoded:&str,raw:&str)->Option<Route> {resolve_request_checked(root,decoded,raw).ok().flatten()}
fn resolve_request_checked(root:&Path,decoded:&str,raw:&str)->Result<Option<Route>,String> {
    let table = brain_pack::routes::discover(root);
    let constraints=brain_pack::routes::constraints(root,&table)?;
    let Some((def, params)) = brain_pack::routes::resolve_request(&table, &constraints, decoded,raw) else {return Ok(None);};
    Ok(Some(Route { view: def.view.clone(), layouts: def.layouts.clone(), params, not_found: def.path == "*" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("brain-route-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for f in [
            "views/home.brain",
            "views/_layout.brain",
            "views/about.brain",
            "views/users/index.brain",
            "views/users/_layout.brain",
            "views/users/new.brain",
            "views/users/[id].brain",
            "views/posts/[slug]/index.brain",
            "views/posts/[slug]/comments.brain",
            "views/404.brain",
        ] {
            let p = d.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "<p></p>").unwrap();
        }
        d
    }

    #[test]
    fn static_dynamic_layouts_404() {
        let d = app();
        let r = resolve(&d, "/").unwrap();
        assert_eq!((r.view.as_str(), r.layouts.clone()), ("views/home.brain", vec!["views/_layout.brain".to_string()]));
        let r = resolve(&d, "/users/42").unwrap();
        assert_eq!(r.view, "views/users/[id].brain");
        assert_eq!(r.params, vec![("id".to_string(), "42".to_string())]);
        assert_eq!(r.layouts, vec!["views/_layout.brain".to_string(), "views/users/_layout.brain".to_string()]);
        assert_eq!(resolve(&d, "/users/new").unwrap().view, "views/users/new.brain", "static wins");
        assert_eq!(resolve(&d, "/users").unwrap().view, "views/users/index.brain");
        let r = resolve(&d, "/posts/hello-world/comments").unwrap();
        assert_eq!((r.view.as_str(), r.params[0].1.as_str()), ("views/posts/[slug]/comments.brain", "hello-world"));
        assert_eq!(resolve(&d, "/posts/x").unwrap().view, "views/posts/[slug]/index.brain");
        let r = resolve(&d, "/does/not-exist").unwrap();
        assert!(r.not_found && r.view == "views/404.brain");
        assert!(resolve(&d, "/users/_layout").unwrap().not_found, "_ entries are not routes");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
