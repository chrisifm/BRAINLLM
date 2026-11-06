//! Operator-only deterministic backfill, one atomic batch per explicit apply. Never executes application JavaScript.
use crate::{Resource, types::Ty};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use std::{collections::BTreeSet, time::Duration};
use uuid::Uuid;
const ENGINE: &str = "brain-backfill-1";
pub const MAX_LIMIT: u32 = 1000;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", content = "args", deny_unknown_fields)]
pub enum Expr {
    Literal(Value),
    Field(String),
    Id,
    Binary(String, Box<Expr>, Box<Expr>),
    Trim(Box<Expr>),
    Lower(Box<Expr>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub name: String,
    pub resource: String,
    pub field: String,
    pub expression: Expr,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Binding {
    pub engine: String,
    pub name: String,
    pub resource: String,
    pub field: String,
    pub definition: String,
    pub contract: String,
    pub connection: String,
    pub database: Uuid,
    pub watermark: String,
    pub cohort: String,
    pub include_null: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request {
    pub binding: Binding,
    pub revision: i64,
    pub cursor: String,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    String,
    Number,
    Bool,
    Null,
}
fn problem() -> String {
    "Backfill definition is invalid; fix: use one declared unrestricted field and a bounded deterministic BrainScript expression".into()
}
fn safe(error: sqlx::Error) -> String {
    let code = error
        .as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
        .filter(|s| {
            s.len() == 5
                && s.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        });
    format!(
        "Backfill transaction failed{}; no values or SQL were reported; fix: inspect constraints, permissions and locks, then preview again",
        code.map(|c| format!(" (SQLSTATE {c})")).unwrap_or_default()
    )
}
pub fn digest<T: Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|_| problem())?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn field_kind(resource: &Resource, name: &str) -> Result<Kind, String> {
    let field = resource.fields.get(name).ok_or_else(problem)?;
    if !crate::visibility::unrestricted(field)
        || matches!(field.access_mode(), Some("private" | "writeonly" | "roles"))
    {
        return Err(problem());
    }
    match field.ty().ok_or_else(problem)? {
        Ty::Str
        | Ty::Email
        | Ty::Date
        | Ty::DateTime
        | Ty::Enum(_)
        | Ty::Ref(_)
        | Ty::Org
        | Ty::Decimal { .. } => Ok(Kind::String),
        Ty::Int | Ty::Num => Ok(Kind::Number),
        Ty::Bool => Ok(Kind::Bool),
        _ => Err(problem()),
    }
}
impl Expr {
    fn kind(
        &self,
        r: &Resource,
        target: &str,
        depth: usize,
        nodes: &mut usize,
        fields: &mut BTreeSet<String>,
    ) -> Result<Kind, String> {
        *nodes += 1;
        if depth > 16 || *nodes > 128 {
            return Err(problem());
        }
        let mut child = |e: &Expr| e.kind(r, target, depth + 1, nodes, fields);
        match self {
            Self::Literal(v) => match v {
                Value::String(s) if s.len() <= 4096 => Ok(Kind::String),
                Value::Number(n) if n.as_f64().is_some_and(f64::is_finite) => Ok(Kind::Number),
                Value::Bool(_) => Ok(Kind::Bool),
                Value::Null => Ok(Kind::Null),
                _ => Err(problem()),
            },
            Self::Id => Ok(Kind::String),
            Self::Field(name) => {
                if name == target {
                    return Err(problem());
                }
                fields.insert(name.clone());
                if fields.len() > 20 {
                    return Err(problem());
                }
                field_kind(r, name)
            }
            Self::Trim(e) | Self::Lower(e) => {
                if child(e)? == Kind::String {
                    Ok(Kind::String)
                } else {
                    Err(problem())
                }
            }
            Self::Binary(op, a, b) => {
                let (a, b) = (child(a)?, child(b)?);
                match op.as_str() {
                    "??" if a == b || a == Kind::Null => Ok(b),
                    "??" if b == Kind::Null => Ok(a),
                    "+" if a == b && (a == Kind::String || a == Kind::Number) => Ok(a),
                    "-" | "*" | "/" if a == Kind::Number && b == Kind::Number => Ok(Kind::Number),
                    _ => Err(problem()),
                }
            }
        }
    }
    fn evaluate(&self, id: Uuid, data: &Value) -> Result<Value, String> {
        let bad = || {
            "Backfill row cannot be computed or validated; batch and checkpoint were rolled back; fix: repair the source data and preview again (values are not reported)".to_string()
        };
        let value = match self {
            Self::Literal(v) => v.clone(),
            Self::Id => json!(id),
            Self::Field(name) => data.get(name).cloned().unwrap_or(Value::Null),
            Self::Trim(e) => json!(
                e.evaluate(id, data)?
                    .as_str()
                    .ok_or_else(bad)?
                    .trim_matches(|c: char| matches!(
                        c,
                        ' ' | '\t'
                            | '\n'
                            | '\r'
                            | '\u{b}'
                            | '\u{c}'
                            | '\u{a0}'
                            | '\u{feff}'
                            | '\u{1680}'
                            | '\u{2000}'
                            ..='\u{200a}'
                                | '\u{2028}'
                                | '\u{2029}'
                                | '\u{202f}'
                                | '\u{205f}'
                                | '\u{3000}'
                    ))
            ),
            Self::Lower(e) => json!(
                e.evaluate(id, data)?
                    .as_str()
                    .ok_or_else(bad)?
                    .to_lowercase()
            ),
            Self::Binary(op, a, b) => {
                let a = a.evaluate(id, data)?;
                if op == "??" {
                    if a.is_null() {
                        b.evaluate(id, data)?
                    } else {
                        a
                    }
                } else {
                    let b = b.evaluate(id, data)?;
                    if op == "+" && a.is_string() && b.is_string() {
                        json!(format!("{}{}", a.as_str().unwrap(), b.as_str().unwrap()))
                    } else {
                        let (a, b) = (a.as_f64().ok_or_else(bad)?, b.as_f64().ok_or_else(bad)?);
                        let number = match op.as_str() {
                            "+" => a + b,
                            "-" => a - b,
                            "*" => a * b,
                            "/" if b != 0.0 => a / b,
                            _ => return Err(bad()),
                        };
                        if !number.is_finite() {
                            return Err(bad());
                        }
                        json!(number)
                    }
                }
            }
        };
        // BrainScript numbers follow finite JavaScript Number arithmetic. Canonical safe whole results become
        // JSON integers so the existing exact int validator accepts 2+3 and 2.0+3 identically.
        let value = if let Value::Number(number) = &value {
            let n = number.as_f64().filter(|n| n.is_finite()).ok_or_else(bad)?;
            if n.fract() == 0.0 && n.abs() <= 9007199254740991.0 {
                json!(n as i64)
            } else {
                json!(n)
            }
        } else {
            value
        };
        if serde_json::to_vec(&value).map_err(|_| bad())?.len() > 16384 {
            return Err(bad());
        }
        Ok(value)
    }
}
impl Definition {
    pub fn validate(&self, r: &Resource) -> Result<(), String> {
        r.validate()?;
        if !crate::contract::identifier(&self.name)
            || self.resource != r.name
            || !crate::contract::identifier(&self.field)
        {
            return Err(problem());
        }
        let target = r.fields.get(&self.field).ok_or_else(problem)?;
        if target.slug
            || ["create", "update", "delete"]
                .iter()
                .any(|op| r.hooks.before(op).is_some() || r.hooks.queue(op).is_some())
        {
            return Err("Backfill requires a non-slug target and a resource without application mutation hooks; fix: review side effects explicitly before choosing this operator-only migration".into());
        }
        let expected = field_kind(r, &self.field)?;
        let actual = self
            .expression
            .kind(r, &self.field, 0, &mut 0, &mut BTreeSet::new())?;
        if actual != expected {
            return Err(problem());
        }
        Ok(())
    }
    fn value(&self, r: &Resource, id: Uuid, data: &Value) -> Result<Value, String> {
        let value = self.expression.evaluate(id, data)?;
        if value.is_null() {
            return Err("Backfill computed null; batch rolled back; fix: declare a non-null deterministic value".into());
        }
        let mut document = data.as_object().ok_or_else(problem)?.clone();
        document.insert(self.field.clone(), value);
        let clean=r.input(&Value::Object(document)).map_err(|_|"Backfill row violates the current resource contract; batch and checkpoint rolled back; fix: repair the source row and preview again (values are not reported)".to_string())?;
        clean.get(&self.field).cloned().ok_or_else(problem)
    }
}
async fn verify(conn: &mut PgConnection, r: &Resource) -> Result<(), String> {
    let custom: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_class c WHERE c.oid IN (to_regclass($1),'public._brain_data_backfills'::regclass,'public._brain_backfill_identity'::regclass) AND (c.relkind<>'r' OR c.relrowsecurity OR c.relforcerowsecurity OR EXISTS(SELECT 1 FROM pg_catalog.pg_rewrite w WHERE w.ev_class=c.oid))").bind(format!("public._brain_res_{}",r.name)).fetch_one(&mut *conn).await.map_err(safe)?;
    if custom != 0 {
        return Err("Backfill refused a custom relation, rewrite rule or database row policy; fix: review database side effects and scope before using this workflow".into());
    }
    let recorded: Option<Value> =
        sqlx::query_scalar("SELECT fields FROM public._brain_resource_schemas WHERE name=$1")
            .bind(&r.name)
            .fetch_optional(&mut *conn)
            .await
            .map_err(safe)?;
    if recorded.as_ref() != Some(&r.recorded_fields()) {
        return Err("Backfill recorded resource contract changed; fix: review and migrate the resource before previewing a new backfill".into());
    }
    // Keep all framework-derived indexes, constraints, search/ref/slug/audit triggers. Never execute unreviewed custom triggers.
    let unknown:i64=sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_proc p ON p.oid=t.tgfoid JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE t.tgrelid=to_regclass($1) AND NOT t.tgisinternal AND t.tgenabled<>'D' AND NOT(n.nspname='public' AND ((t.tgname='_brain_search' AND p.proname='_brain_search_update') OR (t.tgname='_brain_refs' AND p.proname='_brain_ref_sync') OR (t.tgname='_brain_slugs' AND p.proname='_brain_slug_track') OR (t.tgname='_brain_per_owner' AND p.proname='_brain_per_owner') OR (t.tgname='_brain_audit' AND p.proname='_brain_audit_row')))").bind(format!("public._brain_res_{}",r.name)).fetch_one(&mut *conn).await.map_err(safe)?;
    if unknown != 0 {
        return Err("Backfill refused an unreviewed database trigger; fix: review custom side effects before using this workflow".into());
    }
    Ok(())
}

fn request(binding: Binding, revision: i64, cursor: Option<Uuid>) -> Result<Request, String> {
    Ok(Request {
        binding,
        revision,
        cursor: digest(&cursor)?,
    })
}
fn state_request(row: &sqlx::postgres::PgRow) -> Result<Request, String> {
    let binding: Binding =
        serde_json::from_value(row.try_get("binding").map_err(safe)?).map_err(|_| {
            "Backfill checkpoint is invalid; fix: restore its recorded definition before continuing"
                .to_string()
        })?;
    request(
        binding,
        row.try_get("revision").map_err(safe)?,
        row.try_get("cursor").map_err(safe)?,
    )
}
async fn guards(conn: &mut PgConnection) -> Result<(), String> {
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_trigger WHERE tgrelid='public._brain_data_backfills'::regclass AND NOT tgisinternal AND tgenabled<>'D'").fetch_one(&mut *conn).await.map_err(safe)?;
    if count != 0 {
        return Err("Backfill checkpoint has an unreviewed trigger; fix: review database side effects before using it".into());
    }
    Ok(())
}
/// Preview never creates state or schema. Apply requires the exact request produced by preview or the previous batch.
/// The connection fingerprint is computed by the CLI from selected environment/normalized options, without passwords.
pub async fn run(
    pool: &PgPool,
    resources: &[Resource],
    definition: &Definition,
    connection: &str,
    include_null: bool,
    limit: u32,
    approved: Option<&Request>,
) -> Result<Value, String> {
    if approved.is_some_and(|request| {
        request.revision < 0
            || request.cursor.len() != 64
            || !request.cursor.bytes().all(|b| b.is_ascii_hexdigit())
    }) {
        return Err(
            "Backfill request revision or cursor is invalid; fix: use unchanged preview output"
                .into(),
        );
    }
    if !(1..=MAX_LIMIT).contains(&limit)
        || connection.len() != 64
        || !connection.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Backfill requires a target fingerprint and --limit 1..1000; fix: use the current app CLI preview".into());
    }
    let resource = resources
        .iter()
        .find(|r| r.name == definition.resource)
        .ok_or_else(problem)?;
    definition.validate(resource)?;
    tokio::time::timeout(Duration::from_secs(30),run_inner(pool,resources,resource,definition,connection,include_null,limit,approved)).await.map_err(|_|"Backfill exceeded its 30 second deadline; uncommitted batch rolled back; fix: preview again with a smaller batch".to_string())?
}
async fn run_inner(
    pool: &PgPool,
    resources: &[Resource],
    r: &Resource,
    d: &Definition,
    connection: &str,
    include_null: bool,
    limit: u32,
    approved: Option<&Request>,
) -> Result<Value, String> {
    crate::db::verify(pool, resources).await?;
    let mut tx = pool.begin().await.map_err(safe)?;
    let apply = approved.is_some();
    if !apply {
        sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(safe)?;
    }
    sqlx::raw_sql("SET LOCAL lock_timeout='500ms'; SET LOCAL statement_timeout='5s'; SET LOCAL idle_in_transaction_session_timeout='5s'").execute(&mut *tx).await.map_err(safe)?;
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind(crate::db::MIGRATION_LOCK)
        .execute(&mut *tx)
        .await
        .map_err(safe)?;
    let identity:Uuid=sqlx::query_scalar("SELECT id FROM public._brain_backfill_identity WHERE singleton=true").fetch_one(&mut *tx).await.map_err(|_|"Backfill requires framework migration 0029; fix: run the explicit brain backend migrate before preview (no schema is created here)".to_string())?;
    let definition = digest(&(ENGINE, d))?;
    let contract = digest(r)?;
    let target =
        sqlx::query_scalar::<_, Value>("SELECT jsonb_build_array(current_database(),current_user)")
            .fetch_one(&mut *tx)
            .await
            .map_err(safe)?;
    let connection = digest(&(connection, target))?;
    let table = format!("public._brain_res_{}", r.name);
    if apply {
        let lock: i64 =
            i64::from_be_bytes(Sha256::digest(d.name.as_bytes())[..8].try_into().unwrap());
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock)
            .execute(&mut *tx)
            .await
            .map_err(safe)?;
        // Hold a normal writer table lock before inspecting triggers; concurrent row writers remain allowed.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "LOCK TABLE {table},public._brain_data_backfills IN ROW EXCLUSIVE MODE"
        )))
        .execute(&mut *tx)
        .await
        .map_err(safe)?;
    }
    verify(&mut tx, r).await?;
    guards(&mut tx).await?;
    let sql = if apply {
        "SELECT binding,cursor,max_id,revision,scanned,written,complete FROM public._brain_data_backfills WHERE name=$1 FOR UPDATE"
    } else {
        "SELECT binding,cursor,max_id,revision,scanned,written,complete FROM public._brain_data_backfills WHERE name=$1"
    };
    let stored = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(&d.name)
        .fetch_optional(&mut *tx)
        .await
        .map_err(safe)?;
    let (mut current, mut cursor, max_id, mut scanned, mut written, mut complete) = if let Some(
        row,
    ) = stored
    {
        let current = state_request(&row)?;
        let binding = &current.binding;
        if binding.engine != ENGINE
            || binding.name != d.name
            || binding.resource != r.name
            || binding.field != d.field
            || binding.definition != definition
            || binding.contract != contract
            || binding.connection != connection
            || binding.database != identity
            || binding.include_null != include_null
        {
            return Err("Backfill definition, resource, connection or options differ from its durable checkpoint; fix: restore the original input or declare a new backfill name".into());
        }
        (
            current,
            row.try_get::<Option<Uuid>, _>("cursor").map_err(safe)?,
            row.try_get::<Option<Uuid>, _>("max_id").map_err(safe)?,
            row.try_get::<i64, _>("scanned").map_err(safe)?,
            row.try_get::<i64, _>("written").map_err(safe)?,
            row.try_get::<bool, _>("complete").map_err(safe)?,
        )
    } else {
        let watermark = if let Some(approved) = approved {
            approved.binding.watermark.clone()
        } else {
            sqlx::query_scalar("SELECT to_char(transaction_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')").fetch_one(&mut *tx).await.map_err(safe)?
        };
        let parsed = chrono::DateTime::parse_from_rfc3339(&watermark).map_err(|_| {
            "Backfill watermark must be an absolute timestamp from preview".to_string()
        })?;
        let _ = parsed;
        let future: bool = sqlx::query_scalar("SELECT $1::timestamptz > transaction_timestamp()")
            .bind(&watermark)
            .fetch_one(&mut *tx)
            .await
            .map_err(safe)?;
        if future {
            return Err("Backfill watermark is in the future; fix: use a new preview".into());
        }
        let max_id: Option<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT id FROM {table} WHERE created_at<=$1::timestamptz ORDER BY id DESC LIMIT 1"
        )))
        .bind(&watermark)
        .fetch_optional(&mut *tx)
        .await
        .map_err(safe)?;
        let binding = Binding {
            engine: ENGINE.into(),
            name: d.name.clone(),
            resource: r.name.clone(),
            field: d.field.clone(),
            definition,
            contract,
            connection,
            database: identity,
            watermark,
            cohort: digest(&max_id)?,
            include_null,
        };
        (request(binding, 0, None)?, None, max_id, 0, 0, false)
    };
    if current.revision < 0
        || current.cursor != digest(&cursor)?
        || current.binding.cohort != digest(&max_id)?
    {
        return Err("Backfill durable checkpoint is inconsistent; fix: restore its recorded state before continuing".into());
    }
    if let Some(approved) = approved {
        if approved.binding != current.binding
            || approved.revision > current.revision
            || approved.revision == current.revision && approved.cursor != current.cursor
        {
            return Err("Backfill request does not match the current definition, cohort, connection or revision; fix: review a fresh preview".into());
        }
        if approved.revision < current.revision || complete {
            let report = json!({"schemaVersion":1,"ok":true,"applied":false,"staleRequest":approved.revision<current.revision,"complete":complete,"updatedRows":0,"request":current,"scannedRows":scanned,"writtenRows":written});
            tx.rollback().await.map_err(safe)?;
            return Ok(report);
        }
    }
    let suffix = if apply { " FOR UPDATE" } else { "" };
    // Select/lock metadata first. Never materialize document payloads before enforcing the aggregate byte budget.
    let eligible = "(NOT(data ? $5) OR ($6::boolean AND data->$5='null'::jsonb))";
    let sql = format!(
        "SELECT id,{eligible} AS eligible,CASE WHEN {eligible} THEN octet_length(data::text)::bigint ELSE 0::bigint END AS bytes FROM {table} WHERE ($1::uuid IS NULL OR id>$1) AND id<=$2 AND created_at<=$3::timestamptz ORDER BY id LIMIT $4{suffix}"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(cursor)
        .bind(max_id)
        .bind(&current.binding.watermark)
        .bind(i64::from(limit))
        .bind(&d.field)
        .bind(include_null)
        .fetch_all(&mut *tx)
        .await
        .map_err(safe)?;
    let mut ids = Vec::new();
    let mut bytes = 0u64;
    let mut preserved = 0u32;
    for row in &rows {
        let size: i64 = row.try_get("bytes").map_err(safe)?;
        if size < 0 || size > 1048576 {
            return Err("Backfill source document exceeds one MiB; fix: compact oversized legacy data before previewing again".into());
        }
        bytes += size as u64;
        if bytes > 1048576 {
            return Err("Backfill batch exceeds one MiB of document bytes; fix: preview with a smaller --limit; no documents were transferred".into());
        }
        if row.try_get::<bool, _>("eligible").map_err(safe)? {
            ids.push(row.try_get::<Uuid, _>("id").map_err(safe)?);
        } else {
            preserved += 1;
        }
    }
    // Apply holds all selected row locks; read-only preview keeps one repeatable-read snapshot. Both fetch only
    // the exact preflight ids, so inserts cannot substitute a larger document between these two queries.
    let documents = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT id,xmin::text AS version,data FROM {table} WHERE id=ANY($1::uuid[]) ORDER BY id"
    )))
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await
    .map_err(safe)?;
    if documents.len() != ids.len() {
        return Err("Backfill selected rows changed; batch rolled back; fix: preview again".into());
    }
    let mut updates = Vec::new();
    let mut invalid = 0u32;
    for row in &documents {
        let id: Uuid = row.try_get("id").map_err(safe)?;
        let data: Value = row.try_get("data").map_err(safe)?;
        match d.value(r, id, &data) {
            Ok(value) => updates.push((
                id,
                row.try_get::<String, _>("version").map_err(safe)?,
                value,
            )),
            Err(_) => invalid += 1,
        }
    }
    if apply && invalid > 0 {
        return Err("Backfill batch contains invalid source rows or computed values; batch and checkpoint rolled back; fix: repair source data and preview again; no values were reported".into());
    }
    let sampled = rows.len() as i64;
    if apply {
        // Binding and row writes are part of the same transaction, so a failed batch never advances durable progress.
        sqlx::query("INSERT INTO public._brain_data_backfills(name,binding,cursor,max_id,revision,scanned,written,complete,updated_at) VALUES($1,$2,$3,$4,0,0,0,false,now()) ON CONFLICT(name) DO NOTHING").bind(&d.name).bind(serde_json::to_value(&current.binding).map_err(|_|problem())?).bind(cursor).bind(max_id).execute(&mut *tx).await.map_err(safe)?;
        crate::audit::mark(&mut tx).await.map_err(safe)?;
        for (id, version, value) in &updates {
            let sql = format!(
                "UPDATE {table} SET data=data||jsonb_build_object($1::text,$2::jsonb) WHERE id=$3 AND xmin::text=$4 AND (NOT(data ? $1) OR ($5::boolean AND data->$1='null'::jsonb))"
            );
            let result = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(&d.field)
                .bind(value)
                .bind(id)
                .bind(version)
                .bind(include_null)
                .execute(&mut *tx)
                .await
                .map_err(safe)?;
            if result.rows_affected() != 1 {
                return Err("Backfill source changed concurrently; batch and checkpoint rolled back; fix: preview the same revision again".into());
            }
        }
        if let Some(row) = rows.last() {
            cursor = Some(row.try_get("id").map_err(safe)?);
        }
        let remaining:bool=sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE ($1::uuid IS NULL OR id>$1) AND id<=$2 AND created_at<=$3::timestamptz)"))).bind(cursor).bind(max_id).bind(&current.binding.watermark).fetch_one(&mut *tx).await.map_err(safe)?;
        complete = !remaining;
        scanned += sampled;
        written += updates.len() as i64;
        let changed=sqlx::query("UPDATE public._brain_data_backfills SET cursor=$2,revision=revision+1,scanned=$3,written=$4,complete=$5,updated_at=now() WHERE name=$1 AND revision=$6 AND binding=$7").bind(&d.name).bind(cursor).bind(scanned).bind(written).bind(complete).bind(current.revision).bind(serde_json::to_value(&current.binding).map_err(|_|problem())?).execute(&mut *tx).await.map_err(safe)?;
        if changed.rows_affected() != 1 {
            return Err(
                "Backfill checkpoint revision changed; batch rolled back; fix: preview again"
                    .into(),
            );
        }
        current = request(current.binding, current.revision + 1, cursor)?;
    }
    let report = json!({"schemaVersion":1,"ok":invalid==0,"applied":apply,"complete":complete,"sampledRows":sampled,"eligibleRows":updates.len(),"preservedRows":preserved,"invalidRows":invalid,"updatedRows":if apply{updates.len()}else{0},"scannedRows":scanned,"writtenRows":written,"batchLimit":limit,"documentBytes":bytes,"includeNull":include_null,"request":if invalid==0 {Some(current)}else{None},"validation":"Only this bounded batch was validated; each apply revalidates its selected rows. Existing values and explicit nulls are preserved unless include-null is approved. updated_at is preserved; normal database integrity and audit triggers run, application hooks do not.","rowValuesCaptured":false,"requestContainsRowIds":false});
    if apply {
        tx.commit().await.map_err(safe)?;
    } else {
        tx.rollback().await.map_err(safe)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resource() -> Resource {
        serde_json::from_value(json!({"name":"records","fields":{"label":{"kind":"str","required":true},"number":{"kind":"int"},"code":{"kind":"str"}},"policy":{"api":false}})).unwrap()
    }
    fn definition() -> Definition {
        Definition {
            name: "codes".into(),
            resource: "records".into(),
            field: "code".into(),
            expression: Expr::Binary(
                "+".into(),
                Box::new(Expr::Lower(Box::new(Expr::Trim(Box::new(Expr::Field(
                    "label".into(),
                )))))),
                Box::new(Expr::Id),
            ),
        }
    }
    #[test]
    fn deterministic_typed_expressions_reuse_resource_validation_and_preserve_values() {
        let r = resource();
        let d = definition();
        d.validate(&r).unwrap();
        let id = Uuid::nil();
        assert_eq!(
            d.value(&r, id, &json!({"label":" Example ","number":2}))
                .unwrap(),
            json!(format!("example{id}"))
        );
        let mut r = r;
        r.fields.get_mut("code").unwrap().pattern = Some("[a-z]+".into());
        assert!(
            d.value(&r, id, &json!({"label":" Example ","number":2}))
                .is_err()
        );
    }
    #[test]
    fn restricted_sources_targets_self_reads_and_mutation_hooks_are_refused_before_sql() {
        let mut r = resource();
        let d = definition();
        r.fields.get_mut("label").unwrap().access = Some("private".into());
        assert!(d.validate(&r).is_err());
        let mut r = resource();
        r.fields.get_mut("code").unwrap().access = Some("writeonly".into());
        assert!(d.validate(&r).is_err());
        let mut r = resource();
        r.hooks.before_update = Some("normalize".into());
        assert!(d.validate(&r).is_err());
        let mut d = definition();
        d.expression = Expr::Field("code".into());
        assert!(d.validate(&resource()).is_err());
    }
    #[test]
    fn arithmetic_is_bounded_and_integer_results_stay_exact() {
        let r = resource();
        let d = Definition {
            name: "sums".into(),
            resource: "records".into(),
            field: "number".into(),
            expression: Expr::Binary(
                "+".into(),
                Box::new(Expr::Literal(json!(2))),
                Box::new(Expr::Literal(json!(3))),
            ),
        };
        d.validate(&r).unwrap();
        assert_eq!(
            d.value(&r, Uuid::nil(), &json!({"label":"a"})).unwrap(),
            json!(5)
        );
        let mut floating = d.clone();
        floating.expression = Expr::Binary(
            "+".into(),
            Box::new(Expr::Literal(json!(2.0))),
            Box::new(Expr::Literal(json!(3))),
        );
        assert_eq!(
            floating
                .value(&r, Uuid::nil(), &json!({"label":"a"}))
                .unwrap(),
            json!(5)
        );
        let div = Expr::Binary(
            "/".into(),
            Box::new(Expr::Literal(json!(1))),
            Box::new(Expr::Literal(json!(0))),
        );
        assert!(div.evaluate(Uuid::nil(), &json!({})).is_err());
    }
    #[test]
    fn definitions_and_requests_are_bound_without_exposing_document_identifiers() {
        let r = resource();
        let d = definition();
        let digest_a = digest(&r).unwrap();
        let mut r2 = r.clone();
        r2.fields.get_mut("code").unwrap().pattern = Some("[a-z]+".into());
        assert_ne!(digest_a, digest(&r2).unwrap());
        let id = Uuid::new_v4();
        let serialized = digest(&id).unwrap();
        assert_eq!(serialized.len(), 64);
        assert!(!serialized.contains(&id.to_string()));
        assert_ne!(
            digest(&d).unwrap(),
            digest(&Definition {
                field: "number".into(),
                ..d
            })
            .unwrap()
        );
    }
}
