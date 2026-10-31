//! Binding a listening socket across a restart. `SO_REUSEADDR` (set by the standard library on Unix) already covers
//! sockets in TIME_WAIT, so `AddrInUse` means a live listener: usually the previous instance still shutting down under a
//! process manager. Waiting briefly covers that without ever allowing two live servers on one port.
use std::{io, net::TcpListener, time::{Duration, Instant}};

/// Long enough for a previous instance to drain and exit, short enough to report a real conflict.
pub const RESTART_WAIT: Duration = Duration::from_secs(10);
const STEP: Duration = Duration::from_millis(100);

/// Bind, retrying only `AddrInUse` for up to `wait`. The final error states the cause and how to find the owner.
pub fn bind_waiting(host: &str, port: u16, wait: Duration) -> io::Result<TcpListener> {
    let started = Instant::now();
    let mut told = false;
    loop {
        match TcpListener::bind((host, port)) {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse && started.elapsed() < wait => {
                if !told {
                    told = true;
                    eprintln!("port {port} is still in use; waiting up to {} s for the previous process to release it", wait.as_secs());
                }
                std::thread::sleep(STEP);
            }
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                return Err(io::Error::new(io::ErrorKind::AddrInUse, in_use_message(host, port, wait)));
            }
            Err(error) => return Err(io::Error::new(error.kind(), format!("cannot listen on {host}:{port}: {error}; fix: use another --host or --port"))),
        }
    }
}

fn in_use_message(host: &str, port: u16, wait: Duration) -> String {
    format!("cannot listen on {host}:{port}: another live process still holds the port after {} s (a previous instance that has not exited, or a different server); fix: find the owner with `lsof -nP -iTCP:{port} -sTCP:LISTEN` (Linux: `ss -ltnp 'sport = :{port}'`), stop it, or pick another --port", wait.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_a_previous_instance_to_release_the_port() {
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        let bound = bind_waiting("127.0.0.1", port, Duration::from_secs(5));
        assert!(bound.is_ok(), "{bound:?}");
        releaser.join().unwrap();
    }

    #[test]
    fn never_shares_a_port_with_a_live_server_and_names_the_cause() {
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();
        let started = Instant::now();
        let error = bind_waiting("127.0.0.1", port, Duration::from_millis(300)).unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(300), "the wait is honored");
        assert!(started.elapsed() < Duration::from_secs(3), "the wait is bounded");
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        let text = error.to_string();
        assert!(text.contains("another live process") && text.contains(&format!("lsof -nP -iTCP:{port}")) && text.contains("--port"), "{text}");
        drop(held);
    }

    #[test]
    fn other_errors_fail_at_once() {
        let started = Instant::now();
        let error = bind_waiting("203.0.113.1", 0, Duration::from_secs(5)).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(started.elapsed() < Duration::from_secs(2), "only AddrInUse waits");
    }
}
