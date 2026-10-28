//! Content collections (INTERNALS §33): `content/<collection>/*.md` with frontmatter → typed entries for the
//! `views/<collection>/[slug].brain` template, tag and pagination listings, RSS 2.0 and Atom feeds, sitemap dates and
//! `BlogPosting` JSON-LD. Entry URLs: `/<collection>/<slug>`, tags `/<collection>/tag/<tag>`, pages
//! `/<collection>/page/<n>` (page 1 is `/<collection>`). Multilingual sites keep one folder per locale
//! (`content/blog/es/*.md`, URLs under `/es/blog/…`); an entry missing in a locale is filled from the default locale
//! with `fallback: true` (B1829 notice; the page is `noindex` and stays out of the sitemap), never a 404.
//!
//! ```text
//! ---
//! title: Offline first
//! description: Why the planner works without network.
//! date: 2026-10-01
//! updated: 2026-10-05
//! tags: [sync, offline]
//! cover: /covers/offline.png
//! author: Ada
//! draft: false
//! ---
//! Markdown body…
//! ```

use crate::config::Config;
use crate::files::xml;
use crate::markdown;
use brain_lex::Json;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Bool(bool),
    List(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub collection: String,
    pub slug: String,
    /// `content/<collection>/<file>.md`
    pub file: String,
    pub title: String,
    pub description: Option<String>,
    pub date: Option<String>,
    pub updated: Option<String>,
    pub tags: Vec<String>,
    pub cover: Option<String>,
    pub author: Option<String>,
    pub draft: bool,
    /// Every other frontmatter key, as written.
    pub fields: Vec<(String, Value)>,
    pub html: String,
    pub headings: Vec<(u8, String, String)>,
    pub words: usize,
    /// Minutes at 200 words per minute, at least 1.
    pub reading_minutes: usize,
    pub excerpt: String,
    /// Locale of the text (multilingual collections only).
    pub locale: Option<String>,
    /// Shown in another locale because this one has no translation yet.
    pub fallback: bool,
    /// URL prefix of the locale (`""` for the default, `/es` otherwise).
    pub prefix: String,
}

