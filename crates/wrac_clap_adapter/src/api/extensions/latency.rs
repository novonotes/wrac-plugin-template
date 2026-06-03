/// CLAP latency extension.
pub trait PluginLatencyExtension: Send + Sync + 'static {
    fn latency_frames(&self) -> u32;
}
