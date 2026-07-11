use crate::interface::{PluginRenderMode, PluginResult};

/// CLAP render extension.
pub trait PluginRenderExtension: Send + Sync + 'static {
    /// Called from CLAP `render.has_hard_realtime_requirement`.
    /// `[realtime-safe & thread-safe]`
    fn has_hard_realtime_requirement(&self) -> bool {
        false
    }

    /// Called from CLAP `render.set`. `[thread-safe]`
    fn set_render_mode(&self, mode: PluginRenderMode) -> PluginResult<()>;
}
