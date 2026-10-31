//! Fallible dev workers. Dispatch owns its original stream and admission slot even when spawning fails.
use super::{ConnectionPermit, Ctx, handle};
use std::{
    io,
    io::Write,
    net::TcpStream,
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(super) enum Role {
    Accept,
    Request,
    Watch,
}
impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::Accept => "brain-dev-accept",
            Self::Request => "brain-dev-request",
            Self::Watch => "brain-dev-watch",
        }
    }
    fn error(self) -> &'static str {
        match self {
            Self::Accept => {
                "BDEV_THREAD_START: dev accept worker unavailable; fix: release owned process/thread capacity and restart the frontend"
            }
            Self::Request => {
                "BDEV_THREAD_DISPATCH: dev request worker unavailable; fix: release owned process/thread capacity and retry"
            }
            Self::Watch => {
                "BDEV_THREAD_START: dev watch worker unavailable; fix: release owned process/thread capacity and restart the frontend"
            }
        }
    }
}

/// Zero-sized in production. Test failures and lifetime probes belong to one server, never global state.
#[derive(Clone, Default)]
pub(super) struct Threads {
    #[cfg(test)]
    state: Arc<TestState>,
}
#[cfg(test)]
#[derive(Default)]
struct TestState {
    failures: [std::sync::atomic::AtomicUsize; 3],
    running: [std::sync::atomic::AtomicUsize; 3],
    context: std::sync::Mutex<Option<std::sync::Weak<Ctx>>>,
}
impl Threads {
    pub(super) fn observe(&self, _ctx: &Arc<Ctx>) {
        #[cfg(test)]
        {
            *self.state.context.lock().unwrap() = Some(Arc::downgrade(_ctx));
        }
    }
    pub(super) fn spawn<F>(&self, role: Role, run: F) -> io::Result<JoinHandle<()>>
    where
        F: FnOnce() + Send + 'static,
    {
        #[cfg(test)]
        {
            use std::sync::atomic::Ordering;
            let failures = &self.state.failures[role as usize];
            let mut remaining = failures.load(Ordering::Acquire);
            while remaining > 0 {
                match failures.compare_exchange_weak(
                    remaining,
                    remaining - 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return Err(io::Error::new(io::ErrorKind::WouldBlock, role.error())),
                    Err(now) => remaining = now,
                }
            }
        }
        #[cfg(test)]
        let state = self.state.clone();
        let builder = std::thread::Builder::new();
        let builder = match role {
            Role::Request => builder,
            _ => builder.name(role.name().into()),
        };
        builder
            .spawn(move || {
                #[cfg(test)]
                let _running = {
                    struct Running(Arc<TestState>, usize);
                    impl Drop for Running {
                        fn drop(&mut self) {
                            self.0.running[self.1]
                                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                        }
                    }
                    state.running[role as usize].fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                    Running(state, role as usize)
                };
                run();
            })
            .map_err(|error| io::Error::new(error.kind(), role.error()))
    }
}

pub(super) fn startup_error(ctx: &Ctx, error: io::Error) -> io::Error {
    ctx.stop.store(true, std::sync::atomic::Ordering::Release);
    if let Some(proxy) = &ctx.native_sockets {
        let _ = proxy.stop();
    }
    error
}

/// If Builder rejects its closure, dropping this value can answer without cloning the FD or allocating a queue.
pub(super) struct PendingRequest {
    stream: Option<TcpStream>,
    _permit: ConnectionPermit,
}
impl PendingRequest {
    pub(super) fn new(stream: TcpStream, permit: ConnectionPermit) -> Self {
        Self {
            stream: Some(stream),
            _permit: permit,
        }
    }
    pub(super) fn run(mut self, ctx: Arc<Ctx>) {
        if let Some(stream) = self.stream.take() {
            let _ = handle(stream, &ctx);
        }
    }
}
impl Drop for PendingRequest {
    fn drop(&mut self) {
        if let Some(mut stream) = self.stream.take() {
            const RESPONSE: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 45\r\nCache-Control: no-store\r\nRetry-After: 1\r\nConnection: close\r\n\r\nDev request worker unavailable; retry shortly";
            let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
            // One bounded best-effort write: no header allocation, duplicate FD or renewed write_all deadline.
            let _ = stream.write(RESPONSE);
        }
    }
}

#[derive(Default)]
pub(super) struct FailureLog {
    last: Option<Instant>,
    pending: u64,
    total: u64,
}
impl FailureLog {
    pub(super) fn failed(&mut self, ctx: &Ctx) {
        self.failed_at(ctx, Instant::now());
    }
    fn failed_at(&mut self, ctx: &Ctx, now: Instant) {
        self.pending = self.pending.saturating_add(1);
        self.total = self.total.saturating_add(1);
        if self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(1))
        {
            self.flush_at(ctx, now);
        }
    }
    fn flush_at(&mut self, ctx: &Ctx, now: Instant) {
        if self.pending == 0 {
            return;
        }
        ctx.log("error", format!("{{\"schemaVersion\":1,\"event\":\"dev-thread-unavailable\",\"phase\":\"request-dispatch\",\"code\":\"BDEV_THREAD_DISPATCH\",\"failures\":{},\"totalFailures\":{},\"fix\":\"Release owned process/thread capacity, reduce concurrent work and retry\",\"valuesCaptured\":false}}", self.pending, self.total));
        self.pending = 0;
        self.last = Some(now);
    }
    pub(super) fn finish(&mut self, ctx: &Ctx) {
        self.flush_at(ctx, Instant::now());
    }
}

#[cfg(test)]
#[path = "dev_thread_tests.rs"]
mod tests;
