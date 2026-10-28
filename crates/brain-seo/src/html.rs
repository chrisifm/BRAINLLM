//! Tolerant HTML scanner for prerendered pages (no third-party parser, CONTRIBUTING invariant 1). It reads only what
//! SEO needs: head tags, headings, links, images, JSON-LD, ids, and the main content as Markdown and plain text.

pub type Attrs = Vec<(String, String)>;

/// Value of attribute `k` (keys are lowercase).
pub fn attr<'a>(a: &'a Attrs, k: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

#[derive(Debug, Clone, Default)]
pub struct Anchor {
    pub href: String,
    pub text: String,
    pub rel: String,
    /// Inside the main content (not nav/header/footer/aside).
    pub in_content: bool,
}

/// A `<video>` element: its own attributes plus the first `<source src>` inside it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoTag {
    pub src: Option<String>,
    pub poster: Option<String>,
    /// `title` or `aria-label`.
    pub title: Option<String>,
    /// `data-duration` (`PT1M30S`).
    pub duration: Option<String>,
    /// `data-upload-date`.
    pub upload: Option<String>,
    pub in_content: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub lang: Option<String>,
    pub title: Option<String>,
    /// Every `<meta>` in the document.
    pub metas: Vec<Attrs>,
    /// Every `<link>` in the document.
    pub links: Vec<Attrs>,
    /// `(level, text)` of every heading in the body.
    pub headings: Vec<(u8, String)>,
    /// Every `<img>` in the body, plus whether it sits in the main content.
    pub images: Vec<(Attrs, bool)>,
    pub anchors: Vec<Anchor>,
    /// Raw contents of every `<script type="application/ld+json">`.
    pub jsonld: Vec<String>,
    /// Every `id` (and `<a name>`) in the document: valid `#fragment` targets.
    pub ids: Vec<String>,
    /// Main content (`<main>`, or the body without nav/header/footer/aside) as Markdown.
    pub markdown: String,
    /// Main content as plain text (one space between words).
    pub text: String,
    /// Plain text of the first `<p>` in the main content.
    pub first_paragraph: String,
    /// `(question, answer)` of every `<details>` inside an element marked `faq` (INTERNALS §33 FAQPage).
    pub faq: Vec<(String, String)>,
    /// `datetime` of every `<time>` in the main content (visible dates).
    pub times: Vec<String>,
    pub videos: Vec<VideoTag>,
}

impl Doc {
    /// Content of `<meta name=k>` or `<meta property=k>` (first one).
    pub fn meta(&self, k: &str) -> Option<&str> {
        self.metas
            .iter()
            .find(|m| {
                attr(m, "name")
                    .or_else(|| attr(m, "property"))
                    .is_some_and(|n| n.eq_ignore_ascii_case(k))
            })
            .and_then(|m| attr(m, "content"))
    }
    /// `href` of the first `<link rel=rel>`.
    pub fn link(&self, rel: &str) -> Option<&str> {
        self.links
            .iter()
            .find(|l| {
                attr(l, "rel").is_some_and(|r| {
                    r.split_ascii_whitespace()
                        .any(|x| x.eq_ignore_ascii_case(rel))
                })
            })
            .and_then(|l| attr(l, "href"))
    }
    pub fn words(&self) -> usize {
        self.text.split_whitespace().count()
    }
}

enum Tok<'a> {
    Open(String, Attrs, bool),
    Close(String),
    Text(&'a str),
    /// script/style/textarea/title: raw content, no markup inside.
    Raw(String, Attrs, &'a str),
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];
/// Content that never reaches the Markdown or the word count.
const SKIP: &[&str] = &[
    "script", "style", "template", "noscript", "svg", "button", "select", "textarea", "dialog",
];
/// Page chrome: left out of the main content when the page has no `<main>`.
const CHROME: &[&str] = &["nav", "header", "footer", "aside"];
/// Phrasing elements: no word break around them in the plain text.
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "em", "i", "kbd", "label", "mark", "q",
    "s", "small", "span", "strong", "sub", "sup", "time", "u", "var",
];
const BLOCK: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "ul",
    "ol",
    "table",
    "figure",
    "figcaption",
    "details",
    "summary",
    "dl",
    "dt",
    "dd",
    "form",
    "fieldset",
    "blockquote",
    "address",
    "hgroup",
    "header",
    "footer",
    "nav",
    "aside",
];

