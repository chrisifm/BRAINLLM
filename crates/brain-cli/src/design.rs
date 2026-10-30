//! Adopt existing design tokens without rewriting styles, fonts, base paths or application data.
use brain_css::CssItem;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::Path;

pub const FILE: &str = "design.brain.json";
const MAX_SOURCE: usize = 262144;
const MAX_BASELINE: usize = 1024 * 1024;
fn invalid(reason: &str) -> Value {
    json!({"schemaVersion":1,"kind":"brain-design-system","validation":{"valid":false,"reason":reason}})
}
fn context(name: &str, value: &str) -> String {
    let value = value.trim();
    match (name, value) {
        ("selector", ":root") => "root".into(),
        ("media", "(prefers-color-scheme: dark)") => "media:dark".into(),
        ("media", "(prefers-color-scheme: light)") => "media:light".into(),
        ("media", "(prefers-reduced-motion: reduce)") => "media:reduced-motion".into(),
        ("layer", v)
            if v.len() < 64
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)) =>
        {
            format!("layer:{v}")
        }
        _ => format!("{name}:{}", &crate::manifests::hash(value.as_bytes())[..12]),
    }
}
fn definitions(
    items: &[CssItem],
    scope: &[String],
    out: &mut BTreeMap<String, Vec<Value>>,
    count: &mut usize,
) -> bool {
    for item in items {
        match item {
            CssItem::Decl { name, value, span } if name.starts_with("--") => {
                *count += 1;
                if *count > 2048
                    || (!out.contains_key(name) && out.len() >= 512)
                    || name.len() > 80
                    || name.len() < 3
                    || !name
                        .bytes()
                        .skip(2)
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                    || value.len() > 1024
                    || scope.len() > 16
                {
                    return false;
                }
                out.entry(name.clone()).or_default().push(
                    json!({"value":value,"context":scope,"line":span.line,"column":span.col}),
                );
            }
            CssItem::Rule {
                selector, items, ..
            } => {
                let mut nested = scope.to_vec();
                nested.push(context("selector", selector));
                if !definitions(items, &nested, out, count) {
                    return false;
                }
            }
            CssItem::At {
                name,
                prelude,
                block: Some(items),
                ..
            } => {
                let mut nested = scope.to_vec();
                nested.push(context(name, prelude));
                if !definitions(items, &nested, out, count) {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}
fn category(name: &str, defs: &[Value]) -> &'static str {
    let value = defs
        .first()
        .and_then(|d| d["value"].as_str())
        .unwrap_or("")
        .trim();
    if name.contains("font") {
        "typography"
    } else if name.contains("space") || name.contains("gap") || name.contains("padding") {
        "spacing"
    } else if name.contains("radius") || name.contains("border") {
        "borders"
    } else if value.starts_with('#')
        || [
            "rgb(", "rgba(", "hsl(", "hsla(", "oklch(", "oklab(", "color(",
        ]
        .iter()
        .any(|p| value.starts_with(p))
    {
        "colors"
    } else {
        "other"
    }
}
fn defaults() -> Value {
    json!({"mobileFirst":true,"minimumTouchCssPx":44,"minimumInputFontCssPx":16,"logicalProperties":true,"safeAreaInsets":true,"reducedMotion":true,"visibleKeyboardFocus":true,"rtl":true,"externalFontFetch":false,"stateHierarchy":["permission","loading","error","empty","success"],"disabledCrosscutting":true,"cssFramework":"none"})
}
/// Pure scaffold/adoption constructor. Existing unknown fields and legacy token values are preserved.
pub fn baseline(tokens_source: &str, token_file: &str, previous: Option<&Value>) -> Value {
    if !brain_check::TOKEN_FILES.contains(&token_file) || tokens_source.len() > MAX_SOURCE {
        return invalid("unsupported token file or token source exceeds 256 KiB");
    }
    if !brain_check::scan_secrets(tokens_source).is_empty() {
        return invalid("token source contains a detected secret");
    }
    if previous.is_some_and(|v| !v.is_object()) {
        return invalid("existing design baseline must be an object");
    }
    if previous
        .and_then(|v| v["schemaVersion"].as_u64())
        .is_some_and(|v| v > 1)
    {
        return invalid("newer design baseline schema requires an explicit migration");
    }
    let Ok(sheet) = brain_css::parse_str(tokens_source, token_file) else {
        return invalid("token stylesheet cannot be parsed");
    };
    let mut declared = BTreeMap::new();
    if !definitions(&sheet.items, &[], &mut declared, &mut 0) {
        return invalid("token declaration metadata exceeds its bounds");
    }
    let mut document = previous.cloned().unwrap_or_else(|| json!({}));
    if let Some(old) = previous {
        if old["schemaVersion"] != 1 {
            document["adoptedLegacyDocument"] = old.clone();
        }
    }
    document["schemaVersion"] = json!(1);
    document["kind"] = json!("brain-design-system");
    if document["name"].is_null() {
        document["name"] = json!("Project foundation");
    }
    if !document["source"].is_object() {
        if !document["source"].is_null() {
            document["legacySourceValue"] = document["source"].clone();
        }
        document["source"] = json!({});
    }
    document["source"]["tokensFile"] = json!(token_file);
    document["source"]["revision"] = json!(crate::manifests::hash(tokens_source.as_bytes()));
    document["source"]["bytes"] = json!(tokens_source.len());
    document["source"]["authority"] = json!("adopted-project-stylesheet");
    if !document["tokens"].is_object() {
        if !document["tokens"].is_null() {
            document["legacyTokensValue"] = document["tokens"].clone();
        }
        document["tokens"] = json!({});
    }
    for token in document["tokens"].as_object_mut().unwrap().values_mut() {
        if token.is_object() {
            token["presentInSource"] = json!(false);
        }
    }
    for (name, defs) in declared {
        let mut token = document["tokens"][&name].clone();
        if !token.is_object() {
            let old = token;
            token = json!({});
            if !old.is_null() {
                token["legacyValue"] = old;
            }
        }
        let semantic: Vec<_> = defs
            .iter()
            .map(|d| json!({"value":d["value"],"context":d["context"]}))
            .collect();
        token["presentInSource"] = json!(true);
        token["category"] = json!(category(&name, &defs));
        token["revision"] = json!(crate::manifests::hash(
            serde_json::to_vec(&semantic).unwrap().as_slice()
        ));
        token["definitions"] = json!(defs);
        document["tokens"][&name] = token;
    }
    if !document["rules"].is_object() {
        if !document["rules"].is_null() {
            document["legacyRulesValue"] = document["rules"].clone();
        }
        document["rules"] = json!({});
    }
    for (key, value) in defaults().as_object().unwrap() {
        document["rules"]
            .as_object_mut()
            .unwrap()
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    document["validation"] =
        json!({"valid":true,"source":"native-css-parser","computedStyles":"not-observed"});
    if !serde_json::to_vec(&document).is_ok_and(|b| b.len() <= MAX_BASELINE) {
        return invalid("design baseline exceeds 1 MiB; reduce token metadata before adoption");
    }
    document
}
pub(crate) fn token_source(root: &Path) -> Result<(String, String), String> {
    for file in brain_check::TOKEN_FILES {
        if crate::changes::source_path(root, file).is_ok_and(|p| p.is_file()) {
            return crate::map::read_source_limit(root, file, MAX_SOURCE)
                .map(|source| ((*file).into(), source));
        }
    }
    Err(
        "no project token stylesheet found; preserve or create tokens.cssbrain before adoption"
            .into(),
    )
}
fn stored(root: &Path) -> Result<(Option<Value>, Value), String> {
    let snapshot = crate::changes::read(root, FILE)?;
    let Some(source) = snapshot["content"].as_str() else {
        return Ok((None, Value::Null));
    };
    if source.len() > MAX_BASELINE {
        return Err("existing design baseline exceeds 1 MiB".into());
    }
    let value: Value = serde_json::from_str(source)
        .map_err(|_| "design.brain.json is invalid; inspect it before adoption")?;
    if !value.is_object() {
        return Err("design baseline must be an object; automatic replacement refused".into());
    }
    Ok((Some(value), snapshot["revision"].clone()))
}
pub fn preview_init(root: &Path, options: &Value) -> Result<Value, String> {
    if !options.is_object() {
        return Err("design init options must be an object".into());
    }
    let (file, source) = token_source(root)?;
    let (previous, revision) = stored(root)?;
    let mut proposed = baseline(&source, &file, previous.as_ref());
    if proposed["validation"]["valid"] != true {
        return Err(proposed["validation"]["reason"]
            .as_str()
            .unwrap_or("cannot prepare design adoption")
            .into());
    }
    if let Some(name) = options.get("name") {
        let name = name
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 80 && brain_check::scan_secrets(s).is_empty())
            .ok_or("design name must contain 1..80 public characters")?;
        proposed["name"] = json!(name);
    }
    if let Some(expected) = options.get("expectedRevision") {
        if expected != &revision {
            return Err("design baseline changed; inspect and prepare a new adoption plan".into());
        }
    }
    let operations = if previous.as_ref() == Some(&proposed) {
        vec![]
    } else {
        vec![
            json!({"path":FILE,"expectedRevision":revision,"content":format!("{}\n",serde_json::to_string_pretty(&proposed).map_err(|_|"cannot serialize baseline")?)}),
        ]
    };
    Ok(
        json!({"schemaVersion":1,"dryRun":true,"adoption":"existing-values-preserved","operations":operations,"source":{"file":file,"revision":proposed["source"]["revision"]},"preserved":["token stylesheet","fonts","base path","application data","unknown design fields"],"next":"Apply these revision-checked operations with the change planner, or install a new baseline with the guarded source creator; no theme rewrite is required."}),
    )
}
fn current(root: &Path) -> Result<(Value, Option<Value>), String> {
    let (file, source) = token_source(root)?;
    let (stored, revision) = stored(root)?;
    let mut current = baseline(&source, &file, None);
    current["baselineReadRevision"] = revision;
    if current["validation"]["valid"] != true {
        return Err(
            "token metadata is invalid; inspect the native stylesheet before adopting it".into(),
        );
    }
    Ok((current, stored))
}
fn check_values(current: &Value, stored: Option<&Value>) -> Value {
    let mut warnings = vec![];
    let mut added = vec![];
    let mut removed = vec![];
    let mut changed = vec![];
    if let Some(old) = stored {
        if old["schemaVersion"] != 1 {
            warnings.push(json!({"kind":"baseline-schema","fix":"preview adoption to schemaVersion 1 while preserving existing fields"}));
        }
        if old["source"]["revision"] != current["source"]["revision"]
            || old["source"]["tokensFile"] != current["source"]["tokensFile"]
        {
            warnings.push(json!({"kind":"token-source-drift","fix":"inspect token changes, then explicitly adopt the current source revision"}));
        }
        for (name, token) in current["tokens"].as_object().unwrap() {
            if old["tokens"].get(name).is_none() {
                added.push(name.clone());
            } else if old["tokens"][name]["revision"] != token["revision"] {
                changed.push(name.clone());
            }
        }
        if let Some(tokens) = old["tokens"].as_object() {
            for (name, token) in tokens {
                if token["presentInSource"] != false && current["tokens"].get(name).is_none() {
                    removed.push(name.clone());
                }
            }
        }
    } else {
        warnings.push(json!({"kind":"baseline-missing","fix":"run brain design init to preview non-destructive adoption"}));
    }
    json!({"schemaVersion":1,"warnings":warnings,"drift":{"added":added,"removed":removed,"changed":changed},"blocking":false,"currentSource":current["source"],"baselineSource":stored.as_ref().map(|v|v["source"].clone()),"qualification":"Source metadata drift; runtime contrast, layout and state behavior require separate checks."})
}
pub fn check(root: &Path) -> Result<Value, String> {
    let (current, stored) = current(root)?;
    Ok(check_values(&current, stored.as_ref()))
}
pub fn query(root: &Path, target: Option<&str>) -> Result<Value, String> {
    let (current, stored) = current(root)?;
    if let Some(name) = target {
        if name != "baseline" {
            let token = current["tokens"]
                .get(name)
                .ok_or("design token not found; inspect the token names first")?;
            return Ok(
                json!({"schemaVersion":1,"name":name,"current":token,"baseline":stored.as_ref().and_then(|v|v["tokens"].get(name)).map(|v|json!({"definitions":v["definitions"],"revision":v["revision"]})),"source":current["source"]}),
            );
        }
    }
    let mut rules = Map::new();
    for (key, default) in defaults().as_object().unwrap() {
        rules.insert(
            key.clone(),
            stored
                .as_ref()
                .and_then(|v| v["rules"].get(key))
                .filter(|v| v.is_boolean() || v.is_number() || v.is_string() || v.is_array())
                .cloned()
                .unwrap_or_else(|| default.clone()),
        );
    }
    Ok(
        json!({"schemaVersion":1,"baselinePresent":stored.is_some(),"file":FILE,"name":stored.as_ref().and_then(|v|v["name"].as_str()).unwrap_or("Project foundation"),"source":current["source"],"tokens":current["tokens"].as_object().unwrap().iter().map(|(name,token)|json!({"name":name,"category":token["category"],"revision":token["revision"]})).collect::<Vec<_>>(),"rules":rules,"check":check_values(&current,stored.as_ref()),"preview":"static contract gallery; no app logic, resource calls or external fonts"}),
    )
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn safe_css(value: &str) -> bool {
    value.len() <= 1024
        && !value.contains(['<', '>', '{', '}', ';'])
        && !value.chars().any(char::is_control)
        && !["url(", "expression(", "@import", "javascript:", "behavior:"]
            .iter()
            .any(|bad| value.to_ascii_lowercase().contains(bad))
}
pub fn gallery(root: &Path) -> Result<Value, String> {
    let (current, stored) = current(root)?;
    let (index, documents) = crate::manifests::documents(root)?;
    let token_file = current["source"]["tokensFile"].as_str().unwrap_or("");
    if index["sourceRevisions"][token_file] != current["source"]["revision"]
        || index["sourceRevisions"][FILE] != current["baselineReadRevision"]
    {
        return Err(
            "design sources changed while preparing the gallery; retry from a fresh snapshot"
                .into(),
        );
    }
    let mut css = String::new();
    let mut cards = String::new();
    let mut tokens = 0usize;
    for (name, token) in current["tokens"].as_object().unwrap() {
        let definitions = token["definitions"].as_array().unwrap();
        let first = definitions
            .iter()
            .find(|d| {
                d["context"].as_array().is_some_and(|c| {
                    c.iter().all(|part| {
                        part == "root" || part.as_str().is_some_and(|s| s.starts_with("layer:"))
                    })
                })
            })
            .or_else(|| definitions.first());
        let value = first.and_then(|d| d["value"].as_str()).unwrap_or("");
        if safe_css(value)
            && first.is_some_and(|d| {
                d["context"].as_array().is_some_and(|c| {
                    c.iter()
                        .all(|v| v == "root" || v.as_str().is_some_and(|s| s.starts_with("layer:")))
                })
            })
        {
            css.push_str(&format!("{name}:{value};"));
        }
        let swatch = if token["category"] == "colors" && safe_css(value) {
            format!(
                "<div class=\"swatch\" style=\"background:{}\" aria-hidden=\"true\"></div>",
                escape(value)
            )
        } else {
            "<div class=\"token-mark\" aria-hidden=\"true\">Aa</div>".into()
        };
        cards.push_str(&format!("<article class=\"token panel\" data-entry data-category=\"tokens\" data-search=\"{}\">{swatch}<h3><code>{}</code></h3><p class=\"value\">{}</p><p class=\"quiet\">{} declaration{}</p></article>",escape(&name.to_lowercase()),escape(name),escape(value),definitions.len(),if definitions.len()==1{""}else{"s"}));
        tokens += 1;
    }
    let mut component_cards = String::new();
    for doc in documents.iter().take(500) {
        let name = doc["name"].as_str().unwrap_or("Unknown component");
        let file = doc["file"].as_str().unwrap_or("");
        let props: Vec<_> = doc["contract"]["props"]
            .as_array()
            .into_iter()
            .flatten()
            .take(12)
            .map(|p| {
                format!(
                    "<li><code>{}</code> <span>{}{}</span></li>",
                    escape(p["name"].as_str().unwrap_or("?")),
                    escape(p["type"].as_str().unwrap_or("unknown")),
                    if p["optional"] == true {
                        " (optional)"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        let states: Vec<_> = doc["states"]["evidence"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s["state"].as_str())
            .collect();
        let emits: Vec<_> = doc["contract"]["emits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        component_cards.push_str(&format!("<article class=\"contract panel\" data-entry data-category=\"components\" data-search=\"{}\"><h3>{}</h3><p class=\"value\">{}</p><p class=\"quiet\">Static contract preview. Application code is not executed.</p><h4>Props</h4><ul>{}</ul><p>Emits: {}</p><p>Declared states: {}</p><p class=\"quiet\">Semantic checks: not run. Use brain check.</p></article>",escape(&format!("{name} {file}").to_lowercase()),escape(name),escape(file),if props.is_empty(){"<li>No declared props</li>".into()}else{props.join("")},escape(&emits.join(", ")),if states.is_empty(){"Unknown".into()}else{escape(&states.join(", "))}));
    }
    let title = stored
        .as_ref()
        .and_then(|v| v["name"].as_str())
        .unwrap_or("Project foundation");
    let replacements = BTreeMap::from([
        ("TITLE", escape(title)),
        ("TOKEN_CSS", css),
        ("TOKENS", cards),
        ("COMPONENTS", component_cards),
        (
            "SOURCE",
            escape(
                current["source"]["tokensFile"]
                    .as_str()
                    .unwrap_or("tokens.cssbrain"),
            ),
        ),
        (
            "COVERAGE",
            if index["coverage"]["complete"] == true {
                "Current source contracts".into()
            } else {
                "Partial source coverage. Inspect skipped files with brain manifest inspect.".into()
            },
        ),
    ]);
    let mut html = String::new();
    let mut template = include_str!("templates/design-gallery.html");
    while let Some(start) = template.find("{{") {
        html.push_str(&template[..start]);
        let Some(end) = template[start + 2..].find("}}") else {
            html.push_str(&template[start..]);
            template = "";
            break;
        };
        let key = &template[start + 2..start + 2 + end];
        if let Some(value) = replacements.get(key) {
            html.push_str(value);
        } else {
            html.push_str(&template[start..start + end + 4]);
        }
        template = &template[start + end + 4..];
    }
    html.push_str(template);
    if documents.is_empty() {
        html=html.replace("<!--empty-components-->","<p class=\"panel\">No component contracts yet. Add a component, then refresh the project manifests.</p>");
    }
    let dir = crate::manifests::private_dir(root, "design")?;
    let _lock = crate::manifests::lock(&dir)?;
    crate::manifests::atomic(&dir.join("gallery.html"), html.as_bytes())?;
    Ok(
        json!({"schemaVersion":1,"private":true,"path":".brain/design/gallery.html","tokens":tokens,"components":documents.len(),"preview":"static-contracts-only","network":"disabled by content security policy; no external fonts or user resources","coverage":index["coverage"],"baselineDrift":check_values(&current,stored.as_ref())}),
    )
}

/// Project-owned editable basics. Existing projects opt in via preview_seed; scaffolds may create these directly.
pub fn new_basics() -> Vec<(&'static str, String)> {
    let field_import = |source: &str| {
        if source.contains("<script>") {
            source.replacen(
                "<script>",
                "<script>\n  import Field from './field.brain'",
                1,
            )
        } else {
            let at = source.find('\n').unwrap_or(source.len());
            format!(
                "{}\n<script>\n  import Field from './field.brain'\n</script>{}",
                &source[..at],
                &source[at..]
            )
        }
    };
    vec![
        (
            "ui/button.brain",
            include_str!("../../../runtime/ui/button.brain")
                .replace("min-height: 40px", "min-height: 44px"),
        ),
        (
            "ui/field.brain",
            include_str!("../../../runtime/ui/field.brain").into(),
        ),
        (
            "ui/input.brain",
            field_import(include_str!("../../../runtime/ui/input.brain")),
        ),
        (
            "ui/select.brain",
            field_import(include_str!("../../../runtime/ui/select.brain")),
        ),
        (
            "ui/checkbox.brain",
            include_str!("../../../runtime/ui/checkbox.brain")
                .replace("label { display:", "label { min-height: 44px; display:"),
        ),
        (
            "ui/dialog.brain",
            include_str!("../../../runtime/ui/dialog.brain")
                .replace("width: 36px; height: 36px", "width: 44px; height: 44px")
                .replace(
                    "padding: 0; border:",
                    "max-height: calc(100dvh - 32px); overflow: auto; padding: 0; border:",
                ),
        ),
        (
            "ui/container.brain",
            include_str!("../../../runtime/ui/container.brain").into(),
        ),
        (
            "ui/card.brain",
            include_str!("templates/design/card.brain").into(),
        ),
        (
            "ui/hero.brain",
            include_str!("templates/design/hero.brain").into(),
        ),
        (
            "ui/search.brain",
            include_str!("templates/design/search.brain").into(),
        ),
        (
            "ui/header.brain",
            include_str!("templates/design/header.brain").into(),
        ),
    ]
}
/// An agent-facing starting checklist. This is a contract inventory, not rendered accessibility proof.
pub fn blueprint(root: &Path) -> Result<Value, String> {
    let foundation = query(root, None)?;
    let mut components = vec![];
    for (file, source) in new_basics() {
        let read = crate::changes::read(root, file)?;
        let name = file.strip_prefix("ui/").unwrap().trim_end_matches(".brain");
        let category = match name {
            "button" | "input" | "select" | "checkbox" | "field" | "search" => "controls",
            "dialog" => "overlays",
            _ => "layout",
        };
        let required_tokens: BTreeMap<_, _> = source
            .split("var(--")
            .skip(1)
            .filter_map(|part| {
                part.split_once(')').map(|(name, _)| {
                    (
                        format!("--{}", name.split(',').next().unwrap_or(name).trim()),
                        true,
                    )
                })
            })
            .filter(|(name, _)| {
                name.bytes()
                    .skip(2)
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            })
            .collect();
        let parsed = brain_parse::parse(&source, Path::new(file))
            .map_err(|_| "built-in project basic cannot be parsed")?;
        let contract = json!({"props":parsed.props.iter().map(|p| json!({"name":p.name,"type":p.ty.as_str(),"optional":p.optional || p.default.is_some(),"bindable":p.bindable})).collect::<Vec<_>>(),"emits":brain_check::meta_names(&parsed,"emits"),"expose":brain_check::meta_names(&parsed,"expose"),"confidence":"canonical seed contract; existing source may differ"});
        components.push(json!({"name":name,"file":file,"category":category,"seedContract":contract,"state":if read["revision"].is_null(){"missing"}else{"project-owned"},"revision":read["revision"],"canonicalContract":"project-owned source; inspect its manifest for the actual current props/emits/slots","requiredTokens":required_tokens.keys().collect::<Vec<_>>(),"import":format!("import {} from '../{file}'",name[..1].to_ascii_uppercase()+&name[1..]),"edit":"Edit this project source, preserve its public contract, then refresh manifests and gallery. Explicit imports select project basics over native-kit names."}));
    }
    let missing: Vec<_> = components
        .iter()
        .filter(|c| c["state"] == "missing")
        .map(|c| c["file"].clone())
        .collect();
    Ok(
        json!({"schemaVersion":1,"mandatoryBeforeUiEdit":true,"foundation":foundation,"components":components,"missingComponents":missing,"workflow":["Read the project design foundation and this blueprint before every UI edit","Inspect current component manifests and use project-owned basics with explicit imports","Preview seed for missing basics or configure for reviewed token changes; apply only through CAS operations","Extend source contracts deliberately; preserve existing values and caller behavior","Refresh manifests/gallery; inspect rendered mobile, tablet, desktop, RTL and keyboard states"],"acceptance":{"responsive":["mobile-first single-column baseline","tablet/desktop expansion without overflow","safe-area and keyboard/zoom behavior","logical alignment for RTL"],"accessibility":["44 CSS pixel targets","16 CSS pixel input text","visible focus and native labels","reduced motion","contrast and screen-reader checks"],"states":["permission when relevant","loading","error with recovery","empty with next action","success","disabled across states"],"navigation":"Give shareable modals an explicit stable urlKey; never serialize credentials or transient private values","qualification":"Static inventory only; runtime layout, contrast, focus and state coverage need actual rendered checks."}}),
    )
}
pub fn preview_seed(root: &Path, options: &Value) -> Result<Value, String> {
    if !options.is_object()
        || options
            .as_object()
            .unwrap()
            .keys()
            .any(|k| !["names", "expectedRevisions"].contains(&k.as_str()))
    {
        return Err("seed options accept only names and expectedRevisions".into());
    }
    let basics = new_basics();
    let mut selected = std::collections::BTreeSet::new();
    if let Some(names) = options.get("names") {
        for name in names
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 11)
            .ok_or("names must contain 1..11 basic names")?
        {
            let name = name.as_str().ok_or("basic names must be strings")?;
            let file = format!("ui/{name}.brain");
            if !basics.iter().any(|(p, _)| *p == file) {
                return Err("unknown basic name; inspect the design blueprint".into());
            }
            selected.insert(file);
        }
    } else {
        selected.extend(basics.iter().map(|(p, _)| p.to_string()));
    }
    if selected.contains("ui/input.brain") || selected.contains("ui/select.brain") {
        selected.insert("ui/field.brain".into());
    }
    if options
        .get("expectedRevisions")
        .is_some_and(|v| !v.is_object())
    {
        return Err("expectedRevisions must be an object".into());
    }
    let mut operations = vec![];
    let mut preserved = vec![];
    for (path, content) in basics.into_iter().filter(|(p, _)| selected.contains(*p)) {
        let existing = crate::changes::read(root, path)?;
        if options
            .get("expectedRevisions")
            .and_then(|r| r.get(path))
            .is_some_and(|r| r != &existing["revision"])
        {
            return Err(
                "project basic changed; inspect the source and prepare a new seed preview".into(),
            );
        }
        if existing["revision"].is_null() {
            operations.push(json!({"path":path,"expectedRevision":null,"content":content}));
        } else {
            preserved.push(json!({"file":path,"revision":existing["revision"]}));
        }
    }
    Ok(
        json!({"schemaVersion":1,"dryRun":true,"operations":operations,"preserved":preserved,"ownership":"project-owned editable source; explicit imports select it","baselineChanged":false,"tokensChanged":false,"next":"Inspect required token names in blueprint; apply reviewed CAS operations, then refresh manifests/gallery and check the rendered states."}),
    )
}
fn configurable_value(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 256
        && safe_css(value)
        && !value.contains(['\\', '@', ':'])
        && !value.contains("//")
        && !value.to_ascii_lowercase().contains("image-set(")
        && !value.contains("/*")
        && !value.contains("*/")
}
fn declaration_spans(
    items: &[CssItem],
    names: &Map<String, Value>,
    out: &mut Vec<(String, usize, usize)>,
) {
    for item in items {
        match item {
            CssItem::Decl { name, span, .. } if names.contains_key(name) => {
                out.push((name.clone(), span.start as usize, span.end as usize))
            }
            CssItem::Rule { items, .. }
            | CssItem::At {
                block: Some(items), ..
            } => declaration_spans(items, names, out),
            _ => {}
        }
    }
}
/// Controlled literal token changes plus a matching baseline. Conditional/ambiguous declarations require explicit source edits.
pub fn preview_configure(root: &Path, options: &Value) -> Result<Value, String> {
    if !options.is_object()
        || options.as_object().unwrap().keys().any(|k| {
            ![
                "tokens",
                "expectedTokenRevision",
                "expectedBaselineRevision",
            ]
            .contains(&k.as_str())
        })
    {
        return Err("configure options accept only tokens and expected source revisions".into());
    }
    let values = options["tokens"]
        .as_object()
        .filter(|m| !m.is_empty() && m.len() <= 32)
        .ok_or("tokens must contain 1..32 custom-property values")?;
    for (name, value) in values {
        if !name.starts_with("--")
            || name.len() < 3
            || name.len() > 80
            || !name
                .bytes()
                .skip(2)
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || !value.as_str().is_some_and(configurable_value)
        {
            return Err("token changes require bounded custom-property names and safe local literal CSS values; URLs, escapes and arbitrary rules are refused".into());
        }
    }
    let (file, source) = token_source(root)?;
    let token_revision = json!(crate::manifests::hash(source.as_bytes()));
    let (previous, baseline_revision) = stored(root)?;
    if options
        .get("expectedTokenRevision")
        .is_some_and(|v| v != &token_revision)
        || options
            .get("expectedBaselineRevision")
            .is_some_and(|v| v != &baseline_revision)
    {
        return Err("design source changed; inspect current tokens/baseline and prepare a new configuration preview".into());
    }
    let before = baseline(&source, &file, None);
    if before["validation"]["valid"] != true {
        return Err("token source is invalid; inspect it before configuring".into());
    }
    for name in values.keys() {
        if let Some(token) = before["tokens"].get(name) {
            let defs = token["definitions"].as_array().unwrap();
            if defs.len() != 1
                || !defs[0]["context"].as_array().is_some_and(|ctx| {
                    !ctx.is_empty()
                        && ctx.iter().all(|c| {
                            c == "root" || c.as_str().is_some_and(|s| s.starts_with("layer:"))
                        })
                })
            {
                return Err("conditional, scoped or repeated token declarations need an explicit reviewed source edit; configure will not flatten them".into());
            }
        }
    }
    let sheet = brain_css::parse_str(&source, &file).map_err(|_| "cannot parse token source")?;
    let mut spans = vec![];
    declaration_spans(&sheet.items, values, &mut spans);
    spans.sort_by_key(|(_, start, _)| std::cmp::Reverse(*start));
    let mut updated = source.clone();
    let mut found = std::collections::BTreeSet::new();
    for (name, start, end) in spans {
        let raw = &source[start..end];
        let colon = raw.find(':').ok_or("token declaration has no colon")?;
        let value_end = if raw.ends_with(';') { end - 1 } else { end };
        updated.replace_range(
            start + colon + 1..value_end,
            &format!(" {}", values[&name].as_str().unwrap()),
        );
        found.insert(name);
    }
    let new: Vec<_> = values
        .iter()
        .filter(|(name, _)| !found.contains(*name))
        .collect();
    if !new.is_empty() {
        updated.push_str("\n:root {\n");
        for (name, value) in new {
            updated.push_str(&format!("  {name}: {};\n", value.as_str().unwrap()));
        }
        updated.push_str("}\n");
    }
    let proposed = baseline(&updated, &file, previous.as_ref());
    if proposed["validation"]["valid"] != true {
        return Err(
            "configured foundation exceeds supported metadata bounds; no changes prepared".into(),
        );
    }
    let mut operations = vec![];
    if updated != source {
        operations.push(json!({"path":file,"expectedRevision":token_revision,"content":updated}));
    }
    if previous.as_ref() != Some(&proposed) {
        operations.push(json!({"path":FILE,"expectedRevision":baseline_revision,"content":format!("{}\n",serde_json::to_string_pretty(&proposed).map_err(|_|"cannot serialize configured foundation")?)}));
    }
    Ok(
        json!({"schemaVersion":1,"dryRun":true,"operations":operations,"changedTokens":values.keys().collect::<Vec<_>>(),"preserved":["unmodified source bytes","unknown baseline fields","fonts unless an explicit font token was selected","conditional token definitions","application data"],"verification":"native CSS parser and metadata only; validate actual contrast, mobile layout and states after applying"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct App(PathBuf);
    impl App {
        fn new(file: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "brain-design-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join(file),
                ":root{--accent:#123456;--font:'Project Sans',serif;--space-1:7px}",
            )
            .unwrap();
            Self(root)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn constructor_adopts_values_and_preserves_unknown_legacy_fields() {
        let old = json!({"schemaVersion":0,"brand":{"name":"Existing"},"tokens":{"--accent":{"description":"Keep me"}},"customRules":["keep"]});
        let next = baseline(":root{--accent:#123456}", "tokens.brain.css", Some(&old));
        assert_eq!(
            next["tokens"]["--accent"]["definitions"][0]["value"],
            "#123456"
        );
        assert_eq!(next["tokens"]["--accent"]["description"], "Keep me");
        assert_eq!(next["brand"], old["brand"]);
        assert_eq!(next["customRules"], old["customRules"]);
        assert_eq!(next["source"]["tokensFile"], "tokens.brain.css");
        assert_eq!(next["rules"]["minimumTouchCssPx"], 44);
    }
    #[test]
    fn init_is_a_revision_checked_preview_and_never_rewrites_legacy_tokens() {
        let app = App::new("tokens.brain.css");
        let before = std::fs::read(app.0.join("tokens.brain.css")).unwrap();
        let preview = preview_init(&app.0, &json!({})).unwrap();
        assert_eq!(preview["operations"].as_array().unwrap().len(), 1);
        assert_eq!(preview["operations"][0]["path"], FILE);
        assert!(preview["operations"][0]["expectedRevision"].is_null());
        assert!(!app.0.join(FILE).exists());
        assert_eq!(
            std::fs::read(app.0.join("tokens.brain.css")).unwrap(),
            before
        );
    }
    #[test]
    fn drift_is_a_warning_and_stale_adoption_expectations_fail() {
        let app = App::new("tokens.cssbrain");
        let preview = preview_init(&app.0, &json!({})).unwrap();
        std::fs::write(
            app.0.join(FILE),
            preview["operations"][0]["content"].as_str().unwrap(),
        )
        .unwrap();
        let revision = crate::changes::read(&app.0, FILE).unwrap()["revision"].clone();
        std::fs::write(app.0.join("tokens.cssbrain"), ":root{--accent:#abcdef}").unwrap();
        let result = check(&app.0).unwrap();
        assert_eq!(result["blocking"], false);
        assert!(!result["warnings"].as_array().unwrap().is_empty());
        std::fs::write(
            app.0.join(FILE),
            "{\"schemaVersion\":1,\"name\":\"Changed\"}",
        )
        .unwrap();
        assert!(preview_init(&app.0, &json!({"expectedRevision":revision})).is_err());
    }
    #[test]
    fn gallery_is_private_mobile_first_and_does_not_execute_app_logic() {
        let app = App::new("tokens.cssbrain");
        std::fs::create_dir(app.0.join("comp")).unwrap();
        std::fs::write(
            app.0.join("comp/card.brain"),
            "<script>throw Error('must not execute')</script><p>Card</p>",
        )
        .unwrap();
        let result = gallery(&app.0).unwrap();
        assert_eq!(result["private"], true);
        let html = std::fs::read_to_string(app.0.join(".brain/design/gallery.html")).unwrap();
        assert!(html.contains("minimum-scale") || html.contains("width=device-width"));
        assert!(html.contains("min-height:44px"));
        assert!(html.contains("font-src 'none'"));
        assert!(!html.contains("must not execute"));
        assert!(!app.0.join("dist").exists());
    }
    #[test]
    fn seed_preserves_existing_and_selects_explicit_field_dependency() {
        let app = App::new("tokens.cssbrain");
        std::fs::create_dir(app.0.join("ui")).unwrap();
        std::fs::write(
            app.0.join("ui/button.brain"),
            "<button>Existing brand</button>",
        )
        .unwrap();
        let preview = preview_seed(&app.0, &json!({"names":["button","input"]})).unwrap();
        let ops = preview["operations"].as_array().unwrap();
        assert_eq!(ops.len(), 2);
        let input = ops.iter().find(|o| o["path"] == "ui/input.brain").unwrap();
        assert!(
            input["content"]
                .as_str()
                .unwrap()
                .contains("import Field from './field.brain'")
        );
        assert!(input["content"].as_str().unwrap().contains("step:str='any'"));
        assert!(input["content"].as_str().unwrap().contains("step={type === 'number' ? step : null}"));
        assert_eq!(preview["preserved"][0]["file"], "ui/button.brain");
        assert!(!app.0.join("ui/input.brain").exists());
        let guide = blueprint(&app.0).unwrap();
        assert_eq!(guide["mandatoryBeforeUiEdit"], true);
        let input_contract=guide["components"].as_array().unwrap().iter().find(|component| component["file"]=="ui/input.brain").unwrap();
        assert!(input_contract["seedContract"]["props"].as_array().unwrap().iter().any(|prop| prop["name"]=="step" && prop["type"]=="str" && prop["optional"]==true));
        assert!(preview_seed(&app.0, &json!({"names":["../escape"]})).is_err());
    }
    #[test]
    fn controlled_configure_preserves_legacy_source_and_unknown_baseline_fields() {
        let app = App::new("tokens.brain.css");
        std::fs::write(app.0.join(FILE), r#"{"custom":{"keep":true}}"#).unwrap();
        let source = std::fs::read_to_string(app.0.join("tokens.brain.css")).unwrap();
        let preview = preview_configure(
            &app.0,
            &json!({"tokens":{"--accent":"#abcdef","--new-space":"12px"}}),
        )
        .unwrap();
        let ops = preview["operations"].as_array().unwrap();
        assert_eq!(ops.len(), 2);
        let css = ops[0]["content"].as_str().unwrap();
        assert!(css.contains("--font:'Project Sans',serif;--space-1:7px"));
        assert!(css.contains("--new-space: 12px;"));
        let baseline: Value = serde_json::from_str(ops[1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(baseline["custom"]["keep"], true);
        assert_eq!(
            ops[0]["expectedRevision"],
            crate::manifests::hash(source.as_bytes())
        );
        assert_eq!(
            std::fs::read_to_string(app.0.join("tokens.brain.css")).unwrap(),
            source
        );
        assert!(
            preview_configure(
                &app.0,
                &json!({"tokens":{"--accent":"red"},"expectedTokenRevision":"stale"})
            )
            .is_err()
        );
        for value in [
            "url(https://example.test)",
            "u\\72l(x)",
            "image-set('https://example.test/x' 1x)",
            "red; color: blue",
        ] {
            assert!(preview_configure(&app.0, &json!({"tokens":{"--accent":value}})).is_err());
        }
    }
    #[test]
    fn configure_refuses_to_flatten_conditional_or_repeated_definitions() {
        let app = App::new("tokens.cssbrain");
        std::fs::write(
            app.0.join("tokens.cssbrain"),
            ":root{--accent:red}@media(prefers-color-scheme:dark){:root{--accent:blue}}",
        )
        .unwrap();
        assert!(preview_configure(&app.0, &json!({"tokens":{"--accent":"green"}})).is_err());
    }
    #[test]
    fn project_basics_compile_with_current_compiler_and_native_tokens() {
        let app = App::new("tokens.cssbrain");
        std::fs::write(
            app.0.join("tokens.cssbrain"),
            include_str!("templates/tokens.cssbrain"),
        )
        .unwrap();
        std::fs::create_dir(app.0.join("ui")).unwrap();
        let basics = new_basics();
        for (file, source) in &basics {
            std::fs::write(app.0.join(file), source).unwrap();
        }
        for (file, source) in basics {
            crate::compiler::compile(&source, Path::new(file), &app.0, true)
                .unwrap_or_else(|errors| panic!("{file}: {errors:?}"));
        }
    }
}
