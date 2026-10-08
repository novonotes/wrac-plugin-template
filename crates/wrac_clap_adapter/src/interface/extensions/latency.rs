/// CLAP latency extension.
pub trait PluginLatencyExtension: Send + Sync + 'static {
    /// Called from CLAP `latency.get`. `[non-realtime & thread-safe]`
    fn latency_frames(&self) -> u32;
}
