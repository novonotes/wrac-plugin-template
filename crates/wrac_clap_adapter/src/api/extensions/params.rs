use crate::{ParamInfo, ParamValueEvent, PluginResult};

/// CLAP params extension.
pub trait PluginParamsExtension: Send + Sync + 'static {
    fn param_count(&self) -> u32;
    fn param_info(&self, index: u32) -> Option<ParamInfo>;
    fn param_value(&self, param_id: u32) -> PluginResult<f64>;
    fn apply_param_value(&self, event: ParamValueEvent) -> PluginResult<f64>;
    fn value_to_text(&self, param_id: u32, value: f64) -> PluginResult<String>;
    fn text_to_value(&self, param_id: u32, text: &str) -> PluginResult<f64>;
}