impl Entry {
    pub fn route(&self) -> String {
        format!("{}/{}/{}", self.prefix, self.collection, self.slug)
    }
    /// `description`, else the first paragraph cut at 160 characters.
    pub fn summary(&self) -> String {
        self.description
            .clone()
            .unwrap_or_else(|| cut(&self.excerpt, 160))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Collection {
    pub name: String,
    /// `Some` in multilingual collections (one `Collection` per locale).
    pub locale: Option<String>,
    /// URL prefix of the locale (`""` or `/es`).
    pub prefix: String,
    /// Newest first (by `date`, then slug).
    pub entries: Vec<Entry>,
}

/// One problem in a content file: `(file, line, message, fix)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub code: &'static str,
    /// Errors fail the build; warnings are reported.
    pub error: bool,
    pub file: String,
    pub line: usize,
    pub msg: String,
    pub fix: String,
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    if let Some(sp) = out.rfind(' ') {
        out.truncate(sp);
    }
    format!("{}\u{2026}", out.trim_end_matches([',', '.', ';', ':']))
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')))
    {
        let inner = &v[1..v.len() - 1];
        if v.starts_with('"') {
            inner.replace("\\\"", "\"").replace("\\n", "\n")
        } else {
            inner.replace("''", "'")
        }
    } else {
        v.to_string()
    }
}

/// A file split into frontmatter fields and Markdown body.
#[derive(Debug, Clone, PartialEq)]
pub struct Front {
    pub fields: Vec<(String, Value)>,
    pub body: String,
}

/// 1-based line of `key:` in the frontmatter of `src` (the opening `---` is line 1), else 1: diagnostics point at
/// the field that is wrong.
pub fn field_line(src: &str, key: &str) -> usize {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    if !src.starts_with("---") {
        return 1;
    }
    src.lines()
        .enumerate()
        .skip(1)
        .take_while(|(_, l)| l.trim_end() != "---")
        .find(|(_, l)| {
            !l.starts_with([' ', '\t', '-'])
                && l.split_once(':').is_some_and(|(k, _)| k.trim() == key)
        })
        .map_or(1, |(n, _)| n + 1)
}

/// Frontmatter (YAML subset: `key: value`, quoted strings, `true`/`false`, `[a, b]` and `- item` lists, `#` comments)
/// and the body after it. `Err((line, message))`.
pub fn frontmatter(src: &str) -> Result<Front, (usize, String)> {
    let src = src
        .strip_prefix('\u{feff}')
        .unwrap_or(src)
        .replace("\r\n", "\n");
    let Some(rest) = src.strip_prefix("---\n") else {
        return Ok(Front {
            fields: vec![],
            body: src,
        });
    };
    let Some(end) = rest
        .find("\n---\n")
        .or_else(|| rest.strip_suffix("\n---").map(|r| r.len()))
    else {
        return Err((1, "frontmatter is not closed".into()));
    };
    let head = &rest[..end];
    let body = rest.get(end + 5..).unwrap_or("").to_string();
    let mut out: Vec<(String, Value)> = Vec::new();
    for (n, line) in head.lines().enumerate() {
        let ln = n + 2;
        let t = line.trim_end();
        if t.trim().is_empty() || t.trim_start().starts_with('#') {
            continue;
        }
        if let Some(item) = t.trim_start().strip_prefix("- ") {
            match out.last_mut() {
                Some((_, Value::List(l))) if line.starts_with([' ', '-']) => l.push(unquote(item)),
                _ => {
                    return Err((
                        ln,
                        format!("list item `{}` has no key above it", item.trim()),
                    ));
                }
            }
            continue;
        }
        let Some((k, v)) = t.split_once(':') else {
            return Err((ln, format!("`{}` is not `key: value`", t.trim())));
        };
        let key = k.trim().to_string();
        if key.is_empty() || key.contains(' ') || line.starts_with(' ') {
            return Err((
                ln,
                format!("`{}` is not a top-level `key: value`", t.trim()),
            ));
        }
        let v = v.trim();
        let value = if v.is_empty() {
            Value::List(vec![])
        } else if let Some(inner) = v.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
            Value::List(
                inner
                    .split(',')
                    .map(unquote)
                    .filter(|x| !x.is_empty())
                    .collect(),
            )
        } else if v == "true" || v == "false" {
            Value::Bool(v == "true")
        } else {
            Value::Str(unquote(v.split(" #").next().unwrap_or(v)))
        };
        if out.iter().any(|(k2, _)| *k2 == key) {
            return Err((ln, format!("`{key}` appears twice")));
        }
        out.push((key, value));
    }
    Ok(Front { fields: out, body })
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    let ymd = b.len() >= 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..10]
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    ymd && s[5..7].parse::<u32>().is_ok_and(|m| (1..=12).contains(&m))
        && s[8..10].parse::<u32>().is_ok_and(|d| (1..=31).contains(&d))
        && (b.len() == 10 || b[10] == b'T')
}

/// Parses one entry file. Problems are returned alongside a best-effort entry (`None` if unusable).
pub fn parse_entry(collection: &str, file_name: &str, src: &str) -> (Option<Entry>, Vec<Problem>) {
    parse_entry_at(
        collection,
        &format!("content/{collection}/{file_name}"),
        file_name,
        src,
    )
}

