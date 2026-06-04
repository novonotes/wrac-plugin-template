/// CLAP tail extension.
pub trait PluginTailExtension: Send + Sync + 'static {
    /// Called from CLAP `tail.get`. `[control-thread,audio-thread]`
    fn tail_frames(&self) -> u32;
}
