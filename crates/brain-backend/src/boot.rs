//! Database boot policy: verified TLS for remote production databases and a bounded,
//! full-jitter connect retry so a backend that starts before Postgres does not crash-loop.
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{net::IpAddr, str::FromStr, time::Duration};

/// How a backend process connects at boot.
#[derive(Clone, Debug)]
pub struct Boot {
    /// Production refuses unverified TLS to non-loopback hosts.
    pub production: bool,
    /// First retry ceiling; doubles per attempt up to `cap`.
    pub base: Duration,
    pub cap: Duration,
    /// Total time spent retrying before giving up. Zero disables retries.
    pub give_up: Duration,
    /// Treat a refused role or missing database as passing: a cluster upgrade or a failover answers that for a while.
    pub wait_out_refusal: bool,
    /// How long a request waits for a pooled connection before it is treated as overload.
    pub acquire: Duration,
}

impl Boot {
    /// Long-running server: retries for up to 60 s and keeps pool waits short.
    pub fn server(production: bool) -> Self {
        Self {
            production,
            base: Duration::from_millis(250),
            cap: Duration::from_secs(5),
            give_up: Duration::from_secs(60),
            wait_out_refusal: true,
            acquire: Duration::from_secs(2),
        }
    }
    /// One-shot command (migrate, bootstrap): no retry, the historical 5 s acquire budget.
    pub fn command(production: bool) -> Self {
        Self { give_up: Duration::ZERO, wait_out_refusal: false, acquire: Duration::from_secs(5), ..Self::server(production) }
    }
    /// Full jitter: a uniform delay in `0..=min(cap, base * 2^attempt)`. `unit` is in `0.0..1.0`.
    pub fn delay(&self, attempt: u32, unit: f64) -> Duration {
        let ceiling = self
            .base
            .checked_mul(1u32.checked_shl(attempt.min(30)).unwrap_or(u32::MAX))
            .unwrap_or(self.cap)
            .min(self.cap);
        ceiling.mul_f64(unit.clamp(0.0, 1.0))
    }
}

fn loopback(options: &PgConnectOptions) -> bool {
    if options.get_socket().is_some() {
        return true;
    }
    let host = options.get_host().trim_start_matches('[').trim_end_matches(']');
    if host.starts_with('/') || host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.to_canonical().is_loopback())
}

/// The database is on this machine. An unparsable URL counts as local: B0762 reports it on its own.
pub fn database_is_local(url: &str) -> bool {
    PgConnectOptions::from_str(url).map_or(true, |options| loopback(&options))
}

/// The account name, as libpq takes it from the password database. sqlx builds whoami without its `std` feature, whose
/// stub answers `anonymous` on every platform, so a URL without a user would never reach the role `psql` connects as.
fn system_user() -> Option<String> {
    let id = std::process::Command::new("id").arg("-un").stdin(std::process::Stdio::null()).output().ok().filter(|o| o.status.success());
    id.and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        .or_else(|| ["USER", "LOGNAME", "USERNAME"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())))
}

fn names_user(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let (location, query) = rest.split_once('?').unwrap_or((rest, ""));
    location.split('/').next().unwrap_or_default().contains('@') || query.split('&').any(|pair| pair.starts_with("user="))
}

/// `DATABASE_URL` as connection options: without a user, `PGUSER` and then the system account, the same as `psql`.
pub fn connect_options(url: &str) -> Result<PgConnectOptions, sqlx::Error> {
    let options = PgConnectOptions::from_str(url)?;
    if names_user(url) || std::env::var_os("PGUSER").is_some() {
        return Ok(options);
    }
    Ok(match system_user() {
        Some(user) => options.username(&user),
        None => options,
    })
}

/// Production databases outside this machine must use `sslmode=verify-full`: the default
/// `prefer` mode encrypts without checking the certificate, so it gives no protection against
/// an attacker in the network path.
pub fn require_verified_tls(url: &str, production: bool) -> Result<PgConnectOptions, String> {
    let options = connect_options(url)
        .map_err(|_| "B0762: DATABASE_URL is not a valid Postgres URL (its value is not logged); fix: use postgres://user:password@host:5432/database".to_string())?;
    if production && !loopback(&options) && !matches!(options.get_ssl_mode(), PgSslMode::VerifyFull) {
        return Err("BDB_TLS_UNVERIFIED (B0762): production DATABASE_URL points at a remote host without sslmode=verify-full; fix: append ?sslmode=verify-full (plus sslrootcert=<ca.pem> for a private CA) or connect through loopback or a unix socket".into());
    }
    Ok(options)
}

