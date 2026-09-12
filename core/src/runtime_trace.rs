//! Opt-in runtime diagnostics. No queue, decoder, or scheduling decisions.
use std::{
    fmt,
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("LIBRESPOT_RUNTIME_TRACE").as_deref() == Ok("1"))
}

pub fn emit(args: fmt::Arguments<'_>) {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let origin = ORIGIN.get_or_init(Instant::now);
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    info!(target: "librespot_runtime", "[runtime] wall_us={} mono_us={} thread={:?} {}",
        wall.as_micros(), origin.elapsed().as_micros(), std::thread::current().id(), args);
}

#[macro_export]
macro_rules! runtime_trace {
    ($($arg:tt)*) => {
        if $crate::runtime_trace::enabled() {
            $crate::runtime_trace::emit(format_args!($($arg)*));
        }
    };
}

/// Only emits unusually slow operations; no regular packet/frame logging.
pub struct SlowOperation {
    start: Option<Instant>,
    name: &'static str,
    limit: Duration,
}

impl SlowOperation {
    pub fn new(name: &'static str, limit_ms: u64) -> Self {
        Self {
            start: enabled().then(Instant::now),
            name,
            limit: Duration::from_millis(limit_ms),
        }
    }
}

impl Drop for SlowOperation {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            let elapsed = start.elapsed();
            if elapsed >= self.limit {
                emit(format_args!(
                    "slow_operation={} elapsed_us={}",
                    self.name,
                    elapsed.as_micros()
                ));
            }
        }
    }
}
