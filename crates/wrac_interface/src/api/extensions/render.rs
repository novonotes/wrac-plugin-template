use crate::{PluginRenderMode, PluginResult};

/// CLAP render extension.
pub trait PluginRenderExtension: Send + Sync + 'static {
    /// Called from CLAP `render.has_hard_realtime_requirement`.
    /// `[thread-safe]`
    fn has_hard_realtime_requirement(&self) -> bool {
        false
    }

    /// Called from CLAP `render.set`. `[thread-safe & control-thread]`
    fn set_render_mode(&self, mode: PluginRenderMode) -> PluginResult<()>;
}
