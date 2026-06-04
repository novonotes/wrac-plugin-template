use crate::{PluginResult, State};

/// CLAP state extension.
pub trait PluginStateExtension: Send + Sync + 'static {
    /// Called from CLAP `state.save`. `[thread-safe & control-thread]`
    fn save_state(&self) -> PluginResult<State>;

    /// Called from CLAP `state.load`. `[thread-safe & control-thread]`
    fn restore_state(&self, state: State) -> PluginResult<()>;
}
