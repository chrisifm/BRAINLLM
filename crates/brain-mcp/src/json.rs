//! The workspace JSON type lives in brain-lex (one serializer for diagnostics, MCP, SEO and build); brain-mcp
//! re-exports it so `brain_mcp::Json` keeps working.

pub use brain_lex::json::*;