/// [`parse_entry`] with the file's path given (locale folders).
pub fn parse_entry_at(
    collection: &str,
    file: &str,
    file_name: &str,
    src: &str,
) -> (Option<Entry>, Vec<Problem>) {
    let file = file.to_string();
    let mut probs = Vec::new();
    let mut p = |code: &'static str, line: usize, msg: String, fix: String| {
        let error = !msg.starts_with("entry has no description");
        probs.push(Problem {
            code,
            error,
            file: file.clone(),
            line,
            msg,
            fix,
        })
    };
    let Front { fields: fm, body } = match frontmatter(src) {
        Ok(x) => x,
        Err((line, msg)) => {
            p(
                "B1823",
                line,
                format!("frontmatter: {msg}"),
                "write `key: value` lines between two `---` lines".into(),
            );
            return (None, probs);
        }
    };
    let get = |k: &str| fm.iter().find(|(a, _)| a == k).map(|(_, v)| v);
    let text = |k: &str| match get(k) {
        Some(Value::Str(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };
    let stem = file_name.strip_suffix(".md").unwrap_or(file_name);
    let slug = text("slug").unwrap_or_else(|| markdown::slug(stem));
    if slug.is_empty() || slug != markdown::slug(&slug) {
        p(
            "B1823",
            field_line(src, "slug"),
            format!("slug `{slug}` is not lowercase words joined by `-`"),
            format!("rename the file or set `slug: {}`", markdown::slug(stem)),
        );
    }
    let mut r = markdown::render(&body);
    // the template renders the title as the page's <h1>; a leading `# Title` in the body would make a second one
    let lead_h1 = r
        .headings
        .first()
        .filter(|h| h.0 == 1)
        .map(|h| (h.1.clone(), h.2.clone()));
    if let Some((_, id)) = &lead_h1
        && let Some(rest) = r
            .html
            .strip_prefix(&format!("<h1 id=\"{id}\">"))
            .and_then(|x| {
                x.split_once("</h1>\n")
                    .map(|(_, b)| b.to_string())
                    .or_else(|| x.split_once("</h1>").map(|(_, b)| b.to_string()))
            })
    {
        r.html = rest;
        r.headings.remove(0);
    }
    let Some(title) = text("title").or_else(|| lead_h1.as_ref().map(|h| h.0.clone())) else {
        p(
            "B1823",
            1,
            "entry has no title".into(),
            "add `title: …` to the frontmatter".into(),
        );
        return (None, probs);
    };
    for k in ["date", "updated"] {
        if let Some(d) = text(k)
            && !is_date(&d)
        {
            p(
                "B1823",
                field_line(src, k),
                format!("`{k}: {d}` is not a date"),
                format!("write `{k}: YYYY-MM-DD`"),
            );
        }
    }
    let tags = match get("tags") {
        Some(Value::List(l)) => l.clone(),
        Some(Value::Str(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => vec![],
    };
    let known = [
        "title",
        "description",
        "date",
        "updated",
        "tags",
        "cover",
        "author",
        "draft",
        "slug",
    ];
    let words = r.text.split_whitespace().count();
    let entry = Entry {
        collection: collection.into(),
        slug,
        file: file.clone(),
        title,
        description: text("description"),
        date: text("date").filter(|d| is_date(d)),
        updated: text("updated").filter(|d| is_date(d)),
        tags,
        cover: text("cover").or_else(|| r.first_image.clone()),
        author: text("author"),
        draft: matches!(get("draft"), Some(Value::Bool(true))),
        fields: fm
            .iter()
            .filter(|(k, _)| !known.contains(&k.as_str()))
            .cloned()
            .collect(),
        html: r.html,
        headings: r.headings,
        words,
        reading_minutes: words.div_ceil(200).max(1),
        excerpt: r.excerpt,
        locale: None,
        fallback: false,
        prefix: String::new(),
    };
    if entry.description.is_none() {
        p(
            "B1823",
            1,
            "entry has no description; the first paragraph is used".into(),
            "add `description: …` (70–155 characters)".into(),
        );
    }
    (Some(entry), probs)
}

/// Entries of one folder (`content/blog` or `content/blog/es`), sorted newest first, duplicates reported.
fn read_dir_entries(
    root: &Path,
    col: &str,
    sub: Option<&str>,
    drafts: bool,
    probs: &mut Vec<Problem>,
) -> Vec<Entry> {
    let rel = match sub {
        Some(l) => format!("content/{col}/{l}"),
        None => format!("content/{col}"),
    };
    let mut files: Vec<_> = std::fs::read_dir(root.join(&rel))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort_by_key(|e| e.file_name());
    let mut entries: Vec<Entry> = Vec::new();
    for f in files {
        let fname = f.file_name().to_string_lossy().to_string();
        let Ok(src) = std::fs::read_to_string(f.path()) else {
            continue;
        };
        let (e, p) = parse_entry_at(col, &format!("{rel}/{fname}"), &fname, &src);
        probs.extend(p);
        if let Some(mut e) = e {
            e.locale = sub.map(str::to_string);
            if let Some(other) = entries.iter().find(|o| o.slug == e.slug) {
                probs.push(Problem {
                    code: "B1824",
                    error: true,
                    file: e.file.clone(),
                    line: field_line(&src, "slug"),
                    msg: format!("slug `{}` is also used by {}", e.slug, other.file),
                    fix: "rename one file or set a different `slug:`".into(),
                });
                continue;
            }
            if !e.draft || drafts {
                entries.push(e);
            }
        }
    }
    entries.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.slug.cmp(&b.slug)));
    entries
}

/// Reads every collection under `<root>/content/`. Drafts are left out unless `drafts`. With `i18n`, a collection
/// whose subfolders are locales becomes one [`Collection`] per locale, missing entries filled from the default.
pub fn load(root: &Path, drafts: bool) -> (Vec<Collection>, Vec<Problem>) {
    load_i18n(root, drafts, None)
}

pub fn load_i18n(
    root: &Path,
    drafts: bool,
    i18n: Option<&crate::hreflang::I18n>,
) -> (Vec<Collection>, Vec<Problem>) {
    let mut cols = Vec::new();
    let mut probs = Vec::new();
    let Ok(rd) = std::fs::read_dir(root.join("content")) else {
        return (cols, probs);
    };
    let mut dirs: Vec<_> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .collect();
    dirs.sort_by_key(|e| e.file_name());
    for d in dirs {
        let name = d.file_name().to_string_lossy().to_string();
        if name.starts_with(['.', '_']) {
            continue;
        }
        let localized = i18n.filter(|i| i.locales.iter().any(|l| d.path().join(l).is_dir()));
        let Some(i) = localized else {
            let entries = read_dir_entries(root, &name, None, drafts, &mut probs);
            cols.push(Collection {
                name,
                locale: None,
                prefix: String::new(),
                entries,
            });
            continue;
        };
        let default = read_dir_entries(root, &name, Some(&i.default), drafts, &mut probs);
        for l in &i.locales {
            let prefix = if *l == i.default {
                String::new()
            } else {
                format!("/{l}")
            };
            let mut entries = if *l == i.default {
                default.clone()
            } else {
                read_dir_entries(root, &name, Some(l), drafts, &mut probs)
            };
            if *l != i.default {
                for e in &default {
                    if !entries.iter().any(|x| x.slug == e.slug) {
                        probs.push(Problem {
                            code: "B1829",
                            error: false,
                            file: format!("content/{name}/{l}/{}.md", e.slug),
                            line: 1,
                            msg: format!("`{slug}` has no {l} version; {prefix}/{name}/{slug} shows the {} text (noindex)", i.default, slug = e.slug),
                            fix: format!("translate {} into content/{name}/{l}/", e.file),
                        });
                        entries.push(Entry {
                            fallback: true,
                            ..e.clone()
                        });
                    }
                }
                entries.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.slug.cmp(&b.slug)));
            }
            for e in &mut entries {
                e.prefix = prefix.clone();
            }
            cols.push(Collection {
                name: name.clone(),
                locale: Some(l.clone()),
                prefix,
                entries,
            });
        }
    }
    (cols, probs)
}

