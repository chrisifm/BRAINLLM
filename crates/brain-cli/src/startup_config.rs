//! What `brain backend serve` needs from the environment, checked from values alone: the same validators run at start
//! (fatal) and in `brain check`/`brain doctor` (warnings), so a bad combination shows up before a process restarts on
//! it. Messages carry the variable name, never its value. JWT and Google settings are validated where they load.

/// One problem with the backend settings of an environment.
#[derive(Debug)]
pub struct Finding {
    pub code: &'static str,
    /// The variable to edit.
    pub key: &'static str,
    /// Stops `brain backend serve`; otherwise the backend starts with that feature off.
    pub fatal: bool,
    /// `<code>: what; fix: how`.
    pub message: String,
}

const POOLS: [&str; 3] = ["BRAIN_DB_HTTP_POOL", "BRAIN_DB_WORKER_POOL", "BRAIN_BACKEND_PROCESSES"];

pub fn findings(get: &dyn Fn(&str) -> Option<String>, production: bool) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |code, key, fatal, message: String| out.push(Finding { code, key, fatal, message });
    let (key, from) = (get("BRAIN_MAIL_RESEND_KEY"), get("BRAIN_MAIL_FROM"));
    match (key, from) {
        (Some(key), Some(from)) => {
            let base = get("PUBLIC_SITE_URL").unwrap_or_else(|| "http://127.0.0.1".into());
            if let Err(message) = brain_backend::auth_flows::MailConfig::resend(key, from, base) {
                let (code, variable) = if message.starts_with("B0764") {
                    ("B0764", "PUBLIC_SITE_URL")
                } else if message.contains("BRAIN_MAIL_FROM") {
                    ("B0760", "BRAIN_MAIL_FROM")
                } else {
                    ("B0760", "BRAIN_MAIL_RESEND_KEY")
                };
                add(code, variable, true, message);
            }
        }
        (Some(_), None) => add("B0760", "BRAIN_MAIL_FROM", true, "B0760: BRAIN_MAIL_RESEND_KEY is set without BRAIN_MAIL_FROM; fix: set BRAIN_MAIL_FROM to a verified sender, as `sender@your-domain` or `Name <sender@your-domain>`".into()),
        (None, Some(_)) => add("B0760", "BRAIN_MAIL_RESEND_KEY", false, "B0760: BRAIN_MAIL_FROM is set without BRAIN_MAIL_RESEND_KEY, so no mail is sent; fix: set the key, or remove BRAIN_MAIL_FROM".into()),
        (None, None) => {}
    }
    match get("BRAIN_GCS_BUCKET") {
        Some(bucket) => match get("BRAIN_GCS_ADAPTER") {
            Some(adapter) => {
                let prefix = get("BRAIN_GCS_PREFIX").unwrap_or_else(|| "brain-media".into());
                if let Err(message) = brain_backend::media::GcsConfig::new(bucket, prefix, adapter.into()) {
                    let variable = ["BRAIN_GCS_BUCKET", "BRAIN_GCS_PREFIX"].into_iter().find(|k| message.contains(k)).unwrap_or("BRAIN_GCS_ADAPTER");
                    add("B0761", variable, true, message);
                }
            }
            None => add("B0761", "BRAIN_GCS_ADAPTER", true, "B0761: BRAIN_GCS_BUCKET is set without BRAIN_GCS_ADAPTER; fix: point BRAIN_GCS_ADAPTER at the absolute path of the private .mjs SDK adapter, or remove the bucket".into()),
        },
        None => {
            for variable in ["BRAIN_GCS_ADAPTER", "BRAIN_GCS_PREFIX"] {
                if get(variable).is_some() {
                    add("B0761", variable, false, format!("B0761: {variable} is set without BRAIN_GCS_BUCKET, so storage stays local; fix: set the bucket, or remove {variable}"));
                }
            }
        }
    }
    if let Some(url) = get("DATABASE_URL")
        && let Err(message) = brain_backend::boot::require_verified_tls(&url, production)
    {
        add("B0762", "DATABASE_URL", true, message);
    }
    for variable in POOLS {
        if get(variable).is_some_and(|v| !v.parse::<u32>().is_ok_and(|n| (1..=64).contains(&n))) {
            add("B0763", variable, true, format!("B0763: {variable} is out of range; fix: a whole number from 1 to 64, or remove it to use the default"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(vars: &[(&str, &str)], production: bool) -> Vec<Finding> {
        findings(&|k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()), production)
    }
    const KEY: &str = "re_0123456789abcdef";

    #[test]
    fn friendly_sender_is_accepted_and_header_injection_is_not() {
        for from in ["hello@shop.example", "Shop Team <hello@shop.example>", "\"Shop, Team\" <hello@shop.example>", "Zoë Müller <hello@shop.example>"] {
            assert!(run(&[("BRAIN_MAIL_RESEND_KEY", KEY), ("BRAIN_MAIL_FROM", from)], true).is_empty(), "{from}");
        }
        for from in ["Name <hello@shop.example>\r\nBcc: x@shop.example", "Name\nBcc: x <hello@shop.example>", "a, b <hello@shop.example>", "Name <hello@shop.example", "Name hello@shop.example>", "Name <not-an-address>", "<a@shop.example> <b@shop.example>", "Name\u{202e} <hello@shop.example>"] {
            let found = run(&[("BRAIN_MAIL_RESEND_KEY", KEY), ("BRAIN_MAIL_FROM", from)], true);
            assert!(found.len() == 1 && found[0].code == "B0760" && found[0].key == "BRAIN_MAIL_FROM" && found[0].fatal, "{from}: {found:?}");
            assert!(!found[0].message.contains("hello@") && found[0].message.contains("fix:"), "the value stays out: {}", found[0].message);
        }
    }

    #[test]
    fn every_combination_names_its_variable_without_its_value() {
        let secret = "s3cr3t-value";
        let cases: [(&[(&str, &str)], &str, &str, bool); 9] = [
            (&[("BRAIN_MAIL_RESEND_KEY", KEY)], "B0760", "BRAIN_MAIL_FROM", true),
            (&[("BRAIN_MAIL_FROM", "a@shop.example")], "B0760", "BRAIN_MAIL_RESEND_KEY", false),
            (&[("BRAIN_MAIL_RESEND_KEY", "s3cr3t-value"), ("BRAIN_MAIL_FROM", "a@shop.example")], "B0760", "BRAIN_MAIL_RESEND_KEY", true),
            (&[("BRAIN_MAIL_RESEND_KEY", KEY), ("BRAIN_MAIL_FROM", "a@shop.example"), ("PUBLIC_SITE_URL", "ftp://s3cr3t-value")], "B0764", "PUBLIC_SITE_URL", true),
            (&[("BRAIN_GCS_BUCKET", "media")], "B0761", "BRAIN_GCS_ADAPTER", true),
            (&[("BRAIN_GCS_BUCKET", "S3CR3T value"), ("BRAIN_GCS_ADAPTER", "/opt/a.mjs")], "B0761", "BRAIN_GCS_BUCKET", true),
            (&[("BRAIN_GCS_ADAPTER", "/opt/a.mjs")], "B0761", "BRAIN_GCS_ADAPTER", false),
            (&[("DATABASE_URL", "s3cr3t-value")], "B0762", "DATABASE_URL", true),
            (&[("BRAIN_DB_HTTP_POOL", "0")], "B0763", "BRAIN_DB_HTTP_POOL", true),
        ];
        for (vars, code, key, fatal) in cases {
            let found = run(vars, true);
            assert!(found.len() == 1 && found[0].code == code && found[0].key == key && found[0].fatal == fatal, "{vars:?}: {found:?}");
            assert!(found[0].message.contains(code) && found[0].message.contains("fix:") && !found[0].message.contains(secret), "{}", found[0].message);
        }
    }

    #[test]
    fn valid_settings_and_development_databases_have_no_findings() {
        assert!(run(&[], true).is_empty());
        assert!(run(&[("DATABASE_URL", "postgres://u:p@db.internal/app")], false).is_empty());
        let tls = run(&[("DATABASE_URL", "postgres://u:p@db.internal/app")], true);
        assert!(tls[0].code == "B0762" && tls[0].message.contains("BDB_TLS_UNVERIFIED"), "{tls:?}");
        assert!(run(&[("DATABASE_URL", "postgres://u:p@db.internal/app?sslmode=verify-full"), ("BRAIN_DB_WORKER_POOL", "4")], true).is_empty());
    }
}
