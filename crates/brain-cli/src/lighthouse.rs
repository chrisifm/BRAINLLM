//! `brain lighthouse <url>`: runs Lighthouse on one deployed page and checks it against `lighthouse.minScores` in
//! brain.json. Pinned, so that a score change means the page changed and not the tool. On a staging deployment the
//! `noindex` is on purpose: the SEO score is recomputed without `is-crawlable` and the report says so.
use brain_lex::Json;
use std::path::Path;
use std::process::{Command, Stdio};

const LIGHTHOUSE: &str = "lighthouse@13.6.0";
const CATEGORIES: [&str; 4] = ["performance", "accessibility", "best-practices", "seo"];
const METRICS: [(&str, &str); 3] = [("largest-contentful-paint", "LCP"), ("cumulative-layout-shift", "CLS"), ("total-blocking-time", "TBT")];
const USAGE: &str = "usage: brain lighthouse <url> [--preset mobile|desktop] [--chrome-flags <flags>] [--expect-noindex] [--json]";
/// Accessibility and SEO depend only on the page, not on the machine or the network, so a page is expected to pass
/// every audit in them unless brain.json lowers the bar on purpose.
const DEFAULT_MINIMUMS: [(&str, f64); 2] = [("accessibility", 100.0), ("seo", 100.0)];
/// How many failing audits a category lists under its score.
const LISTED_AUDITS: usize = 12;

fn num(v: Option<&Json>) -> Option<f64> {
    match v {
        Some(Json::Num(n)) => Some(*n),
        _ => None,
    }
}