/// Entry rendered at `route` (`/blog/offline-first`, `/es/blog/offline-first`), if any.
pub fn entry_at<'a>(cols: &'a [Collection], route: &str) -> Option<&'a Entry> {
    let r = crate::audit::norm(route);
    cols.iter()
        .flat_map(|c| &c.entries)
        .find(|e| e.route() == r)
}

/// `route → entry` for every entry (fast lookups on big sites).
pub fn index(cols: &[Collection]) -> std::collections::HashMap<String, &Entry> {
    cols.iter()
        .flat_map(|c| &c.entries)
        .map(|e| (e.route(), e))
        .collect()
}

/// `(tag, slug, count)` of a collection, most used first.
pub fn tags(c: &Collection) -> Vec<(String, String, usize)> {
    let mut v: Vec<(String, String, usize)> = Vec::new();
    for t in c.entries.iter().flat_map(|e| &e.tags) {
        match v.iter_mut().find(|(n, _, _)| n.eq_ignore_ascii_case(t)) {
            Some(x) => x.2 += 1,
            None => v.push((t.clone(), markdown::slug(t), 1)),
        }
    }
    v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
    v
}

/// Route of listing page `n` (1-based) of a collection (`col` may carry a locale prefix: `es/blog`).
pub fn page_route(col: &str, n: usize) -> String {
    if n <= 1 {
        format!("/{col}")
    } else {
        format!("/{col}/page/{n}")
    }
}

