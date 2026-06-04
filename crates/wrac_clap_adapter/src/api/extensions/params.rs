use crate::{ParamInfo, ParamValueEvent, PluginResult};

/// CLAP params extension.
///
/// Native CLAP marks parameter queries `[main-thread]`, but wrappers may query them
/// from `[control-thread]`. Query methods are not audio-thread APIs.
pub trait PluginParamsExtension: Send + Sync + 'static {
    /// Called from CLAP `params.count`. `[thread-safe & control-thread]`
    fn param_count(&self) -> u32;

    /// Called from CLAP `params.get_info`. `[thread-safe & control-thread]`
    fn param_info(&self, index: u32) -> Option<ParamInfo>;

    /// Called from CLAP `params.get_value`. `[thread-safe & control-thread]`
    fn param_value(&self, param_id: u32) -> PluginResult<f64>;

    /// Called from CLAP `params.flush` input parameter events.
    /// `[control-thread,audio-thread]`
    ///
    /// CLAP may call `params.flush` on the audio thread while active, but not
    /// concurrently with `plugin.process`. Parameter events delivered to
    /// `plugin.process` are handled by `Processor::process`, not this method.
    fn apply_param_value(&self, event: ParamValueEvent) -> PluginResult<f64>;

    /// Called from CLAP `params.value_to_text`. `[thread-safe & control-thread]`
    fn value_to_text(&self, param_id: u32, value: f64) -> PluginResult<String>;

    /// Called from CLAP `params.text_to_value`. `[thread-safe & control-thread]`
    fn text_to_value(&self, param_id: u32, text: &str) -> PluginResult<f64>;
}
