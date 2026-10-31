//! Exact field types for native resources: parsing, lossless normalization, JSON Schema and Postgres CHECK text.
//!
//! Legacy kinds (`str`, `num`, `bool`, `email`, `obj`) keep their original validation and never emit database
//! constraints, so contracts recorded before these types existed are byte-for-byte unchanged.
use serde_json::{Value, json};
use std::cmp::Ordering;

/// Largest integer a JavaScript client can hold exactly (2^53 - 1). `int` values stay inside it.
pub const SAFE_INT: i64 = 9_007_199_254_740_991;
/// Default and maximum element counts for `list<str>`.
pub const LIST_DEFAULT_ITEMS: usize = 64;
pub const LIST_MAX_ITEMS: usize = 256;
/// Byte limit for one `list<str>` element.
pub const LIST_ITEM_BYTES: usize = 1024;
/// Decimal precision limit (PostgreSQL `numeric(p,s)` accepts more; 38 keeps the contract reviewable).
pub const DECIMAL_MAX_PRECISION: u8 = 38;

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Str,
    Num,
    Bool,
    Email,
    Obj,
    Int,
    Decimal { precision: u8, scale: u8 },
    Date,
    DateTime,
    Enum(Vec<String>),
    ListStr { max: usize },
    /// Reference to another resource's record id (`ref:<resource>`), stored as a canonical lowercase UUID string
    /// and mirrored into a real `ref_<field> uuid` column with a foreign key (see `relations.rs`).
    Ref(String),
}

fn enum_value(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}

/// Parses a field kind such as `decimal(10,2)`, `enum(draft|published)` or `list<str,20>`.
pub fn parse(kind: &str) -> Option<Ty> {
    Some(match kind {
        "str" => Ty::Str,
        "num" => Ty::Num,
        "bool" => Ty::Bool,
        "email" => Ty::Email,
        "obj" => Ty::Obj,
        "int" => Ty::Int,
        "date" => Ty::Date,
        "datetime" => Ty::DateTime,
        "list<str>" => Ty::ListStr {
            max: LIST_DEFAULT_ITEMS,
        },
        _ => {
            if let Some(target) = kind.strip_prefix("ref:") {
                if !crate::contract::identifier(target) {
                    return None;
                }
                return Some(Ty::Ref(target.to_string()));
            }
            if let Some(args) = kind
                .strip_prefix("decimal(")
                .and_then(|s| s.strip_suffix(')'))
            {
                let (p, s) = args.split_once(',')?;
                let precision = canonical_small(p)?;
                let scale = canonical_small(s)?;
                if precision == 0 || precision > DECIMAL_MAX_PRECISION || scale > precision {
                    return None;
                }
                Ty::Decimal { precision, scale }
            } else if let Some(args) = kind.strip_prefix("enum(").and_then(|s| s.strip_suffix(')')) {
                let values: Vec<String> = args.split('|').map(str::to_string).collect();
                let unique: std::collections::BTreeSet<_> = values.iter().collect();
                if values.is_empty()
                    || values.len() > 64
                    || unique.len() != values.len()
                    || !values.iter().all(|v| enum_value(v))
                {
                    return None;
                }
                Ty::Enum(values)
            } else if let Some(n) = kind.strip_prefix("list<str,").and_then(|s| s.strip_suffix('>')) {
                let max = canonical_small(n)? as usize;
                if max == 0 || max > LIST_MAX_ITEMS {
                    return None;
                }
                // `list<str,64>` is spelled `list<str>`; one canonical form keeps drift comparison exact.
                if max == LIST_DEFAULT_ITEMS {
                    return None;
                }
                Ty::ListStr { max }
            } else {
                return None;
            }
        }
    })
}
/// Unsigned decimal without sign, spaces or leading zeros, at most 255.
fn canonical_small(s: &str) -> Option<u8> {
    if s.is_empty() || s.len() > 3 || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    if !s.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse::<u16>().ok().filter(|n| *n <= 255).map(|n| n as u8)
}

pub const KIND_HELP: &str = "str|num|bool|email|obj|int|decimal(p,s)|date|datetime|enum(a|b)|list<str>|list<str,N>|ref:<resource>";