/// `(prev, next)` routes of listing page `n` when the collection has `total` entries.
pub fn page_links(
    col: &str,
    n: usize,
    total: usize,
    per_page: usize,
) -> (Option<String>, Option<String>) {
    let pages = total.div_ceil(per_page.max(1)).max(1);
    (
        (n > 1).then(|| page_route(col, n - 1)),
        (n < pages).then(|| page_route(col, n + 1)),
    )
}

impl Collection {
    /// `blog` or `es/blog`: the collection's path without the leading slash.
    pub fn base(&self) -> String {
        format!("{}/{}", self.prefix.trim_start_matches('/'), self.name)
            .trim_start_matches('/')
            .to_string()
    }
}

/// `brain/content` data for `paths()`/`load()`: every collection with its entries (HTML included), tags and the
/// page size. Deterministic.
pub fn to_json(cols: &[Collection], per_page: usize) -> Json {
    let entry = |e: &Entry| {
        let mut kv = vec![
            ("slug".to_string(), Json::str(&e.slug)),
            ("url".into(), Json::str(e.route())),
            ("title".into(), Json::str(&e.title)),
            ("description".into(), Json::str(e.summary())),
            ("date".into(), e.date.as_ref().map_or(Json::Null, Json::str)),
            (
                "updated".into(),
                e.updated.as_ref().map_or(Json::Null, Json::str),
            ),
            (
                "tags".into(),
                Json::Arr(e.tags.iter().map(Json::str).collect()),
            ),
            (
                "cover".into(),
                e.cover.as_ref().map_or(Json::Null, Json::str),
            ),
            (
                "author".into(),
                e.author.as_ref().map_or(Json::Null, Json::str),
            ),
            ("readingMinutes".into(), Json::Num(e.reading_minutes as f64)),
            (
                "headings".into(),
                Json::Arr(
                    e.headings
                        .iter()
                        .map(|(l, t, id)| {
                            Json::obj([
                                ("level", Json::Num(f64::from(*l))),
                                ("text", Json::str(t)),
                                ("id", Json::str(id)),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("html".into(), Json::str(&e.html)),
        ];
        if let Some(l) = &e.locale {
            kv.push(("locale".into(), Json::str(l)));
            kv.push(("fallback".into(), Json::Bool(e.fallback)));
        }
        for (k, v) in &e.fields {
            let j = match v {
                Value::Str(s) => Json::str(s),
                Value::Bool(b) => Json::Bool(*b),
                Value::List(l) => Json::Arr(l.iter().map(Json::str).collect()),
            };
            kv.push((k.clone(), j));
        }
        Json::Obj(kv)
    };
    let body = |c: &Collection| -> Vec<(String, Json)> {
        vec![
            (
                "entries".into(),
                Json::Arr(c.entries.iter().map(entry).collect()),
            ),
            (
                "tags".into(),
                Json::Arr(
                    tags(c)
                        .iter()
                        .map(|(t, s, n)| {
                            Json::obj([
                                ("tag", Json::str(t)),
                                ("slug", Json::str(s)),
                                ("count", Json::Num(*n as f64)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]
    };
    let mut out: Vec<(String, Json)> = Vec::new();
    for c in cols.iter().filter(|c| c.prefix.is_empty()) {
        let mut kv = vec![("perPage".to_string(), Json::Num(per_page as f64))];
        kv.extend(body(c));
        let others: Vec<(String, Json)> = cols
            .iter()
            .filter(|o| o.name == c.name && !o.prefix.is_empty())
            .filter_map(|o| Some((o.locale.clone()?, Json::Obj(body(o)))))
            .collect();
        if !others.is_empty() {
            kv.push(("locales".into(), Json::Obj(others)));
        }
        out.push((c.name.clone(), Json::Obj(kv)));
    }
    Json::Obj(out)
}

/// Source of the server-only `brain/content` module that views import in `paths()`/`load()`:
/// `collection(name)`, `entry(name, slug)`, `entries(name)`, `tags(name)`, `tagged(name, tag)`, `pages(name)` (one
/// `{ n }` per listing page from 2 on: page 1 is `/<name>` itself) and `page(name, n)` (`{ entries, n, pages, prev,
/// next }`). Multilingual sites: the toolchain calls `__setLocale(locale)` before running `paths()`/`load()` for each
/// locale, and every function then answers for that locale (untranslated entries come with `fallback: true`).
pub fn module_js(cols: &[Collection], per_page: usize) -> String {
    format!(
        "const C = {};\n\
let L = null;\n\
export const __setLocale = (l) => {{ L = l }};\n\
const col = (name) => {{ const c = C[name]; if (!c) throw new Error(`content/${{name}}/ does not exist; collections: ${{Object.keys(C).join(', ') || 'none'}}`); const v = L && c.locales && c.locales[L]; return v ? {{ perPage: c.perPage, ...v }} : c }};\n\
export const collection = (name) => col(name);\n\
export const entries = (name) => col(name).entries;\n\
export const entry = (name, slug) => col(name).entries.find((e) => e.slug === slug) ?? null;\n\
export const tags = (name) => col(name).tags;\n\
export const tagged = (name, tag) => col(name).entries.filter((e) => e.tags.some((t) => t.toLowerCase() === String(tag).toLowerCase() || t.toLowerCase().replace(/[^a-z0-9]+/g, '-') === tag));\n\
export const pages = (name) => {{ const c = col(name); const n = Math.max(1, Math.ceil(c.entries.length / c.perPage)); return Array.from({{ length: n - 1 }}, (_, i) => ({{ n: String(i + 2) }})) }};\n\
export const page = (name, n) => {{ const c = col(name); const k = Number(n) || 1; const total = Math.max(1, Math.ceil(c.entries.length / c.perPage)); const url = (i) => (i <= 1 ? `/${{name}}` : `/${{name}}/page/${{i}}`); return {{ entries: c.entries.slice((k - 1) * c.perPage, k * c.perPage), n: k, pages: total, prev: k > 1 ? url(k - 1) : null, next: k < total ? url(k + 1) : null }} }};\n",
        to_json(cols, per_page).stringify()
    )
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date (days_from_civil).
fn days(d: &str) -> Option<i64> {
    let (y, m, dd): (i64, i64, i64) = (
        d.get(..4)?.parse().ok()?,
        d.get(5..7)?.parse().ok()?,
        d.get(8..10)?.parse().ok()?,
    );
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// RFC 822 date for RSS (`Wed, 01 Oct 2026 00:00:00 +0000`).
pub fn rfc822(d: &str) -> Option<String> {
    let n = days(d)?;
    let wd = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][n.rem_euclid(7) as usize];
    let mon = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][d[5..7].parse::<usize>().ok()? - 1];
    Some(format!(
        "{wd}, {} {mon} {} 00:00:00 +0000",
        &d[8..10],
        &d[..4]
    ))
}

fn rfc3339(d: &str) -> String {
    if d.len() > 10 {
        d.to_string()
    } else {
        format!("{d}T00:00:00Z")
    }
}

/// Entries per feed (newest first).
pub const FEED_MAX: usize = 50;

/// RSS 2.0 (`<col>/rss.xml`) and Atom (`<col>/atom.xml`) per collection with dated entries. Needs `site`.
pub fn feeds(cfg: &Config, cols: &[Collection], site_name: &str) -> Vec<(String, String)> {
    let Some(site) = &cfg.site else { return vec![] };
    let mut out = Vec::new();
    for c in cols {
        let items: Vec<&Entry> = c
            .entries
            .iter()
            .filter(|e| e.date.is_some() && !e.fallback)
            .take(FEED_MAX)
            .collect();
        if items.is_empty() {
            continue;
        }
        let home = cfg.abs(&format!("/{}", c.base()));
        let title = format!("{site_name} — {}", c.name);
        let newest = items
            .iter()
            .filter_map(|e| e.updated.as_ref().or(e.date.as_ref()))
            .max()
            .cloned()
            .unwrap_or_default();
        let mut rss = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n<channel>\n<title>{}</title>\n<link>{}</link>\n<description>{}</description>\n<atom:link href=\"{}\" rel=\"self\" type=\"application/rss+xml\"/>\n",
            xml(&title),
            xml(&home),
            xml(cfg.description.as_deref().unwrap_or(&title)),
            xml(&format!("{home}/rss.xml"))
        );
        if let Some(l) = c.locale.as_ref().or(cfg.lang.as_ref()) {
            rss.push_str(&format!("<language>{}</language>\n", xml(l)));
        }
        if let Some(d) = rfc822(&newest) {
            rss.push_str(&format!("<lastBuildDate>{d}</lastBuildDate>\n"));
        }
        let mut atom = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\">\n<title>{}</title>\n<id>{}</id>\n<link href=\"{}\"/>\n<link href=\"{}\" rel=\"self\"/>\n<updated>{}</updated>\n",
            xml(&title),
            xml(&home),
            xml(&home),
            xml(&format!("{home}/atom.xml")),
            rfc3339(&newest)
        );
        for e in items {
            let url = cfg.abs(&e.route());
            rss.push_str(&format!("<item>\n<title>{}</title>\n<link>{}</link>\n<guid isPermaLink=\"true\">{}</guid>\n", xml(&e.title), xml(&url), xml(&url)));
            if let Some(d) = e.date.as_deref().and_then(rfc822) {
                rss.push_str(&format!("<pubDate>{d}</pubDate>\n"));
            }
            rss.push_str(&format!(
                "<description>{}</description>\n",
                xml(&e.summary())
            ));
            for t in &e.tags {
                rss.push_str(&format!("<category>{}</category>\n", xml(t)));
            }
            rss.push_str("</item>\n");
            atom.push_str(&format!(
                "<entry>\n<title>{}</title>\n<id>{}</id>\n<link href=\"{}\"/>\n<updated>{}</updated>\n",
                xml(&e.title),
                xml(&url),
                xml(&url),
                rfc3339(e.updated.as_ref().or(e.date.as_ref()).unwrap())
            ));
            if let Some(d) = &e.date {
                atom.push_str(&format!("<published>{}</published>\n", rfc3339(d)));
            }
            if let Some(a) = e.author.as_ref().or(cfg.name.as_ref()) {
                atom.push_str(&format!("<author><name>{}</name></author>\n", xml(a)));
            }
            atom.push_str(&format!(
                "<summary>{}</summary>\n<content type=\"html\">{}</content>\n</entry>\n",
                xml(&e.summary()),
                xml(&absolutize(
                    &e.html,
                    &cfg.root().unwrap_or_else(|| site.clone())
                ))
            ));
        }
        rss.push_str("</channel>\n</rss>\n");
        atom.push_str("</feed>\n");
        out.push((format!("{}/rss.xml", c.base()), rss));
        out.push((format!("{}/atom.xml", c.base()), atom));
    }
    out
}

/// Root-relative `href`/`src` → absolute (feed readers have no base URL).
fn absolutize(html: &str, site: &str) -> String {
    html.replace("href=\"/", &format!("href=\"{site}/"))
        .replace("src=\"/", &format!("src=\"{site}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const POST: &str = "---\ntitle: \"Offline: first\"\ndescription: Why the planner works without network.\ndate: 2026-10-01\ntags: [sync, Offline]\nauthor: Ada\nseries: basics\n---\n# Offline first\n\nThe planner keeps working. ![Cover](/c.png)\n";

    #[test]
    fn frontmatter_subset() {
        let Front { fields: fm, body } = frontmatter(
            "---\na: 1\nb: 'it''s'\nc: true\nd:\n  - x\n  - \"y\"\ne: [p, q] \n# note\n---\nBody",
        )
        .unwrap();
        assert_eq!(fm[1], ("b".into(), Value::Str("it's".into())));
        assert_eq!(fm[2], ("c".into(), Value::Bool(true)));
        assert_eq!(
            fm[3],
            ("d".into(), Value::List(vec!["x".into(), "y".into()]))
        );
        assert_eq!(
            fm[4],
            ("e".into(), Value::List(vec!["p".into(), "q".into()]))
        );
        assert_eq!(body, "Body");
        assert_eq!(
            frontmatter("---\na: 1\na: 2\n---\n").unwrap_err(),
            (3, "`a` appears twice".into())
        );
        assert!(frontmatter("---\na: 1\n").is_err());
        assert_eq!(
            frontmatter("# no frontmatter").unwrap().body,
            "# no frontmatter"
        );
    }

    #[test]
    fn entries() {
        let (e, p) = parse_entry("blog", "2026-offline-first.md", POST);
        let e = e.unwrap();
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(
            (e.slug.as_str(), e.title.as_str(), e.route()),
            (
                "2026-offline-first",
                "Offline: first",
                "/blog/2026-offline-first".into()
            )
        );
        assert_eq!(e.cover.as_deref(), Some("/c.png"));
        assert_eq!(
            e.fields,
            vec![("series".into(), Value::Str("basics".into()))]
        );
        assert_eq!(e.reading_minutes, 1);
        assert!(
            e.html.starts_with("<p>The planner keeps working."),
            "leading H1 removed: {}",
            e.html
        );
        assert!(e.headings.is_empty());
        let (untitled, _) = parse_entry("blog", "x.md", "# From the body\n\nText.");
        assert_eq!(untitled.unwrap().title, "From the body");
        let (bad, p) = parse_entry("blog", "x.md", "---\ndate: 01/10/2026\n---\nno title here");
        assert!(bad.is_none());
        assert_eq!(p[0].msg, "entry has no title");
        let (_, p) = parse_entry("blog", "x.md", "---\ntitle: T\ndate: 2026-13-01\n---\ntext");
        let d = p.iter().find(|x| x.msg.contains("is not a date")).unwrap();
        assert_eq!(d.line, 3, "points at the `date:` line");
        let (_, p) = parse_entry(
            "blog",
            "x.md",
            "\u{feff}---\r\ntitle: T\r\n# note\r\nslug: Not_Valid\r\n---\r\ntext",
        );
        assert_eq!(p[0].line, 4, "{p:?}");
        assert_eq!(field_line("no frontmatter", "date"), 1);
        assert_eq!(
            field_line("---\ntitle: T\n---\ndate: x", "date"),
            1,
            "only inside the frontmatter"
        );
    }

    #[test]
    fn dates_and_pages() {
        assert_eq!(
            rfc822("2026-10-01").as_deref(),
            Some("Thu, 01 Oct 2026 00:00:00 +0000")
        );
        assert_eq!(
            rfc822("1970-01-01").as_deref(),
            Some("Thu, 01 Jan 1970 00:00:00 +0000")
        );
        assert_eq!(
            page_links("blog", 1, 25, 10),
            (None, Some("/blog/page/2".into()))
        );
        assert_eq!(
            page_links("blog", 3, 25, 10),
            (Some("/blog/page/2".into()), None)
        );
        assert_eq!(cut("one two three four", 10), "one two\u{2026}");
    }

    #[test]
    fn feeds_and_json() {
        let (e, _) = parse_entry("blog", "offline-first.md", POST);
        let (old, _) = parse_entry(
            "blog",
            "older.md",
            "---\ntitle: Older\ndate: 2026-01-05\ntags: [sync]\n---\nSee [home](/).\n",
        );
        let col = Collection {
            name: "blog".into(),
            entries: vec![e.unwrap(), old.unwrap()],
            ..Collection::default()
        };
        let cfg = Config {
            site: Some("https://acme.dev".into()),
            lang: Some("en".into()),
            ..Config::default()
        };
        let f = feeds(&cfg, std::slice::from_ref(&col), "Acme");
        assert_eq!(
            f.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
            vec!["blog/rss.xml", "blog/atom.xml"]
        );
        let rss = &f[0].1;
        assert!(rss.contains("<item>\n<title>Offline: first</title>\n<link>https://acme.dev/blog/offline-first</link>"));
        assert!(
            rss.contains("<pubDate>Thu, 01 Oct 2026 00:00:00 +0000</pubDate>")
                && rss.contains("<lastBuildDate>Thu, 01 Oct 2026")
        );
        assert!(f[1].1.contains(
            "<content type=\"html\">&lt;p&gt;See &lt;a href=&quot;https://acme.dev/&quot;&gt;home"
        ));
        assert!(feeds(&Config::default(), std::slice::from_ref(&col), "Acme").is_empty());
        assert_eq!(
            tags(&col),
            vec![
                ("sync".into(), "sync".into(), 2),
                ("Offline".into(), "offline".into(), 1)
            ]
        );
        let j = to_json(&[col], 10).stringify();
        assert!(
            j.starts_with("{\"blog\":{\"perPage\":10,\"entries\":[{\"slug\":\"offline-first\"")
        );
        assert!(j.contains("\"series\":\"basics\""));
        assert!(
            module_js(&[], 10).contains("length: n - 1 }, (_, i) => ({ n: String(i + 2) })"),
            "pages() starts at 2"
        );
    }
}
