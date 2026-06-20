use log::Level;
use std::array;
use std::fmt::{self, Write as _};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const RT_LOG_CAPACITY: usize = 4096;
const RT_MESSAGE_CAPACITY: usize = 256;
const RT_TARGET_CAPACITY: usize = 96;

static RT_LOG: OnceLock<RtLogInner> = OnceLock::new();
static RT_DRAIN_STATE: Mutex<RtDrainState> = Mutex::new(RtDrainState::new());

/// Keeps realtime log draining alive for one plugin instance.
///
/// Dropping the last session stops the background drain worker before the plugin
/// binary can be unloaded by the host.
#[must_use = "keep LogSession alive for the plugin instance lifetime"]
pub struct LogSession {
    _private: (),
}

impl LogSession {
    pub(crate) fn start() -> Self {
        start_log_session();
        Self { _private: () }
    }
}

impl Drop for LogSession {
    fn drop(&mut self) {
        release_log_session();
    }
}

/// Configuration for the background realtime log drain worker.
pub struct RtDrainConfig {
    interval: Duration,
}

impl Default for RtDrainConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(100),
        }
    }
}

impl RtDrainConfig {
    /// Sets how often the background worker drains realtime logs.
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }
}

/// Starts the realtime log drain worker once for the current process.
///
/// This is called automatically by [`crate::init!`] in debug builds and when
/// `WRAC_RT_LOG` is set. Calling it directly is useful for tests or custom host
/// integration.
pub fn init_rt_log_drain_once(config: RtDrainConfig) {
    let Ok(mut state) = RT_DRAIN_STATE.lock() else {
        return;
    };
    state.start_worker_if_absent(config);
}

fn release_log_session() {
    let Ok(mut state) = RT_DRAIN_STATE.lock() else {
        return;
    };
    if state.session_count == 0 {
        return;
    }
    state.session_count -= 1;
    if state.session_count != 0 {
        return;
    }
    if let Some(worker) = state.worker.take() {
        // Keep the lifecycle state locked until the old worker exits. Otherwise a
        // new session could skip startup against a worker that is about to be removed.
        stop_drain_worker(worker);
    } else {
        drain_existing_rt_logs_once();
    }
}

#[cfg(test)]
fn shutdown_rt_log_drain() {
    if let Ok(mut state) = RT_DRAIN_STATE.lock() {
        if let Some(worker) = state.worker.take() {
            stop_drain_worker(worker);
        } else {
            drain_existing_rt_logs_once();
        }
    }
}

fn stop_drain_worker(worker: RtDrainWorker) {
    worker.stop_requested.store(true, Ordering::Release);
    let thread = worker.handle.thread().clone();
    thread.unpark();
    if thread.id() != thread::current().id() {
        let _ = worker.handle.join();
    }
    drain_existing_rt_logs_once();
}

/// Drains the global realtime log once on the current thread.
pub fn drain_rt_logs_once() {
    rt_log().drain_to_log();
}

fn drain_existing_rt_logs_once() {
    if let Some(rt_log) = RT_LOG.get() {
        rt_log.drain_to_log();
    }
}

fn start_log_session() {
    // Initialize from the non-realtime setup path so the first RT log write only
    // touches atomics and the fixed buffer.
    let _ = rt_log();

    let drain_config = (cfg!(debug_assertions) || std::env::var_os("WRAC_RT_LOG").is_some())
        .then(RtDrainConfig::default);
    let Ok(mut state) = RT_DRAIN_STATE.lock() else {
        return;
    };
    state.session_count += 1;
    if let Some(config) = drain_config {
        state.start_worker_if_absent(config);
    }
}

/// Writes one realtime log record into the fixed-size global buffer.
///
/// This function is public so the exported `rt*` macros can call it through
/// `$crate`. Plugin code should not call or rely on this function directly; use
/// the `rt*` logging macros instead.
pub fn write_rt_log(level: Level, target: &'static str, args: fmt::Arguments<'_>) {
    rt_log().write_fmt(level, target, args);
}

struct RtDrainWorker {
    stop_requested: std::sync::Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

struct RtDrainState {
    session_count: usize,
    worker: Option<RtDrainWorker>,
}

impl RtDrainState {
    const fn new() -> Self {
        Self {
            session_count: 0,
            worker: None,
        }
    }

