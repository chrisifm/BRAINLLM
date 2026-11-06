//! Cloudflare Cache Rules, with no account discovery, global settings or full-ruleset replacement.
//! Only the fixed official API is reachable in production. Unit tests inject a transport, not an API URL.
use super::audit::{self, Options};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};

const API: &str = "https://api.cloudflare.com/client/v4";
const PHASE: &str = "http_request_cache_settings";
fn digest(value: &Value) -> String {
    brain_pack::sha::sha256(value.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
trait Transport {
    fn request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>, String>;
}
struct Http {
    client: reqwest::Client,
    runtime: crate::http_observer::Runtime,
    token: String,
    deadline: Instant,
}
impl Http {
    fn new(token: String) -> Result<Self, String> {
        let client = crate::http_observer::client(reqwest::Client::builder())
            .no_proxy()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(2))
            .build()
            .map_err(|_| "cannot create Cloudflare client")?;
        let runtime = crate::http_observer::Runtime::new()?;
        Ok(Self {
            client,
            runtime,
            token,
            deadline: Instant::now() + Duration::from_secs(45),
        })
    }
}
impl Transport for Http {
    fn request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>, String> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or("Cloudflare operation exceeded its total deadline")?;
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| "invalid Cloudflare method")?;
        let mut request = self
            .client
            .request(method, format!("{API}{path}"))
            .bearer_auth(&self.token)
            .timeout(remaining.min(Duration::from_secs(5)));
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body.to_string());
        }
        // Never print the provider body, request headers, token or transport error (which can contain a URL).
        self.runtime.block_on(async {
            let mut response=request.send().await.map_err(|_|"Cloudflare request failed; mutation outcome may be unknown; inspect a fresh audit before retrying")?;
            if response.status()==reqwest::StatusCode::NOT_FOUND {return Ok(None);}
            if !response.status().is_success() {return Err("Cloudflare rejected the operation; check scoped token permissions and run a fresh audit".into());}
            let mut bytes=Vec::new();
            while let Some(chunk)=response.chunk().await.map_err(|_|"cannot read Cloudflare response")? {
                if bytes.len()+chunk.len()>2*1024*1024 {return Err("Cloudflare response exceeds 2 MiB".into());}
                bytes.extend_from_slice(&chunk);
            }
            let envelope:Value=serde_json::from_slice(&bytes).map_err(|_|"Cloudflare returned invalid JSON")?;
            if envelope["success"]!=true || !envelope["result"].is_object() {return Err("Cloudflare did not confirm the operation".into());}
            Ok(Some(envelope["result"].clone()))
        })
    }
}
fn entry(zone: &str) -> String {
    format!("/zones/{zone}/rulesets/phases/{PHASE}/entrypoint")
}
fn validate_snapshot(value: &Value) -> Result<(), String> {
    if !value["id"].as_str().is_some_and(id)
        || !value["version"].is_string()
        || value["phase"] != PHASE
        || value["kind"] != "zone"
        || !value["rules"].is_array()
        || value["rules"].as_array().unwrap().len() > 1000
    {
        return Err("Cloudflare entrypoint is not a bounded zone Cache Rules ruleset".into());
    }
    Ok(())
}
fn desired(plan: &Value, origin: &reqwest::Url) -> Result<Vec<Value>, String> {
    let base = plan["base"].as_str().unwrap_or("/");
    if !audit::safe_path(base) || audit::protected(base) {
        return Err("edge provider requires a safe built base path".into());
    }
    let paths: Vec<_> = plan["immutableAssets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if paths.is_empty() {
        return Err("no exact immutable asset inventory is available; fix: build with host files enabled before planning provider rules".into());
    }
    if paths.len() > 128
        || paths.iter().any(|p| {
            !audit::safe_path(p)
                || audit::protected(p)
                || !p.starts_with(base)
                || p.ends_with(".html")
                || p.ends_with(".brain")
                || p.ends_with(".md")
        })
    {
        return Err("provider policy requires at most 128 exact public immutable assets under the built base".into());
    }
    let host = origin
        .host_str()
        .ok_or("edge provider origin requires a host")?;
    let scope = format!(
        "(http.host eq {} and ssl and starts_with(http.request.uri.path, {}))",
        json!(host),
        json!(base)
    );
    let path_set = paths
        .iter()
        .map(|p| json!(p).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let eligible = format!(
        "(http.request.method in {{\"GET\" \"HEAD\"}} and http.request.uri.path in {{{path_set}}} and not any(lower(http.request.headers.names[*])[*] eq \"cookie\") and not any(lower(http.request.headers.names[*])[*] eq \"authorization\"))"
    );
    let key = &digest(&json!([origin.origin().ascii_serialization(), base]))[..16];
    let assets = json!({"ref":format!("brain_edge_{key}_assets"),"description":"BRAINLLM exact immutable assets; respect origin","action":"set_cache_settings","enabled":true,"expression":format!("{scope} and {eligible}"),"action_parameters":{"cache":true,"origin_cache_control":true,"edge_ttl":{"mode":"bypass_by_default"},"browser_ttl":{"mode":"respect_origin"}}});
    let bypass = json!({"ref":format!("brain_edge_{key}_bypass"),"description":"BRAINLLM bypass documents, private paths and credentials","action":"set_cache_settings","enabled":true,"expression":format!("{scope} and not {eligible}"),"action_parameters":{"cache":false}});
    if assets["expression"].as_str().unwrap().len() > 4000
        || bypass["expression"].as_str().unwrap().len() > 4000
    {
        return Err("exact asset policy exceeds the portable Cloudflare expression budget; split the deployment manually".into());
    }
    Ok(vec![assets, bypass])
}
fn managed<'a>(snapshot: &'a Value, wanted: &Value) -> Result<Option<&'a Value>, String> {
    let rules = snapshot["rules"]
        .as_array()
        .ok_or("Cloudflare snapshot has no rules")?;
    let matches: Vec<_> = rules.iter().filter(|r| r["ref"] == wanted["ref"]).collect();
    if matches.len() > 1 {
        return Err("multiple rules claim the managed reference; inspect the zone manually".into());
    }
    let found = matches.first().copied();
    if let Some(rule) = found {
        if !rule["id"].as_str().is_some_and(id) || rule["action"] != "set_cache_settings" {
            return Err("managed rule identity is invalid; inspect the zone manually".into());
        }
        let scope = wanted["expression"]
            .as_str()
            .unwrap_or("")
            .split(" and not (http.request.method")
            .next()
            .unwrap_or("")
            .split(" and (http.request.method")
            .next()
            .unwrap_or("");
        if scope.is_empty()
            || !rule["expression"]
                .as_str()
                .is_some_and(|expression| expression.starts_with(&format!("{scope} and ")))
        {
            return Err("managed reference is attached to a different host or base scope; inspect the zone manually".into());
        }
    }
    Ok(found)
}
fn definition(rule: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "ref",
        "description",
        "action",
        "enabled",
        "expression",
        "action_parameters",
    ] {
        if let Some(value) = rule.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    Value::Object(out)
}
fn revision(snapshot: &Value) -> Value {
    json!({"rulesetId":snapshot["id"],"version":snapshot["version"],"digest":digest(snapshot)})
}
fn preview(
    plan: &Value,
    zone: &str,
    origin: &reqwest::Url,
    snapshot: &Value,
) -> Result<Value, String> {
    validate_snapshot(snapshot)?;
    let rules = desired(plan, origin)?;
    for wanted in &rules {
        managed(snapshot, wanted)?;
    }
    Ok(
        json!({"schemaVersion":1,"provider":"cloudflare","zoneId":zone,"origin":origin.origin().ascii_serialization(),"base":plan["base"],"buildDigest":digest(plan),"revision":revision(snapshot),"managedRules":rules}),
    )
}
fn zone_check(
    transport: &mut impl Transport,
    zone: &str,
    origin: &reqwest::Url,
) -> Result<(), String> {
    let details = transport
        .request("GET", &format!("/zones/{zone}"), None)?
        .ok_or("Cloudflare zone does not exist")?;
    let name = details["name"]
        .as_str()
        .ok_or("Cloudflare zone has no name")?;
    let host = origin.host_str().ok_or("edge origin has no host")?;
    if details["id"] != zone || !(host == name || host.ends_with(&format!(".{name}"))) {
        return Err("configured zone does not contain the requested origin host".into());
    }
    Ok(())
}
fn foreign(snapshot: &Value, wanted: &[Value]) -> Value {
    json!(
        snapshot["rules"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| !wanted.iter().any(|w| w["ref"] == r["ref"]))
            .cloned()
            .collect::<Vec<_>>()
    )
}
fn policy_matches(snapshot: &Value, wanted: &[Value]) -> bool {
    wanted.iter().all(|w| {
        managed(snapshot, w)
            .ok()
            .flatten()
            .is_some_and(|r| definition(r) == *w)
    }) && snapshot["rules"].as_array().is_some_and(|rules| {
        rules.len() >= wanted.len()
            && rules[rules.len() - wanted.len()..]
                .iter()
                .all(|r| wanted.iter().any(|w| w["ref"] == r["ref"]))
    })
}
fn apply(
    transport: &mut impl Transport,
    plan: &Value,
    zone: &str,
    origin: &reqwest::Url,
    request: &Value,
    initial: &Value,
) -> Result<Value, String> {
    let expected = preview(plan, zone, origin, initial)?;
    if request != &expected {
        return Err("apply request is stale, altered or belongs to another zone, origin or build; fix: run edge plan --provider cloudflare again".into());
    }
    let wanted = expected["managedRules"].as_array().unwrap();
    let mut current = initial.clone();
    let initial_foreign = foreign(initial, wanted);
    let mut operations = 0;
    // Install/update bypass first. Eligibility and bypass are complementary, so their final order is immaterial.
    for rule in wanted.iter().rev() {
        // Re-read the entire revision before EACH narrow mutation. No atomic CAS is claimed by this API.
        let latest = transport
            .request("GET", &entry(zone), None)?
            .ok_or("Cloudflare entrypoint disappeared")?;
        validate_snapshot(&latest)?;
        if revision(&latest) != revision(&current) {
            return Err("Cloudflare revision changed before mutation; no further rules were changed; run a fresh audit".into());
        }
        let existing = managed(&latest, rule)?;
        let last = latest["rules"].as_array().unwrap().last();
        if existing.is_some_and(|r| definition(r) == *rule)
            && last.is_some_and(|r| r["ref"] == rule["ref"])
        {
            current = latest;
            continue;
        }
        let mut body = rule.clone();
        body["position"] = json!({"after":""});
        let ruleset = latest["id"].as_str().unwrap();
        let (method, path) = if let Some(existing) = existing {
            (
                "PATCH",
                format!(
                    "/zones/{zone}/rulesets/{ruleset}/rules/{}",
                    existing["id"].as_str().unwrap()
                ),
            )
        } else {
            ("POST", format!("/zones/{zone}/rulesets/{ruleset}/rules"))
        };
        transport.request(method, &path, Some(&body))?.ok_or(
            "Cloudflare mutation was not confirmed; inspect a fresh audit before retrying",
        )?;
        operations += 1;
        current = transport
            .request("GET", &entry(zone), None)?
            .ok_or("Cloudflare entrypoint missing after mutation")?;
        validate_snapshot(&current)?;
        if managed(&current, rule)?.is_none_or(|r| definition(r) != *rule)
            || current["rules"]
                .as_array()
                .unwrap()
                .last()
                .is_none_or(|r| r["ref"] != rule["ref"])
        {
            return Err("Cloudflare readback does not match the requested managed rule; inspect a fresh audit".into());
        }
        if foreign(&current, wanted) != initial_foreign {
            return Err("foreign rules changed concurrently; this command only wrote managed rule endpoints; inspect a fresh audit".into());
        }
    }
    if !policy_matches(&current, wanted) {
        return Err(
            "managed rules changed or were overridden during apply; inspect a fresh audit".into(),
        );
    }
    Ok(
        json!({"schemaVersion":1,"ok":true,"provider":"cloudflare","origin":origin.origin().ascii_serialization(),"managedOperations":operations,"revision":revision(&current),"concurrency":"optimistic-read-before-each-write; no atomic CAS; concurrent edits to the same managed rule remain possible","coverage":"zone Cache Rules only; no global settings, Workers, other phases or existing cache purge"}),
    )
}
pub(super) fn run(plan: &Value, options: &Options) -> Result<Value, String> {
    let origin = audit::origin(
        options
            .origin
            .as_deref()
            .ok_or("provider operations require --url <origin>")?,
    )?;
    if origin.scheme() != "https" || origin.port().is_some() {
        return Err("Cloudflare policy requires a public HTTPS origin on its default port".into());
    }
    let zone = std::env::var("CLOUDFLARE_ZONE_ID")
        .map_err(|_| "set CLOUDFLARE_ZONE_ID to the explicitly selected zone")?;
    if !id(&zone) {
        return Err("CLOUDFLARE_ZONE_ID must be a 32 digit hexadecimal zone id".into());
    }
    let token = std::env::var("CLOUDFLARE_API_TOKEN")
        .map_err(|_| "set CLOUDFLARE_API_TOKEN with permissions limited to the selected zone")?;
    if token.is_empty() || token.len() > 4096 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("CLOUDFLARE_API_TOKEN is invalid".into());
    }
    // Validate local inputs before any provider request. Tokens are sent only to the fixed official HTTPS API.
    desired(plan, &origin)?;
    let request = if options.command == "apply" {
        let input = options
            .request
            .as_deref()
            .ok_or("edge apply requires --request <preview JSON or file>")?;
        if input.len() > 256 * 1024 {
            return Err("apply request exceeds 256 KiB".into());
        }
        let source = if input.trim_start().starts_with('{') {
            input.to_string()
        } else {
            audit::read(Path::new(input), 256 * 1024)?
        };
        let value: Value =
            serde_json::from_str(&source).map_err(|_| "apply request must be JSON")?;
        Some(value.get("applyRequest").unwrap_or(&value).clone())
    } else {
        None
    };
    let mut transport = Http::new(token)?;
    execute(&mut transport, plan, options, &zone, &origin, request)
}
fn execute(
    transport: &mut impl Transport,
    plan: &Value,
    options: &Options,
    zone: &str,
    origin: &reqwest::Url,
    request: Option<Value>,
) -> Result<Value, String> {
    zone_check(transport, zone, origin)?;
    let Some(snapshot) = transport.request("GET", &entry(zone), None)? else {
        return Err("zone Cache Rules entrypoint is missing; create it manually and then run a fresh plan; no shared entrypoint was created".into());
    };
    let preview = preview(plan, zone, origin, &snapshot)?;
    if let Some(request) = request {
        return apply(transport, plan, zone, origin, &request, &snapshot);
    }
    let matches = policy_matches(&snapshot, preview["managedRules"].as_array().unwrap());
    let http = if matches!(options.command.as_str(), "check" | "audit") {
        Some(audit::run(plan, origin.as_str())?)
    } else {
        None
    };
    Ok(
        json!({"schemaVersion":1,"provider":"cloudflare","ok":http.as_ref().is_none_or(|h|h["ok"]==true && matches),"changed":false,"managedPolicyMatches":matches,"httpAudit":http,"foreignRuleCount":foreign(&snapshot,preview["managedRules"].as_array().unwrap()).as_array().unwrap().len(),"coverage":{"policy":"exact immutable assets only; all other scoped requests bypass","htmlAcceptKeyConfigured":false,"otherPhasesWorkersAndGlobalSettingsVerified":false,"atomicCompareAndSwap":false},"applyRequest":preview}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    const Z: &str = "00000000000000000000000000000001";
    const R: &str = "00000000000000000000000000000002";
    const A: &str = "00000000000000000000000000000003";
    const B: &str = "00000000000000000000000000000004";
    struct Mock {
        steps: VecDeque<(&'static str, String, Option<Value>, Option<Value>)>,
    }
    impl Transport for Mock {
        fn request(
            &mut self,
            method: &str,
            path: &str,
            body: Option<&Value>,
        ) -> Result<Option<Value>, String> {
            let (m, p, b, result) = self.steps.pop_front().expect("unexpected provider request");
            assert_eq!(method, m);
            assert_eq!(path, p);
            assert_eq!(body, b.as_ref());
            Ok(result)
        }
    }
    fn plan() -> Value {
        json!({"base":"/app/","build":"synthetic-build","immutableAssets":["/app/scripts/chunk-a1.js"]})
    }
    fn origin() -> reqwest::Url {
        audit::origin("https://www.example.com").unwrap()
    }
    fn snapshot(version: &str, rules: Vec<Value>) -> Value {
        json!({"id":R,"kind":"zone","phase":PHASE,"version":version,"rules":rules})
    }
    fn installed(rule: &Value, id: &str) -> Value {
        let mut value = rule.clone();
        value["id"] = json!(id);
        value["version"] = json!("1");
        value
    }
    #[test]
    fn policy_is_exact_anonymous_and_bypasses_documents() {
        let rules = desired(&plan(), &origin()).unwrap();
        let expression = rules[0]["expression"].as_str().unwrap();
        assert!(expression.contains("http.host eq \"www.example.com\""));
        assert!(expression.contains("/app/scripts/chunk-a1.js"));
        assert!(expression.contains("cookie") && expression.contains("authorization"));
        assert!(expression.contains("lower("));
        assert_eq!(
            rules[0]["action_parameters"]["edge_ttl"]["mode"],
            "bypass_by_default"
        );
        assert_eq!(rules[1]["action_parameters"]["cache"], false);
        assert!(
            rules[1]["expression"]
                .as_str()
                .unwrap()
                .contains("and not (http.request.method")
        );
        for path in [
            "/app/api/x.js",
            "/app/_r/x.js",
            "/app/_brain/x.js",
            "/app/s/x.js",
            "/app/healthz",
            "/app/sw.js",
            "/app/page.html",
            "/else/chunk.js",
        ] {
            let mut plan = plan();
            plan["immutableAssets"] = json!([path]);
            assert!(desired(&plan, &origin()).is_err(), "{path}");
        }
    }
    #[test]
    fn apply_creates_only_owned_rules_and_preserves_foreign_rules() {
        let p = plan();
        let o = origin();
        let rules = desired(&p, &o).unwrap();
        let other = json!({"id":"00000000000000000000000000000005","ref":"foreign","action":"set_cache_settings","expression":"http.host eq \"else.example.com\"","action_parameters":{"cache":true}});
        let first = snapshot("1", vec![other.clone()]);
        let second = snapshot("2", vec![other.clone(), installed(&rules[1], B)]);
        let third = snapshot(
            "3",
            vec![
                other.clone(),
                installed(&rules[1], B),
                installed(&rules[0], A),
            ],
        );
        let mut a = rules[0].clone();
        a["position"] = json!({"after":""});
        let mut b = rules[1].clone();
        b["position"] = json!({"after":""});
        let mut mock = Mock {
            steps: VecDeque::from([
                ("GET", entry(Z), None, Some(first.clone())),
                (
                    "POST",
                    format!("/zones/{Z}/rulesets/{R}/rules"),
                    Some(b),
                    Some(second.clone()),
                ),
                ("GET", entry(Z), None, Some(second.clone())),
                ("GET", entry(Z), None, Some(second.clone())),
                (
                    "POST",
                    format!("/zones/{Z}/rulesets/{R}/rules"),
                    Some(a),
                    Some(third.clone()),
                ),
                ("GET", entry(Z), None, Some(third.clone())),
            ]),
        };
        let result = apply(
            &mut mock,
            &p,
            Z,
            &o,
            &preview(&p, Z, &o, &first).unwrap(),
            &first,
        )
        .unwrap();
        assert_eq!(result["managedOperations"], 2);
        assert!(mock.steps.is_empty());
        assert_eq!(foreign(&third, &rules), json!([other]));
    }
    #[test]
    fn stale_or_altered_request_never_writes_and_revision_race_never_writes() {
        let p = plan();
        let o = origin();
        let first = snapshot("1", vec![]);
        let mut request = preview(&p, Z, &o, &first).unwrap();
        request["origin"] = json!("https://else.example.com");
        let mut empty = Mock {
            steps: VecDeque::new(),
        };
        assert!(apply(&mut empty, &p, Z, &o, &request, &first).is_err());
        let mut changed = first.clone();
        changed["version"] = json!("2");
        let mut mock = Mock {
            steps: VecDeque::from([("GET", entry(Z), None, Some(changed))]),
        };
        assert!(
            apply(
                &mut mock,
                &p,
                Z,
                &o,
                &preview(&p, Z, &o, &first).unwrap(),
                &first
            )
            .is_err()
        );
        assert!(mock.steps.is_empty());
    }
    #[test]
    fn updating_uses_only_the_owned_rule_endpoint_and_checks_readback() {
        let p = plan();
        let o = origin();
        let rules = desired(&p, &o).unwrap();
        let mut old = installed(&rules[1], B);
        old["enabled"] = json!(false);
        let first = snapshot("1", vec![installed(&rules[0], A), old]);
        let second = snapshot("2", vec![installed(&rules[0], A), installed(&rules[1], B)]);
        let mut body = rules[1].clone();
        body["position"] = json!({"after":""});
        let mut mock = Mock {
            steps: VecDeque::from([
                ("GET", entry(Z), None, Some(first.clone())),
                (
                    "PATCH",
                    format!("/zones/{Z}/rulesets/{R}/rules/{B}"),
                    Some(body),
                    Some(second),
                ),
                ("GET", entry(Z), None, Some(first.clone())),
            ]),
        };
        assert!(
            apply(
                &mut mock,
                &p,
                Z,
                &o,
                &preview(&p, Z, &o, &first).unwrap(),
                &first
            )
            .is_err()
        );
        assert!(mock.steps.is_empty());
    }
    #[test]
    fn scope_cannot_select_an_unrelated_zone() {
        let mut mock = Mock {
            steps: VecDeque::from([(
                "GET",
                format!("/zones/{Z}"),
                None,
                Some(json!({"id":Z,"name":"other.example.com"})),
            )]),
        };
        assert!(zone_check(&mut mock, Z, &origin()).is_err());
        assert!(mock.steps.is_empty());
    }
    #[test]
    fn missing_entrypoint_is_read_only_and_requires_manual_setup() {
        let options = Options::parse(&["plan".into()]).unwrap();
        let mut mock = Mock {
            steps: VecDeque::from([
                (
                    "GET",
                    format!("/zones/{Z}"),
                    None,
                    Some(json!({"id":Z,"name":"example.com"})),
                ),
                ("GET", entry(Z), None, None),
            ]),
        };
        assert!(
            execute(&mut mock, &plan(), &options, Z, &origin(), None)
                .unwrap_err()
                .contains("no shared entrypoint was created")
        );
        assert!(mock.steps.is_empty());
    }
    #[test]
    fn apply_updates_each_owned_rule_and_confirms_final_definitions() {
        let p = plan();
        let o = origin();
        let rules = desired(&p, &o).unwrap();
        let mut old_a = installed(&rules[0], A);
        old_a["enabled"] = json!(false);
        let mut old_b = installed(&rules[1], B);
        old_b["enabled"] = json!(false);
        let first = snapshot("1", vec![old_a.clone(), old_b]);
        let second = snapshot("2", vec![old_a, installed(&rules[1], B)]);
        let third = snapshot("3", vec![installed(&rules[1], B), installed(&rules[0], A)]);
        let mut a = rules[0].clone();
        a["position"] = json!({"after":""});
        let mut b = rules[1].clone();
        b["position"] = json!({"after":""});
        let mut mock = Mock {
            steps: VecDeque::from([
                ("GET", entry(Z), None, Some(first.clone())),
                (
                    "PATCH",
                    format!("/zones/{Z}/rulesets/{R}/rules/{B}"),
                    Some(b),
                    Some(second.clone()),
                ),
                ("GET", entry(Z), None, Some(second.clone())),
                ("GET", entry(Z), None, Some(second)),
                (
                    "PATCH",
                    format!("/zones/{Z}/rulesets/{R}/rules/{A}"),
                    Some(a),
                    Some(third.clone()),
                ),
                ("GET", entry(Z), None, Some(third)),
            ]),
        };
        assert_eq!(
            apply(
                &mut mock,
                &p,
                Z,
                &o,
                &preview(&p, Z, &o, &first).unwrap(),
                &first
            )
            .unwrap()["managedOperations"],
            2
        );
        assert!(mock.steps.is_empty());
    }
    #[test]
    fn ownership_collisions_and_later_foreign_rules_are_detected() {
        let p = plan();
        let o = origin();
        let rules = desired(&p, &o).unwrap();
        let valid = installed(&rules[0], A);
        assert!(
            preview(
                &p,
                Z,
                &o,
                &snapshot("1", vec![valid.clone(), valid.clone()])
            )
            .is_err()
        );
        let mut wrong = valid.clone();
        wrong["expression"] = json!("http.host eq \"other.example.com\"");
        assert!(preview(&p, Z, &o, &snapshot("1", vec![wrong])).is_err());
        let foreign = json!({"ref":"foreign","expression":"true","action":"set_cache_settings","action_parameters":{"cache":true}});
        assert!(!policy_matches(
            &snapshot("1", vec![valid, installed(&rules[1], B), foreign]),
            &rules
        ));
    }
    #[test]
    fn concurrent_foreign_edit_after_write_stops_without_a_second_mutation() {
        let p = plan();
        let o = origin();
        let rules = desired(&p, &o).unwrap();
        let first = snapshot("1", vec![]);
        let after = snapshot(
            "2",
            vec![
                json!({"ref":"foreign","expression":"true"}),
                installed(&rules[1], B),
            ],
        );
        let mut body = rules[1].clone();
        body["position"] = json!({"after":""});
        let mut mock = Mock {
            steps: VecDeque::from([
                ("GET", entry(Z), None, Some(first.clone())),
                (
                    "POST",
                    format!("/zones/{Z}/rulesets/{R}/rules"),
                    Some(body),
                    Some(after.clone()),
                ),
                ("GET", entry(Z), None, Some(after)),
            ]),
        };
        assert!(
            apply(
                &mut mock,
                &p,
                Z,
                &o,
                &preview(&p, Z, &o, &first).unwrap(),
                &first
            )
            .unwrap_err()
            .contains("foreign rules changed concurrently")
        );
        assert!(mock.steps.is_empty());
    }
}
