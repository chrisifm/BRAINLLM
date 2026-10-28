//! INTERNALS §8.1: colors and spacing only through tokens.
//! - B0304 (error): a literal color in a color property; the edit points at the token with the same value, or the
//!   nearest color token.
//! - B0305: a literal length in a spacing property. Equal to a token value → info (normalized to the token);
//!   anything else → warning. `0`, `auto`, percentages, keywords and `var()`/`calc()` contents are allowed.
//! - B0408: `opacity` below 1 on a rule that sets a text color (its contrast can no longer be verified).

use crate::{parse_hex, Checker};
use brain_css::CssItem;
use brain_parse::Span;

const COLOR_PROPS: &[&str] = &[
    "color", "background", "background-color", "border", "border-top", "border-right", "border-bottom", "border-left",
    "border-color", "border-top-color", "border-right-color", "border-bottom-color", "border-left-color",
    "border-block", "border-inline", "border-block-color", "border-inline-color", "outline", "outline-color", "fill",
    "stroke", "caret-color", "text-decoration-color", "accent-color", "column-rule-color",
];

const COLOR_FNS: &[&str] = &["rgb(", "rgba(", "hsl(", "hsla(", "hwb(", "lab(", "lch(", "oklab(", "oklch(", "color("];

/// Named colors with their value (the rest of the CSS list is rare enough to only be reported, without an edit).
const NAMED: &[(&str, &str)] = &[
    ("black", "#000000"), ("white", "#ffffff"), ("red", "#ff0000"), ("green", "#008000"), ("blue", "#0000ff"),
    ("yellow", "#ffff00"), ("orange", "#ffa500"), ("purple", "#800080"), ("pink", "#ffc0cb"), ("gray", "#808080"),
    ("grey", "#808080"), ("silver", "#c0c0c0"), ("maroon", "#800000"), ("navy", "#000080"), ("teal", "#008080"),
    ("olive", "#808000"), ("lime", "#00ff00"), ("aqua", "#00ffff"), ("cyan", "#00ffff"), ("fuchsia", "#ff00ff"),
    ("magenta", "#ff00ff"), ("brown", "#a52a2a"), ("gold", "#ffd700"), ("indigo", "#4b0082"), ("violet", "#ee82ee"),
    ("crimson", "#dc143c"), ("coral", "#ff7f50"), ("salmon", "#fa8072"), ("tomato", "#ff6347"), ("khaki", "#f0e68c"),
    ("beige", "#f5f5dc"), ("ivory", "#fffff0"), ("lavender", "#e6e6fa"), ("turquoise", "#40e0d0"), ("tan", "#d2b48c"),
    ("darkgray", "#a9a9a9"), ("darkgrey", "#a9a9a9"), ("lightgray", "#d3d3d3"), ("lightgrey", "#d3d3d3"),
    ("dimgray", "#696969"), ("whitesmoke", "#f5f5f5"), ("gainsboro", "#dcdcdc"), ("darkblue", "#00008b"),
    ("darkred", "#8b0000"), ("darkgreen", "#006400"), ("skyblue", "#87ceeb"), ("steelblue", "#4682b4"),
    ("royalblue", "#4169e1"), ("slategray", "#708090"), ("orchid", "#da70d6"), ("plum", "#dda0dd"), ("chocolate", "#d2691e"),
];

/// Other CSS named colors: reported, no edit.
const OTHER_NAMED: &[&str] = &[
    "aliceblue", "antiquewhite", "aquamarine", "azure", "bisque", "blanchedalmond", "blueviolet", "burlywood", "cadetblue",
    "chartreuse", "cornflowerblue", "cornsilk", "darkcyan", "darkgoldenrod", "darkkhaki", "darkmagenta", "darkolivegreen",
    "darkorange", "darkorchid", "darksalmon", "darkseagreen", "darkslateblue", "darkslategray", "darkslategrey",
    "darkturquoise", "darkviolet", "deeppink", "deepskyblue", "dimgrey", "dodgerblue", "firebrick", "floralwhite",
    "forestgreen", "ghostwhite", "goldenrod", "greenyellow", "honeydew", "hotpink", "indianred", "lavenderblush",
    "lawngreen", "lemonchiffon", "lightblue", "lightcoral", "lightcyan", "lightgoldenrodyellow", "lightgreen", "lightpink",
    "lightsalmon", "lightseagreen", "lightskyblue", "lightslategray", "lightslategrey", "lightsteelblue", "lightyellow",
    "limegreen", "linen", "mediumaquamarine", "mediumblue", "mediumorchid", "mediumpurple", "mediumseagreen",
    "mediumslateblue", "mediumspringgreen", "mediumturquoise", "mediumvioletred", "midnightblue", "mintcream", "mistyrose",
    "moccasin", "navajowhite", "oldlace", "olivedrab", "orangered", "palegoldenrod", "palegreen", "paleturquoise",
    "palevioletred", "papayawhip", "peachpuff", "peru", "powderblue", "rebeccapurple", "rosybrown", "saddlebrown",
    "sandybrown", "seagreen", "seashell", "sienna", "slateblue", "slategrey", "snow", "springgreen", "thistle", "wheat",
    "yellowgreen",
];

