//! Bounded anonymous HTTP observations, not browser performance or cache-state inference.
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const MAX_BODY: usize = 256 * 1024;
const USAGE: &str = "usage: brain bench --url <http(s)://host/path> [--requests 3] [--json]";

pub fn run(args: &[String]) -> Result<Value, String> {
    let mut url = None;
    let mut count = None;
    let mut tokens = args.iter();
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--json" | "--quiet" | "-q" => (),
            "--progress"
                if tokens
                    .next()
                    .is_some_and(|v| matches!(v.as_str(), "off" | "plain" | "rich")) =>
            {
                ()
            }
            "--progress=off" | "--progress=plain" | "--progress=rich" => (),
            "--url" if url.is_none() => url = Some(tokens.next().ok_or(USAGE)?.clone()),
            "--requests" if count.is_none() => count = Some(tokens.next().ok_or(USAGE)?.clone()),
            _ if token.starts_with("--url=") && url.is_none() => url = Some(token[6..].into()),
            _ if token.starts_with("--requests=") && count.is_none() => {
                count = Some(token[11..].into())
            }
            _ => return Err(USAGE.into()),
        }
    }
    let url = target(url.as_deref().ok_or(USAGE)?)?;
    let count = count.map_or(Ok(3usize), |value| {
        value
            .parse::<usize>()
            .map_err(|_| "requests must be between 1 and 20")
    })?;
    if !(1..=20).contains(&count) {
        return Err("requests must be between 1 and 20".into());
    }
    let runtime = crate::http_observer::Runtime::new()?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), observe(url, count))
            .await
            .map_err(|_| "benchmark reached its 30 second total deadline".to_string())?
    })
}

fn target(raw: &str) -> Result<reqwest::Url, String> {
    let fail =
        "bench needs one HTTP(S) URL without credentials, query, fragment or control characters";
    if raw.len() > 2048 || raw.bytes().any(|b| b <= 32 || b == 127) || raw.contains('\\') {
        return Err(fail.into());
    }
    let url = reqwest::Url::parse(raw).map_err(|_| fail)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(fail.into());
    }
    Ok(url)
}

async fn observe(url: reqwest::Url, count: usize) -> Result<Value, String> {
    let client = crate::http_observer::client(reqwest::Client::builder())
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .user_agent(concat!("brain-bench/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "cannot create HTTP observer")?;
    let mut observations = Vec::with_capacity(count);
    for index in 0..count {
        let started = Instant::now();
        let observed = async {
            let mut response = client.get(url.clone()).header("accept-encoding", "identity").send().await.map_err(|_| "request-failed")?;
            let ttfb = started.elapsed().as_secs_f64() * 1000.0;
            let status = response.status().as_u16();
            if response.content_length().is_some_and(|n| n > MAX_BODY as u64) { return Err("response-too-large"); }
            let mut bytes = 0usize;
            while let Some(chunk) = response.chunk().await.map_err(|_| "body-read-failed")? {
                bytes = bytes.checked_add(chunk.len()).ok_or("response-too-large")?;
                if bytes > MAX_BODY { return Err("response-too-large"); }
            }
            Ok(json!({"index":index + 1,"phase":if index == 0 {"firstObserved"} else {"repeatObserved"},"status":status,"ttfbMs":ttfb,"totalMs":started.elapsed().as_secs_f64()*1000.0,"bodyBytes":bytes,"ok":(200..300).contains(&status)}))
        }.await;
        match observed {
            Ok(value) => observations.push(value),
            Err(code) => observations.push(json!({"index":index + 1,"phase":if index == 0 {"firstObserved"} else {"repeatObserved"},"ok":false,"error":code,"totalMs":started.elapsed().as_secs_f64()*1000.0})),
        }
    }
    let ok = observations.iter().all(|row| row["ok"] == true);
    let repeats = observations
        .iter()
        .skip(1)
        .filter_map(|row| row["totalMs"].as_f64().filter(|_| row["ok"] == true))
        .collect::<Vec<_>>();
    let mean = if repeats.is_empty() {
        None
    } else {
        Some(repeats.iter().sum::<f64>() / repeats.len() as f64)
    };
    Ok(
        json!({"schemaVersion":1,"ok":ok,"mode":"anonymous-http-observations","url":url.as_str(),"requested":count,"completed":observations.len(),"limits":{"requestMs":3000,"totalMs":30000,"bodyBytes":MAX_BODY,"redirects":0,"retries":0,"concurrency":1},"firstObserved":observations.first(),"repeatObserved":{"count":repeats.len(),"meanTotalMs":mean},"observations":observations,"scope":"Headers-ready time includes connection and server work. No browser, rendering, cache-state, cold/warm, field-vitals or build-budget inference."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_sensitive_and_ambiguous_targets_before_network() {
        for raw in [
            "https://example.com/?token=secret",
            "https://example.com/#part",
            "file:///tmp/source",
            "https://example.com/\n",
        ] {
            assert!(target(raw).is_err());
        }
        let mut credentials = reqwest::Url::parse("https://example.com/").unwrap();
        credentials.set_username("sample").unwrap();
        credentials.set_password(Some("placeholder")).unwrap();
        assert!(target(credentials.as_str()).is_err());
        assert!(target("http://127.0.0.1:8080/path").is_ok());
    }
}