fn tokens(src: &str) -> Vec<Tok<'_>> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text0 = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let next = b.get(i + 1).copied().unwrap_or(0);
        let markup = next.is_ascii_alphabetic() || next == b'/' || next == b'!' || next == b'?';
        if !markup {
            i += 1;
            continue;
        }
        if text0 < i {
            out.push(Tok::Text(&src[text0..i]));
        }
        if src[i..].starts_with("<!--") {
            i = src[i + 4..].find("-->").map_or(b.len(), |e| i + 4 + e + 3);
        } else if next == b'!' || next == b'?' {
            i = src[i..].find('>').map_or(b.len(), |e| i + e + 1);
        } else if next == b'/' {
            let n0 = i + 2;
            let mut n = n0;
            while n < b.len() && (b[n].is_ascii_alphanumeric() || b[n] == b'-') {
                n += 1;
            }
            out.push(Tok::Close(src[n0..n].to_ascii_lowercase()));
            i = src[n..].find('>').map_or(b.len(), |e| n + e + 1);
        } else {
            let n0 = i + 1;
            let mut n = n0;
            while n < b.len() && (b[n].is_ascii_alphanumeric() || b[n] == b'-' || b[n] == b':') {
                n += 1;
            }
            let name = src[n0..n].to_ascii_lowercase();
            // end of the tag, skipping `>` inside quoted values
            let mut e = n;
            let mut q = 0u8;
            while e < b.len() {
                match b[e] {
                    c if q != 0 => {
                        if c == q {
                            q = 0;
                        }
                    }
                    b'"' | b'\'' => q = b[e],
                    b'>' => break,
                    _ => {}
                }
                e += 1;
            }
            let inner = &src[n..e.min(b.len())];
            let self_closing = inner.trim_end().ends_with('/');
            let attrs = parse_attrs(inner);
            i = (e + 1).min(b.len());
            if matches!(name.as_str(), "script" | "style" | "textarea" | "title") && !self_closing {
                let close = format!("</{name}");
                let end = find_ci(&src[i..], &close).map_or(b.len(), |x| i + x);
                out.push(Tok::Raw(name, attrs, &src[i..end]));
                i = src[end..].find('>').map_or(b.len(), |x| end + x + 1);
            } else {
                out.push(Tok::Open(name, attrs, self_closing));
            }
        }
        text0 = i;
    }
    if text0 < b.len() {
        out.push(Tok::Text(&src[text0..]));
    }
    out
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    (0..h.len().saturating_sub(n.len() - 1)).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Attributes of a start tag (the text between the name and `>`). Keys lowercase, values entity-decoded.
pub fn parse_attrs(s: &str) -> Attrs {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let k0 = i;
        while i < b.len()
            && !b[i].is_ascii_whitespace()
            && b[i] != b'='
            && b[i] != b'/'
            && b[i] != b'>'
        {
            i += 1;
        }
        if k0 == i {
            break;
        }
        let key = s[k0..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let v0 = i + 1;
                i = v0;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                val = decode(&s[v0..i.min(b.len())]);
                i += 1;
            } else {
                let v0 = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                val = decode(&s[v0..i]);
            }
        }
        out.push((key, val));
    }
    out
}