fn is_spacing(prop: &str) -> bool {
    matches!(prop, "gap" | "row-gap" | "column-gap") || ["padding", "margin", "inset", "scroll-padding", "scroll-margin"].iter().any(|p| prop == *p || prop.starts_with(&format!("{p}-")))
}

/// One literal found in a declaration value: byte range inside the value and its text.
struct Lit {
    start: usize,
    end: usize,
    text: String,
}

/// Top-level words of a CSS value, skipping the contents of `var()`/`calc()`/`env()` and any other function except
/// the color functions (returned whole).
fn words(v: &str) -> Vec<Lit> {
    let b = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() || c == ',' || c == '/' {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && !(b[i] as char).is_whitespace() && b[i] != b',' && b[i] != b'(' && b[i] != b'/' {
            i += 1;
        }
        if i < b.len() && b[i] == b'(' {
            // Skip to the matching parenthesis.
            let mut depth = 0;
            while i < b.len() {
                match b[i] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        out.push(Lit { start, end: i, text: v[start..i].to_string() });
    }
    out
}

fn is_color_literal(w: &str) -> bool {
    let l = w.to_ascii_lowercase();
    l.starts_with('#') || COLOR_FNS.iter().any(|f| l.starts_with(f)) || NAMED.iter().any(|(n, _)| *n == l) || OTHER_NAMED.contains(&l.as_str())
}

/// Hex of a literal color when it is cheap to know (hex, `rgb()`/`rgba()` with integers, common names).
fn literal_hex(w: &str) -> Option<(f64, f64, f64)> {
    let l = w.to_ascii_lowercase();
    if let Some((_, h)) = NAMED.iter().find(|(n, _)| *n == l) {
        return parse_hex(h);
    }
    if l.starts_with('#') {
        return parse_hex(&l);
    }
    let inner = l.strip_prefix("rgba(").or_else(|| l.strip_prefix("rgb("))?.strip_suffix(')')?;
    let n: Vec<f64> = inner.split([',', ' ', '/']).filter(|x| !x.is_empty()).take(3).filter_map(|x| x.parse::<f64>().ok()).collect();
    (n.len() == 3).then(|| (n[0] / 255.0, n[1] / 255.0, n[2] / 255.0))
}

fn is_length_literal(w: &str) -> bool {
    let w = w.trim_start_matches(['-', '+']);
    let digits = w.len() - w.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.').len();
    if digits == 0 {
        return false;
    }
    let unit = &w[digits..];
    let num: f64 = w[..digits].parse().unwrap_or(1.0);
    num != 0.0 && !unit.is_empty() && unit != "%" && unit.chars().all(|c| c.is_ascii_alphabetic())
}

impl Checker<'_> {
    /// §8.1 over the expanded sheet. `tokens` = `None` when the app has no tokens file (no edits then).
    pub(crate) fn literals(&mut self, items: &[CssItem], tokens: Option<&[(String, String)]>) {
        for it in items {
            match it {
                CssItem::Rule { items: inner, .. } => {
                    self.opacity_on_text(inner);
                    for d in inner {
                        if let CssItem::Decl { name, value, span } = d {
                            self.literal_decl(name, value, *span, tokens);
                        }
                    }
                    self.literals(inner, tokens);
                }
                CssItem::At { block: Some(b), .. } => self.literals(b, tokens),
                _ => {}
            }
        }
    }

    fn opacity_on_text(&mut self, decls: &[CssItem]) {
        let has_color = decls.iter().any(|d| matches!(d, CssItem::Decl { name, .. } if name == "color"));
        for d in decls {
            if let CssItem::Decl { name, value, span } = d
                && name == "opacity"
                && has_color
            {
                let v = value.trim();
                let n = v.strip_suffix('%').map(|p| p.parse::<f64>().map(|x| x / 100.0)).unwrap_or_else(|| v.parse::<f64>());
                if n.is_ok_and(|n| n < 1.0) {
                    self.push(
                        "B0408",
                        *span,
                        format!("`opacity: {v}` on a rule that sets a text color: the contrast can no longer be verified"),
                        "use a text color token that is already muted, for example `color: var(--muted)`, and remove `opacity`",
                    );
                }
            }
        }
    }

    fn literal_decl(&mut self, name: &str, value: &str, span: Span, tokens: Option<&[(String, String)]>) {
        if name.starts_with("--") {
            return;
        }
        let decl = self.text(span).to_string();
        // Offset of the value inside the declaration text (the expanded value equals its source text).
        let Some(voff) = decl.find(':').map(|c| c + 1).and_then(|after| decl[after..].find(value.trim()).map(|i| after + i)) else { return };
        let base = span.start as usize + voff;
        if COLOR_PROPS.contains(&name) {
            let lits: Vec<Lit> = words(value.trim()).into_iter().filter(|w| is_color_literal(&w.text)).collect();
            if let Some(first) = lits.first() {
                let tokens = tokens.unwrap_or(&[]);
                let mut edits = Vec::new();
                let mut best_name = None;
                for l in &lits {
                    // A translucent veil behind content (overlays, modals) is the `--scrim` token (SPEC «Tokens»).
                    if matches!(name, "background" | "background-color") && translucent(&l.text) {
                        edits.push(((base + l.start) as u32, (base + l.end) as u32, "var(--scrim)".to_string()));
                        best_name.get_or_insert("--scrim");
                        continue;
                    }
                    if let Some(rgb) = literal_hex(&l.text)
                        && let Some((tn, _)) = nearest_color(rgb, tokens)
                    {
                        edits.push(((base + l.start) as u32, (base + l.end) as u32, format!("var({tn})")));
                        best_name.get_or_insert(tn);
                    }
                }
                let fix = match best_name {
                    Some(t) => format!("use a color token instead of a literal: `var({t})`"),
                    None => {
                        let names: Vec<&str> = tokens.iter().filter(|(_, v)| parse_hex(v).is_some()).map(|(n, _)| n.as_str()).collect();
                        if names.is_empty() { "declare the color in tokens.cssbrain and use `var(--name)`".to_string() } else { format!("use a color token: {}", names.join(", ")) }
                    }
                };
                let edits = if edits.len() == lits.len() { edits } else { Vec::new() };
                self.push_edits("B0304", span, format!("literal color `{}` in `{name}`: colors come from tokens", first.text), fix, edits);
            }
        } else if is_spacing(name) {
            let lits: Vec<Lit> = words(value.trim()).into_iter().filter(|w| is_length_literal(&w.text)).collect();
            let Some(first) = lits.first() else { return };
            let tokens = tokens.unwrap_or(&[]);
            let matched: Vec<Option<&str>> = lits.iter().map(|l| tokens.iter().find(|(n, v)| n.starts_with("--space") && v.trim() == l.text).or_else(|| tokens.iter().find(|(_, v)| v.trim() == l.text)).map(|(n, _)| n.as_str())).collect();
            if matched.iter().all(Option::is_some) {
                let edits: Vec<(u32, u32, String)> = lits.iter().zip(&matched).map(|(l, t)| ((base + l.start) as u32, (base + l.end) as u32, format!("var({})", t.unwrap_or_default()))).collect();
                let t = matched[0].unwrap_or_default();
                self.push_info("B0305", span, format!("literal `{}` in `{name}` equals `{t}` (normalized to the token)", first.text), format!("write `var({t})`"), edits);
            } else {
                let names: Vec<&str> = tokens.iter().filter(|(n, _)| n.starts_with("--space")).map(|(n, _)| n.as_str()).collect();
                let fix = if names.is_empty() { "use a spacing token (`var(--space-N)`) declared in tokens.cssbrain".to_string() } else { format!("use a spacing token: {}", names.join(", ")) };
                self.push("B0305", span, format!("literal spacing `{}` in `{name}`: spacing comes from tokens", first.text), fix);
            }
        }
    }
}

/// Exact token match first, else the closest color token by RGB distance.
fn nearest_color(c: (f64, f64, f64), tokens: &[(String, String)]) -> Option<(&str, f64)> {
    tokens
        .iter()
        .filter_map(|(n, v)| parse_hex(v).map(|t| (n.as_str(), (t.0 - c.0).powi(2) + (t.1 - c.1).powi(2) + (t.2 - c.2).powi(2))))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// `rgb(0 0 0 / .5)`, `rgba(0,0,0,0.4)`, `hsl(…/ 50%)`, `#0008`, `#00000080`: a color with alpha below 1.
fn translucent(w: &str) -> bool {
    let l = w.to_ascii_lowercase();
    if let Some(h) = l.strip_prefix('#') {
        return match h.len() {
            4 => !h.ends_with('f'),
            8 => !h.ends_with("ff"),
            _ => false,
        };
    }
    let Some(inner) = ["rgba(", "rgb(", "hsla(", "hsl("].iter().find_map(|f| l.strip_prefix(f)).and_then(|x| x.strip_suffix(')')) else { return false };
    let alpha = match inner.split_once('/') {
        Some((_, a)) => Some(a.trim()),
        None => {
            let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
            (parts.len() == 4).then(|| parts[3])
        }
    };
    let Some(a) = alpha else { return false };
    let v = match a.strip_suffix('%') {
        Some(p) => p.parse::<f64>().map(|x| x / 100.0),
        None => a.parse::<f64>(),
    };
    v.is_ok_and(|v| v < 1.0)
}
