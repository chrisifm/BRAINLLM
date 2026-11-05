//! Compiler-owned server capabilities carried through build and development pipelines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerMetadata {
    pub load: bool,
    pub paths: bool,
    pub sitemap: bool,
    pub forms: bool,
    /// Exact RPC IDs emitted by codegen, never inferred from source strings.
    pub rpc: Vec<String>,
}
