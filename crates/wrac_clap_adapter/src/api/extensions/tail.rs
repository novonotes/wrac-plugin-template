/// CLAP tail extension.
pub trait PluginTailExtension: Send + Sync + 'static {
    /// Called from CLAP `tail.get`. `[thread-safe]`
    ///
    /// CLAP may call this from the audio thread, so implementations must be realtime-safe.
    fn tail_frames(&self) -> u32;
}