/// Decodes the character references prerendered HTML contains (named basics and numeric).
pub fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut o = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(a) = rest.find('&') {
        o.push_str(&rest[..a]);
        rest = &rest[a..];
        let semi = rest[1..].find(';').map(|x| x + 1).filter(|&x| x <= 10);
        let decoded = semi.and_then(|x| {
            let name = &rest[1..x];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => match name.strip_prefix('#') {
                    Some(h) if h.starts_with(['x', 'X']) => u32::from_str_radix(&h[1..], 16)
                        .ok()
                        .and_then(char::from_u32),
                    Some(d) => d.parse().ok().and_then(char::from_u32),
                    None => None,
                },
            };
            c.map(|c| (c, x + 1))
        });
        match decoded {
            Some((c, len)) => {
                o.push(c);
                rest = &rest[len..];
            }
            None => {
                o.push('&');
                rest = &rest[1..];
            }
        }
    }
    o.push_str(rest);
    o
}

/// Markdown writer with HTML whitespace collapsing.
#[derive(Default)]
struct Md {
    out: String,
    /// A space is owed before the next word.
    space: bool,
    /// Right after a line start or an opening marker: owed spaces are dropped.
    fresh: bool,
}

impl Md {
    fn text(&mut self, s: &str) {
        if s.starts_with(char::is_whitespace) {
            self.space = true;
        }
        let mut words = s.split_whitespace().peekable();
        while let Some(w) = words.next() {
            if self.space && !self.fresh && !self.out.is_empty() {
                self.out.push(' ');
            }
            self.out.push_str(w);
            self.fresh = false;
            self.space = words.peek().is_some() || s.ends_with(char::is_whitespace);
        }
    }
    fn open(&mut self, marker: &str) {
        if self.space && !self.fresh && !self.out.is_empty() {
            self.out.push(' ');
        }
        self.out.push_str(marker);
        self.space = false;
        self.fresh = true;
    }
    fn close(&mut self, marker: &str) {
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        self.out.push_str(marker);
        self.fresh = false;
    }
    fn trim(&mut self) {
        while self.out.ends_with(' ') {
            self.out.pop();
        }
    }
    fn block(&mut self) {
        self.trim();
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push_str(if self.out.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            });
        }
        self.space = false;
        self.fresh = true;
    }
    fn line(&mut self) {
        self.trim();
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.space = false;
        self.fresh = true;
    }
}

