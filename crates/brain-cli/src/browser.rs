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
        // Page.navigate can acknowledge before the old document is replaced. Its readyState may still be
        // complete, so waiting on readyState alone occasionally types into the page we are leaving.
        self.chrome.eval(&self.session, "globalThis.__brainMcpNavigationPending = true")?;
        let nav = self.chrome.call(Some(&self.session), "Page.navigate", Json::obj([("url", Json::str(&url))]))?;
        if let Some(error) = nav.get("errorText").and_then(Json::as_str) { return Err(format!("navigation failed: {error}")); }
        let document = if nav.get("loaderId").is_some() {
            // Redirects may legitimately change the final URL; the old-context marker still proves replacement.
            "!globalThis.__brainMcpNavigationPending".to_string()
        } else {
            format!("location.href === new URL({}, location.href).href", Json::str(&url).stringify())
        };
        let ready = format!("{document} && document.readyState === 'complete' && (!document.getElementById('brain-route') || document.getElementById('brain-view')?.dataset.brainReady === 'true')");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let previous = self.chrome.deadline;
        let reached = loop {
            let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) else { break false };
            self.chrome.deadline = Some(left);
            // Context destruction during a real navigation is expected; retry only inside the total budget.
            if self.chrome.eval(&self.session, &ready).ok() == Some(Json::Bool(true)) { break true; }
            std::thread::sleep(Duration::from_millis(10));
        };
        self.chrome.deadline = previous;
        if !reached { return Err("navigation did not reach a ready document and mounted view within 10 seconds".into()); }
        self.chrome.eval(&self.session, "delete globalThis.__brainMcpNavigationPending")?;
        // The view is mounted; let its layout and paint settle for screenshots.
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
        if width == 0 || height == 0 || width > 4096 || height > 4096 || u64::from(width) * u64::from(height) > 4_194_304 {
            return Err("viewport must be 1-4096 pixels per edge and at most 4 megapixels".into());
        }
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
        self.chrome.eval(&self.session, "(() => { const s=document.createElement('style');s.textContent='*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}';document.head.append(s);return true })()")?;
        self.settle()?;
        let shot = self.chrome.call(Some(&self.session), "Page.captureScreenshot", Json::obj([("format", Json::str("png"))]))?;
        let data = shot.get("data").and_then(Json::as_str).ok_or("captureScreenshot returned no data")?;
        if data.len() > 12 * 1024 * 1024 { return Err("screenshot exceeds encoded PNG limit".into()); }
        let png = base64_decode(data).ok_or("invalid base64 PNG")?;
        crate::shot::png_size(&png)?;
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new(); route.hash(&mut hash);
        let file = crate::shot::safe_output(root, &format!(".brain/shots/{:016x}-{width}x{height}.png", hash.finish()))?;
        crate::shot::write_output(&file, &png)?;
        let mut issues = Vec::new();
        audit(&mut self.chrome, &self.session, route, &mut issues)?;
        Ok((file, issues, info))
    }

    /// Accessible roles/names only; input values are deliberately omitted.
    pub fn accessibility_text(&mut self) -> Result<String, String> {
        let tree = self.chrome.call(Some(&self.session), "Accessibility.getFullAXTree", Json::Obj(vec![]))?;
        let Some(Json::Arr(nodes)) = tree.get("nodes") else { return Err("Chrome returned no accessibility tree".into()) };
        if nodes.len() > 10000 { return Err("accessibility tree exceeds 10,000 nodes".into()); }
        let mut text = String::new();
        for node in nodes {
            if node.get("ignored") == Some(&Json::Bool(true)) { continue; }
            let value = |key| node.get(key).and_then(|v| v.get("value")).and_then(Json::as_str).unwrap_or("");
            let line = format!("{} {}\n", value("role"), value("name").replace(['\n','\r'], " "));
            if text.len() + line.len() > 1024 * 1024 { return Err("accessibility text exceeds 1 MiB".into()); }
            text.push_str(&line);
        }
        Ok(text)
    }
    pub fn compare_png(&mut self, baseline: &[u8], current: &[u8], allowed: f64) -> Result<Json, String> {
        if !allowed.is_finite() || !(0.0..=1.0).contains(&allowed) { return Err("maxDiff must be a ratio between 0 and 1".into()); }
        let a = crate::shot::png_size(baseline)?;
        let b = crate::shot::png_size(current)?;
        if a != b { return Ok(Json::obj([("ok",Json::Bool(false)),("reason",Json::str("Baseline dimensions differ")),("ratio",Json::Num(1.0))])); }
        // Decode in a fresh blank tab. The tested app cannot read or override baseline pixels.
        let expr = format!("({})({},{},{})", include_str!("image-diff.js"), Json::str(crate::shot::base64(baseline)).stringify(), Json::str(crate::shot::base64(current)).stringify(), allowed);
        self.compare_in_tab(&expr)
    }

    fn compare_in_tab(&mut self, expr: &str) -> Result<Json,String> {
        let session = self.chrome.new_page()?;
        let target = self.chrome.call(Some(&session), "Target.getTargetInfo", Json::Obj(vec![]))
            .ok().and_then(|v|v.get("targetInfo").and_then(|v|v.get("targetId")).cloned());
        let result = self.chrome.eval(&session, &expr);
        if let Some(target) = target {
            let previous = self.chrome.deadline;
            let deadline = std::time::Instant::now()+Duration::from_secs(2);
            let cleanup = (|| -> Result<(),String> {
                self.chrome.deadline=Some(Duration::from_secs(2));
                self.chrome.call(None,"Target.closeTarget",Json::obj([("targetId",target.clone())]))?;
                loop {
                    let left=deadline.checked_duration_since(std::time::Instant::now()).ok_or("comparison tab did not close within 2 seconds")?;
                    self.chrome.deadline=Some(left);
                    let current=self.chrome.call(None,"Target.getTargets",Json::Obj(vec![]))?;
                    let Some(Json::Arr(targets))=current.get("targetInfos") else {return Err("Chrome returned no target list".into())};
                    if !targets.iter().any(|t|t.get("targetId")==Some(&target)){return Ok(())}
                    std::thread::sleep(Duration::from_millis(10).min(left));
                }
            })();
            self.chrome.deadline=previous;
            cleanup.map_err(|e|format!("comparison tab cleanup failed: {e}"))?;
        } else {
            let _=self.chrome.call(Some(&session),"Page.close",Json::Obj(vec![]));
            return Err("could not verify comparison tab ownership/cleanup".into());
        }
        result
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
    fn comparison_closes_only_its_temporary_tab_on_success_and_evaluation_failure() {
        if crate::cdp::find_chrome().is_none() { eprintln!("skipped: Chrome not installed"); return; }
        use brain_mcp::Json;
        let mut browser=super::Browser::open("http://127.0.0.1:1").unwrap();
        let image=browser.eval("(() => { const c=document.createElement('canvas'); c.width=2;c.height=2;return c.toDataURL('image/png').split(',')[1]; })()").unwrap();
        let png=super::base64_decode(image.as_str().unwrap()).unwrap();
        let targets=|b:&mut super::Browser| {
            let v=b.chrome.call(None,"Target.getTargets",Json::Obj(vec![])).unwrap();
            match v.get("targetInfos").unwrap(){Json::Arr(a)=>a.iter().filter(|t|t.get("type").and_then(Json::as_str)==Some("page")).count(),_=>panic!("missing targets")}
        };
        let before=targets(&mut browser);
        assert_eq!(browser.compare_png(&png,&png,0.0).unwrap().get("ok"),Some(&Json::Bool(true)));
        assert_eq!(targets(&mut browser),before);
        // A rejected decode promise uses this same evaluation-error cleanup path.
        assert!(browser.compare_in_tab("Promise.reject(new Error('Invalid PNG'))").is_err());
        assert_eq!(targets(&mut browser),before);
        assert_eq!(browser.eval("2+2").unwrap(),Json::Num(4.0));
    }
    #[test]
    fn base64_roundtrip() {
        assert_eq!(super::base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(super::base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(super::base64_decode("Zg==").unwrap(), b"f");
    }
}
