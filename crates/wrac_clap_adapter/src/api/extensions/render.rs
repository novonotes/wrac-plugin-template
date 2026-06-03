use crate::{PluginRenderMode, PluginResult};

/// CLAP render extension.
pub trait PluginRenderExtension: Send + Sync + 'static {
    fn has_hard_realtime_requirement(&self) -> bool {
        false
    }

    fn set_render_mode(&self, mode: PluginRenderMode) -> PluginResult<()>;
}