/// Parses a page. Never fails: malformed markup yields whatever could be read.
pub fn parse(src: &str) -> Doc {
    let toks = tokens(src);
    let has_main = toks
        .iter()
        .any(|t| matches!(t, Tok::Open(n, ..) if n == "main"));
    let mut d = Doc::default();
    let mut stack: Vec<String> = Vec::new();
    let mut md = Md::default();
    let mut plain = String::new();
    // open anchors: (href, rel, in_content, text so far, markdown start index)
    let mut anchors: Vec<(String, String, bool, String)> = Vec::new();
    let mut heading: Option<(u8, String)> = None;
    let mut first_p: Option<String> = None;
    let mut lists: Vec<(bool, usize)> = Vec::new(); // (ordered, counter)
    let mut in_pre = 0usize;
    let mut faq_at: Option<usize> = None; // stack depth of the open `faq` group
    let mut qa: Option<(String, String, bool)> = None; // (question, answer, inside <summary>)
    let in_any = |stack: &[String], set: &[&str]| stack.iter().any(|s| set.contains(&s.as_str()));
    let content = |stack: &[String]| -> bool {
        let in_body = stack.iter().any(|s| s == "body") || !stack.iter().any(|s| s == "head");
        if !in_body || in_any(stack, SKIP) {
            return false;
        }
        if has_main {
            stack.iter().any(|s| s == "main")
        } else {
            !in_any(stack, CHROME)
        }
    };
    for t in &toks {
        match t {
            Tok::Open(name, attrs, self_closing) => {
                if !INLINE.contains(&name.as_str()) {
                    plain.push(' ');
                    if let Some(p) = first_p.as_mut() {
                        p.push(' ');
                    }
                    if let Some(q) = qa.as_mut() {
                        q.1.push(' ');
                    }
                }
                if faq_at.is_none() && !self_closing && attr(attrs, "faq").is_some() {
                    faq_at = Some(stack.len());
                }
                if faq_at.is_some() {
                    match name.as_str() {
                        "details" => qa = Some((String::new(), String::new(), false)),
                        "summary" => {
                            if let Some(q) = qa.as_mut() {
                                q.2 = true;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(id) = attr(attrs, "id").filter(|v| !v.is_empty()) {
                    d.ids.push(id.to_string());
                }
                let c = content(&stack);
                match name.as_str() {
                    "html" => d.lang = attr(attrs, "lang").map(str::to_string),
                    "meta" => d.metas.push(attrs.clone()),
                    "time" if c => d
                        .times
                        .push(attr(attrs, "datetime").unwrap_or("").to_string()),
                    "video" => {
                        let get =
                            |k: &str| attr(attrs, k).filter(|v| !v.is_empty()).map(str::to_string);
                        d.videos.push(VideoTag {
                            src: get("src"),
                            poster: get("poster"),
                            title: get("title").or_else(|| get("aria-label")),
                            duration: get("data-duration"),
                            upload: get("data-upload-date"),
                            in_content: c,
                        });
                    }
                    "source" if stack.iter().any(|x| x == "video") => {
                        if let (Some(v), Some(src)) = (d.videos.last_mut(), attr(attrs, "src")) {
                            v.src.get_or_insert_with(|| src.to_string());
                        }
                    }
                    "link" => d.links.push(attrs.clone()),
                    "img" => {
                        if c {
                            let alt = attr(attrs, "alt").unwrap_or("");
                            md.open(&format!(
                                "![{}]({})",
                                alt.replace(']', ""),
                                attr(attrs, "src").unwrap_or("")
                            ));
                            md.fresh = false;
                        }
                        d.images.push((attrs.clone(), c));
                    }
                    "a" => {
                        if let Some(n) = attr(attrs, "name").filter(|v| !v.is_empty()) {
                            d.ids.push(n.to_string());
                        }
                        let href = attr(attrs, "href").unwrap_or("").to_string();
                        if c && !href.is_empty() {
                            md.open("[");
                        }
                        anchors.push((
                            href,
                            attr(attrs, "rel").unwrap_or("").to_string(),
                            c,
                            String::new(),
                        ));
                    }
                    "br" if c => md.line(),
                    "hr" if c => {
                        md.block();
                        md.out.push_str("---");
                        md.block();
                    }
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        let lvl = name.as_bytes()[1] - b'0';
                        heading = Some((lvl, String::new()));
                        if c {
                            md.block();
                            md.open(&format!("{} ", "#".repeat(lvl as usize)));
                        }
                    }
                    "li" if c => {
                        md.line();
                        let depth = lists.len().saturating_sub(1);
                        let marker = match lists.last_mut() {
                            Some((true, n)) => {
                                *n += 1;
                                format!("{n}. ")
                            }
                            _ => "- ".to_string(),
                        };
                        md.open(&format!("{}{marker}", "  ".repeat(depth)));
                    }
                    "ul" | "ol" => {
                        if c && lists.is_empty() {
                            md.block();
                        }
                        lists.push((name == "ol", 0));
                    }
                    "pre" if c => {
                        md.block();
                        md.out.push_str("```\n");
                        in_pre += 1;
                    }
                    "strong" | "b" if c => md.open("**"),
                    "em" | "i" if c => md.open("*"),
                    "code" if c && in_pre == 0 => md.open("`"),
                    "tr" if c => md.line(),
                    "td" | "th" if c => md.open("| "),
                    "p" if c => {
                        md.block();
                        if first_p.is_none() {
                            first_p = Some(String::new());
                        }
                    }
                    n if c && BLOCK.contains(&n) => md.block(),
                    _ => {}
                }
                if !self_closing && !VOID.contains(&name.as_str()) {
                    stack.push(name.clone());
                }
            }
            Tok::Close(name) => {
                if !INLINE.contains(&name.as_str()) {
                    plain.push(' ');
                    if let Some(p) = first_p.as_mut() {
                        p.push(' ');
                    }
                }
                let Some(pos) = stack.iter().rposition(|s| s == name) else {
                    continue;
                };
                stack.truncate(pos);
                match name.as_str() {
                    "summary" => {
                        if let Some(q) = qa.as_mut() {
                            q.2 = false;
                        }
                    }
                    "details" => {
                        if let Some((q, a, _)) = qa.take() {
                            let (q, a) = (
                                q.split_whitespace().collect::<Vec<_>>().join(" "),
                                a.split_whitespace().collect::<Vec<_>>().join(" "),
                            );
                            if !q.is_empty() && !a.is_empty() {
                                d.faq.push((q, a));
                            }
                        }
                    }
                    _ => {}
                }
                if faq_at.is_some_and(|depth| stack.len() <= depth) {
                    faq_at = None;
                }
                let c = content(&stack) && !SKIP.contains(&name.as_str());
                match name.as_str() {
                    "a" => {
                        if let Some((href, rel, in_c, text)) = anchors.pop() {
                            if c && !href.is_empty() {
                                md.close(&format!("]({href})"));
                            }
                            d.anchors.push(Anchor {
                                href,
                                text: text.split_whitespace().collect::<Vec<_>>().join(" "),
                                rel,
                                in_content: in_c,
                            });
                        }
                    }
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        if let Some((lvl, text)) = heading.take() {
                            d.headings
                                .push((lvl, text.split_whitespace().collect::<Vec<_>>().join(" ")));
                        }
                        if c {
                            md.block();
                        }
                    }
                    "ul" | "ol" => {
                        lists.pop();
                        if c && lists.is_empty() {
                            md.block();
                        }
                    }
                    "pre" if c => {
                        in_pre = in_pre.saturating_sub(1);
                        if !md.out.ends_with('\n') {
                            md.out.push('\n');
                        }
                        md.out.push_str("```");
                        md.block();
                    }
                    "strong" | "b" if c => md.close("**"),
                    "em" | "i" if c => md.close("*"),
                    "code" if c && in_pre == 0 => md.close("`"),
                    "td" | "th" if c => md.close(" "),
                    "p" if c => {
                        md.block();
                        if let Some(p) = first_p.as_mut()
                            && !p.is_empty()
                            && d.first_paragraph.is_empty()
                        {
                            d.first_paragraph = p.split_whitespace().collect::<Vec<_>>().join(" ");
                        }
                    }
                    n if c && BLOCK.contains(&n) => md.block(),
                    _ => {}
                }
            }
            Tok::Text(s) => {
                let s = decode(s);
                if let Some((_, h)) = heading.as_mut() {
                    h.push_str(&s);
                }
                for a in anchors.iter_mut() {
                    a.3.push_str(&s);
                }
                if let Some(q) = qa.as_mut() {
                    if q.2 {
                        q.0.push_str(&s)
                    } else {
                        q.1.push_str(&s)
                    }
                }
                if content(&stack) {
                    if in_pre > 0 {
                        md.out.push_str(&s);
                    } else {
                        md.text(&s);
                    }
                    plain.push_str(&s);
                    if let Some(p) = first_p.as_mut()
                        && d.first_paragraph.is_empty()
                        && stack.iter().any(|x| x == "p")
                    {
                        p.push_str(&s);
                    }
                }
            }
            Tok::Raw(name, attrs, body) => match name.as_str() {
                "title" if d.title.is_none() && !stack.iter().any(|s| s == "svg") => {
                    d.title = Some(
                        decode(body)
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                }
                "script"
                    if attr(attrs, "type")
                        .is_some_and(|t| t.eq_ignore_ascii_case("application/ld+json")) =>
                {
                    d.jsonld.push(body.to_string());
                }
                _ => {}
            },
        }
    }
    md.trim();
    d.markdown = md.out.trim().to_string();
    d.text = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Tasks &amp; more</title>
<meta name="description" content="Plan &quot;today&quot;"><link rel="canonical" href="https://x.dev/tasks">
<script type="application/ld+json">{"@type":"WebPage"}</script><style>p > a { color: red }</style></head>
<body><header><nav><a href="/">Home</a></nav></header><main id="top"><h1>My <em>tasks</em></h1>
<p>First  paragraph with a <a href="/about" rel="nofollow">link</a>.</p><ul><li>One</li><li>Two<ol><li>a</li></ol></li></ul>
<img src="/a.png" alt="Chart"><pre>let x = 1;
x++</pre><!-- <a href="/hidden"> --><script>if (a < b) {}</script></main><footer><a href="#top">Up</a></footer></body></html>"##;

    #[test]
    fn reads_head_and_body() {
        let d = parse(PAGE);
        assert_eq!(d.lang.as_deref(), Some("en"));
        assert_eq!(d.title.as_deref(), Some("Tasks & more"));
        assert_eq!(d.meta("description"), Some("Plan \"today\""));
        assert_eq!(d.link("canonical"), Some("https://x.dev/tasks"));
        assert_eq!(d.jsonld, vec![r#"{"@type":"WebPage"}"#]);
        assert_eq!(d.headings, vec![(1, "My tasks".to_string())]);
        assert_eq!(d.first_paragraph, "First paragraph with a link.");
        assert!(d.ids.contains(&"top".to_string()));
        let hrefs: Vec<_> = d
            .anchors
            .iter()
            .map(|a| (a.href.as_str(), a.in_content))
            .collect();
        assert_eq!(hrefs, vec![("/", false), ("/about", true), ("#top", false)]);
        assert_eq!(d.anchors[1].rel, "nofollow");
        assert_eq!(d.images.len(), 1);
        assert!(!d.text.contains("Home") && !d.text.contains("Up") && !d.text.contains("if (a"));
    }

    #[test]
    fn main_as_markdown() {
        let d = parse(PAGE);
        assert_eq!(
            d.markdown,
            "# My *tasks*\n\nFirst paragraph with a [link](/about).\n\n- One\n- Two\n  1. a\n\n![Chart](/a.png)\n\n```\nlet x = 1;\nx++\n```"
        );
    }

    #[test]
    fn without_main_drops_chrome() {
        let d = parse(
            "<body><nav><a href='/'>Home</a></nav><h2>Hi</h2><p>Body text</p><footer>c</footer></body>",
        );
        assert_eq!(d.markdown, "## Hi\n\nBody text");
        assert_eq!(d.words(), 3);
    }

    #[test]
    fn faq_groups() {
        let d = parse(
            "<main><section faq data-b=x><h2>FAQ</h2><details><summary>Is it free?</summary><p>Yes, for up to <b>ten</b> people.</p><p>Then flat.</p></details>\
<details><summary>Offline?</summary>Fully.</details></section><details><summary>Not FAQ</summary>x</details></main>",
        );
        assert_eq!(
            d.faq,
            vec![
                (
                    "Is it free?".into(),
                    "Yes, for up to ten people. Then flat.".into()
                ),
                ("Offline?".into(), "Fully.".into())
            ]
        );
    }

    #[test]
    fn entities_and_attrs() {
        assert_eq!(
            decode("a &amp; b &#233; &#x41; &bogus; &"),
            "a & b \u{e9} A &bogus; &"
        );
        let a = parse_attrs(r#" href="/x?a=1&amp;b=2" data-x='>' hidden async=yes"#);
        assert_eq!(attr(&a, "href"), Some("/x?a=1&b=2"));
        assert_eq!(attr(&a, "data-x"), Some(">"));
        assert_eq!(attr(&a, "hidden"), Some(""));
        assert_eq!(attr(&a, "async"), Some("yes"));
    }
}
