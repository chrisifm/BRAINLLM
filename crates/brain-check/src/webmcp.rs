//! WebMCP across a page: what one file cannot see. A view, its layouts and its components register together.
//! - B1709 the same tool name registered by two files of one page (the browser refuses the second).
//! - B1708 coverage per route, as advice from `brain doctor`.
use crate::tools;
use std::path::Path;

/// `(file, tool, other file, view)` for every name two files of one page register, once per pair.
pub fn duplicates(root: &Path) -> Vec<(String, String, String, String)> {
    let (pages, components) = crate::doctor::pages(root);
    let mut out: Vec<(String, String, String, String)> = Vec::new();
    for (view, files) in pages {
        let mut seen: Vec<(String, &str)> = Vec::new();
        for file in &files {
            let Some(c) = components.get(file) else { continue };
            for name in tools::names(c) {
                match seen.iter().find(|(tool, owner)| *tool == name && *owner != file) {
                    Some((_, owner)) => {
                        if !out.iter().any(|d| d.0 == *file && d.1 == name && d.2 == *owner) {
                            out.push((file.clone(), name, owner.to_string(), view.clone()));
                        }
                    }
                    None => seen.push((name, file)),
                }
            }
        }
    }
    out
}

/// `(view, exposed, actions)` for each view with at least one form or tool. Sorted by view.
pub fn coverage(root: &Path) -> Vec<(String, usize, usize)> {
    let (pages, components) = crate::doctor::pages(root);
    let mut out = Vec::new();
    for (view, files) in pages {
        let (mut exposed, mut actions) = (0, 0);
        for c in files.iter().filter_map(|f| components.get(f)) {
            let named = tools::names(c).len();
            exposed += named;
            actions += named + tools::forms(&c.template).iter().filter(|e| !e.attrs.iter().any(|a| a.name == "tool")).count();
        }
        if actions > 0 {
            out.push((view, exposed, actions));
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_name_two_files_of_one_page_register_is_a_duplicate_and_coverage_counts_the_whole_page() {
        let root = std::env::temp_dir().join(format!("brain-webmcp-{}", std::process::id()));
        let write = |rel: &str, src: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, src).unwrap();
        };
        let tool = "<script>const add_task = ({ text }) => text</script>\n<meta tools=\"add_task: Add a task to the list (text:str!)\">\n<p>x</p>\n";
        write("comp/one.brain", tool);
        write("comp/two.brain", tool);
        write("views/both.brain", "<script>import One from '../comp/one.brain'\nimport Two from '../comp/two.brain'</script>\n<One /><Two />\n<form><input aria-label=\"q\" /></form>\n");
        write("views/single.brain", "<script>import One from '../comp/one.brain'</script>\n<One />\n");
        write("views/none.brain", "<p>x</p>\n");
        let found = duplicates(&root);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].1.as_str(), found[0].3.as_str()), ("add_task", "views/both.brain"));
        write("comp/one.brain", tool);
        write("views/single.brain", "<script>import One from '../comp/one.brain'</script>\n<One />\n<form><input aria-label=\"q\" /></form>\n");
        write("views/none.brain", "<p>x</p>\n");
        let counted = coverage(&root);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(counted, [("views/single.brain".to_string(), 1, 2)]);
    }
}
