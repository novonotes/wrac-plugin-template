use crate::interface::{PluginDescriptor, PluginInstance, PluginInstanceContext};

/// Product factory behind the adapter's immutable ABI descriptor cache.
///
/// The adapter snapshots descriptor metadata during serialized cache initialization, while product
/// instances may be created concurrently by independent plugin initialization callbacks.
pub trait PluginFactory: Send + Sync + 'static {
    /// `[non-realtime]`
    fn plugin_count(&self) -> u32;

    /// `[non-realtime]`
    fn plugin_descriptor(&self, index: u32) -> Option<PluginDescriptor>;

    /// Independent plugin instances may initialize concurrently, so shared factory state must be
    /// synchronized.
    /// `[non-realtime & thread-safe]`
    fn create_plugin(
        &self,
        plugin_id: &str,
        context: PluginInstanceContext,
    ) -> Option<Box<dyn PluginInstance>>;
}
