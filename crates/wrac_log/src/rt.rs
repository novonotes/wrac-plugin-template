use log::Level;
use std::array;
use std::fmt::{self, Write as _};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

const RT_LOG_CAPACITY: usize = 4096;
const RT_MESSAGE_CAPACITY: usize = 256;
const RT_TARGET_CAPACITY: usize = 96;

static RT_LOG: OnceLock<RtLogInner> = OnceLock::new();
static RT_DRAIN_WORKER: OnceLock<()> = OnceLock::new();

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
    RT_DRAIN_WORKER.get_or_init(|| {
        let interval = config.interval;
        let _ = thread::Builder::new()
            .name("wrac-rt-log-drain".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(interval);
                    drain_rt_logs_once();
                }
            });
    });
}

/// Drains the global realtime log once on the current thread.
pub fn drain_rt_logs_once() {
    rt_log().drain_to_log();
}

pub(crate) fn start_drain_if_enabled() {
    // Initialize from the non-realtime setup path so the first RT log write only
    // touches atomics and the fixed buffer.
    let _ = rt_log();

    if cfg!(debug_assertions) || std::env::var_os("WRAC_RT_LOG").is_some() {
        init_rt_log_drain_once(RtDrainConfig::default());
    }
}

#[doc(hidden)]
pub fn write_rt_log(level: Level, target: &'static str, args: fmt::Arguments<'_>) {
    rt_log().write_fmt(level, target, args);
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
}