    fn start_worker_if_absent(&mut self, config: RtDrainConfig) {
        if self.worker.is_some() {
            return;
        }

        let interval = config.interval;
        let stop_requested = std::sync::Arc::new(AtomicBool::new(false));
        let thread_stop_requested = stop_requested.clone();
        let Ok(handle) = thread::Builder::new()
            .name("wrac-rt-log-drain".to_string())
            .spawn(move || {
                while !thread_stop_requested.load(Ordering::Acquire) {
                    thread::park_timeout(interval);
                    if thread_stop_requested.load(Ordering::Acquire) {
                        break;
                    }
                    drain_existing_rt_logs_once();
                }
                drain_existing_rt_logs_once();
            })
        else {
            return;
        };
        self.worker = Some(RtDrainWorker {
            stop_requested,
            handle,
        });
    }
}

fn rt_log() -> &'static RtLogInner {
    RT_LOG.get_or_init(RtLogInner::new)
}

struct RtLogInner {
    next_sequence: AtomicU64,
    drain_sequence: AtomicU64,
    dropped: AtomicU64,
    slots: Vec<RtLogSlot>,
}

impl RtLogInner {
    fn new() -> Self {
        Self {
            next_sequence: AtomicU64::new(0),
            drain_sequence: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            // Keep fixed-size slots on the heap to avoid large plugin-instance stack frames.
            slots: (0..RT_LOG_CAPACITY).map(|_| RtLogSlot::new()).collect(),
        }
    }

    fn write_fmt(&self, level: Level, target: &'static str, args: fmt::Arguments<'_>) {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        let drain_sequence = self.drain_sequence.load(Ordering::Acquire);
        if sequence.saturating_sub(drain_sequence) >= RT_LOG_CAPACITY as u64 {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }

        self.slots[sequence as usize % RT_LOG_CAPACITY].write(sequence, level, target, args);
    }

    fn drain_to_log(&self) {
        let total = self.next_sequence.load(Ordering::Acquire);
        let retained_start = total.saturating_sub(RT_LOG_CAPACITY as u64);
        let start = self
            .drain_sequence
            .load(Ordering::Acquire)
            .max(retained_start);

        let previous_drain_sequence = self.drain_sequence.load(Ordering::Acquire);
        let dropped = self.dropped.swap(0, Ordering::AcqRel);
        if dropped > 0 || start > previous_drain_sequence {
            log::warn!(
                target: "wrac_log::rt",
                "[rt] dropped={} skipped={}",
                dropped,
                start.saturating_sub(previous_drain_sequence),
            );
        }

        let mut drained_until = start;
        for sequence in start..total {
            if let Some(record) = self.slots[sequence as usize % RT_LOG_CAPACITY].read(sequence) {
                log::log!(
                    target: record.target.as_str(),
                    record.level,
                    "[rt] seq={} {}",
                    record.sequence,
                    record.message.as_str(),
                );
                drained_until = sequence + 1;
            } else {
                // The writer reserves the sequence before publishing the slot. Stop at the first
                // gap so a record published immediately after this drain is not skipped forever.
                break;
            }
        }
        self.drain_sequence.store(drained_until, Ordering::Release);
    }
}

struct RtLogSlot {
    sequence: AtomicU64,
    level: AtomicU8,
    target_len: AtomicUsize,
    target: [AtomicU8; RT_TARGET_CAPACITY],
    message_len: AtomicUsize,
    message: [AtomicU8; RT_MESSAGE_CAPACITY],
}

impl RtLogSlot {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            level: AtomicU8::new(level_to_u8(Level::Debug)),
            target_len: AtomicUsize::new(0),
            target: array::from_fn(|_| AtomicU8::new(0)),
            message_len: AtomicUsize::new(0),
            message: array::from_fn(|_| AtomicU8::new(0)),
        }
    }

    fn write(&self, sequence: u64, level: Level, target: &str, args: fmt::Arguments<'_>) {
        self.sequence.store(0, Ordering::Release);
        self.level.store(level_to_u8(level), Ordering::Relaxed);
        write_atomic_bytes(&self.target, &self.target_len, target.as_bytes());

        let mut message = FixedMessage::new();
        let _ = message.write_fmt(args);
        write_atomic_bytes(&self.message, &self.message_len, message.as_bytes());
        self.sequence.store(sequence + 1, Ordering::Release);
    }

    fn read(&self, sequence: u64) -> Option<RtLogRecord> {
        if self.sequence.load(Ordering::Acquire) != sequence + 1 {
            return None;
        }

        let record = RtLogRecord {
            sequence,
            level: u8_to_level(self.level.load(Ordering::Relaxed)),
            target: read_atomic_string::<RT_TARGET_CAPACITY>(&self.target, &self.target_len),
            message: read_atomic_string::<RT_MESSAGE_CAPACITY>(&self.message, &self.message_len),
        };

        if self.sequence.load(Ordering::Acquire) == sequence + 1 {
            Some(record)
        } else {
            None
        }
    }
}

