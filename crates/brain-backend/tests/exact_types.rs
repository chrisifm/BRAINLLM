//! Exact field types, defaults and declarative constraints (migration requirements B-02..B-07).
use brain_backend::{
    contract::{Check, Constraints, Field, Hooks, Policy, Resource},
    db,
};
use serde_json::json;
use std::collections::BTreeMap;

fn field(kind: &str, required: bool) -> Field {
    Field {
        kind: kind.into(),
        required,
        ..Default::default()
    }
}
fn legacy() -> Resource {
    Resource {
        name: "notes".into(),
        search: vec!["title".into()],
        fields: BTreeMap::from([
            ("title".into(), field("str", true)),
            ("score".into(), field("num", false)),
            ("done".into(), field("bool", false)),
            ("meta".into(), field("obj", false)),
            ("contact".into(), field("email", false)),
        ]),
        policy: Policy::default(),
        hooks: Hooks::default(),
        constraints: Constraints::default(),
        query: Default::default(),
    }
}
fn vacancies() -> Resource {
    let mut fields = BTreeMap::from([
        ("title".into(), field("str", true)),
        ("openings".into(), field("int", false)),
        ("salary_min".into(), field("decimal(10,2)", false)),
        ("salary_max".into(), field("decimal(10,2)", false)),
        ("currency".into(), field("enum(MXN|USD)", false)),
        ("status".into(), field("enum(draft|published|closed)", true)),
        ("closes_on".into(), field("date", false)),
        ("published_at".into(), field("datetime", false)),
        ("tags".into(), field("list<str,3>", false)),
        ("slug".into(), field("str", true)),
    ]);
    fields.get_mut("status").unwrap().default = Some(json!("draft"));
    fields.get_mut("openings").unwrap().default = Some(json!(1));
    fields.get_mut("slug").unwrap().unique = true;
    Resource {
        name: "vacancies".into(),
        search: vec!["title".into()],
        fields,
        policy: Policy::default(),
        hooks: Hooks::default(),
        query: Default::default(),
        constraints: Constraints {
            unique: vec![vec!["title".into(), "closes_on".into()]],
            check: vec![
                Check {
                    field: Some("openings".into()),
                    min: Some(json!(1)),
                    max: Some(json!(500)),
                    ..Default::default()
                },
                Check {
                    compare: Some(vec!["salary_min".into(), "<=".into(), "salary_max".into()]),
                    ..Default::default()
                },
                Check {
                    together: Some(vec!["salary_min".into(), "salary_max".into(), "currency".into()]),
                    ..Default::default()
                },
            ],
        },
    }
}

