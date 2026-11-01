//! Per-build observations. A build owns one `Build`; each thread working for it attaches, so concurrent builds in one
//! process never mix their numbers. Storage is fixed-size, and nothing here reaches generated files or build hashes.
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

/// Stages of a build, in order. Wall time is the elapsed time on the build thread; CPU adds every attached worker.
pub const STAGES: [&str; 10] = ["check", "compile", "minify", "pack", "prerender", "seo", "cache-sw", "compress", "write", "other"];
const OTHER: usize = STAGES.len() - 1;

macro_rules! counters {
    ($($variant:ident => $name:literal),* $(,)?) => {
        /// Work actually executed: an operation that a cache or a shared snapshot avoided is not counted.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(usize)]
        pub enum Counter { $($variant),* }
        pub const COUNTERS: &[&str] = &[$($name),*];
    };
}
counters! {
    Parses => "parses",
    ContextLoads => "contextLoads",
    Checks => "checks",
    SharedFileReads => "sharedFileReads",
    CompileRuns => "compileRuns",
    CompileCacheHits => "compileCacheHits",
    CompileCacheMisses => "compileCacheMisses",
    DependencyHashes => "dependencyHashes",
    DependencyHashesReused => "dependencyHashesReused",
    DependencyHashBytes => "dependencyHashBytes",
    JsLexes => "jsLexes",
    ExportShakes => "exportShakes",
    BuildCacheHits => "buildCacheHits",
    BuildCacheMisses => "buildCacheMisses",
    BuildCacheWrites => "buildCacheWrites",
    BuildCacheWritesSkipped => "buildCacheWritesSkipped",
    CacheDirectoryScans => "cacheDirectoryScans",
    CacheMetadataReads => "cacheMetadataReads",
    CacheEvictions => "cacheEvictions",
    Compressions => "compressions",
    PrerenderedPages => "prerenderedPages",
    SeoViewParses => "seoViewParses",
    SeoViewParsesReused => "seoViewParsesReused",
    SeoHtmlParses => "seoHtmlParses",
}

#[derive(Default)]
struct Stage {
    wall_ns: AtomicU64,
    cpu_ns: AtomicU64,
    items: AtomicU64,
}

pub struct Build {
    id: u64,
    started_at: SystemTime,
    started: Instant,
    stages: [Stage; STAGES.len()],
    counters: Vec<AtomicU64>,
    active: Mutex<(usize, Instant, u64)>,
    current: AtomicUsize,
}

/// One closed stage. `wall_ms` of a stage that never ran is zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageReport {
    pub name: &'static str,
    pub wall_ms: f64,
    pub cpu_ms: f64,
    pub items: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub wall_ms: f64,
    pub stages: Vec<StageReport>,
    pub counters: Vec<(&'static str, u64)>,
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Build>>> = const { RefCell::new(None) };
}

/// CPU consumed by this thread. Child processes (Node prerender, headless Chrome) are never part of it.
fn thread_cpu_ns() -> u64 {
    #[cfg(unix)]
    {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `t` is a valid, writable timespec for the duration of the call.
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) } == 0 {
            return (t.tv_sec as u64).saturating_mul(1_000_000_000).saturating_add(t.tv_nsec as u64);
        }
    }
    0
}

fn stage_index(name: &str) -> usize {
    let name = match name {
        "SEO" => "seo",
        "cache/SW" => "cache-sw",
        other => other,
    };
    STAGES.iter().position(|s| *s == name).unwrap_or(OTHER)
}