impl Ty {
    /// Kinds that existed before exact types; they keep legacy validation and never get database checks.
    pub fn legacy(&self) -> bool {
        matches!(self, Ty::Str | Ty::Num | Ty::Bool | Ty::Email | Ty::Obj)
    }
    /// Kinds whose canonical JSON text compares exactly, so `->>` equality is value equality.
    pub fn unique_eligible(&self) -> bool {
        matches!(
            self,
            Ty::Str | Ty::Email | Ty::Int | Ty::Decimal { .. } | Ty::Date | Ty::DateTime | Ty::Enum(_) | Ty::Ref(_)
        )
    }
    /// Kinds with a total order supported by `check` ranges and comparisons.
    pub fn ordered(&self) -> bool {
        matches!(self, Ty::Int | Ty::Decimal { .. } | Ty::Date | Ty::DateTime)
    }
    pub fn expected(&self) -> String {
        match self {
            Ty::Str => "Expected str".into(),
            Ty::Num => "Expected num".into(),
            Ty::Bool => "Expected bool".into(),
            Ty::Email => "Expected email".into(),
            Ty::Obj => "Expected obj".into(),
            Ty::Int => format!("Expected an integer JSON number between -{SAFE_INT} and {SAFE_INT}"),
            Ty::Decimal { precision, scale } => format!(
                "Expected a decimal string with at most {} integer digits and {scale} decimals, like \"{}\"",
                precision - scale,
                example_decimal(*precision, *scale)
            ),
            Ty::Date => "Expected a calendar date string YYYY-MM-DD".into(),
            Ty::DateTime => "Expected an RFC 3339 date-time with Z or an offset, like \"2026-10-30T12:00:00Z\" (at most millisecond precision)".into(),
            Ty::Enum(values) => format!("Expected one of: {}", values.join(", ")),
            Ty::ListStr { max } => format!(
                "Expected an array of at most {max} strings, each at most {LIST_ITEM_BYTES} bytes"
            ),
            Ty::Ref(target) => format!("Expected the UUID string of an existing {target} record"),
        }
    }
    /// Validates and returns the canonical stored value. `required` matters only for non-empty text/list rules.
    pub fn normalize(&self, v: &Value, required: bool) -> Option<Value> {
        match self {
            Ty::Str => v
                .as_str()
                .filter(|s| s.len() <= 16384 && (!required || !s.trim().is_empty()))
                .map(|_| v.clone()),
            Ty::Email => v
                .as_str()
                .filter(|s| {
                    s.len() <= 254
                        && !s.chars().any(char::is_whitespace)
                        && s.split_once('@').is_some_and(|(a, b)| {
                            !a.is_empty() && b.contains('.') && !b.contains('@')
                        })
                })
                .map(|_| v.clone()),
            Ty::Num => v.as_f64().filter(|n| n.is_finite()).map(|_| v.clone()),
            Ty::Bool => v.is_boolean().then(|| v.clone()),
            Ty::Obj => (v.is_object() && v.to_string().len() <= 32768).then(|| v.clone()),
            Ty::Int => v
                .as_i64()
                .filter(|n| (-SAFE_INT..=SAFE_INT).contains(n))
                .map(|n| json!(n)),
            Ty::Decimal { precision, scale } => v
                .as_str()
                .and_then(|s| decimal(s, *precision, *scale))
                .map(Value::String),
            Ty::Date => v
                .as_str()
                .filter(|s| date(s).is_some())
                .map(|s| json!(s)),
            Ty::DateTime => v.as_str().and_then(datetime).map(Value::String),
            Ty::Enum(values) => v
                .as_str()
                .filter(|s| values.iter().any(|x| x == s))
                .map(|s| json!(s)),
            Ty::ListStr { max } => {
                let items = v.as_array()?;
                if items.len() > *max || required && items.is_empty() {
                    return None;
                }
                items
                    .iter()
                    .all(|item| item.as_str().is_some_and(|s| s.len() <= LIST_ITEM_BYTES))
                    .then(|| v.clone())
            }
            Ty::Ref(_) => v
                .as_str()
                .filter(|s| s.len() == 36)
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(|id| json!(id.hyphenated().to_string())),
        }
    }
    pub fn json_schema(&self, required: bool) -> Value {
        match self {
            Ty::Obj => json!({"type":"object"}),
            Ty::Num => json!({"type":"number"}),
            Ty::Bool => json!({"type":"boolean"}),
            Ty::Email => json!({"type":"string","format":"email","maxLength":254}),
            Ty::Str => {
                json!({"type":"string","maxLength":16384, "minLength": if required {1} else {0}})
            }
            Ty::Int => json!({"type":"integer","minimum":-SAFE_INT,"maximum":SAFE_INT}),
            Ty::Decimal { precision, scale } => json!({
                "type":"string",
                "pattern":decimal_pattern(*precision, *scale),
                "x-brain-decimal":{"precision":precision,"scale":scale,"transport":"canonical string","storage":"jsonb string castable to numeric"},
                "examples":[example_decimal(*precision, *scale)]
            }),
            Ty::Date => json!({"type":"string","format":"date","pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"}),
            Ty::DateTime => json!({
                "type":"string",
                "format":"date-time",
                "x-brain-canonical":"UTC YYYY-MM-DDTHH:MM:SS.sssZ (offset input is converted)"
            }),
            Ty::Enum(values) => json!({"type":"string","enum":values}),
            Ty::ListStr { max } => json!({
                "type":"array",
                "maxItems":max,
                "minItems": if required {1} else {0},
                "items":{"type":"string","x-brain-maxBytes":LIST_ITEM_BYTES}
            }),
            Ty::Ref(target) => json!({"type":"string","format":"uuid","x-brain-ref":target}),
        }
    }
    /// Postgres boolean expression for one present value at `data->'<field>'`, or None for legacy kinds.
    /// Field names are validated identifiers and every literal is built from a restricted alphabet.
    pub fn sql_value_check(&self, field: &str) -> Option<String> {
        let j = format!("data->'{field}'");
        let t = format!("data->>'{field}'");
        Some(match self {
            _ if self.legacy() => return None,
            Ty::Int => format!(
                "jsonb_typeof({j})='number' AND ({j})::numeric=trunc(({j})::numeric) AND abs(({j})::numeric)<={SAFE_INT}"
            ),
            Ty::Decimal { precision, scale } => format!(
                "jsonb_typeof({j})='string' AND ({t}) ~ '{}'",
                decimal_pattern(*precision, *scale)
            ),
            Ty::Date => format!(
                "jsonb_typeof({j})='string' AND ({t}) ~ '^[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}$'"
            ),
            Ty::DateTime => format!(
                "jsonb_typeof({j})='string' AND ({t}) ~ '^[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}T[0-9]{{2}}:[0-9]{{2}}:[0-9]{{2}}[.][0-9]{{3}}Z$'"
            ),
            Ty::Enum(values) => format!(
                "jsonb_typeof({j})='string' AND ({t}) IN ({})",
                values
                    .iter()
                    .map(|v| format!("'{v}'"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Ty::Ref(_) => format!(
                "jsonb_typeof({j})='string' AND ({t}) ~ '^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$'"
            ),
            Ty::ListStr { max } => format!(
                "jsonb_typeof({j})='array' AND jsonb_array_length({j})<={max} AND NOT jsonb_path_exists({j},'$[*] ? (@.type() != \"string\")')"
            ),
            _ => return None,
        })
    }
    /// SQL operand for ordered comparisons. Date/date-time canonical strings are fixed width and compare as text.
    pub fn sql_operand(&self, field: &str) -> String {
        match self {
            Ty::Int => format!("(data->'{field}')::numeric"),
            Ty::Decimal { .. } => format!("(data->>'{field}')::numeric"),
            _ => format!("(data->>'{field}') COLLATE \"C\""),
        }
    }
    /// SQL literal for a canonical bound value of this type (restricted alphabet, no quotes inside).
    pub fn sql_literal(&self, v: &Value) -> String {
        match self {
            Ty::Int => v.to_string(),
            Ty::Decimal { .. } => v.as_str().unwrap_or("0").to_string(),
            _ => format!("'{}' COLLATE \"C\"", v.as_str().unwrap_or("")),
        }
    }
    /// Total order of two canonical values of this ordered type.
    pub fn compare(&self, a: &Value, b: &Value) -> Option<Ordering> {
        match self {
            Ty::Int => Some(a.as_i64()?.cmp(&b.as_i64()?)),
            Ty::Decimal { .. } => Some(compare_decimal(a.as_str()?, b.as_str()?)),
            Ty::Date | Ty::DateTime => Some(a.as_str()?.cmp(b.as_str()?)),
            _ => None,
        }
    }
}

fn decimal_pattern(precision: u8, scale: u8) -> String {
    let int_digits = precision - scale;
    let int = if int_digits == 0 {
        "0".to_string()
    } else if int_digits == 1 {
        "[0-9]".to_string()
    } else {
        format!("(0|[1-9][0-9]{{0,{}}})", int_digits - 1)
    };
    let frac = if scale == 0 {
        String::new()
    } else {
        format!("[.][0-9]{{{scale}}}")
    };
    format!("^-?{int}{frac}$")
}
fn example_decimal(precision: u8, scale: u8) -> String {
    let int = if precision > scale { "19" } else { "0" };
    let int = &int[..int.len().min((precision - scale).max(1) as usize)];
    if scale == 0 {
        int.to_string()
    } else {
        format!("{int}.{}", "9".repeat(scale as usize))
    }
}

/// Canonical decimal: optional `-`, no leading zeros, exactly `scale` decimals; `-0` becomes `0`.
/// Extra decimals are accepted only when they are zeros, so normalization never rounds.
pub fn decimal(s: &str, precision: u8, scale: u8) -> Option<String> {
    if s.is_empty() || s.len() > 80 {
        return None;
    }
    let (negative, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, f),
        None => (body, ""),
    };
    if int.is_empty()
        || !int.bytes().all(|c| c.is_ascii_digit())
        || !frac.bytes().all(|c| c.is_ascii_digit())
        || body.contains('.') && frac.is_empty()
    {
        return None;
    }
    let int = int.trim_start_matches('0');
    let scale = scale as usize;
    if frac.len() > scale && frac[scale..].bytes().any(|c| c != b'0') {
        return None;
    }
    if int.len() > precision as usize - scale {
        return None;
    }
    let mut digits = frac[..frac.len().min(scale)].to_string();
    while digits.len() < scale {
        digits.push('0');
    }
    let zero = int.is_empty() && digits.bytes().all(|c| c == b'0');
    let mut out = String::new();
    if negative && !zero {
        out.push('-');
    }
    out.push_str(if int.is_empty() { "0" } else { int });
    if scale > 0 {
        out.push('.');
        out.push_str(&digits);
    }
    Some(out)
}
/// Compares two canonical decimals of the same scale.
fn compare_decimal(a: &str, b: &str) -> Ordering {
    let neg_a = a.starts_with('-');
    let neg_b = b.starts_with('-');
    match (neg_a, neg_b) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        _ => {
            let (ma, mb) = (a.trim_start_matches('-'), b.trim_start_matches('-'));
            let (ia, fa) = ma.split_once('.').unwrap_or((ma, ""));
            let (ib, fb) = mb.split_once('.').unwrap_or((mb, ""));
            let width = fa.len().max(fb.len());
            let (fa, fb) = (format!("{fa:0<width$}"), format!("{fb:0<width$}"));
            let magnitude = ia.len().cmp(&ib.len()).then(ia.cmp(ib)).then(fa.cmp(&fb));
            if neg_a { magnitude.reverse() } else { magnitude }
        }
    }
}

fn digits(s: &str) -> Option<u32> {
    (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}
fn leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}
fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}
/// Calendar date `YYYY-MM-DD` with years 0001..9999; returns (year, month, day).
pub fn date(s: &str) -> Option<(i64, u32, u32)> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d) = (digits(&s[0..4])? as i64, digits(&s[5..7])?, digits(&s[8..10])?);
    (y >= 1 && (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)).then_some((y, m, d))
}
/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
/// RFC 3339 date-time with a mandatory `Z` or `±HH:MM` offset, converted to canonical UTC
/// `YYYY-MM-DDTHH:MM:SS.sssZ` (the JavaScript `toISOString` form). Sub-millisecond digits must be zeros.
pub fn datetime(s: &str) -> Option<String> {
    if s.len() < 20 || s.len() > 40 || !s.is_ascii() {
        return None;
    }
    let (y, mo, d) = date(&s[0..10])?;
    let b = s.as_bytes();
    if !matches!(b[10], b'T' | b't') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (h, mi, sec) = (digits(&s[11..13])?, digits(&s[14..16])?, digits(&s[17..19])?);
    if h > 23 || mi > 59 || sec > 59 {
        return None;
    }
    let mut rest = &s[19..];
    let mut millis = 0u32;
    if let Some(frac) = rest.strip_prefix('.') {
        let n = frac.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 || n > 9 || frac[3.min(n)..n].bytes().any(|c| c != b'0') {
            return None;
        }
        let mut ms = frac[..n.min(3)].to_string();
        while ms.len() < 3 {
            ms.push('0');
        }
        millis = ms.parse().ok()?;
        rest = &frac[n..];
    }
    let offset_minutes: i64 = match rest {
        "Z" | "z" => 0,
        _ => {
            let rb = rest.as_bytes();
            if rb.len() != 6 || !matches!(rb[0], b'+' | b'-') || rb[3] != b':' {
                return None;
            }
            let (oh, om) = (digits(&rest[1..3])? as i64, digits(&rest[4..6])? as i64);
            if oh > 23 || om > 59 {
                return None;
            }
            let total = oh * 60 + om;
            if rb[0] == b'-' { -total } else { total }
        }
    };
    let seconds = days_from_civil(y, mo, d) * 86400 + h as i64 * 3600 + mi as i64 * 60 + sec as i64
        - offset_minutes * 60;
    let (days, secs) = (seconds.div_euclid(86400), seconds.rem_euclid(86400));
    let (y, mo, d) = civil_from_days(days);
    if !(1..=9999).contains(&y) {
        return None;
    }
    Some(format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kinds_parse_only_canonical_spellings() {
        assert_eq!(parse("decimal(10,2)"), Some(Ty::Decimal { precision: 10, scale: 2 }));
        for bad in [
            "decimal(0,0)",
            "decimal(39,2)",
            "decimal(2,3)",
            "decimal(010,2)",
            "decimal(10, 2)",
            "enum()",
            "enum(a|a)",
            "enum(a b)",
            "enum(a'b)",
            "list<str,0>",
            "list<str,64>",
            "list<str,257>",
            "list<int>",
            "integer",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(parse("list<str,10>"), Some(Ty::ListStr { max: 10 }));
        assert_eq!(parse("enum(draft|published)").unwrap().expected(), "Expected one of: draft, published");
    }
    #[test]
    fn decimals_normalize_without_rounding_or_float_conversion() {
        assert_eq!(decimal("19.9", 10, 2).as_deref(), Some("19.90"));
        assert_eq!(decimal("0019.900", 10, 2).as_deref(), Some("19.90"));
        assert_eq!(decimal("-0.00", 10, 2).as_deref(), Some("0.00"));
        assert_eq!(decimal("-5", 10, 2).as_deref(), Some("-5.00"));
        assert_eq!(decimal("12345678.99", 10, 2).as_deref(), Some("12345678.99"));
        assert_eq!(decimal("90071992547409931.25", 20, 2).as_deref(), Some("90071992547409931.25"));
        for bad in ["19.999", "123456789.00", "1e3", "+1", " 1", "1.", ".5", "", "--1", "1.2.3", "0x10"] {
            assert_eq!(decimal(bad, 10, 2), None, "{bad}");
        }
        assert_eq!(decimal("7", 3, 0).as_deref(), Some("7"));
        assert_eq!(decimal("0.5", 1, 1).as_deref(), Some("0.5"));
        assert_eq!(decimal("1.5", 1, 1), None);
        let t = Ty::Decimal { precision: 10, scale: 2 };
        assert_eq!(t.normalize(&json!(19.99), false), None, "JSON numbers are rejected for decimals");
        assert_eq!(compare_decimal("-2.00", "-10.00"), Ordering::Greater);
        assert_eq!(compare_decimal("10.00", "9.99"), Ordering::Greater);
        assert_eq!(compare_decimal("0.10", "0.09"), Ordering::Greater);
        assert_eq!(compare_decimal("10.5", "10.50"), Ordering::Equal);
        assert_eq!(compare_decimal("7", "6.999"), Ordering::Greater);
    }
    #[test]
    fn dates_and_instants_validate_calendars_and_normalize_offsets() {
        assert!(date("2024-02-29").is_some());
        for bad in ["2023-02-29", "2026-13-01", "0000-01-01", "2026-1-01", "2026-01-32", "2026/01/01"] {
            assert!(date(bad).is_none(), "{bad}");
        }
        assert_eq!(datetime("2026-10-30T12:00:00Z").as_deref(), Some("2026-10-30T12:00:00.000Z"));
        assert_eq!(datetime("2026-10-30T00:30:00.5-06:00").as_deref(), Some("2026-10-30T06:30:00.500Z"));
        assert_eq!(datetime("2026-01-01T01:00:00+02:00").as_deref(), Some("2025-12-31T23:00:00.000Z"));
        assert_eq!(datetime("2024-02-28T23:00:00.120000-01:00").as_deref(), Some("2024-02-29T00:00:00.120Z"));
        for bad in [
            "2026-10-30T12:00:00",
            "2026-10-30 12:00:00Z",
            "2026-10-30T24:00:00Z",
            "2026-10-30T12:00:60Z",
            "2026-10-30T12:00:00.1234Z",
            "2026-10-30T12:00:00+5:00",
            "0001-01-01T00:00:00+01:00",
            "9999-12-31T23:00:00-02:00",
        ] {
            assert!(datetime(bad).is_none(), "{bad}");
        }
        for days in [-719162, -1, 0, 1, 19000, 2932896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }
    #[test]
    fn ints_stay_inside_the_javascript_safe_range() {
        let t = Ty::Int;
        assert_eq!(t.normalize(&json!(SAFE_INT), false), Some(json!(SAFE_INT)));
        assert_eq!(t.normalize(&json!(SAFE_INT + 1), false), None);
        assert_eq!(t.normalize(&json!(1.5), false), None);
        assert_eq!(t.normalize(&json!(1.0), false), None);
        assert_eq!(t.normalize(&json!("1"), false), None);
    }
    #[test]
    fn lists_and_enums_are_bounded() {
        let l = Ty::ListStr { max: 2 };
        assert!(l.normalize(&json!(["a", "b"]), true).is_some());
        assert!(l.normalize(&json!(["a", "b", "c"]), false).is_none());
        assert!(l.normalize(&json!([]), true).is_none());
        assert!(l.normalize(&json!([]), false).is_some());
        assert!(l.normalize(&json!(["a", 1]), false).is_none());
        assert!(l.normalize(&json!(["x".repeat(LIST_ITEM_BYTES + 1)]), false).is_none());
        let e = parse("enum(a|b)").unwrap();
        assert!(e.normalize(&json!("a"), false).is_some());
        assert!(e.normalize(&json!("A"), false).is_none());
    }
    #[test]
    fn sql_checks_exist_only_for_exact_kinds_and_use_restricted_literals() {
        for legacy in ["str", "num", "bool", "email", "obj"] {
            assert_eq!(parse(legacy).unwrap().sql_value_check("f"), None);
        }
        let check = parse("enum(draft|in-review)").unwrap().sql_value_check("status").unwrap();
        assert!(check.contains("IN ('draft','in-review')"));
        assert_eq!(decimal_pattern(10, 2), "^-?(0|[1-9][0-9]{0,7})[.][0-9]{2}$");
        assert_eq!(decimal_pattern(2, 2), "^-?0[.][0-9]{2}$");
        assert_eq!(decimal_pattern(3, 0), "^-?(0|[1-9][0-9]{0,2})$");
    }
}