/// Why a connection failed, naming what was tried: the host, the database and the user, which libpq and sqlx take from
/// `PGUSER` or the system account when the URL has none. The password never appears.
pub fn connect_failure(options: &PgConnectOptions, error: &sqlx::Error) -> String {
    let user = options.get_username();
    let database = options.get_database().unwrap_or(user);
    let tried = format!("{}:{} database `{database}` as user `{user}`", options.get_host(), options.get_port());
    let (reason, fix) = match error {
        sqlx::Error::Database(db) => match db.code().as_deref() {
            Some("28P01" | "28000") => ("the server refused this user", format!("create the role (`createuser {user}`) or name another one: postgres://user:password@host:5432/{database}")),
            Some("3D000") => ("this database does not exist", format!("create it: `createdb {database}`")),
            _ => ("the server refused the connection", "check DATABASE_URL and the server log".to_string()),
        },
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut => ("nothing answered there", "start Postgres or correct the host and port in DATABASE_URL".to_string()),
        _ => ("the connection failed", "check DATABASE_URL".to_string()),
    };
    format!("Cannot connect to Postgres at {tried}: {reason} (B0762); fix: {fix} (credentials are not logged)")
}

fn retryable(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut => true,
        // 57P03 cannot_connect_now (starting up), 53300 too_many_connections.
        sqlx::Error::Database(db) => matches!(db.code().as_deref(), Some("57P03" | "53300")),
        _ => false,
    }
}

fn refused(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if matches!(db.code().as_deref(), Some("28P01" | "28000" | "3D000")))
}

fn unit_random() -> f64 {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 4];
    if ring::rand::SystemRandom::new().fill(&mut bytes).is_err() {
        return 0.5;
    }
    f64::from(u32::from_le_bytes(bytes)) / (f64::from(u32::MAX) + 1.0)
}

