//! `brain lighthouse <url>`: runs Lighthouse on one deployed page and checks it against `lighthouse.minScores` in
//! brain.json. Pinned, so that a score change means the page changed and not the tool. On a staging deployment the
//! `noindex` is on purpose: the SEO score is recomputed without `is-crawlable` and the report says so.
use brain_lex::Json;
use std::path::Path;
use std::process::{Command, Stdio};

const LIGHTHOUSE: &str = "lighthouse@13.6.0";
const CATEGORIES: [&str; 4] = ["performance", "accessibility", "best-practices", "seo"];
const METRICS: [(&str, &str); 3] = [("largest-contentful-paint", "LCP"), ("cumulative-layout-shift", "CLS"), ("total-blocking-time", "TBT")];
const USAGE: &str = "usage: brain lighthouse <url> [--preset mobile|desktop] [--expect-noindex] [--json]";
/// Accessibility and SEO depend only on the page, not on the machine or the network, so a page is expected to pass
/// every audit in them unless brain.json lowers the bar on purpose.
const DEFAULT_MINIMUMS: [(&str, u32); 2] = [("accessibility", 100), ("seo", 100)];
/// How many failing audits a category lists under its score.
const LISTED_AUDITS: usize = 12;

fn num(v: Option<&Json>) -> Option<f64> {
    match v {
        Some(Json::Num(n)) => Some(*n),
        _ => None,
    }
}

/// A category's score out of 100, leaving out one audit if asked: the weighted mean Lighthouse itself computes.
fn score(report: &Json, category: &str, without: Option<&str>) -> Option<u32> {
    let cat = report.get("categories")?.get(category)?;
    let Some(left_out) = without else { return num(cat.get("score")).map(|s| (s * 100.0).round() as u32) };
    let Json::Arr(refs) = cat.get("auditRefs")? else { return None };
    let (mut sum, mut total) = (0.0, 0.0);
    for r in refs {
        let id = r.get("id")?.as_str()?;
        let weight = num(r.get("weight")).unwrap_or(0.0);
        if id == left_out || weight == 0.0 {
            continue;
        }
        if let Some(s) = num(report.get("audits")?.get(id).and_then(|a| a.get("score"))) {
            sum += s * weight;
            total += weight;
        }
    }
    (total > 0.0).then(|| (sum / total * 100.0).round() as u32)
}

/// The audits of a category that did not pass, as `id: title`, heaviest first: what to fix to reach the minimum.
fn failing(report: &Json, category: &str, without: Option<&str>) -> Vec<String> {
    let audits = report.get("audits");
    let Some(Json::Arr(refs)) = report.get("categories").and_then(|c| c.get(category)).and_then(|c| c.get("auditRefs")) else { return Vec::new() };
    let mut out: Vec<(f64, String)> = refs
        .iter()
        .filter_map(|r| {
            let id = r.get("id")?.as_str()?;
            let weight = num(r.get("weight")).unwrap_or(0.0);
            let audit = audits?.get(id)?;
            let passed = num(audit.get("score")).is_none_or(|s| s >= 1.0);
            if weight == 0.0 || passed || Some(id) == without {
                return None;
            }
            Some((weight, format!("{id}: {}", audit.get("title").and_then(Json::as_str).unwrap_or(id))))
        })
        .collect();
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.into_iter().take(LISTED_AUDITS).map(|(_, line)| line).collect()
}

