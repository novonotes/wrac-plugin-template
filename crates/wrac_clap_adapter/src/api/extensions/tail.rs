/// CLAP tail extension.
pub trait PluginTailExtension: Send + Sync + 'static {
    fn tail_frames(&self) -> u32;
}
