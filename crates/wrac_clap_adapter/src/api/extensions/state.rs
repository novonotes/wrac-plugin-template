use crate::{PluginResult, State};

/// CLAP state extension.
pub trait PluginStateExtension: Send + Sync + 'static {
    fn save_state(&self) -> PluginResult<State>;
    fn restore_state(&self, state: State) -> PluginResult<()>;
}