/// Judges a Lighthouse report. Separate from running it so that tests can feed it one.
pub fn evaluate(report: &Json, mins: &[(String, u32)], expect_noindex: bool) -> (i32, Json, String) {
    let blocked = report.get("audits").and_then(|a| a.get("is-crawlable")).is_some_and(|a| matches!(num(a.get("score")), Some(s) if s == 0.0));
    let on_purpose = blocked && expect_noindex;
    let (mut lines, mut scores, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for category in CATEGORIES {
        let without = (on_purpose && category == "seo").then_some("is-crawlable");
        let Some(value) = score(report, category, without) else { continue };
        let min = mins.iter().find(|(name, _)| name == category).map(|(_, m)| *m);
        let under = min.is_some_and(|m| value < m);
        if under {
            failed.push(category);
        }
        let bar = min.map_or(String::new(), |m| format!(" (minimum {m})"));
        let note = if without.is_some() { ", blocked from search on purpose; is-crawlable left out" } else { "" };
        lines.push(format!("{category}: {value}{bar}{note}"));
        if under {
            lines.extend(failing(report, category, without).into_iter().map(|a| format!("  - {a}")));
        }
        scores.push((category.to_string(), Json::Num(value as f64)));
    }
    if blocked && !expect_noindex {
        lines.push("is-crawlable fails: the page answers noindex. On staging, pass --expect-noindex".to_string());
    }
    let audits = report.get("audits");
    let metrics: Vec<(String, Json)> = METRICS
        .iter()
        .filter_map(|(id, name)| {
            let shown = audits?.get(id)?.get("displayValue")?.as_str()?;
            lines.push(format!("{name}: {shown}"));
            Some((name.to_string(), Json::str(shown)))
        })
        .collect();
    let ok = failed.is_empty();
    if !ok {
        lines.push(format!("under the minimum: {}", failed.join(", ")));
    }
    let json = Json::obj([
        ("schemaVersion", Json::Num(1.0)),
        ("ok", Json::Bool(ok)),
        ("url", report.get("finalDisplayedUrl").cloned().unwrap_or(Json::Null)),
        ("scores", Json::Obj(scores)),
        ("metrics", Json::Obj(metrics)),
        ("blockedOnPurpose", Json::Bool(on_purpose)),
    ]);
    (i32::from(!ok), json, lines.join("\n"))
}

fn minimums(root: Option<&Path>) -> Vec<(String, u32)> {
    let config = root.and_then(|r| std::fs::read_to_string(r.join("brain.json")).ok()).and_then(|s| Json::parse(&s).ok());
    let wanted = config.as_ref().and_then(|c| c.get("lighthouse")).and_then(|l| l.get("minScores"));
    let default = |c: &str| DEFAULT_MINIMUMS.iter().find(|(name, _)| *name == c).map(|(_, m)| *m);
    CATEGORIES.iter().filter_map(|c| Some((c.to_string(), num(wanted.and_then(|w| w.get(c))).map(|n| n as u32).or_else(|| default(c))?))).collect()
}

pub fn run(root: Option<&Path>, args: &[String]) -> (i32, String) {
    let json = args.iter().any(|a| a == "--json");
    let fail = |code: i32, message: String| (code, if json { Json::obj([("ok", Json::Bool(false)), ("error", Json::str(message.as_str()))]).stringify() } else { message });
    let preset = args.iter().position(|a| a == "--preset").and_then(|i| args.get(i + 1)).map_or("mobile", String::as_str);
    let Some(url) = args.iter().find(|a| a.starts_with("http://") || a.starts_with("https://")) else { return fail(2, USAGE.into()) };
    if !["mobile", "desktop"].contains(&preset) {
        return fail(2, USAGE.into());
    }
    let staging = root.and_then(|r| brain_seo::Config::load(r).ok()).is_some_and(|c| c.staging);
    let expect_noindex = staging || args.iter().any(|a| a == "--expect-noindex");
    let mut command = Command::new(if cfg!(windows) { "npx.cmd" } else { "npx" });
    command.args(["-y", LIGHTHOUSE, url, "--quiet", "--output=json", "--output-path=stdout", "--chrome-flags=--headless=new"]);
    command.arg(format!("--only-categories={}", CATEGORIES.join(",")));
    if preset == "desktop" {
        command.arg("--preset=desktop");
    }
    let output = match command.stdin(Stdio::null()).stderr(Stdio::piped()).output() {
        Ok(output) => output,
        Err(e) => return fail(2, format!("could not start npx ({e}); fix: install Node.js and Chrome")),
    };
    let Ok(report) = Json::parse(&String::from_utf8_lossy(&output.stdout)) else {
        let cause = String::from_utf8_lossy(&output.stderr);
        return fail(2, format!("Lighthouse did not produce a report; fix: check that Chrome is installed\n{}", cause.trim()));
    };
    let (code, value, text) = evaluate(&report, &minimums(root), expect_noindex);
    (code, if json { value.stringify() } else { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report(crawlable: f64) -> Json {
        Json::parse(&format!(
            r#"{{"finalDisplayedUrl":"https://a.test/","categories":{{"performance":{{"score":0.93}},"seo":{{"score":0.66,"auditRefs":[{{"id":"is-crawlable","weight":4.0}},{{"id":"meta-description","weight":1.0}},{{"id":"manual","weight":0.0}}]}}}},
            "audits":{{"is-crawlable":{{"score":{crawlable}}},"meta-description":{{"score":1}},"largest-contentful-paint":{{"displayValue":"1.6 s"}}}}}}"#
        ))
        .unwrap()
    }
    #[test]
    fn a_score_under_its_minimum_fails_and_names_it() {
        let (code, _, text) = evaluate(&report(1.0), &[("performance".into(), 95)], false);
        assert_eq!(code, 1);
        assert!(text.contains("performance: 93 (minimum 95)") && text.contains("under the minimum: performance") && text.contains("LCP: 1.6 s"), "{text}");
        assert_eq!(evaluate(&report(1.0), &[("performance".into(), 90)], false).0, 0);
    }
    #[test]
    fn a_category_under_its_minimum_lists_the_audits_to_fix_heaviest_first() {
        let r = Json::parse(
            r#"{"categories":{"accessibility":{"score":0.9,"auditRefs":[{"id":"label","weight":7},{"id":"color-contrast","weight":10},{"id":"lang","weight":0},{"id":"ok","weight":3}]}},
            "audits":{"label":{"score":0,"title":"Form elements have labels"},"color-contrast":{"score":0,"title":"Enough contrast"},"lang":{"score":0,"title":"Manual"},"ok":{"score":1,"title":"Fine"}}}"#,
        )
        .unwrap();
        let (code, _, text) = evaluate(&r, &[("accessibility".into(), 100)], false);
        assert_eq!(code, 1);
        assert!(text.contains("accessibility: 90 (minimum 100)\n  - color-contrast: Enough contrast\n  - label: Form elements have labels"), "{text}");
        assert!(!text.contains("lang") && !text.contains("Fine"), "weightless and passing audits are not listed: {text}");
        let (_, _, text) = evaluate(&r, &[("accessibility".into(), 80)], false);
        assert!(!text.contains("  - "), "a category that meets its minimum lists nothing: {text}");
    }
    #[test]
    fn accessibility_and_seo_must_be_perfect_unless_brain_json_lowers_them() {
        let dir = std::env::temp_dir().join(format!("brain-lh-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mins = minimums(Some(&dir));
        assert_eq!(mins, vec![("accessibility".to_string(), 100), ("seo".to_string(), 100)]);
        std::fs::write(dir.join("brain.json"), r#"{"lighthouse":{"minScores":{"performance":90,"seo":95}}}"#).unwrap();
        let mins = minimums(Some(&dir));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(mins, vec![("performance".to_string(), 90), ("accessibility".to_string(), 100), ("seo".to_string(), 95)]);
    }
    #[test]
    fn expected_noindex_leaves_only_is_crawlable_out_of_seo() {
        let mins = [("seo".to_string(), 90)];
        let (code, json, text) = evaluate(&report(0.0), &mins, true);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("seo: 100 (minimum 90), blocked from search on purpose"), "{text}");
        assert!(json.stringify().contains(r#""blockedOnPurpose":true"#));
        let (code, _, text) = evaluate(&report(0.0), &mins, false);
        assert_eq!(code, 1, "an unexpected noindex is a real failure: {text}");
        assert!(text.contains("seo: 66") && text.contains("--expect-noindex"), "{text}");
    }
}
