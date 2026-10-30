//! Bounded literal-free feedback accompanies optimized output, including cache hits.
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Optimization {
    pub source: String,
    pub original_source_hash: Option<String>,
    pub stage: &'static str,
    pub detail: String,
}
fn bounded(mut report: brain_min::CleanupReport) -> String {
    if report.facts.len() > 64 {
        report.omitted_facts += report.facts.len() - 64;
        report.facts.truncate(64);
        report.facts_truncated = true;
    }
    report.to_json()
}
pub(crate) fn minify(root: &Path, source: &str) -> Result<(String, String), String> {
    let bytes = crate::cached_bytes(root, "min-feedback-v1", source.as_bytes(), || {
        let result = brain_min::minify_module_with_report(source, true).map_err(|e| e.to_json())?;
        let mut bytes = (result.code.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(result.code.as_bytes());
        bytes.extend_from_slice(bounded(result.report).as_bytes());
        Ok(bytes)
    })?;
    if bytes.len() < 8 { return Err("invalid optimization cache frame; remove the owned build cache and rebuild".into()); }
    let size = usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap())).map_err(|_| "optimization cache length overflow")?;
    let end = 8usize.checked_add(size).filter(|end|*end < bytes.len()).ok_or("invalid optimization cache boundary")?;
    let code = String::from_utf8(bytes[8..end].to_vec()).map_err(|_| "optimization cache code is not UTF-8")?;
    let report = String::from_utf8(bytes[end..].to_vec()).map_err(|_| "optimization cache feedback is not UTF-8")?;
    Ok((code, report))
}
pub(crate) fn shake(source: &str, keep: &std::collections::HashSet<String>) -> Result<(String, String), String> {
    let result = brain_min::tree_shake_with_report(source, keep).map_err(|e| e.to_json())?;
    Ok((result.code, bounded(result.report)))
}
