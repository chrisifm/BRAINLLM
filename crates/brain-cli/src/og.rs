//! Social images for brain-seo (INTERNALS §33): the `Shoot` closure that turns `views/_og.brain` (or brain-seo's
//! built-in template) plus a page's props into a 1200×630 PNG. The template is compiled like any component and
//! mounted in headless Chrome with no server: its runtime import is rewritten to a `data:` module, the tokens and its
//! scoped CSS are inlined, fonts are awaited, and the screenshot is taken at deviceScaleFactor 1 on an opaque
//! background. Chrome starts once per build, on the first image that is not in brain-seo's cache.

use brain_mcp::Json;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

pub struct Shooter {
    root: PathBuf,
    chrome: Mutex<Option<(crate::cdp::Chrome, String)>>,
}

fn data_js(src: &str) -> String {
    let mut o = String::from("data:text/javascript,");
    for b in src.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'(' | b')' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

impl Shooter {
    pub fn new(root: &Path) -> Shooter {
        Shooter { root: root.to_path_buf(), chrome: Mutex::new(None) }
    }

    /// Renders `tpl` with `props` to PNG bytes (exactly 1200×630).
    pub fn shoot(&self, props: &[(String, String)], tpl: &str) -> Result<Vec<u8>, String> {
        let out = crate::compiler::compile(tpl, Path::new("views/_og.brain"), &self.root, false)
            .map_err(|ds| format!("views/_og.brain does not compile: {}", ds.iter().map(|d| format!("{} {}", d.code, d.msg)).collect::<Vec<_>>().join("; ")))?;
        let rt = crate::runtime_files();
        let core = rt.iter().find(|(n, _)| n == "brain.js").map(|(_, s)| s.clone()).ok_or("runtime brain.js is not embedded")?;
        let core_url = data_js(&core);
        // the component and the page script import the same data: URL, so they share one runtime instance
        let mut js = out.js.replace("\"/@brain/brain.js\"", &brain_serve::json_str(&core_url));
        for (n, src) in rt.iter().filter(|(n, _)| n.ends_with(".js") && !n.contains('/') && n != "brain.js") {
            let pat = format!("\"/@brain/{n}\"");
            if js.contains(&pat) {
                js = js.replace(&pat, &brain_serve::json_str(&data_js(&src.replace("'./brain.js'", &format!("'{core_url}'")))));
            }
        }
        let tokens = std::fs::read_to_string(self.root.join("tokens.cssbrain")).unwrap_or_default();
        let props_json = Json::Obj(props.iter().map(|(k, v)| (k.clone(), Json::str(v))).collect()).stringify();
        let page = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><style>{}</style><style>{tokens}</style><style>{}</style><style>html,body{{margin:0;width:1200px;height:630px;overflow:hidden}}</style></head><body><div id=\"og\"></div><script type=\"module\">import{{mount}}from{};import C from{};mount(C,document.getElementById('og'),{props_json});await document.fonts.ready;window.__brainOg=true</script></body></html>",
            brain_css::LAYER_ORDER,
            out.css,
            brain_serve::json_str(&core_url),
            brain_serve::json_str(&data_js(&js)),
        );
        let mut guard = self.chrome.lock().unwrap();
        if guard.is_none() {
            let mut c = crate::cdp::Chrome::launch(Duration::from_secs(120)).map_err(|e| e.to_string())?;
            let s = c.new_page()?;
            c.call(Some(&s), "Emulation.setDeviceMetricsOverride", Json::obj([("width", Json::Num(1200.0)), ("height", Json::Num(630.0)), ("deviceScaleFactor", Json::Num(1.0)), ("mobile", Json::Bool(false))]))?;
            c.call(Some(&s), "Emulation.setDefaultBackgroundColorOverride", Json::obj([("color", Json::obj([("r", Json::Num(255.0)), ("g", Json::Num(255.0)), ("b", Json::Num(255.0)), ("a", Json::Num(1.0))]))]))?;
            *guard = Some((c, s));
        }
        let (c, s) = guard.as_mut().unwrap();
        c.call(Some(s), "Page.navigate", Json::obj([("url", Json::str(format!("data:text/html;charset=utf-8,{}", pct(&page))))]))?;
        let mut ready = false;
        for _ in 0..200 {
            if matches!(c.eval(s, "window.__brainOg === true"), Ok(Json::Bool(true))) {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !ready {
            let err = c.eval(s, "document.body ? document.body.innerText.slice(0, 200) : ''").unwrap_or(Json::Null);
            return Err(format!("views/_og.brain did not render in 5 s ({})", err.as_str().unwrap_or("no output")));
        }
        let shot = c.call(
            Some(s),
            "Page.captureScreenshot",
            Json::obj([("format", Json::str("png")), ("clip", Json::obj([("x", Json::Num(0.0)), ("y", Json::Num(0.0)), ("width", Json::Num(1200.0)), ("height", Json::Num(630.0)), ("scale", Json::Num(1.0))]))]),
        )?;
        let data = shot.get("data").and_then(Json::as_str).ok_or("captureScreenshot without data")?;
        crate::browser::base64_decode(data).ok_or_else(|| "invalid base64 PNG".to_string())
    }
}

fn pct(s: &str) -> String {
    let mut o = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}
