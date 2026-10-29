/// CLAP tail extension.
pub trait PluginTailExtension: Send + Sync + 'static {
    /// Called from CLAP `tail.get`. `[realtime-safe & thread-safe]`
    fn tail_frames(&self) -> u32;
}
