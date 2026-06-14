use crate::{ParamInfo, PluginResult};

/// Host-queryable parameter metadata, values, and text conversion.
///
/// CLAP defines params as an optional extension, but WRAC treats this query surface
/// as basic adapter API because params are synchronized through `process` and
/// `flush_params`. Plugins without parameters return `0` from `count` and make param
/// flush no-op.
///
/// This trait has no setter: parameter changes flow as CLAP events through
/// `ProcessContext` and `ParamFlushContext`. Keep GUI workflows, smoothing,
/// automation policy, and product-domain abstractions out of this trait.
pub trait PluginParamsQuery: Send + Sync + 'static {
    /// Called from CLAP `params.count`. `[thread-safe]`
    fn count(&self) -> u32;

    /// Called from CLAP `params.get_info`. `[thread-safe]`
    fn get_info(&self, index: u32) -> Option<ParamInfo>;

    /// Called from CLAP `params.get_value`. `[thread-safe]`
    fn get_value(&self, param_id: u32) -> PluginResult<f64>;

    /// Called from CLAP `params.value_to_text`. `[thread-safe & control-thread]`
    fn value_to_text(&self, param_id: u32, value: f64) -> PluginResult<String>;

    /// Called from CLAP `params.text_to_value`. `[thread-safe & control-thread]`
    fn text_to_value(&self, param_id: u32, text: &str) -> PluginResult<f64>;
}
