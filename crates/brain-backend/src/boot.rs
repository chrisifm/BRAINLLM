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
            acquire: Duration::from_secs(2),
        }
    }
    /// One-shot command (migrate, bootstrap): no retry, the historical 5 s acquire budget.
    pub fn command(production: bool) -> Self {
        Self { give_up: Duration::ZERO, acquire: Duration::from_secs(5), ..Self::server(production) }
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

/// Production databases outside this machine must use `sslmode=verify-full`: the default
/// `prefer` mode encrypts without checking the certificate, so it gives no protection against
/// an attacker in the network path.
pub fn require_verified_tls(url: &str, production: bool) -> Result<PgConnectOptions, String> {
    let options = PgConnectOptions::from_str(url)
        .map_err(|_| "DATABASE_URL is not a valid Postgres URL (its value is not logged)".to_string())?;
    if production && !loopback(&options) && !matches!(options.get_ssl_mode(), PgSslMode::VerifyFull) {
        return Err("BDB_TLS_UNVERIFIED: production DATABASE_URL points at a remote host without sslmode=verify-full; append ?sslmode=verify-full (plus sslrootcert=<ca.pem> for a private CA) or connect through loopback or a unix socket".into());
    }
    Ok(options)
}

fn retryable(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut => true,
        // 57P03 cannot_connect_now (starting up), 53300 too_many_connections.
        sqlx::Error::Database(db) => matches!(db.code().as_deref(), Some("57P03" | "53300")),
        _ => false,
    }
}

fn unit_random() -> f64 {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 4];
    if ring::rand::SystemRandom::new().fill(&mut bytes).is_err() {
        return 0.5;
    }
    f64::from(u32::from_le_bytes(bytes)) / (f64::from(u32::MAX) + 1.0)
}

/// Connects with the boot policy. Retries only transient failures (refused connection, server
/// starting up, connection slots full); bad credentials or TLS failures stop immediately.
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
        if !retryable(&error) || elapsed >= boot.give_up {
            return Err(if retryable(&error) && !boot.give_up.is_zero() {
                format!("BDB_CONNECT_TIMEOUT: Postgres stayed unreachable for {}s over {} attempts; check that it is running and DATABASE_URL is correct (credentials are not logged)", elapsed.as_secs(), attempt + 1)
            } else if matches!(error, sqlx::Error::Tls(_)) {
                "Cannot establish verified TLS with Postgres; check sslrootcert and that the certificate names the DATABASE_URL host".into()
            } else {
                "Cannot connect to Postgres; check DATABASE_URL (credentials are not logged)".into()
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

    #[tokio::test]
    async fn refused_connections_retry_until_the_budget_then_explain() {
        // Port 1 on loopback refuses immediately on every supported platform.
        let boot = Boot { give_up: Duration::from_millis(300), base: Duration::from_millis(20), ..Boot::server(false) };
        let err = connect("postgres://shop@127.0.0.1:1/shop", &boot).await.unwrap_err();
        assert!(err.starts_with("BDB_CONNECT_TIMEOUT"), "{err}");
        let err = connect("postgres://shop@127.0.0.1:1/shop", &Boot::command(false)).await.unwrap_err();
        assert!(err.starts_with("Cannot connect to Postgres"), "{err}");
    }
}
