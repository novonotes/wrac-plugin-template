/// CLAP latency extension.
pub trait PluginLatencyExtension: Send + Sync + 'static {
    /// Called from CLAP `latency.get`. `[thread-safe & control-thread]`
    fn latency_frames(&self) -> u32;
}