#[test]
fn legacy_contracts_serialize_and_record_exactly_as_before() {
    let r = legacy();
    r.validate().unwrap();
    // The recorded value compared by BDB_SCHEMA_DRIFT must be the pre-existing serialization.
    assert_eq!(
        r.recorded_fields(),
        json!({"contact":{"kind":"email","required":false},"done":{"kind":"bool","required":false},"meta":{"kind":"obj","required":false},"score":{"kind":"num","required":false},"title":{"kind":"str","required":true}})
    );
    let encoded = serde_json::to_value(&r).unwrap();
    assert!(encoded.get("constraints").is_none());
    assert!(r.sql_checks().is_empty() && r.unique_sets().is_empty());
    assert_eq!(
        r.schema(),
        json!({"type":"object","additionalProperties":false,"properties":{"contact":{"type":"string","format":"email","maxLength":254},"done":{"type":"boolean"},"meta":{"type":"object"},"score":{"type":"number"},"title":{"type":"string","maxLength":16384,"minLength":1}},"required":["title"]})
    );
    // Legacy values are accepted exactly as before, including floating-point numbers.
    assert_eq!(
        r.input(&json!({"title":"Hi","score":1.5,"done":true})).unwrap(),
        json!({"title":"Hi","score":1.5,"done":true})
    );
    assert_eq!(
        r.input(&json!({"title":"Hi","score":"1"})).unwrap_err()["score"],
        "Expected num"
    );
    // Previously recorded rows deserialize unchanged.
    let old: Resource = serde_json::from_value(json!({"name":"notes","fields":{"title":{"kind":"str","required":true}},"policy":{"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}})).unwrap();
    assert!(old.validate().is_ok() && old.constraints.is_empty());
}

#[test]
fn exact_values_are_normalized_defaulted_and_checked() {
    let r = vacancies();
    r.validate().unwrap();
    let saved = r
        .input(&json!({
            "title":"Engineer","slug":"engineer","salary_min":"25000.5","salary_max":"30000",
            "currency":"MXN","closes_on":"2026-12-31","published_at":"2026-10-30T08:15:00-06:00",
            "tags":["rust","remote"]
        }))
        .unwrap();
    assert_eq!(
        saved,
        json!({"title":"Engineer","slug":"engineer","salary_min":"25000.50","salary_max":"30000.00","currency":"MXN","closes_on":"2026-12-31","published_at":"2026-10-30T14:15:00.000Z","tags":["rust","remote"],"status":"draft","openings":1})
    );
    let errors = r
        .input(&json!({
            "title":"A","slug":"a","openings":2.5,"salary_min":19.99,"status":"archived",
            "closes_on":"2026-02-30","published_at":"2026-10-30T08:15:00","tags":["a","b","c","d"]
        }))
        .unwrap_err();
    for key in ["openings", "salary_min", "status", "closes_on", "published_at", "tags"] {
        assert!(errors.contains_key(key), "{key}: {errors:?}");
    }
    assert!(errors["salary_min"].contains("\"19.99\""), "decimal fix shows the string form");
    assert!(errors["status"].contains("draft, published, closed"));
    // Declared checks run on normalized values.
    let errors = r
        .input(&json!({"title":"A","slug":"a","openings":0,"salary_min":"10.00","salary_max":"9.99","currency":"USD"}))
        .unwrap_err();
    assert_eq!(errors["openings"], "Must be >= 1");
    assert_eq!(errors["salary_min"], "Must be <= salary_max");
    let errors = r.input(&json!({"title":"A","slug":"a","salary_min":"1"})).unwrap_err();
    assert!(errors.contains_key("salary_max") && errors.contains_key("currency"));
    assert!(r
        .input(&json!({"title":"A","slug":"a","salary_min":"10","salary_max":"10.00","currency":"USD"}))
        .is_ok());
}

#[test]
fn invalid_declarations_fail_with_actionable_messages() {
    let mut r = vacancies();
    r.fields.get_mut("status").unwrap().default = Some(json!("archived"));
    assert!(r.validate().unwrap_err().contains("default does not match"));
    let mut r = vacancies();
    r.fields.insert("price".into(), field("decimal(10,2)", false));
    r.fields.get_mut("price").unwrap().default = Some(json!(9.5));
    assert!(r.validate().unwrap_err().contains("decimal string"));
    let mut r = vacancies();
    r.fields.insert("meta".into(), Field { kind: "obj".into(), unique: true, ..Default::default() });
    assert!(r.validate().unwrap_err().contains("cannot be unique"));
    let mut r = vacancies();
    r.constraints.unique.push(vec!["slug".into()]);
    assert!(r.validate().unwrap_err().contains("unique: true"));
    let mut r = vacancies();
    r.constraints.unique.push(vec!["closes_on".into(), "title".into()]);
    assert!(r.validate().unwrap_err().contains("duplicate"));
    let mut r = vacancies();
    r.constraints.check.push(Check { compare: Some(vec!["salary_min".into(), "<=".into(), "closes_on".into()]), ..Default::default() });
    assert!(r.validate().unwrap_err().contains("same type"));
    let mut r = vacancies();
    r.constraints.check.push(Check { field: Some("title".into()), min: Some(json!("a")), ..Default::default() });
    assert!(r.validate().is_err());
    let mut r = vacancies();
    r.constraints.check.push(Check { field: Some("openings".into()), together: Some(vec!["a".into()]), ..Default::default() });
    assert!(r.validate().unwrap_err().contains("exactly one form"));
    let mut r = vacancies();
    r.constraints.check.push(Check { together: Some(vec!["salary_min".into(), "missing".into()]), ..Default::default() });
    assert!(r.validate().unwrap_err().contains("undeclared field `missing`"));
    let mut r = vacancies();
    r.fields.insert("bad".into(), field("money", false));
    assert!(r.validate().unwrap_err().contains("decimal(p,s)"));
}

#[test]
fn database_objects_are_deterministic_bounded_and_documented_in_openapi() {
    let r = vacancies();
    let checks = r.sql_checks();
    // int, two decimals, two enums, date, datetime, list + three declared checks.
    assert_eq!(checks.len(), 11);
    assert_eq!(checks, vacancies().sql_checks());
    for (name, expr) in &checks {
        assert!(name.starts_with("_brain_ck_") && name.len() <= 63, "{name}");
        assert!(!expr.contains(';') && !expr.contains("--"), "{expr}");
    }
    assert!(checks.iter().any(|(_, e)| e.starts_with("data ? 'status' AND") && e.contains("IN ('draft','published','closed')")));
    assert!(checks.iter().any(|(_, e)| e.contains("(data->>'salary_min')::numeric<=(data->>'salary_max')::numeric")));
    assert!(checks.iter().any(|(_, e)| e == "((data ? 'salary_min')::int+(data ? 'salary_max')::int+(data ? 'currency')::int) IN (0,3)"));
    let uniques = r.unique_sets();
    assert_eq!(uniques.iter().map(|(_, f)| f.clone()).collect::<Vec<_>>(), vec![vec!["slug".to_string()], vec!["title".into(), "closes_on".into()]]);
    let mut changed = vacancies();
    changed.fields.get_mut("currency").unwrap().kind = "enum(MXN|USD|EUR)".into();
    assert_ne!(changed.sql_checks(), checks, "a changed definition gets a new object name");
    assert!(r.recorded_fields()["$constraints"]["unique"].is_array());

    let api = brain_backend::contract::openapi(&[r]);
    let post = &api["paths"]["/api/vacancies"]["post"]["requestBody"]["content"]["application/json"]["schema"];
    assert_eq!(post["properties"]["openings"]["type"], "integer");
    assert_eq!(post["properties"]["openings"]["default"], 1);
    assert_eq!(post["properties"]["salary_min"]["pattern"], "^-?(0|[1-9][0-9]{0,7})[.][0-9]{2}$");
    assert_eq!(post["properties"]["status"]["enum"], json!(["draft", "published", "closed"]));
    assert_eq!(post["properties"]["published_at"]["format"], "date-time");
    assert_eq!(post["properties"]["tags"]["maxItems"], 3);
    assert_eq!(post["properties"]["slug"]["x-brain-unique"], true);
    assert_eq!(post["required"], json!(["slug", "title"]), "a defaulted required field is optional input");
    let row = &api["paths"]["/api/vacancies/{id}"]["get"]["responses"]["200"]["content"]["application/json"]["schema"];
    assert_eq!(row["required"], json!(["id", "data", "created_at", "updated_at"]), "owner is hidden from public reads unless expose_owner");
    assert_eq!(row["properties"]["created_at"]["format"], "date-time");
}

/// Requires PostgreSQL: run through scripts/backend-check.mjs (owned disposable cluster, no Docker).
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_enforces_exact_types_constraints_and_exposes_timestamps() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL")
        .expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let pool = db::connect(&url).await.unwrap();
    let (typed, plain) = (vacancies(), legacy());
    db::migrate(&pool, &[typed.clone(), plain.clone()]).await.unwrap();
    db::migrate(&pool, &[typed.clone(), plain.clone()]).await.unwrap();
    db::verify(&pool, &[typed.clone(), plain.clone()]).await.unwrap();
    let legacy_checks: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_constraint WHERE conrelid='_brain_res_notes'::regclass AND conname LIKE '\\_brain\\_ck\\_%'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(legacy_checks, 0, "legacy resources gain no database constraints");
    let user = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role) VALUES($1,$2,'x','admin')")
        .bind(user).bind(format!("{user}@example.test")).execute(&pool).await.unwrap();
    let data = typed.input(&json!({"title":"Engineer","slug":"engineer","salary_min":"90071992547409.99","salary_max":"99999999.99","currency":"USD","published_at":"2026-10-30T08:15:00.25-06:00"}));
    assert!(data.is_err(), "precision 10 rejects 14 integer digits before storage");
    // Unique per run so the test can be repeated against the same disposable database.
    let run = uuid::Uuid::new_v4().simple().to_string();
    let data = typed.input(&json!({"title":format!("Engineer {run}"),"slug":format!("engineer-{run}"),"salary_min":"12345678.91","salary_max":"99999999.99","currency":"USD","published_at":"2026-10-30T08:15:00.25-06:00","closes_on":"2026-12-31"})).unwrap();
    let row = db::create(&pool, &typed, user, data.clone()).await.unwrap();
    let iso = |v: &serde_json::Value| v.as_str().is_some_and(|s| s.len() == 24 && s.ends_with('Z') && s.as_bytes()[10] == b'T');
    assert!(iso(&row["created_at"]) && iso(&row["updated_at"]), "{row}");
    assert_eq!(row["data"]["salary_min"], "12345678.91");
    let exact: String = sqlx::query_scalar("SELECT ((data->>'salary_min')::numeric(10,2)+0.09)::text FROM _brain_res_vacancies WHERE id=$1")
        .bind(uuid::Uuid::parse_str(row["id"].as_str().unwrap()).unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!(exact, "12345679.00", "stored decimals cast losslessly to numeric");
    let listed = db::list(&pool, &typed, None, "", 100, 0).await.unwrap();
    let again = listed.iter().find(|r| r["id"] == row["id"]).expect("created record is listed");
    assert_eq!(again["created_at"], row["created_at"]);
    // Unique field and composite indexes report the declared fields.
    let mut duplicate = data.clone();
    duplicate["title"] = json!(format!("Other {run}"));
    let error = db::create(&pool, &typed, user, duplicate).await.unwrap_err();
    assert_eq!(db::constraint_fields(&typed, &error), Some(("unique", vec!["slug".to_string()])));
    let mut duplicate = data.clone();
    duplicate["slug"] = json!(format!("other-{run}"));
    let error = db::create(&pool, &typed, user, duplicate).await.unwrap_err();
    assert_eq!(db::constraint_fields(&typed, &error), Some(("unique", vec!["title".to_string(), "closes_on".to_string()])));
    // Writes that bypass application validation are still rejected by CHECK constraints.
    for bad in [
        json!({"title":"B","slug":"b1","status":"archived"}),
        json!({"title":"B","slug":"b2"}),
        json!({"title":"B","slug":"b3","status":"draft","openings":1.5}),
        json!({"title":"B","slug":"b4","status":"draft","openings":0}),
        json!({"title":"B","slug":"b5","status":"draft","salary_min":19.99,"salary_max":"20.00","currency":"USD"}),
        json!({"title":"B","slug":"b6","status":"draft","salary_min":"30.00","salary_max":"20.00","currency":"USD"}),
        json!({"title":"B","slug":"b7","status":"draft","salary_min":"10.00"}),
        json!({"title":"B","slug":"b8","status":"draft","closes_on":"31/12/2026"}),
        json!({"title":"B","slug":"b9","status":"draft","published_at":"2026-10-30T08:15:00Z"}),
        json!({"title":"B","slug":"b10","status":"draft","tags":["a",1]}),
        json!({"title":"B","slug":"b11","status":"draft","tags":["a","b","c","d"]}),
    ] {
        let error = db::create(&pool, &typed, user, bad.clone()).await.unwrap_err();
        let code = error.as_database_error().and_then(|e| e.code()).map(|c| c.into_owned());
        assert!(matches!(code.as_deref(), Some("23514") | Some("22P02")), "{bad}: {code:?}");
    }
    // A changed contract is refused instead of silently altering deployed data.
    let mut changed = typed.clone();
    changed.fields.get_mut("currency").unwrap().kind = "enum(MXN|USD|EUR)".into();
    assert!(db::migrate(&pool, &[changed.clone()]).await.unwrap_err().contains("BDB_SCHEMA_DRIFT"));
    assert!(db::verify(&pool, &[changed]).await.is_err());
    // A dropped constraint is detected before serving.
    let (name, _) = typed.sql_checks().remove(0);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE _brain_res_vacancies DROP CONSTRAINT {name}"))).execute(&pool).await.unwrap();
    assert!(db::verify(&pool, &[typed.clone()]).await.unwrap_err().contains("BDB_SCHEMA_DRIFT"));
    db::migrate(&pool, &[typed.clone()]).await.unwrap();
    db::verify(&pool, &[typed]).await.unwrap();
}
