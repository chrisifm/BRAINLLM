//! INTERNALS §37: languages with our own standard (no library).
//! `brain.json` → `"i18n": { "default": "en", "locales": ["en", "es"] }`; `i18n/<locale>.json` holds flat dotted keys,
//! each a string with `{var}` placeholders or a plural object (`{ "one": "{n} post", "other": "{n} posts" }`).
//!
//! - B1401 a `t('key')` whose key is missing in a locale (one diagnostic per locale file, edit adds the key).
//! - B1402 a value identical to the default locale's (warning; info for a single word).
//! - B1403 (warning) fixed text in markup when the site has 2+ locales (edits move it to a key).
//! - B1404 placeholders that differ between locales, or a `t()` call missing a variable the text uses (error).
//! - B1405 a locale code in `brain.json` or `<meta route>` that is not a valid language tag.
//! - B0306 (warning) a physical CSS property (`margin-left`…) when a right-to-left locale is configured.

use brain_parse::{Diag, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// ISO 639-1 language codes.
const LANGS: &str = "aa ab ae af ak am an ar as av ay az ba be bg bh bi bm bn bo br bs ca ce ch co cr cs cu cv cy da de dv dz ee el en eo es et eu fa ff fi fj fo fr fy ga gd gl gn gu gv ha he hi ho hr ht hu hy hz ia id ie ig ii ik io is it iu ja jv ka kg ki kj kk kl km kn ko kr ks ku kv kw ky la lb lg li ln lo lt lu lv mg mh mi mk ml mn mr ms mt my na nb nd ne ng nl nn no nr nv ny oc oj om or os pa pi pl ps pt qu rm rn ro ru rw sa sc sd se sg si sk sl sm sn so sq sr ss st su sv sw ta te tg th ti tk tl tn to tr ts tt tw ty ug uk ur uz ve vi vo wa wo xh yi yo za zh zu";
const RTL: &[&str] = &["ar", "he", "fa", "ur", "ps", "dv", "yi", "ug", "sd", "ku"];

/// `es`, `pt-BR`, `zh-Hant`, `es-419`.
pub fn valid_locale(code: &str) -> bool {
    let mut parts = code.split('-');
    let Some(lang) = parts.next() else { return false };
    if !LANGS.split(' ').any(|l| l == lang) {
        return false;
    }
    parts.all(|p| (p.len() == 4 && p.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && p[1..].chars().all(|c| c.is_ascii_lowercase())) || (p.len() == 2 && p.chars().all(|c| c.is_ascii_uppercase())) || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit())))
}