/// Connects with the boot policy. Retries transient failures (refused connection, server starting up, connection slots
/// full) and, for a long-lived process, a refused role or missing database, naming the cause on the first refusal; a
/// one-shot command fails on those at once, and TLS failures always stop immediately.
pub async fn connect(url: &str, boot: &Boot) -> Result<PgPool, String> {
    let options = require_verified_tls(url, boot.production)?;
    let started = tokio::time::Instant::now();
    let mut attempt = 0u32;
    loop {
        // One direct connection first: it fails fast on a refused port, whereas a pool keeps
        // retrying internally until its acquire timeout and hides the cause.
        let probe = match tokio::time::timeout(boot.acquire.max(Duration::from_secs(5)), options.connect()).await {
            Ok(result) => result,
            Err(_) => Err(sqlx::Error::PoolTimedOut),
        };
        let pool = match probe {
            Ok(conn) => {
                use sqlx::Connection;
                let _ = conn.close().await;
                PgPoolOptions::new()
                    .max_connections(8)
                    .acquire_timeout(boot.acquire)
                    .after_connect(|conn, _| Box::pin(async move {
                        // Keep pg_catalog implicit: it resolves first, while unqualified CREATE targets public.
                        sqlx::raw_sql("SET search_path=public; SET statement_timeout='5s'; SET lock_timeout='2s'; SET idle_in_transaction_session_timeout='10s';").execute(conn).await?;
                        Ok(())
                    }))
                    .connect_with(options.clone())
                    .await
            }
            Err(error) => Err(error),
        };
        let error = match pool {
            Ok(pool) => return Ok(pool),
            Err(error) => error,
        };
        let elapsed = started.elapsed();
        let waits_out = boot.wait_out_refusal && refused(&error);
        if waits_out && elapsed >= boot.give_up {
            return Err(format!("{}; gave up after {}s over {} attempts", connect_failure(&options, &error), elapsed.as_secs(), attempt + 1));
        }
        if waits_out && attempt == 0 {
            eprintln!("{}", serde_json::json!({"schemaVersion":1,"event":"database-connect-refused","code":"B0762","msg":connect_failure(&options, &error),"retrySeconds":boot.give_up.as_secs()}));
        }
        if !(waits_out || retryable(&error)) || elapsed >= boot.give_up {
            return Err(if retryable(&error) && !boot.give_up.is_zero() {
                format!("BDB_CONNECT_TIMEOUT (B0762): Postgres stayed unreachable for {}s over {} attempts; fix: check that it is running and DATABASE_URL is correct (credentials are not logged)", elapsed.as_secs(), attempt + 1)
            } else if matches!(error, sqlx::Error::Tls(_)) {
                "Cannot establish verified TLS with Postgres (B0762); fix: check sslrootcert and that the certificate names the DATABASE_URL host".into()
            } else {
                connect_failure(&options, &error)
            });
        }
        let delay = boot.delay(attempt, unit_random()).min(boot.give_up - elapsed);
        eprintln!("{}", serde_json::json!({"schemaVersion":1,"event":"database-connect-retry","attempt":attempt + 1,"delayMs":delay.as_millis() as u64,"valuesCaptured":false}));
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_requires_verify_full_for_remote_hosts_only() {
        let remote = "postgres://shop:secret@db.shop.example:5432/shop";
        let err = require_verified_tls(remote, true).unwrap_err();
        assert!(err.starts_with("BDB_TLS_UNVERIFIED"), "{err}");
        assert!(!err.contains("secret") && !err.contains("db.shop.example"));
        assert!(require_verified_tls(&format!("{remote}?sslmode=require"), true).is_err());
        assert!(require_verified_tls(&format!("{remote}?sslmode=verify-ca"), true).is_err());
        assert!(require_verified_tls(&format!("{remote}?sslmode=verify-full"), true).is_ok());
        assert!(require_verified_tls(remote, false).is_ok(), "development keeps today's behavior");
        for local in [
            "postgres://shop@127.0.0.1:5432/shop",
            "postgres://shop@localhost/shop",
            "postgres://shop@[::1]:5432/shop",
            "postgres://shop@127.8.9.10/shop",
            "postgres:///shop?host=/var/run/postgresql",
        ] {
            assert!(require_verified_tls(local, true).is_ok(), "{local}");
        }
        assert!(require_verified_tls("mysql://nope", true).is_err());
    }

    #[test]
    fn full_jitter_stays_under_the_doubling_ceiling_and_cap() {
        let boot = Boot::server(true);
        assert_eq!(boot.delay(0, 1.0), Duration::from_millis(250));
        assert_eq!(boot.delay(2, 1.0), Duration::from_millis(1000));
        assert_eq!(boot.delay(10, 1.0), Duration::from_secs(5));
        assert_eq!(boot.delay(40, 1.0), Duration::from_secs(5));
        assert_eq!(boot.delay(3, 0.0), Duration::ZERO);
        assert_eq!(boot.delay(1, 0.5), Duration::from_millis(250));
        let u = unit_random();
        assert!((0.0..1.0).contains(&u));
    }

    #[test]
    fn a_url_without_a_user_connects_as_the_system_account() {
        let account = std::env::var("PGUSER").ok().unwrap_or_else(|| String::from_utf8(std::process::Command::new("id").arg("-un").output().unwrap().stdout).unwrap().trim().to_string());
        assert_ne!(account, "anonymous");
        for url in ["postgresql://127.0.0.1:5432/db", "postgres://db.internal/db?sslmode=require", "postgres:///db"] {
            assert_eq!(connect_options(url).unwrap().get_username(), account, "{url}");
        }
        for (url, user) in [("postgres://alice@127.0.0.1/db", "alice"), ("postgres://alice:p%40ss@127.0.0.1/db", "alice"), ("postgres://127.0.0.1/db?user=bob", "bob")] {
            assert_eq!(connect_options(url).unwrap().get_username(), user, "{url}");
        }
    }

    #[tokio::test]
    async fn refused_connections_retry_until_the_budget_then_explain() {
        // Port 1 on loopback refuses immediately on every supported platform.
        let boot = Boot { give_up: Duration::from_millis(300), base: Duration::from_millis(20), ..Boot::server(false) };
        let err = connect("postgres://shop@127.0.0.1:1/shop", &boot).await.unwrap_err();
        assert!(err.starts_with("BDB_CONNECT_TIMEOUT"), "{err}");
        let err = connect("postgres://alice:s3cret@127.0.0.1:1/shop", &Boot::command(false)).await.unwrap_err();
        assert!(err.starts_with("Cannot connect to Postgres"), "{err}");
        assert!(err.contains("as user `alice`") && err.contains("127.0.0.1:1") && err.contains("database `shop`") && !err.contains("s3cret"), "{err}");
    }

    /// A server that answers every connection with the given Postgres error, counting the attempts.
    async fn refusing(code: &'static str) -> (String, std::sync::Arc<std::sync::atomic::AtomicU32>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("postgres://someone@{}/db?sslmode=disable", listener.local_addr().unwrap());
        let count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut startup = [0u8; 512];
                let _ = socket.read(&mut startup).await;
                let fields = format!("SFATAL\0C{code}\0Mrefused\0\0");
                let mut reply = vec![b'E'];
                reply.extend_from_slice(&(fields.len() as u32 + 4).to_be_bytes());
                reply.extend_from_slice(fields.as_bytes());
                let _ = socket.write_all(&reply).await;
            }
        });
        (url, count)
    }

    #[tokio::test]
    async fn a_long_lived_process_waits_out_a_refused_role_and_a_command_does_not() {
        for code in ["28P01", "28000", "3D000"] {
            let (url, count) = refusing(code).await;
            let quick = Boot { base: Duration::from_millis(5), cap: Duration::from_millis(20), give_up: Duration::from_millis(300), ..Boot::server(false) };
            let error = connect(&url, &quick).await.unwrap_err();
            let attempts = count.load(std::sync::atomic::Ordering::SeqCst);
            assert!(attempts > 2, "{code}: {attempts} attempts");
            assert!(error.contains("B0762") && error.contains("attempts"), "{error}");
            let (url, count) = refusing(code).await;
            let error = connect(&url, &Boot::command(false)).await.unwrap_err();
            assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1, "{code}: a command fails fast");
            assert!(error.contains("B0762"), "{error}");
        }
    }

}