struct RtLogRecord {
    sequence: u64,
    level: Level,
    target: FixedString<RT_TARGET_CAPACITY>,
    message: FixedString<RT_MESSAGE_CAPACITY>,
}

struct FixedMessage {
    bytes: [u8; RT_MESSAGE_CAPACITY],
    len: usize,
}

impl FixedMessage {
    fn new() -> Self {
        Self {
            bytes: [0; RT_MESSAGE_CAPACITY],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl fmt::Write for FixedMessage {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let remaining = RT_MESSAGE_CAPACITY.saturating_sub(self.len);
        let count = utf8_boundary_len(value, remaining);
        self.bytes[self.len..self.len + count].copy_from_slice(&value.as_bytes()[..count]);
        self.len += count;
        Ok(())
    }
}

fn utf8_boundary_len(value: &str, limit: usize) -> usize {
    if value.len() <= limit {
        return value.len();
    }
    let mut count = limit.min(value.len());
    while count > 0 && !value.is_char_boundary(count) {
        count -= 1;
    }
    count
}

struct FixedString<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> FixedString<N> {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("<invalid utf8>")
    }
}

fn write_atomic_bytes<const N: usize>(target: &[AtomicU8; N], len: &AtomicUsize, bytes: &[u8]) {
    let count = N.min(bytes.len());
    for index in 0..count {
        target[index].store(bytes[index], Ordering::Relaxed);
    }
    len.store(count, Ordering::Relaxed);
}

fn read_atomic_string<const N: usize>(source: &[AtomicU8; N], len: &AtomicUsize) -> FixedString<N> {
    let len = len.load(Ordering::Relaxed).min(N);
    let mut bytes = [0; N];
    for index in 0..len {
        bytes[index] = source[index].load(Ordering::Relaxed);
    }
    FixedString { bytes, len }
}

const fn level_to_u8(level: Level) -> u8 {
    match level {
        Level::Error => 1,
        Level::Warn => 2,
        Level::Info => 3,
        Level::Debug => 4,
        Level::Trace => 5,
    }
}

fn u8_to_level(level: u8) -> Level {
    match level {
        1 => Level::Error,
        2 => Level::Warn,
        3 => Level::Info,
        5 => Level::Trace,
        _ => Level::Debug,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static SESSION_TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn reset_sessions_for_test() {
        shutdown_rt_log_drain();
        if let Ok(mut state) = RT_DRAIN_STATE.lock() {
            state.session_count = 0;
        }
    }

    fn drain_worker_is_running() -> bool {
        RT_DRAIN_STATE
            .lock()
            .map(|state| state.worker.is_some())
            .unwrap_or(false)
    }

    fn log_session_count() -> usize {
        RT_DRAIN_STATE
            .lock()
            .map(|state| state.session_count)
            .unwrap_or_default()
    }

    #[test]
    fn drain_stops_before_unpublished_slot() {
        let log = RtLogInner::new();
        log.next_sequence.store(1, Ordering::Release);

        log.drain_to_log();
        assert_eq!(log.drain_sequence.load(Ordering::Acquire), 0);

        log.slots[0].write(0, Level::Debug, "test", format_args!("published"));
        log.drain_to_log();
        assert_eq!(log.drain_sequence.load(Ordering::Acquire), 1);
    }

    #[test]
    fn fixed_message_truncates_at_utf8_boundary() {
        let mut message = FixedMessage::new();
        let value = "a".repeat(RT_MESSAGE_CAPACITY - 1) + "é";

        message.write_str(&value).unwrap();

        assert_eq!(message.len, RT_MESSAGE_CAPACITY - 1);
        assert_eq!(
            std::str::from_utf8(message.as_bytes()).unwrap().len(),
            message.len,
        );
    }

    #[test]
    fn log_session_stops_drain_worker_after_last_drop() {
        let _guard = SESSION_TEST_MUTEX
            .lock()
            .expect("session test mutex poisoned");
        reset_sessions_for_test();

        let first_session = LogSession::start();
        let second_session = LogSession::start();

        assert_eq!(log_session_count(), 2);
        assert!(drain_worker_is_running());

        drop(first_session);
        assert_eq!(log_session_count(), 1);
        assert!(drain_worker_is_running());

        drop(second_session);
        assert_eq!(log_session_count(), 0);
        assert!(!drain_worker_is_running());

        let restarted_session = LogSession::start();
        assert_eq!(log_session_count(), 1);
        assert!(drain_worker_is_running());

        drop(restarted_session);
        reset_sessions_for_test();
    }
}
