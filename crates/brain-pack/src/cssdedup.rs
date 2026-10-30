//! Generated-CSS interning shares exact declaration bodies only inside proven equivalent adjacent contexts.
pub use brain_css::intern::{Change, Report};
pub fn dedupe(css: &str) -> (String, Report) {
    brain_css::intern::dedupe(css)
}
