use std::io::Write as _;

pub(super) fn drain_rt_records_to_destination(shared: &super::LazyFileWriterShared) {
    crate::rt::drain_rt_logs_to(|record| {
        let mut line = Vec::new();
        let _ = writeln!(
            line,
            "[{} {} {}] [rt] seq={} {}",
            super::get_timestamp(),
            record.level(),
            record.target(),
            record.sequence(),
            record.message(),
        );
        let _ = super::write_log_bytes_blocking(&mut shared.destination.lock().unwrap(), &line);
    });
}
