//! Audit and internal-retargeting queries over `_brain_analytics_*`. Read-only; every value is bound.
use super::INTERACTIONS;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

fn window(days: u32) -> Result<i32, String> {
    if (1..=3650).contains(&days) { Ok(days as i32) } else { Err("The window must be between 1 and 3650 days".into()) }
}
fn interactions() -> Vec<String> {
    INTERACTIONS.iter().map(|s| s.to_string()).collect()
}

/// Usage benchmark: totals, per-route behavior, per-element usage, transitions and traffic sources.
/// Element rows carry the browser descriptor; the CLI joins them with the build's element map (`file:line`).
pub async fn report(pool: &PgPool, days: u32) -> Result<Value, String> {
    let d = window(days)?;
    let safe = crate::db::safe;
    let totals = sqlx::query("SELECT count(DISTINCT visitor)::int8 AS visitors,count(DISTINCT session)::int8 AS sessions,count(*) FILTER (WHERE kind='page')::int8 AS views,count(*)::int8 AS events,count(DISTINCT user_id)::int8 AS users FROM public._brain_analytics_events WHERE at>now()-make_interval(days=>$1)")
        .bind(d).fetch_one(pool).await.map_err(safe)?;
    // One row per page view: dwell and scroll from its `leave`, first interaction from the earliest interaction.
    let routes = sqlx::query(r#"
        WITH pv AS (
          SELECT session,page,min(route) FILTER (WHERE kind='page') AS route,min(visitor::text) AS visitor,
                 max(value) FILTER (WHERE kind='leave') AS dwell,
                 max((props->>'scroll')::numeric) FILTER (WHERE kind='leave') AS scroll,
                 min(since_ms) FILTER (WHERE kind=ANY($2)) AS ttfi,
                 count(*) FILTER (WHERE kind=ANY($2)) AS acts,
                 count(*) FILTER (WHERE kind='rage') AS rage,
                 count(*) FILTER (WHERE kind='dead') AS dead
          FROM public._brain_analytics_events WHERE at>now()-make_interval(days=>$1) GROUP BY session,page)
        SELECT route,count(*)::int8 AS views,count(DISTINCT visitor)::int8 AS visitors,
               round(avg(dwell))::int8 AS avg_dwell_ms,
               round(percentile_cont(0.5) WITHIN GROUP (ORDER BY dwell))::int8 AS median_dwell_ms,
               round(avg(scroll))::int8 AS avg_scroll_pct,
               round(percentile_cont(0.5) WITHIN GROUP (ORDER BY ttfi))::int8 AS median_ttfi_ms,
               (count(*) FILTER (WHERE acts=0))::int8 AS views_without_interaction,
               sum(rage)::int8 AS rage_clicks,sum(dead)::int8 AS dead_clicks
        FROM pv WHERE route IS NOT NULL GROUP BY route ORDER BY views DESC,route LIMIT 200"#)
        .bind(d).bind(interactions()).fetch_all(pool).await.map_err(safe)?;
    let elements = sqlx::query(r#"
        SELECT element,
               (count(*) FILTER (WHERE kind=ANY($2)))::int8 AS interactions,
               count(DISTINCT visitor)::int8 AS visitors,
               (count(*) FILTER (WHERE kind='rage'))::int8 AS rage,
               (count(*) FILTER (WHERE kind='dead'))::int8 AS dead,
               (count(*) FILTER (WHERE kind='hover'))::int8 AS hovers,
               (count(*) FILTER (WHERE kind='focus'))::int8 AS focuses,
               array_agg(DISTINCT kind ORDER BY kind) AS kinds,
               array_agg(DISTINCT route ORDER BY route) AS routes
        FROM public._brain_analytics_events WHERE at>now()-make_interval(days=>$1) AND element IS NOT NULL
        GROUP BY element ORDER BY interactions DESC,visitors DESC LIMIT 500"#)
        .bind(d).bind(interactions()).fetch_all(pool).await.map_err(safe)?;
    let transitions = sqlx::query(r#"
        WITH p AS (SELECT session,route,lag(route) OVER (PARTITION BY session ORDER BY page,since_ms) AS prev
                   FROM public._brain_analytics_events WHERE kind='page' AND at>now()-make_interval(days=>$1))
        SELECT prev AS "from",route AS "to",count(*)::int8 AS sessions FROM p WHERE prev IS NOT NULL AND prev<>route
        GROUP BY prev,route ORDER BY sessions DESC,prev,route LIMIT 100"#)
        .bind(d).fetch_all(pool).await.map_err(safe)?;
    let exits = sqlx::query(r#"
        WITH last AS (SELECT DISTINCT ON (session) route FROM public._brain_analytics_events
                      WHERE kind='page' AND at>now()-make_interval(days=>$1) ORDER BY session,page DESC,since_ms DESC)
        SELECT route,count(*)::int8 AS sessions FROM last GROUP BY route ORDER BY sessions DESC,route LIMIT 50"#)
        .bind(d).fetch_all(pool).await.map_err(safe)?;
    let sources = sqlx::query(r#"
        SELECT coalesce(utm->>'source',referrer_host,'(direct)') AS source,coalesce(utm->>'campaign','') AS campaign,count(*)::int8 AS sessions
        FROM public._brain_analytics_sessions WHERE started_at>now()-make_interval(days=>$1)
        GROUP BY 1,2 ORDER BY sessions DESC,source LIMIT 50"#)
        .bind(d).fetch_all(pool).await.map_err(safe)?;
    let tracked = sqlx::query(r#"
        SELECT props->>'name' AS name,count(*)::int8 AS events,count(DISTINCT visitor)::int8 AS visitors
        FROM public._brain_analytics_events WHERE kind='track' AND at>now()-make_interval(days=>$1)
        GROUP BY 1 ORDER BY events DESC,name LIMIT 100"#)
        .bind(d).fetch_all(pool).await.map_err(safe)?;
    let num = |row: &sqlx::postgres::PgRow, col: &str| row.try_get::<Option<i64>, _>(col).ok().flatten();
    Ok(json!({
        "schemaVersion": 1,
        "windowDays": days,
        "totals": {
            "visitors": num(&totals,"visitors"), "sessions": num(&totals,"sessions"), "pageViews": num(&totals,"views"),
            "events": num(&totals,"events"), "signedInUsers": num(&totals,"users"),
        },
        "routes": routes.iter().map(|r| json!({
            "route": r.get::<String,_>("route"), "views": num(r,"views"), "visitors": num(r,"visitors"),
            "avgDwellMs": num(r,"avg_dwell_ms"), "medianDwellMs": num(r,"median_dwell_ms"), "avgScrollPct": num(r,"avg_scroll_pct"),
            "medianTimeToFirstInteractionMs": num(r,"median_ttfi_ms"), "viewsWithoutInteraction": num(r,"views_without_interaction"),
            "rageClicks": num(r,"rage_clicks"), "deadClicks": num(r,"dead_clicks"),
        })).collect::<Vec<_>>(),
        "elements": elements.iter().map(|r| json!({
            "element": r.get::<Value,_>("element"), "interactions": num(r,"interactions"), "visitors": num(r,"visitors"),
            "rageClicks": num(r,"rage"), "deadClicks": num(r,"dead"), "hovers": num(r,"hovers"), "focuses": num(r,"focuses"),
            "kinds": r.get::<Vec<String>,_>("kinds"),
            "routes": r.get::<Vec<String>,_>("routes"),
        })).collect::<Vec<_>>(),
        "transitions": transitions.iter().map(|r| json!({"from": r.get::<String,_>("from"), "to": r.get::<String,_>("to"), "sessions": num(r,"sessions")})).collect::<Vec<_>>(),
        "exits": exits.iter().map(|r| json!({"route": r.get::<String,_>("route"), "sessions": num(r,"sessions")})).collect::<Vec<_>>(),
        "sources": sources.iter().map(|r| json!({"source": r.get::<String,_>("source"), "campaign": r.get::<String,_>("campaign"), "sessions": num(r,"sessions")})).collect::<Vec<_>>(),
        "tracked": tracked.iter().map(|r| json!({"name": r.get::<Option<String>,_>("name"), "events": num(r,"events"), "visitors": num(r,"visitors")})).collect::<Vec<_>>(),
    }))
}

/// Route matcher: an exact route, or a prefix ending in `*` (`/blog/*`).
fn pattern(route: &str) -> Result<(String, bool), String> {
    if route.len() > 256 || !route.starts_with('/') {
        return Err("Routes start with / and are at most 256 bytes".into());
    }
    Ok(match route.strip_suffix('*') {
        Some(prefix) => (prefix.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_") + "%", true),
        None => (route.to_string(), false),
    })
}
fn route_sql(column: &str, param: usize, prefix: bool) -> String {
    if prefix { format!("{column} LIKE ${param}") } else { format!("{column}=${param}") }
}

#[derive(Debug, Serialize)]
pub struct FunnelStep {
    pub route: String,
    pub visitors: i64,
}
/// Ordered funnel by visitor: each step counts visitors who viewed it after the previous step, within the window.
pub async fn funnel(pool: &PgPool, steps: &[String], days: u32) -> Result<Vec<FunnelStep>, String> {
    let d = window(days)?;
    if !(2..=8).contains(&steps.len()) {
        return Err("A funnel has 2 to 8 steps".into());
    }
    let mut visitors: Vec<Uuid> = Vec::new();
    let mut reached: Vec<f64> = Vec::new();
    let mut out = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let (p, prefix) = pattern(step)?;
        let rows = if i == 0 {
            // Audited SQL: only the fixed column name and parameter number are formatted.
            sqlx::query(sqlx::AssertSqlSafe(format!("SELECT visitor,extract(epoch FROM min(at))::float8 AS t FROM public._brain_analytics_events WHERE kind='page' AND {} AND at>now()-make_interval(days=>$2) GROUP BY visitor", route_sql("route", 1, prefix))))
                .bind(&p).bind(d).fetch_all(pool).await.map_err(crate::db::safe)?
        } else {
            sqlx::query(sqlx::AssertSqlSafe(format!("SELECT e.visitor,extract(epoch FROM min(e.at))::float8 AS t FROM public._brain_analytics_events e JOIN unnest($2::uuid[],$3::float8[]) AS prev(v,t) ON prev.v=e.visitor WHERE e.kind='page' AND {} AND e.at>=to_timestamp(prev.t) GROUP BY e.visitor", route_sql("e.route", 1, prefix))))
                .bind(&p).bind(&visitors).bind(&reached).fetch_all(pool).await.map_err(crate::db::safe)?
        };
        visitors = rows.iter().map(|r| r.get("visitor")).collect();
        reached = rows.iter().map(|r| r.get("t")).collect();
        out.push(FunnelStep { route: step.clone(), visitors: visitors.len() as i64 });
    }
    Ok(out)
}

/// Declarative segment for internal retargeting, for example "viewed /pricing at least twice in 7 days and never
/// reached /checkout". Usable from server code (personalize, notify) and from `brain analytics segment`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Segment {
    /// Route (or `/prefix/*`) the visitor viewed.
    pub viewed: String,
    /// Minimum number of views of `viewed` (default 1).
    #[serde(default)]
    pub min_views: Option<u32>,
    /// Window in days (default 7).
    #[serde(default)]
    pub within_days: Option<u32>,
    /// Route (or prefix) the visitor must NOT have viewed in the window.
    #[serde(default)]
    pub not_viewed: Option<String>,
    /// Custom `track()` event name the visitor must have sent in the window.
    #[serde(default)]
    pub tracked: Option<String>,
    /// Only visitors linked to an account (`true`) or only anonymous ones (`false`).
    #[serde(default)]
    pub signed_in: Option<bool>,
    /// At most this many members, most recent first (default 1000, maximum 10000).
    #[serde(default)]
    pub limit: Option<u32>,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub visitor: Uuid,
    pub user_id: Option<Uuid>,
    pub views: i64,
    pub last_seen: String,
}
pub async fn segment(pool: &PgPool, s: &Segment) -> Result<Vec<Member>, String> {
    let d = window(s.within_days.unwrap_or(7))?;
    let min = s.min_views.unwrap_or(1).clamp(1, 10000) as i64;
    let limit = s.limit.unwrap_or(1000).clamp(1, 10000) as i64;
    let (viewed, vp) = pattern(&s.viewed)?;
    let (not, np) = match &s.not_viewed {
        Some(r) => { let (p, x) = pattern(r)?; (Some(p), x) }
        None => (None, false),
    };
    if s.tracked.as_ref().is_some_and(|t| t.is_empty() || t.len() > 64) {
        return Err("Tracked event names are 1 to 64 bytes".into());
    }
    // Audited SQL: only fixed column names, parameter numbers and operators are formatted.
    let sql = format!(r#"
        SELECT e.visitor,v.user_id,count(*)::int8 AS views,
               to_char(max(e.at) AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS last_seen
        FROM public._brain_analytics_events e JOIN public._brain_analytics_visitors v ON v.id=e.visitor
        WHERE e.kind='page' AND {viewed} AND e.at>now()-make_interval(days=>$2)
          AND ($4::text IS NULL OR NOT EXISTS (SELECT 1 FROM public._brain_analytics_events x WHERE x.visitor=e.visitor AND x.kind='page' AND {not} AND x.at>now()-make_interval(days=>$2)))
          AND ($5::text IS NULL OR EXISTS (SELECT 1 FROM public._brain_analytics_events t WHERE t.visitor=e.visitor AND t.kind='track' AND t.props->>'name'=$5 AND t.at>now()-make_interval(days=>$2)))
          AND ($6::bool IS NULL OR (v.user_id IS NOT NULL)=$6)
        GROUP BY e.visitor,v.user_id HAVING count(*)>=$3 ORDER BY max(e.at) DESC LIMIT $7"#,
        viewed = route_sql("e.route", 1, vp), not = route_sql("x.route", 4, np));
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(viewed).bind(d).bind(min).bind(not).bind(&s.tracked).bind(s.signed_in).bind(limit)
        .fetch_all(pool).await.map_err(crate::db::safe)?;
    Ok(rows.iter().map(|r| Member { visitor: r.get("visitor"), user_id: r.get("user_id"), views: r.get("views"), last_seen: r.get("last_seen") }).collect())
}

/// Raw events for an audit, oldest first, at most `limit` (1–100000) rows. CSV columns are fixed and quoted.
pub async fn export(pool: &PgPool, days: u32, limit: u32, csv: bool) -> Result<String, String> {
    let d = window(days)?;
    let rows = sqlx::query(r#"
        SELECT to_char(at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS at,visitor::text,session::text,user_id::text,
               page,since_ms,kind,route,element::text,value,props::text
        FROM public._brain_analytics_events WHERE at>now()-make_interval(days=>$1) ORDER BY at,id LIMIT $2"#)
        .bind(d).bind(limit.clamp(1, 100_000) as i64).fetch_all(pool).await.map_err(crate::db::safe)?;
    let cols = ["at", "visitor", "session", "user_id", "page", "since_ms", "kind", "route", "element", "value", "props"];
    let cell = |r: &sqlx::postgres::PgRow, c: &str| -> Option<String> {
        match c {
            "page" | "since_ms" | "value" => r.try_get::<Option<i32>, _>(c).ok().flatten().map(|n| n.to_string()),
            _ => r.try_get::<Option<String>, _>(c).ok().flatten(),
        }
    };
    if csv {
        let mut out = cols.join(",");
        out.push('\n');
        for r in &rows {
            let line: Vec<String> = cols.iter().map(|c| {
                let v = cell(r, c).unwrap_or_default();
                // Formula-leading cells are prefixed so a spreadsheet never evaluates them.
                let v = if v.starts_with(['=', '+', '-', '@']) { format!("'{v}") } else { v };
                format!("\"{}\"", v.replace('"', "\"\""))
            }).collect();
            out.push_str(&line.join(","));
            out.push('\n');
        }
        Ok(out)
    } else {
        let list: Vec<Value> = rows.iter().map(|r| {
            let mut o = serde_json::Map::new();
            for c in cols {
                let v = cell(r, c);
                let v = match (c, v) {
                    ("page" | "since_ms" | "value", Some(n)) => n.parse::<i64>().map(Value::from).unwrap_or(Value::Null),
                    ("element" | "props", Some(j)) => serde_json::from_str(&j).unwrap_or(Value::Null),
                    (_, Some(s)) => Value::from(s),
                    (_, None) => Value::Null,
                };
                o.insert(c.into(), v);
            }
            Value::Object(o)
        }).collect();
        Ok(serde_json::to_string(&json!({"schemaVersion":1,"windowDays":days,"events":list})).unwrap_or_default())
    }
}
