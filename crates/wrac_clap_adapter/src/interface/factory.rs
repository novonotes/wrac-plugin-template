use crate::interface::{PluginDescriptor, PluginInstance, PluginInstanceContext};

/// Product factory behind the adapter's immutable ABI descriptor cache.
///
/// The adapter snapshots descriptor metadata during cache initialization.
pub trait PluginFactory: Send + Sync + 'static {
    /// `[non-realtime]`
    fn plugin_count(&self) -> u32;

    /// `[non-realtime]`
    fn plugin_descriptor(&self, index: u32) -> Option<PluginDescriptor>;

    /// `[non-realtime & thread-safe]`
    fn create_plugin(
        &self,
        plugin_id: &str,
        context: PluginInstanceContext,
    ) -> Option<Box<dyn PluginInstance>>;
}