pub fn is_rtl(code: &str) -> bool {
    RTL.contains(&code.split('-').next().unwrap_or(""))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Text {
    Plain(String),
    /// Plural category → text (`one`, `other`, …).
    Plural(BTreeMap<String, String>),
}

impl Text {
    /// `{var}` names across every form.
    pub fn vars(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut scan = |s: &str| {
            let mut rest = s;
            while let Some(i) = rest.find('{') {
                match rest[i + 1..].find('}') {
                    Some(e) => {
                        let v = rest[i + 1..i + 1 + e].trim();
                        if !v.is_empty() && v.chars().all(|c| c.is_alphanumeric() || c == '_') {
                            out.insert(v.to_string());
                        }
                        rest = &rest[i + 2 + e..];
                    }
                    None => break,
                }
            }
        };
        match self {
            Text::Plain(s) => scan(s),
            Text::Plural(m) => m.values().for_each(|s| scan(s)),
        }
        out
    }

    pub fn as_plain(&self) -> Option<&str> {
        match self {
            Text::Plain(s) => Some(s),
            Text::Plural(_) => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LocaleFile {
    pub locale: String,
    /// Path relative to the app root (`i18n/es.json`).
    pub file: String,
    pub src: String,
    /// key → (text, 1-based line, byte offset of the key).
    pub entries: BTreeMap<String, (Text, u32, usize)>,
    /// Offset of the closing `}` of the top-level object (where new keys are inserted).
    pub close: usize,
    /// JSON syntax error, if any: (line, message).
    pub error: Option<(u32, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct I18n {
    pub default: String,
    pub locales: Vec<String>,
    /// Same order as `locales`; a locale without a file is absent.
    pub files: Vec<LocaleFile>,
    /// Locales declared in brain.json whose `i18n/<locale>.json` does not exist.
    pub missing: Vec<String>,
}

impl I18n {
    pub fn multi(&self) -> bool {
        self.locales.len() > 1
    }

    pub fn file(&self, locale: &str) -> Option<&LocaleFile> {
        self.files.iter().find(|f| f.locale == locale)
    }

    pub fn default_file(&self) -> Option<&LocaleFile> {
        self.file(&self.default)
    }

    pub fn rtl(&self) -> bool {
        self.locales.iter().any(|l| is_rtl(l))
    }
}

// ── minimal JSON (objects, strings, plural objects; anything else is skipped) ──

struct J<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl<'a> J<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn line(&self, at: usize) -> u32 {
        self.s[..at.min(self.s.len())].matches('\n').count() as u32 + 1
    }
    fn string(&mut self) -> Result<String, String> {
        if self.b.get(self.i) != Some(&b'"') {
            return Err("expected a string".into());
        }
        self.i += 1;
        let mut out = String::new();
        while self.i < self.b.len() {
            let c = self.s[self.i..].chars().next().unwrap_or('\0');
            self.i += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.s[self.i..].chars().next().unwrap_or('\0');
                    self.i += e.len_utf8();
                    match e {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'u' => {
                            let hex = self.s.get(self.i..self.i + 4).unwrap_or("");
                            if let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                                out.push(ch);
                            }
                            self.i += 4;
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        Err("unclosed string".into())
    }
    fn skip_value(&mut self) -> Result<(), String> {
        self.ws();
        match self.b.get(self.i) {
            Some(b'"') => self.string().map(|_| ()),
            Some(b'{') | Some(b'[') => {
                let mut depth = 0;
                while self.i < self.b.len() {
                    match self.b[self.i] {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                self.i += 1;
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                    self.i += 1;
                }
                Err("unclosed object".into())
            }
            _ => {
                while self.i < self.b.len() && !matches!(self.b[self.i], b',' | b'}' | b']') {
                    self.i += 1;
                }
                Ok(())
            }
        }
    }
}

/// Parses an `i18n/<locale>.json` file.
pub fn parse_locale(src: &str, locale: &str, file: &str) -> LocaleFile {
    let mut f = LocaleFile { locale: locale.into(), file: file.into(), src: src.into(), ..LocaleFile::default() };
    let mut j = J { s: src, b: src.as_bytes(), i: 0 };
    let res: Result<(), String> = (|| {
        j.ws();
        if j.b.get(j.i) != Some(&b'{') {
            return Err("a locale file is one JSON object".into());
        }
        j.i += 1;
        loop {
            j.ws();
            match j.b.get(j.i) {
                Some(b'}') => {
                    f.close = j.i;
                    return Ok(());
                }
                Some(b',') => {
                    j.i += 1;
                    continue;
                }
                Some(b'"') => {}
                _ => return Err("expected a key in quotes".into()),
            }
            let at = j.i;
            let key = j.string()?;
            j.ws();
            if j.b.get(j.i) != Some(&b':') {
                return Err(format!("expected `:` after \"{key}\""));
            }
            j.i += 1;
            j.ws();
            let text = match j.b.get(j.i) {
                Some(b'"') => Text::Plain(j.string()?),
                Some(b'{') => {
                    j.i += 1;
                    let mut m = BTreeMap::new();
                    loop {
                        j.ws();
                        match j.b.get(j.i) {
                            Some(b'}') => {
                                j.i += 1;
                                break;
                            }
                            Some(b',') => {
                                j.i += 1;
                            }
                            Some(b'"') => {
                                let k = j.string()?;
                                j.ws();
                                if j.b.get(j.i) != Some(&b':') {
                                    return Err(format!("expected `:` in \"{key}\""));
                                }
                                j.i += 1;
                                j.ws();
                                if j.b.get(j.i) == Some(&b'"') {
                                    m.insert(k, j.string()?);
                                } else {
                                    j.skip_value()?;
                                }
                            }
                            _ => return Err(format!("malformed plural object \"{key}\"")),
                        }
                    }
                    Text::Plural(m)
                }
                _ => {
                    j.skip_value()?;
                    continue;
                }
            };
            let line = j.line(at);
            f.entries.insert(key, (text, line, at));
        }
    })();
    if let Err(e) = res {
        f.error = Some((j.line(j.i), e));
    }
    f
}

/// `"i18n": { "default": "en", "locales": [...] }` from `brain.json` text.
fn config(json: &str) -> Option<(String, Vec<String>)> {
    let i = json.find("\"i18n\"")?;
    let rest = &json[i..];
    let obj = &rest[rest.find('{')?..];
    let end = obj.find('}')?;
    let obj = &obj[..=end];
    let default = obj.find("\"default\"").and_then(|d| obj[d + 9..].split('"').nth(1)).map(str::to_string);
    let locales = obj.find("\"locales\"").and_then(|l| {
        let r = &obj[l..];
        let a = r.find('[')?;
        let b = r.find(']')?;
        Some(r[a + 1..b].split(',').map(|x| x.trim().trim_matches('"').to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>())
    });
    let locales = locales.unwrap_or_default();
    let default = default.or_else(|| locales.first().cloned())?;
    Some((default, if locales.is_empty() { vec![] } else { locales }))
}

/// Loads the app's i18n setup (`None` = no `i18n` in brain.json and no `i18n/` folder).
pub fn load(root: &Path) -> Option<I18n> {
    let (default, mut locales) = match std::fs::read_to_string(root.join("brain.json")).ok().and_then(|j| config(&j)) {
        Some(c) => c,
        None => {
            if !root.join("i18n").is_dir() {
                return None;
            }
            ("en".to_string(), Vec::new())
        }
    };
    if locales.is_empty() {
        locales.push(default.clone());
    }
    let mut missing = Vec::new();
    let files = locales
        .iter()
        .filter_map(|l| {
            let file = format!("i18n/{l}.json");
            let f = std::fs::read_to_string(root.join(&file)).ok().map(|src| parse_locale(&src, l, &file));
            if f.is_none() {
                missing.push(l.clone());
            }
            f
        })
        .collect();
    Some(I18n { default, locales, files, missing })
}

fn span(src: &str, at: usize, line: u32) -> Span {
    let line_start = src[..at.min(src.len())].rfind('\n').map(|i| i + 1).unwrap_or(0);
    Span { start: at as u32, end: at as u32, line, col: (src[line_start..at.min(src.len())].chars().count() + 1) as u32 }
}

fn json_str(s: &str) -> String {
    let mut out = String::new();
    brain_parse::push_json_str(&mut out, s);
    out
}

/// Edit that appends `"key": value` to a locale file's object.
pub fn insert_edit(f: &LocaleFile, key: &str, value: &str) -> (u32, u32, String) {
    let before = f.src[..f.close].trim_end();
    let at = before.len() as u32;
    if before.ends_with('{') {
        // Empty object: the first key replaces the inside; the rest go in on the next `brain fix` round.
        return (at, f.close as u32, format!("\n  {}: {}\n", json_str(key), json_str(value)));
    }
    // A zero-width insertion right after the last entry: any number of them compose in one `brain fix` round
    // (QA: 30 keys used to collide on the same range and only one per round went in).
    (at, at, format!(",\n  {}: {}", json_str(key), json_str(value)))
}

/// B1401 for one `t('key')` call: a diagnostic per locale file that lacks the key, each with its own edit.
pub fn missing_key(i18n: &I18n, key: &str, used_at: &str) -> Vec<Diag> {
    let default_text = i18n.default_file().and_then(|f| f.entries.get(key)).and_then(|(t, _, _)| t.as_plain().map(str::to_string));
    let mut out = Vec::new();
    for f in &i18n.files {
        if f.error.is_some() || f.entries.contains_key(key) {
            continue;
        }
        let value = default_text.clone().unwrap_or_else(|| key.rsplit('.').next().unwrap_or(key).replace(['_', '-'], " "));
        let edit = insert_edit(f, key, &value);
        let line = f.src[..f.close].matches('\n').count() as u32 + 1;
        let d = Diag::new(
            "B1401",
            &f.file,
            span(&f.src, f.close, line),
            format!("key `{key}` (used in {used_at}) is missing in {}", f.file),
            if f.locale == i18n.default { format!("add `\"{key}\": \"…\"` to {}", f.file) } else { format!("add `\"{key}\"` to {} translated (the edit copies the default text; B1402 lists it until it is translated)", f.file) },
        )
        .edit(edit.0, edit.1, edit.2);
        out.push(d);
    }
    out
}

/// B1402, B1404 (between files) and B1405 (codes) for the whole i18n setup: the CLI runs it once per app.
pub fn check_locales(i18n: &I18n) -> Vec<Diag> {
    let mut out = Vec::new();
    for l in &i18n.locales {
        if !valid_locale(l) {
            out.push(Diag::new("B1405", "brain.json", Span { start: 0, end: 0, line: 1, col: 1 }, format!("`{l}` is not a language code"), "use an ISO 639-1 code with optional region or script: `es`, `pt-BR`, `zh-Hant`"));
        }
    }
    for f in &i18n.files {
        if let Some((line, msg)) = &f.error {
            out.push(Diag::new("B1401", &f.file, Span { start: 0, end: 0, line: *line, col: 1 }, format!("{} is not valid: {msg}", f.file), "a locale file is one flat JSON object: `{ \"nav.home\": \"Home\" }`"));
        }
    }
    // A declared locale without its file: B1401 on the file, with an edit that creates it from the default locale.
    for l in &i18n.missing {
        let file = format!("i18n/{l}.json");
        let mut body = String::from("{\n");
        if let Some(def) = i18n.default_file() {
            let n = def.entries.len();
            for (i, (k, (t, _, _))) in def.entries.iter().enumerate() {
                let value = match t {
                    Text::Plain(s) => json_str(s),
                    Text::Plural(m) => format!("{{ {} }}", m.iter().map(|(c, v)| format!("{}: {}", json_str(c), json_str(v))).collect::<Vec<_>>().join(", ")),
                };
                body.push_str(&format!("  {}: {}{}\n", json_str(k), value, if i + 1 < n { "," } else { "" }));
            }
        }
        body.push_str("}\n");
        out.push(
            Diag::new("B1401", &file, Span { start: 0, end: 0, line: 1, col: 1 }, format!("locale `{l}` is declared in brain.json but {file} does not exist"), format!("create {file} (the edit copies every key of the default locale; B1402 lists them until they are translated) or remove `{l}` from brain.json"))
                .edit(0, 0, body),
        );
    }
    let Some(def) = i18n.default_file() else { return out };
    for f in i18n.files.iter().filter(|f| f.locale != i18n.default) {
        for (key, (text, line, at)) in &f.entries {
            let Some((dtext, _, _)) = def.entries.get(key) else { continue };
            if text.vars() != dtext.vars() {
                let want: Vec<String> = dtext.vars().into_iter().map(|v| format!("{{{v}}}")).collect();
                out.push(Diag::new("B1404", &f.file, span(&f.src, *at, *line), format!("`{key}` has different placeholders than {}", def.file), format!("use exactly {} in every locale", if want.is_empty() { "no placeholders".into() } else { want.join(", ") })));
            }
            if text == dtext {
                let one_word = text.as_plain().is_some_and(|s| !s.trim().contains(char::is_whitespace));
                let d = Diag::new("B1402", &f.file, span(&f.src, *at, *line), format!("`{key}` is identical to {}: probably untranslated", def.file), format!("translate `{key}` into `{}`", f.locale));
                out.push(if one_word { d.info() } else { d });
            }
        }
        // Keys of the default locale missing here are reported where they are used (B1401).
    }
    out
}

/// Text with letters (not just numbers, punctuation or whitespace): what B1403 considers translatable.
pub fn translatable(s: &str) -> bool {
    s.chars().filter(|c| c.is_alphabetic()).count() >= 2
}

/// `<file stem>.<first words>` key for a fixed text (B1403).
pub fn key_for(file: &str, text: &str) -> String {
    let stem = Path::new(file).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let stem = stem.trim_start_matches('[').trim_end_matches(']').trim_start_matches('_').replace(['[', ']'], "");
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(4)
        .map(|w| w.to_lowercase())
        .collect();
    format!("{}.{}", stem.replace('.', "-"), words.join("_"))
}

/// Physical → logical CSS property (B0306).
pub fn logical(prop: &str) -> Option<&'static str> {
    Some(match prop {
        "margin-left" => "margin-inline-start",
        "margin-right" => "margin-inline-end",
        "padding-left" => "padding-inline-start",
        "padding-right" => "padding-inline-end",
        "border-left" => "border-inline-start",
        "border-right" => "border-inline-end",
        "border-left-width" => "border-inline-start-width",
        "border-right-width" => "border-inline-end-width",
        "border-left-color" => "border-inline-start-color",
        "border-right-color" => "border-inline-end-color",
        "left" => "inset-inline-start",
        "right" => "inset-inline-end",
        "border-top-left-radius" => "border-start-start-radius",
        "border-top-right-radius" => "border-start-end-radius",
        "border-bottom-left-radius" => "border-end-start-radius",
        "border-bottom-right-radius" => "border-end-end-radius",
        _ => return None,
    })
}

/// First characters of a text for a message.
pub fn short(s: &str) -> String {
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > 40 { format!("{}…", s.chars().take(40).collect::<String>()) } else { s }
}
pub use crate::i18n_rules::route_slugs;
