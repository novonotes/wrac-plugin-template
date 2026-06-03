use crate::{GuiApi, GuiConfig, GuiResizeHints, GuiSize, HostWindow, PluginResult};

/// CLAP gui extension.
pub trait PluginGuiExtension: Send + Sync + 'static {
    fn is_api_supported(&self, api: GuiApi, is_floating: bool) -> bool;
    fn preferred_api(&self) -> Option<GuiConfig>;
    fn create(&self, configuration: GuiConfig) -> PluginResult<()>;
    fn destroy(&self);
    fn set_scale(&self, scale: f64) -> PluginResult<()>;
    fn get_size(&self) -> PluginResult<GuiSize>;
    fn can_resize(&self) -> bool;
    fn resize_hints(&self) -> Option<GuiResizeHints>;
    fn adjust_size(&self, size: GuiSize) -> PluginResult<GuiSize>;
    fn set_size(&self, size: GuiSize) -> PluginResult<()>;
    fn set_parent(&self, window: HostWindow) -> PluginResult<()>;
    fn set_transient(&self, window: HostWindow) -> PluginResult<()>;
    fn suggest_title(&self, title: &str);
    fn show(&self) -> PluginResult<()>;
    fn hide(&self) -> PluginResult<()>;
}
