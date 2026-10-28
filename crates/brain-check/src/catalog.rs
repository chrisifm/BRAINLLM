//! Every diagnostic the compiler can emit, for `brain explain <code>` and the MCP tool `brain_explain`
//! (INTERNALS §25): when an agent repeats the same error, it gets `example_ok` instead of the same message.

/// `(code, title, why, example_ok)` — a view of the single table in `brain_lex::codes::CODES` (which also holds the
/// default level), sorted by code.
pub static CATALOG: std::sync::LazyLock<Vec<(&'static str, &'static str, &'static str, &'static str)>> =
    std::sync::LazyLock::new(|| brain_parse::codes::CODES.iter().map(|r| (r.0, r.2, r.3, r.4)).collect());

/// The row of one code.
pub fn explain(code: &str) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    CATALOG.iter().find(|r| r.0 == code).copied()
}
