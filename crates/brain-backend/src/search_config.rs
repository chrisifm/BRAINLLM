//! Text-search settings per resource (B-14), declared under `search` in `query.jsbrain`:
//! `{ language: 'spanish', weights: { title: 'A', summary: 'B', body: 'C' } }`.
//! The language picks a built-in Postgres text-search configuration (stemming and stop words); weights rank matches
//! in some fields above others. Both travel to the generic search trigger as arguments (`=spanish`, `title:A`), so a
//! resource without settings keeps exactly the arguments, index contents and ranking it had before.
use crate::Resource;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Built-in configurations shipped with every Postgres 13+ server. Only these names reach SQL.
pub const LANGUAGES: &[&str] = &[
    "simple", "arabic", "armenian", "basque", "catalan", "danish", "dutch", "english", "finnish", "french", "german",
    "greek", "hindi", "hungarian", "indonesian", "irish", "italian", "lithuanian", "nepali", "norwegian", "portuguese",
    "romanian", "russian", "serbian", "spanish", "swedish", "tamil", "turkish", "yiddish",
];

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextSearch {
    /// Postgres text-search configuration; default `simple` (no stemming, no stop words).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Searchable field → weight `A` (highest) .. `D`. Unlisted fields keep the default weights.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub weights: BTreeMap<String, String>,
}

fn settings(r: &Resource) -> Option<&TextSearch> {
    r.query.search.as_ref()
}

/// The configuration name for `websearch_to_tsquery`/`to_tsvector`; always one of [`LANGUAGES`].
pub fn regconfig(r: &Resource) -> &'static str {
    let wanted = settings(r).and_then(|s| s.language.as_deref()).unwrap_or("simple");
    LANGUAGES.iter().copied().find(|l| *l == wanted).unwrap_or("simple")
}

/// Arguments of the `_brain_search` trigger, in order. Without settings: exactly the searchable field names.
pub fn trigger_args(r: &Resource) -> Vec<String> {
    let mut args = Vec::new();
    let language = regconfig(r);
    if language != "simple" {
        args.push(format!("={language}"));
    }
    let weights = settings(r).map(|s| &s.weights);
    for field in &r.search {
        match weights.and_then(|w| w.get(field)) {
            Some(weight) => args.push(format!("{field}:{weight}")),
            None => args.push(field.clone()),
        }
    }
    args
}

/// `pg_trigger.tgargs` bytes for [`trigger_args`] (each argument NUL-terminated).
pub fn trigger_args_bytes(r: &Resource) -> Vec<u8> {
    trigger_args(r).iter().flat_map(|a| a.as_bytes().iter().copied().chain([0u8])).collect()
}

pub fn validate(r: &Resource) -> Result<(), String> {
    let Some(s) = settings(r) else { return Ok(()) };
    if let Some(language) = &s.language {
        if !LANGUAGES.contains(&language.as_str()) {
            return Err(format!("{}: query search.language `{language}` is not a built-in text-search configuration; fix: use one of {}", r.name, LANGUAGES.join(", ")));
        }
    }
    for (field, weight) in &s.weights {
        if !r.search.contains(field) {
            return Err(format!("{}: query search.weights `{field}` is not a searchable field; fix: list it in search.jsbrain first", r.name));
        }
        if !["A", "B", "C", "D"].contains(&weight.as_str()) {
            return Err(format!("{}: query search.weights `{field}` must be A, B, C or D (A ranks highest)", r.name));
        }
    }
    Ok(())
}
