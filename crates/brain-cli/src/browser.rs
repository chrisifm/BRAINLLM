//! Debug browser for `brain mcp` (INTERNALS §24): a persistent headless Chrome, driven by the same CDP client as
//! `brain test`, that only navigates to the local dev server (127.0.0.1). Used to reproduce a bug:
//! navigate → click/type → eval → shot.

use crate::cdp::Chrome;
use crate::testrun::{audit, AxIssue};
use brain_mcp::Json;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Browser {
    chrome: Chrome,
    session: String,
    /// Dev server origin; every navigation stays inside it.
    origin: String,
}

impl Browser {
    pub fn open(origin: &str) -> Result<Browser, String> {
        if !(origin.starts_with("http://127.0.0.1:") || origin.starts_with("http://localhost:")) {
            return Err(format!("the debug browser only opens the local dev server, not `{origin}`"));
        }
        // Long session: the watchdog only guards against a Chrome hung for hours.
        let mut chrome = Chrome::launch(Duration::from_secs(12 * 3600)).map_err(|e| e.to_string())?;
        // one tool call never blocks the MCP server: each CDP answer has 30 s at most (QA #294)
        chrome.deadline = Some(Duration::from_secs(30));
        let session = chrome.new_page()?;
        for d in ["Runtime.enable", "Page.enable", "Accessibility.enable"] {
            chrome.call(Some(&session), d, Json::Obj(vec![]))?;
        }
        Ok(Browser { chrome, session, origin: origin.trim_end_matches('/').to_string() })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Navigates to `route` (a dev server path, e.g. `/users/42?tab=x`) and waits for the view to mount.
    pub fn navigate(&mut self, route: &str) -> Result<Json, String> {
        if !route.starts_with('/') || route.starts_with("//") {
            return Err(format!("`{route}` is not a local route; fix: pass something like `/users/42`"));
        }
        let url = format!("{}{route}", self.origin);
        self.chrome.call(Some(&self.session), "Page.navigate", Json::obj([("url", Json::str(&url))]))?;
        for _ in 0..300 {
            if self.chrome.eval(&self.session, "document.readyState === 'complete'")? == Json::Bool(true) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // two frames: the view modules have run and mounted
        self.chrome.eval(&self.session, "new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true))))")?;
        let info = self.chrome.eval(
            &self.session,
            "({ url: location.pathname + location.search, title: document.title, h1: document.querySelector('h1')?.textContent?.trim() ?? null, overlay: document.getElementById('__brain_overlay')?.textContent ?? null })",
        )?;
        Ok(info)
    }

    /// Real click (CDP mouse events) at the center of the element.
    pub fn click(&mut self, selector: &str) -> Result<Json, String> {
        let pos = self.chrome.eval(
            &self.session,
            &format!(
                "(() => {{ const el = document.querySelector({}); if (!el) throw new Error('no element matches the selector'); el.scrollIntoView({{block:'center'}}); const r = el.getBoundingClientRect(); return {{ x: r.x + r.width / 2, y: r.y + r.height / 2, tag: el.tagName.toLowerCase(), text: el.textContent.trim().slice(0, 80) }} }})()",
                Json::str(selector).stringify()
            ),
        ).map_err(|e| format!("`{selector}`: {e}"))?;
        let (x, y) = match (pos.get("x"), pos.get("y")) {
            (Some(Json::Num(x)), Some(Json::Num(y))) => (*x, *y),
            _ => return Err("could not locate the element".into()),
        };
        for t in ["mousePressed", "mouseReleased"] {
            self.chrome.call(
                Some(&self.session),
                "Input.dispatchMouseEvent",
                Json::obj([("type", Json::str(t)), ("x", Json::Num(x)), ("y", Json::Num(y)), ("button", Json::str("left")), ("clickCount", Json::Num(1.0))]),
            )?;
        }
        self.settle()?;
        Ok(pos)
    }

    /// Focuses the element, clears its value and types `text` the way a keyboard would (Input.insertText).
    pub fn type_text(&mut self, selector: &str, text: &str) -> Result<Json, String> {
        self.chrome
            .eval(
                &self.session,
                &format!(
                    "(() => {{ const el = document.querySelector({}); if (!el) throw new Error('no element matches the selector'); el.focus(); if ('value' in el) {{ el.value = ''; el.dispatchEvent(new Event('input', {{bubbles:true}})) }} return true }})()",
                    Json::str(selector).stringify()
                ),
            )
            .map_err(|e| format!("`{selector}`: {e}"))?;
        self.chrome.call(Some(&self.session), "Input.insertText", Json::obj([("text", Json::str(text))]))?;
        self.settle()?;
        self.chrome.eval(&self.session, &format!("document.querySelector({})?.value ?? null", Json::str(selector).stringify()))
    }

    /// Evaluates an expression in the live page (await allowed) and returns its JSON value.
    pub fn eval(&mut self, expr: &str) -> Result<Json, String> {
        self.chrome.eval(&self.session, expr)
    }

    /// The modal dialogs (alert/confirm/prompt/beforeunload) the page opened since the last call; each was dismissed
    /// (confirm → false), so an agent learns why a navigation did not happen.
    pub fn take_dialogs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.chrome.dialogs)
    }

    /// PNG capture of `route` at the given viewport + accessibility tree audit. Saved under `.brain/shots/`.
    pub fn shot(&mut self, root: &Path, route: &str, width: u32, height: u32) -> Result<(PathBuf, Vec<AxIssue>, Json), String> {
        self.chrome.call(
            Some(&self.session),
            "Emulation.setDeviceMetricsOverride",
            Json::obj([
                ("width", Json::Num(width as f64)),
                ("height", Json::Num(height as f64)),
                ("deviceScaleFactor", Json::Num(1.0)),
                ("mobile", Json::Bool(width < 600)),
            ]),
        )?;
        let info = self.navigate(route)?;
        let shot = self.chrome.call(Some(&self.session), "Page.captureScreenshot", Json::obj([("format", Json::str("png"))]))?;
        let data = shot.get("data").and_then(Json::as_str).ok_or("captureScreenshot returned no data")?;
        let png = base64_decode(data).ok_or("invalid base64 PNG")?;
        let name = route.trim_matches('/').replace(['/', '?', '&', '='], "_");
        let file = root.join(".brain/shots").join(format!("{}-{width}x{height}.png", if name.is_empty() { "home" } else { &name }));
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&file, png).map_err(|e| e.to_string())?;
        let mut issues = Vec::new();
        audit(&mut self.chrome, &self.session, route, &mut issues)?;
        Ok((file, issues, info))
    }

    fn settle(&mut self) -> Result<(), String> {
        self.chrome.eval(&self.session, "new Promise(r => requestAnimationFrame(() => setTimeout(() => r(true), 0)))").map(|_| ())
    }
}

pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        let mut k: usize = 0;
        for &c in chunk {
            if c == b'=' {
                break;
            }
            n = n << 6 | val(c)?;
            k += 1;
        }
        n <<= 6 * (4 - k);
        let b = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&b[..k.saturating_sub(1)]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_roundtrip() {
        assert_eq!(super::base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(super::base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(super::base64_decode("Zg==").unwrap(), b"f");
    }
}
