//! Private explicit project memory. Evidence is reported, never inferred execution or model training.
use serde_json::{Value, json};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: usize = 1024 * 1024;
const MAX_RECORDS: usize = 1000;
const PRODUCER: &str = "brain-project-history";
type Result<T> = std::result::Result<T, String>;
fn empty() -> Value {
    json!({"schemaVersion":1,"producer":PRODUCER,"sequence":0,"tasks":[],"evidence":[],"lessons":[],"incidents":[],"timeline":[]})
}
fn load(dir: &Path) -> Result<(Value, Value)> {
    let path = dir.join("history.json");
    if !path
        .try_exists()
        .map_err(|_| "cannot inspect project history")?
    {
        return Ok((empty(), Value::Null));
    }
    let value = crate::manifests::read_json(&path, MAX_BYTES)?;
    if value["schemaVersion"] != 1 || value["producer"] != PRODUCER {
        return Err(
            "unsupported or unowned project history; preserve it and inspect its schema".into(),
        );
    }
    let serialized = serde_json::to_string(&value).map_err(|_| "invalid project history")?;
    if !brain_check::scan_secrets(&serialized).is_empty() {
        return Err(
            "project history contains detected sensitive content; inspect the private file locally"
                .into(),
        );
    }
    for field in ["tasks", "evidence", "lessons", "incidents", "timeline"] {
        if !value[field]
            .as_array()
            .is_some_and(|a| a.len() <= MAX_RECORDS)
        {
            return Err("project history collection is invalid or exceeds its budget".into());
        }
    }
    let revision = json!(crate::manifests::hash(
        &serde_json::to_vec(&value).map_err(|_| "invalid history")?
    ));
    Ok((value, revision))
}
fn text(args: &Value, key: &str, max: usize) -> Result<String> {
    let s = args[key]
        .as_str()
        .filter(|s| {
            !s.trim().is_empty()
                && s.len() <= max
                && !s.chars().any(|c| c.is_control() && c != '\n')
        })
        .ok_or_else(|| format!("{key} must be public text of 1..{max} bytes"))?;
    // A secret detector cannot prove arbitrary prose safe. Refuse high-risk forms instead of retaining payloads.
    let lower = s.to_ascii_lowercase();
    if !brain_check::scan_secrets(s).is_empty()
        || [
            "authorization:",
            "bearer ",
            "cookie:",
            "password=",
            "token=",
            "secret=",
            "database_url=",
            "-----begin",
        ]
        .iter()
        .any(|v| lower.contains(v))
    {
        return Err(format!(
            "{key} contains sensitive-looking content; provide a public summary"
        ));
    }
    if s.split_whitespace().any(|word| {
        word.split_once("://").is_some_and(|(_, rest)| {
            rest.split(['/', '?', '#'])
                .next()
                .is_some_and(|authority| authority.contains('@'))
        })
    }) {
        return Err(format!(
            "{key} contains URL credentials; provide a public summary"
        ));
    }
    Ok(s.split_whitespace()
        .map(|word| {
            if word.contains("https://") || word.contains("http://") {
                word.split(['?', '#']).next().unwrap_or(word)
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" "))
}
fn choice<'a>(args: &'a Value, key: &str, choices: &[&str], default: &'a str) -> Result<&'a str> {
    let value = args.get(key).map_or(Ok(default), |v| {
        v.as_str().ok_or("enum value must be a string")
    })?;
    if !choices.contains(&value) {
        return Err(format!("invalid {key}; expected {}", choices.join(", ")));
    }
    Ok(value)
}
fn task_index(data: &Value, args: &Value) -> Result<usize> {
    let id = args["taskId"].as_str().ok_or("taskId is required")?;
    data["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .position(|t| t["id"] == id)
        .ok_or_else(|| "task not found".into())
}
fn lesson_evidence(data: &Value, args: &Value) -> Result<Vec<String>> {
    let Some(ids) = args.get("evidenceIds") else {
        return Ok(vec![]);
    };
    let ids = ids
        .as_array()
        .filter(|a| a.len() <= 32)
        .ok_or("evidenceIds must contain at most 32 evidence IDs")?;
    ids.iter()
        .map(|id| {
            let id = id.as_str().ok_or("evidence ID must be a string")?;
            if !data["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["id"] == id && e["taskId"] == args["taskId"])
            {
                return Err(
                    "lesson evidence must reference existing evidence for the same task".into(),
                );
            }
            Ok(id.to_owned())
        })
        .collect()
}
fn location(root: &Path, args: &Value) -> Result<Value> {
    let Some(file) = args.get("file") else {
        return Ok(Value::Null);
    };
    let file = file
        .as_str()
        .ok_or("file must be an app-relative source path")?;
    crate::changes::source_path(root, file)?;
    let source = crate::map::read_source_limit(root, file, 262144)?;
    let line = args["line"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= source.lines().count().max(1) as u64)
        .ok_or("line must exist in the current source")?;
    Ok(
        json!({"file":file,"line":line,"column":null,"sourceRevision":crate::manifests::hash(source.as_bytes()),"precision":"declared-source-line","proof":"caller-reported; not inferred root cause"}),
    )
}
/// Bounded metadata queries. `limit` is 1..100; `cursor` is a numeric offset.
pub fn query(root: &Path, op: &str, args: &Value) -> Result<Value> {
    if !args.is_object() {
        return Err("history arguments must be an object".into());
    }
    let dir = crate::manifests::private_dir(root, "history")?;
    let (data, revision) = load(&dir)?;
    if op == "status" {
        return Ok(
            json!({"schemaVersion":1,"revision":revision,"sequence":data["sequence"],"counts":{"tasks":data["tasks"].as_array().unwrap().len(),"evidence":data["evidence"].as_array().unwrap().len(),"lessons":data["lessons"].as_array().unwrap().len(),"incidents":data["incidents"].as_array().unwrap().len()},"private":true,"learning":"explicit local proposals; no model retraining"}),
        );
    }
    if op == "task" {
        let i = task_index(&data, args)?;
        return Ok(json!({"schemaVersion":1,"revision":revision,"task":data["tasks"][i]}));
    }
    if !["tasks", "evidence", "lessons", "incidents", "timeline"].contains(&op) {
        return Err("unknown history query".into());
    }
    let limit = args
        .get("limit")
        .map_or(Some(25), Value::as_u64)
        .filter(|n| *n > 0 && *n <= 100)
        .ok_or("history limit must be 1..100")? as usize;
    let cursor = args
        .get("cursor")
        .map_or(Some(0), Value::as_u64)
        .filter(|n| *n <= MAX_RECORDS as u64)
        .ok_or("invalid history cursor")? as usize;
    let rows = data[op].as_array().unwrap();
    let filtered: Vec<_> = rows
        .iter()
        .filter(|r| {
            args.get("taskId")
                .is_none_or(|id| r["taskId"] == *id || (op == "tasks" && r["id"] == *id))
        })
        .collect();
    let items: Vec<_> = filtered.iter().skip(cursor).take(limit).copied().collect();
    Ok(
        json!({"schemaVersion":1,"revision":revision,"items":items,"total":filtered.len(),"nextCursor":if cursor+items.len()<filtered.len(){Some(cursor+items.len())}else{None},"confidence":"explicit caller reports; command execution and root causes not independently verified"}),
    )
}
/// CAS edit; expectedRevision must be explicit (null only for a missing store). Never executes commands or publishes.
pub fn edit(root: &Path, op: &str, args: &Value) -> Result<Value> {
    if !args.is_object() || args.get("expectedRevision").is_none() {
        return Err(
            "history edits require an explicit expectedRevision; query status first".into(),
        );
    }
    let dir = crate::manifests::private_dir(root, "history")?;
    let _lock = crate::manifests::lock(&dir)?;
    let (mut data, revision) = load(&dir)?;
    if args["expectedRevision"] != revision {
        return Err("project history changed; query its revision and retry".into());
    }
    let sequence = data["sequence"]
        .as_u64()
        .ok_or("invalid history sequence")?
        .checked_add(1)
        .ok_or("history sequence exhausted")?;
    let id = format!("h{sequence}");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock precedes epoch")?
        .as_secs();
    let (collection, mut record) = match op {
        "task-create" => (
            "tasks",
            json!({"id":id,"title":text(args,"title",160)?,"summary":text(args,"summary",1024)?,"state":"open"}),
        ),
        "task-update" => {
            let i = task_index(&data, args)?;
            let state = choice(
                args,
                "state",
                &["open", "active", "blocked", "done", "cancelled"],
                "open",
            )?;
            let summary = text(args, "summary", 1024)?;
            data["tasks"][i]["state"] = json!(state);
            data["tasks"][i]["summary"] = json!(summary);
            data["tasks"][i]["updatedSequence"] = json!(sequence);
            ("", data["tasks"][i].clone())
        }
        "evidence" => {
            task_index(&data, args)?;
            let kind = choice(
                args,
                "kind",
                &["command", "manual", "artifact", "observation"],
                "observation",
            )?;
            let state = choice(
                args,
                "state",
                &["passed", "failed", "unknown", "not-run"],
                "unknown",
            )?;
            let tool = if kind == "command" {
                Some(text(args, "tool", 80)?)
            } else {
                None
            };
            if tool.as_ref().is_some_and(|s| {
                !s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-./".contains(&b))
            }) {
                return Err(
                    "tool must name an executable only; command arguments/output are not stored"
                        .into(),
                );
            }
            let exit = args
                .get("exitCode")
                .map(|v| {
                    v.as_i64()
                        .filter(|n| (-255..=255).contains(n))
                        .ok_or("exitCode must be -255..255")
                })
                .transpose()?;
            (
                "evidence",
                json!({"id":id,"taskId":args["taskId"],"kind":kind,"state":state,"summary":text(args,"summary",1024)?,"tool":tool,"exitCode":exit,"verification":"caller-reported"}),
            )
        }
        "lesson" => {
            task_index(&data, args)?;
            (
                "lessons",
                json!({"id":id,"taskId":args["taskId"],"proposal":text(args,"proposal",1024)?,"rationale":text(args,"rationale",1024)?,"status":"proposed","evidenceIds":lesson_evidence(&data,args)?,"application":"explicit future review required; no automatic rule or model update"}),
            )
        }
        "incident" => {
            let classification = choice(
                args,
                "classification",
                &["app", "framework", "unknown"],
                "unknown",
            )?;
            let mut record = json!({"id":id,"classification":classification,"classificationProof":"caller-reported; unverified","summary":text(args,"summary",1024)?,"cause":text(args,"cause",1024)?,"fix":text(args,"fix",1024)?,"location":location(root,args)?,"publication":"local-only"});
            if args.get("rid").is_some() {
                let rid = text(args, "rid", 128)?;
                if !rid
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
                {
                    return Err("rid must be an opaque identifier".into());
                }
                record["rid"] = json!(rid);
            }
            if args.get("taskId").is_some() {
                task_index(&data, args)?;
                record["taskId"] = args["taskId"].clone();
            }
            ("incidents", record)
        }
        _ => return Err(
            "unknown history edit; expected task-create, task-update, evidence, lesson or incident"
                .into(),
        ),
    };
    record.as_object_mut().unwrap().remove("checksum");
    record["sequence"] = json!(sequence);
    record["reportedAtUnixSeconds"] = json!(timestamp);
    let checksum = crate::manifests::hash(
        &serde_json::to_vec(&record).map_err(|_| "cannot serialize history record")?,
    );
    record["checksum"] = json!(checksum);
    if collection.is_empty() {
        let i = task_index(&data, args)?;
        data["tasks"][i] = record.clone();
    } else {
        let rows = data[collection].as_array_mut().unwrap();
        if rows.len() >= MAX_RECORDS {
            return Err("history collection full; explicitly archive a private reviewed copy before further edits".into());
        }
        rows.push(record.clone());
    }
    let events = data["timeline"].as_array_mut().unwrap();
    if events.len() >= MAX_RECORDS {
        return Err("history timeline full; preserve and archive it explicitly".into());
    }
    events.push(json!({"sequence":sequence,"operation":op,"recordId":record["id"],"taskId":args.get("taskId"),"checksum":checksum,"reportedAtUnixSeconds":timestamp}));
    data["sequence"] = json!(sequence);
    let bytes = serde_json::to_vec(&data).map_err(|_| "cannot serialize project history")?;
    if bytes.len() > MAX_BYTES {
        return Err(
            "project history exceeds 1 MiB; explicitly archive a reviewed private copy".into(),
        );
    }
    crate::manifests::atomic(&dir.join("history.json"), &bytes)?;
    Ok(
        json!({"schemaVersion":1,"revision":crate::manifests::hash(&bytes),"record":record,"private":true,"published":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct App(std::path::PathBuf);
    impl App {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "brain-history-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn cas_tasks_evidence_and_proposals_remain_explicit() {
        let app = App::new();
        let first = edit(
            &app.0,
            "task-create",
            &json!({"expectedRevision":null,"title":"Fix card","summary":"Review empty state"}),
        )
        .unwrap();
        assert!(
            edit(
                &app.0,
                "task-update",
                &json!({"expectedRevision":null,"taskId":"h1","state":"done","summary":"Done"})
            )
            .is_err()
        );
        let e=edit(&app.0,"evidence",&json!({"expectedRevision":first["revision"],"taskId":"h1","kind":"manual","state":"unknown","summary":"Browser interaction not checked"})).unwrap();
        let l=edit(&app.0,"lesson",&json!({"expectedRevision":e["revision"],"taskId":"h1","proposal":"Review keyboard state","rationale":"Manual evidence is unknown"})).unwrap();
        assert_eq!(l["record"]["status"], "proposed");
        assert_eq!(
            query(&app.0, "timeline", &json!({"limit":2})).unwrap()["nextCursor"],
            2
        );
        assert_eq!(
            query(&app.0, "task", &json!({"taskId":"h1"})).unwrap()["task"]["state"],
            "open"
        );
    }
    #[test]
    fn incident_is_local_source_revision_only_and_sensitive_content_refused() {
        let app = App::new();
        std::fs::write(app.0.join("app.brain"), "<p>private-source-body</p>\n").unwrap();
        let i = edit(
            &app.0,
            "incident",
            &json!({"expectedRevision":null,"summary":"Failure at https://example.test/path?token=private","cause":"Unknown","fix":"Inspect trace","file":"app.brain","line":1}),
        );
        assert!(i.is_err());
        let i=edit(&app.0,"incident",&json!({"expectedRevision":null,"summary":"Failure at https://example.test/path?search=private","cause":"Unknown","fix":"Inspect trace","file":"app.brain","line":1})).unwrap();
        assert!(!i.to_string().contains("search=private"));
        assert!(!i.to_string().contains("private-source-body"));
        assert_eq!(i["record"]["classification"], "unknown");
        assert_eq!(i["published"], false);
        assert!(edit(&app.0,"incident",&json!({"expectedRevision":i["revision"],"summary":"Failure","cause":"Unknown","fix":"Inspect","file":"../outside","line":1})).is_err());
    }
    #[test]
    fn updated_task_checksum_and_lesson_evidence_are_revision_consistent() {
        let app = App::new();
        let first = edit(
            &app.0,
            "task-create",
            &json!({"expectedRevision":null,"title":"Card","summary":"Review"}),
        )
        .unwrap();
        let updated=edit(&app.0,"task-update",&json!({"expectedRevision":first["revision"],"taskId":"h1","state":"done","summary":"Reported complete; tests unknown"})).unwrap();
        let task = query(&app.0, "task", &json!({"taskId":"h1"})).unwrap();
        let mut body = task["task"].clone();
        let checksum = body.as_object_mut().unwrap().remove("checksum").unwrap();
        assert_eq!(
            checksum,
            crate::manifests::hash(&serde_json::to_vec(&body).unwrap())
        );
        assert!(edit(&app.0,"lesson",&json!({"expectedRevision":updated["revision"],"taskId":"h1","proposal":"Check evidence","rationale":"Unknown","evidenceIds":["missing"]})).is_err());
    }
    #[test]
    fn strict_bounds_and_no_raw_command_payload() {
        let app = App::new();
        assert!(
            edit(
                &app.0,
                "task-create",
                &json!({"expectedRevision":null,"title":"x".repeat(161),"summary":"s"})
            )
            .is_err()
        );
        assert!(query(&app.0, "tasks", &json!({"limit":101})).is_err());
        assert!(edit(&app.0, "task-create", &json!({"title":"t","summary":"s"})).is_err());
    }
}
