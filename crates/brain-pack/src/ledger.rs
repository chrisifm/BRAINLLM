//! Byte accounting from final emitted assets, independent of source-file gzip estimates.
use crate::{cache, navigation, prerender_json, specifiers, tag_end};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bytes {
    pub raw: usize,
    pub gzip: usize,
    pub brotli: usize,
}
impl Bytes {
    fn add(&mut self, bytes: &Self) {
        self.raw += bytes.raw;
        self.gzip += bytes.gzip;
        self.brotli += bytes.brotli;
    }
    fn json(&self) -> String {
        format!(
            "{{\"raw\":{},\"gzip\":{},\"brotli\":{}}}",
            self.raw, self.gzip, self.brotli
        )
    }
}
#[derive(Debug, Clone)]
pub struct Asset {
    pub path: String,
    pub source: Option<String>,
    pub kind: &'static str,
    pub initial: bool,
    pub bytes: Bytes,
}
#[derive(Debug, Clone, Default)]
pub struct PageLedger {
    pub route: String,
    pub html: Bytes,
    pub inline_js_bytes: usize,
    pub entry_js: Bytes,
    pub deferred_js: Bytes,
    pub conservative_js: Bytes,
    pub assets: Vec<Asset>,
}
impl PageLedger {
    pub fn to_json(&self) -> String {
        let assets = self
            .assets
            .iter()
            .map(|a| {
                format!(
                    "{{\"path\":{},\"source\":{},\"kind\":{},\"initial\":{},\"bytes\":{}}}",
                    prerender_json(&a.path),
                    a.source
                        .as_ref()
                        .map(|s| prerender_json(s))
                        .unwrap_or_else(|| "null".into()),
                    prerender_json(a.kind),
                    a.initial,
                    a.bytes.json()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"schemaVersion\":1,\"route\":{},\"scope\":\"emitted-asset-graph; not measured network\",\"html\":{},\"inlineJsBytes\":{},\"entryJs\":{},\"deferredJs\":{},\"conservativeJs\":{},\"assets\":[{}]}}",
            prerender_json(&self.route),
            self.html.json(),
            self.inline_js_bytes,
            self.entry_js.json(),
            self.deferred_js.json(),
            self.conservative_js.json(),
            assets
        )
    }
}
pub(crate) fn javascript(path: &str) -> bool {
    path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with(".jsbrain")
}
fn sizes(path: &str, content: &[u8], binary: &BTreeMap<String, Vec<u8>>) -> Bytes {
    Bytes {
        raw: content.len(),
        gzip: crate::transfer_size(
            content,
            binary.get(&format!("{path}.gz")).map(Vec::as_slice),
        ),
        brotli: crate::transfer_size(
            content,
            binary.get(&format!("{path}.br")).map(Vec::as_slice),
        ),
    }
}
fn key(spec: &str, base: &str) -> Option<String> {
    let path = spec
        .strip_prefix(base.trim_end_matches('/'))
        .unwrap_or(spec);
    path.starts_with("/_b/")
        .then(|| path.trim_start_matches('/').to_string())
}
fn references(text: &str, base: &str) -> Vec<String> {
    text.match_indices("/_b/")
        .map(|(at, _)| {
            text[at + 1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
                .collect::<String>()
        })
        .filter_map(|p| key(&format!("/{p}"), base))
        .collect()
}
// Both the legacy conservative report and the detailed ledger use indices from this stripped string.
pub(crate) fn html_asset_refs(html: &str, base: &str) -> Vec<String> {
    references(&navigation::without_plan(html), base)
}

// Static imports/reexports only. Dynamic imports and worker URLs belong to deferred graph accounting.
fn static_refs(source: &str, base: &str) -> Result<Vec<String>, String> {
    let tokens = brain_min::lex::lex(source)?;
    Ok(tokens
        .windows(2)
        .filter_map(|pair| {
            (pair[1].kind == brain_min::lex::Kind::Str
                && (pair[0].ident("from") || pair[0].ident("import")))
            .then(|| key(&pair[1].text[1..pair[1].text.len() - 1], base))
            .flatten()
        })
        .collect())
}
fn initial_refs(html: &str, base: &str) -> Result<BTreeSet<String>, String> {
    let mut seeds = BTreeSet::new();
    let mut cursor = 0;
    while let Some(at) = html[cursor..].find('<') {
        let start = cursor + at;
        let Some(end) = tag_end(html, start) else {
            break;
        };
        let tag = &html[start..end];
        cursor = end;
        if tag.starts_with("<link") {
            let rel = cache::attr(tag, "rel").unwrap_or_default();
            if rel
                .split_whitespace()
                .any(|v| matches!(v, "stylesheet" | "modulepreload" | "preload"))
            {
                if let Some(href) = cache::attr(tag, "href").and_then(|href| key(&href, base)) {
                    seeds.insert(href);
                }
            }
        } else if tag.starts_with("<script") {
            if let Some(path) = cache::attr(tag, "src").and_then(|src| key(&src, base)) {
                seeds.insert(path);
            }
            if let Some(close) = html[cursor..].find("</script") {
                cursor += close;
            }
        } else if tag.starts_with("<style") {
            if let Some(close) = html[cursor..].find("</style") {
                cursor += close;
            }
        }
    }
    for script in cache::inline_scripts(html) {
        seeds.extend(static_refs(&script, base)?);
    }
    Ok(seeds)
}
pub fn page(
    route: &str,
    file: &str,
    html: &str,
    base: &str,
    files: &BTreeMap<String, String>,
    binary: &BTreeMap<String, Vec<u8>>,
    sources: &HashMap<String, String>,
) -> Result<PageLedger, String> {
    let clean = navigation::without_plan(html);
    let mut initial = initial_refs(&clean, base)?;
    let mut queue: Vec<_> = initial.iter().cloned().collect();
    while let Some(path) = queue.pop() {
        if let Some(source) = files.get(&path).filter(|_| javascript(&path)) {
            for dependency in static_refs(source, base)? {
                if initial.insert(dependency.clone()) {
                    queue.push(dependency);
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut graph = html_asset_refs(html, base);
    graph.extend(initial.iter().cloned());
    while let Some(path) = graph.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(source) = files.get(&path) else {
            continue;
        };
        if javascript(&path) {
            for (_, _, spec) in specifiers(source)? {
                if let Some(path) = key(&spec, base) {
                    graph.push(path);
                }
            }
        }
    }
    let mut result = PageLedger {
        route: route.into(),
        html: sizes(file, html.as_bytes(), binary),
        inline_js_bytes: cache::inline_scripts(html).iter().map(String::len).sum(),
        ..Default::default()
    };
    for path in seen {
        let Some(source) = files.get(&path) else {
            continue;
        };
        let kind = if javascript(&path) {
            "js"
        } else if path.ends_with(".css") || path.ends_with(".cssbrain") {
            "css"
        } else {
            continue;
        };
        let bytes = sizes(&path, source.as_bytes(), binary);
        let is_initial = initial.contains(&path);
        if kind == "js" {
            result.conservative_js.add(&bytes);
            if is_initial {
                result.entry_js.add(&bytes)
            } else {
                result.deferred_js.add(&bytes)
            }
        }
        result.assets.push(Asset {
            source: sources.get(&path).cloned(),
            path,
            kind,
            initial: is_initial,
            bytes,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mjs_payloads_and_their_static_dependencies_are_counted_as_javascript() {
        let files: BTreeMap<String, String> = [
            ("_b/entry.js", "import {value} from '/_b/sdk.mjs';value();"),
            ("_b/sdk.mjs", "export {value} from '/_b/dep.mjs';"),
            ("_b/dep.mjs", "export const value=42;"),
        ]
        .into_iter()
        .map(|(path, code)| (path.into(), code.into()))
        .collect();
        let page = page(
            "/",
            "index.html",
            "<script type='module' src='/_b/entry.js'></script>",
            "/",
            &files,
            &BTreeMap::new(),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(page.assets.len(), 3);
        assert!(
            page.assets
                .iter()
                .all(|asset| asset.kind == "js" && asset.initial)
        );
        assert_eq!(
            page.conservative_js.raw,
            files.values().map(String::len).sum::<usize>()
        );
        assert_eq!(page.entry_js.raw, page.conservative_js.raw);
    }
}