impl Build {
    pub fn new() -> Arc<Build> {
        let now = Instant::now();
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Arc::new(Build {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            started_at: SystemTime::now(),
            started: now,
            stages: Default::default(),
            counters: COUNTERS.iter().map(|_| AtomicU64::new(0)).collect(),
            active: Mutex::new((OTHER, now, thread_cpu_ns())),
            current: AtomicUsize::new(OTHER),
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn started_at(&self) -> SystemTime {
        self.started_at
    }

    /// Closes the running stage on the build thread and opens `name`; the same stage again only records `items`.
    pub fn phase(&self, name: &str, items: usize) {
        let next = stage_index(name);
        let mut active = self.active.lock().unwrap();
        if active.0 != next {
            Self::close(&self.stages[active.0], &active);
            *active = (next, Instant::now(), thread_cpu_ns());
            self.current.store(next, Ordering::Relaxed);
        }
        self.stages[next].items.fetch_max(items as u64, Ordering::Relaxed);
    }

    fn close(stage: &Stage, active: &(usize, Instant, u64)) {
        stage.wall_ns.fetch_add(active.1.elapsed().as_nanos() as u64, Ordering::Relaxed);
        stage.cpu_ns.fetch_add(thread_cpu_ns().saturating_sub(active.2), Ordering::Relaxed);
    }

    pub fn add(&self, counter: Counter, n: u64) {
        self.counters[counter as usize].fetch_add(n, Ordering::Relaxed);
    }

    pub fn get(&self, counter: Counter) -> u64 {
        self.counters[counter as usize].load(Ordering::Relaxed)
    }

    /// Closes the running stage and returns the observations of this build.
    pub fn finish(&self) -> Report {
        {
            let mut active = self.active.lock().unwrap();
            Self::close(&self.stages[active.0], &active);
            *active = (OTHER, Instant::now(), thread_cpu_ns());
            self.current.store(OTHER, Ordering::Relaxed);
        }
        let ms = |ns: &AtomicU64| ns.load(Ordering::Relaxed) as f64 / 1_000_000.0;
        Report {
            wall_ms: self.started.elapsed().as_nanos() as f64 / 1_000_000.0,
            stages: STAGES
                .iter()
                .zip(&self.stages)
                .filter(|(_, s)| s.wall_ns.load(Ordering::Relaxed) > 0 || s.cpu_ns.load(Ordering::Relaxed) > 0)
                .map(|(name, s)| StageReport { name, wall_ms: ms(&s.wall_ns), cpu_ms: ms(&s.cpu_ns), items: s.items.load(Ordering::Relaxed) })
                .collect(),
            counters: COUNTERS.iter().zip(&self.counters).map(|(n, c)| (*n, c.load(Ordering::Relaxed))).collect(),
        }
    }
}

/// While alive, this thread reports to `build`. Dropping restores what was attached before (nested builds stay separate).
pub struct Attached(Option<Arc<Build>>);

pub fn attach(build: &Arc<Build>) -> Attached {
    Attached(CURRENT.with(|c| c.borrow_mut().replace(build.clone())))
}

impl Drop for Attached {
    fn drop(&mut self) {
        let previous = self.0.take();
        CURRENT.with(|c| *c.borrow_mut() = previous);
    }
}

pub fn current() -> Option<Arc<Build>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Adds to the attached build; a no-op outside a build (dev server, tests, `brain check`).
pub fn count(counter: Counter, n: u64) {
    CURRENT.with(|c| {
        if let Some(b) = c.borrow().as_ref() {
            b.add(counter, n)
        }
    })
}

pub fn phase(name: &str, items: usize) {
    CURRENT.with(|c| {
        if let Some(b) = c.borrow().as_ref() {
            b.phase(name, items)
        }
    })
}

/// A worker thread of the running stage: attaches and credits its CPU to that stage when dropped.
pub struct Worker {
    _attached: Attached,
    build: Arc<Build>,
    stage: usize,
    cpu: u64,
}

pub fn worker(build: &Arc<Build>) -> Worker {
    Worker { _attached: attach(build), build: build.clone(), stage: build.current.load(Ordering::Relaxed), cpu: thread_cpu_ns() }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.build.stages[self.stage].cpu_ns.fetch_add(thread_cpu_ns().saturating_sub(self.cpu), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spin(ms: u64) {
        let t = Instant::now();
        let mut x = 0u64;
        while t.elapsed().as_millis() < ms as u128 {
            x = std::hint::black_box(x.wrapping_add(1));
        }
    }

    #[test]
    fn stages_keep_wall_cpu_and_items_apart() {
        let b = Build::new();
        let _a = attach(&b);
        phase("check", 0);
        spin(3);
        phase("compress", 7);
        std::thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| {
                    let _w = worker(&b);
                    count(Counter::Compressions, 1);
                    spin(3);
                });
            }
        });
        let r = b.finish();
        let c = r.stages.iter().find(|s| s.name == "compress").unwrap();
        assert_eq!(c.items, 7);
        assert!(c.cpu_ms > 3.0, "workers credit their CPU: {c:?}");
        assert!(r.stages.iter().any(|s| s.name == "check"));
        assert_eq!(b.get(Counter::Compressions), 2);
    }

    #[test]
    fn concurrent_builds_are_independent() {
        std::thread::scope(|s| {
            for n in 1..=4u64 {
                s.spawn(move || {
                    let b = Build::new();
                    let _a = attach(&b);
                    for _ in 0..n * 100 {
                        count(Counter::Parses, 1);
                    }
                    assert_eq!(b.get(Counter::Parses), n * 100);
                });
            }
        });
    }

    #[test]
    fn outside_a_build_nothing_is_recorded_and_nesting_restores() {
        count(Counter::Parses, 1);
        assert!(current().is_none());
        let outer = Build::new();
        let _o = attach(&outer);
        {
            let inner = Build::new();
            let _i = attach(&inner);
            count(Counter::Parses, 1);
            assert_eq!(inner.get(Counter::Parses), 1);
        }
        count(Counter::Parses, 1);
        assert_eq!(outer.get(Counter::Parses), 1);
    }

    #[test]
    fn unknown_stage_names_stay_bounded() {
        let b = Build::new();
        b.phase("some-new-stage", 1);
        b.phase("another-stage", 1);
        let r = b.finish();
        assert!(r.stages.len() <= STAGES.len());
    }
}