/// A category's score out of 100, leaving out one audit if asked: the weighted mean Lighthouse itself computes.
fn score(report: &Json, category: &str, without: Option<&str>) -> Option<f64> {
    let cat = report.get("categories")?.get(category)?;
    let Some(left_out) = without else { return num(cat.get("score")).map(|s| s * 100.0) };
    let Json::Arr(refs) = cat.get("auditRefs")? else { return None };
    let (mut sum, mut total) = (0.0, 0.0);
    for r in refs {
        let id = r.get("id")?.as_str()?;
        let weight = num(r.get("weight"))?;
        if !weight.is_finite() || weight < 0.0 {return None}
        if id == left_out || weight == 0.0 {continue}
        // Null is a legitimate non-applicable audit. Missing objects/fields and invalid values are not.
        match report.get("audits")?.get(id)?.get("score")? {
            Json::Null=>{},
            Json::Num(s) if s.is_finite() && (0.0..=1.0).contains(s)=>{sum += s * weight;total += weight;},
            _=>return None,
        }
    }
    (total > 0.0 && total.is_finite() && sum.is_finite()).then(|| sum / total * 100.0)
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

fn incomplete_report() -> (i32,Json,String) {
    let message="Lighthouse did not complete all requested categories; fix: inspect its runtime error, verify that the page loads and rerun the audit";
    (2,Json::obj([("schemaVersion",Json::Num(1.0)),("ok",Json::Bool(false)),("error",Json::str(message))]),message.into())
}

/// Judges a Lighthouse report. Separate from running it so that tests can feed it one.
pub fn evaluate(report: &Json, mins: &[(String, f64)], expect_noindex: bool) -> (i32, Json, String) {
    // A runtime failure or absent category is not a successful audit with zero observations.
    let runtime_error=report.get("runtimeError").is_some_and(|value|*value!=Json::Null);
    let complete=CATEGORIES.iter().all(|category|num(report.get("categories").and_then(|c|c.get(category)).and_then(|c|c.get("score"))).is_some_and(|score|score.is_finite() && (0.0..=1.0).contains(&score)));
    if runtime_error || !complete {return incomplete_report()}
    let blocked = report.get("audits").and_then(|a| a.get("is-crawlable")).is_some_and(|a| matches!(num(a.get("score")), Some(s) if s == 0.0));
    let on_purpose = blocked && expect_noindex;
    let (mut lines, mut scores, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for category in CATEGORIES {
        let without = (on_purpose && category == "seo").then_some("is-crawlable");
        let Some(value) = score(report, category, without) else {return incomplete_report()};
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

fn minimums(root: Option<&Path>) -> Result<Vec<(String,f64)>,String> {
    let invalid=||"invalid lighthouse.minScores; fix: use finite numeric category minimums from 0 to 100".to_string();
    let config=match root.map(|root|std::fs::read_to_string(root.join("brain.json"))) {
        None=>None,Some(Ok(text))=>Some(Json::parse(&text).map_err(|_|invalid())?),
        Some(Err(error)) if error.kind()==std::io::ErrorKind::NotFound=>None,
        Some(Err(_))=>return Err("cannot read brain.json Lighthouse minimums".into()),
    };
    let lighthouse=config.as_ref().and_then(|c|c.get("lighthouse"));
    if lighthouse.is_some_and(|v|!matches!(v,Json::Obj(_))){return Err(invalid())}
    let wanted=lighthouse.and_then(|c|c.get("minScores"));
    if wanted.is_some_and(|v|!matches!(v,Json::Obj(_))){return Err(invalid())}
    let mut out=Vec::new();
    for category in CATEGORIES {
        let value=match wanted.and_then(|w|w.get(category)) {
            Some(Json::Num(n)) if n.is_finite() && (0.0..=100.0).contains(n)=>Some(*n),
            Some(_)=>return Err(invalid()),
            None=>DEFAULT_MINIMUMS.iter().find(|(name,_)|*name==category).map(|(_,minimum)|*minimum),
        };
        if let Some(value)=value {out.push((category.to_string(),value));}
    }
    Ok(out)
}

fn option<'a>(args:&'a [String],name:&str)->Option<&'a str> {
    args.iter().enumerate().find_map(|(i,arg)|if arg==name {args.get(i+1).map(String::as_str)} else {arg.strip_prefix(name).and_then(|rest|rest.strip_prefix('='))})
}

pub fn run(root: Option<&Path>, args: &[String]) -> (i32, String) {
    let json = args.iter().any(|a| a == "--json");
    let fail = |code: i32, message: String| (code, if json { Json::obj([("ok", Json::Bool(false)), ("error", Json::str(message.as_str()))]).stringify() } else { message });
    let mins=match minimums(root) {Ok(mins)=>mins,Err(error)=>return fail(2,error)};
    let preset = option(args,"--preset").unwrap_or("mobile");
    let chrome_flags=option(args,"--chrome-flags").unwrap_or("");
    if chrome_flags.len()>4096 || chrome_flags.chars().any(char::is_control) {return fail(2,"invalid --chrome-flags; fix: pass one bounded, quoted Chrome flag string".into())}
    let Some(url) = args.iter().find(|a| a.starts_with("http://") || a.starts_with("https://")) else { return fail(2, USAGE.into()) };
    if !["mobile", "desktop"].contains(&preset) {
        return fail(2, USAGE.into());
    }
    let staging = root.and_then(|r| brain_seo::Config::load(r).ok()).is_some_and(|c| c.staging);
    let expect_noindex = staging || args.iter().any(|a| a == "--expect-noindex");
    let mut command = Command::new(if cfg!(windows) { "npx.cmd" } else { "npx" });
    command.args(["-y", LIGHTHOUSE, url, "--quiet", "--output=json", "--output-path=stdout"]);
    // One argument, never a shell command. CHROME_PATH is inherited by Lighthouse unchanged.
    command.arg(if chrome_flags.is_empty() {"--chrome-flags=--headless=new".to_string()} else {format!("--chrome-flags=--headless=new {chrome_flags}")});
    command.arg(format!("--only-categories={}", CATEGORIES.join(",")));
    if preset == "desktop" {
        command.arg("--preset=desktop");
    }
    let output = match command.stdin(Stdio::null()).stderr(Stdio::piped()).output() {
        Ok(output) => output,
        Err(e) => return fail(2, format!("could not start npx ({e}); fix: install Node.js and Chrome")),
    };
    if !output.status.success() {
        return fail(2,format!("Lighthouse exited unsuccessfully ({}); fix: check the page, Chrome and CHROME_PATH; use --chrome-flags for explicitly required container options\n{}",output.status,String::from_utf8_lossy(&output.stderr).trim()));
    }
    let Ok(report) = Json::parse(&String::from_utf8_lossy(&output.stdout)) else {
        let cause = String::from_utf8_lossy(&output.stderr);
        return fail(2, format!("Lighthouse did not produce a report; fix: check that Chrome is installed\n{}", cause.trim()));
    };
    let (code, value, text) = evaluate(&report, &mins, expect_noindex);
    (code, if json { value.stringify() } else { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report(crawlable: f64) -> Json {
        Json::parse(&format!(
            r#"{{"finalDisplayedUrl":"https://a.test/","categories":{{"performance":{{"score":0.93}},"accessibility":{{"score":1}},"best-practices":{{"score":1}},"seo":{{"score":0.66,"auditRefs":[{{"id":"is-crawlable","weight":4.0}},{{"id":"meta-description","weight":1.0}},{{"id":"manual","weight":0.0}}]}}}},
            "audits":{{"is-crawlable":{{"score":{crawlable}}},"meta-description":{{"score":1}},"largest-contentful-paint":{{"displayValue":"1.6 s"}}}}}}"#
        ))
        .unwrap()
    }
    #[test]
    fn a_score_under_its_minimum_fails_and_names_it() {
        let (code, _, text) = evaluate(&report(1.0), &[("performance".into(), 95.0)], false);
        assert_eq!(code, 1);
        assert!(text.contains("performance: 93 (minimum 95)") && text.contains("under the minimum: performance") && text.contains("LCP: 1.6 s"), "{text}");
        assert_eq!(evaluate(&report(1.0), &[("performance".into(), 90.0)], false).0, 0);
    }
    #[test]
    fn a_category_under_its_minimum_lists_the_audits_to_fix_heaviest_first() {
        let r = Json::parse(
            r#"{"categories":{"performance":{"score":1},"best-practices":{"score":1},"seo":{"score":1},"accessibility":{"score":0.9,"auditRefs":[{"id":"label","weight":7},{"id":"color-contrast","weight":10},{"id":"lang","weight":0},{"id":"ok","weight":3}]}},
            "audits":{"label":{"score":0,"title":"Form elements have labels"},"color-contrast":{"score":0,"title":"Enough contrast"},"lang":{"score":0,"title":"Manual"},"ok":{"score":1,"title":"Fine"}}}"#,
        )
        .unwrap();
        let (code, _, text) = evaluate(&r, &[("accessibility".into(), 100.0)], false);
        assert_eq!(code, 1);
        assert!(text.contains("accessibility: 90 (minimum 100)\n  - color-contrast: Enough contrast\n  - label: Form elements have labels"), "{text}");
        assert!(!text.contains("lang") && !text.contains("Fine"), "weightless and passing audits are not listed: {text}");
        let (_, _, text) = evaluate(&r, &[("accessibility".into(), 80.0)], false);
        assert!(!text.contains("  - "), "a category that meets its minimum lists nothing: {text}");
    }
    #[test]
    fn accessibility_and_seo_must_be_perfect_unless_brain_json_lowers_them() {
        let dir = std::env::temp_dir().join(format!("brain-lh-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mins = minimums(Some(&dir)).unwrap();
        assert_eq!(mins, vec![("accessibility".to_string(), 100.0), ("seo".to_string(), 100.0)]);
        std::fs::write(dir.join("brain.json"), r#"{"lighthouse":{"minScores":{"performance":90,"seo":95}}}"#).unwrap();
        let mins = minimums(Some(&dir)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(mins, vec![("performance".to_string(), 90.0), ("accessibility".to_string(), 100.0), ("seo".to_string(), 95.0)]);
    }
    #[test]
    fn absent_or_failed_categories_cannot_report_success() {
        for source in ["{}",r#"{"runtimeError":{"code":"FAILED_DOCUMENT_REQUEST"}}"#,r#"{"categories":{"performance":{"score":1}}}"#] {
            let (code,result,_)=evaluate(&Json::parse(source).unwrap(),&[],false);
            assert_eq!(code,2);assert_eq!(result.get("ok"),Some(&Json::Bool(false)));
        }
        let mut failed=report(1.0);
        if let Json::Obj(fields)=&mut failed {fields.push(("runtimeError".into(),Json::obj([("code",Json::str("PAGE_HUNG"))])));}
        assert_eq!(evaluate(&failed,&[],false).0,2);
    }

    #[test]
    fn expected_noindex_leaves_only_is_crawlable_out_of_seo() {
        let mins = [("seo".to_string(), 90.0)];
        let (code, json, text) = evaluate(&report(0.0), &mins, true);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("seo: 100 (minimum 90), blocked from search on purpose"), "{text}");
        assert!(json.stringify().contains(r#""blockedOnPurpose":true"#));
        let (code, _, text) = evaluate(&report(0.0), &mins, false);
        assert_eq!(code, 1, "an unexpected noindex is a real failure: {text}");
        assert!(text.contains("seo: 66") && text.contains("--expect-noindex"), "{text}");
    }
}
