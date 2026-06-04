use crate::{GuiApi, GuiConfig, GuiResizeHints, GuiSize, HostWindow, PluginResult};

/// CLAP gui extension for embedded host-owned editor windows.
pub trait PluginGuiExtension: Send + Sync + 'static {
    /// Called from CLAP `gui.is_api_supported`. `[main-thread]`
    fn is_api_supported(&self, api: GuiApi, is_floating: bool) -> bool;

    /// Called from CLAP `gui.get_preferred_api`. `[main-thread]`
    fn preferred_api(&self) -> Option<GuiConfig>;

    /// Called from CLAP `gui.create`. `[main-thread]`
    fn create(&self, configuration: GuiConfig) -> PluginResult<()>;

    /// Called from CLAP `gui.destroy` or plugin destruction. `[main-thread]`
    fn destroy(&self);

    /// Called from CLAP `gui.set_scale`. `[main-thread]`
    fn set_scale(&self, scale: f64) -> PluginResult<()>;

    /// Called from CLAP `gui.get_size`. `[main-thread]`
    fn get_size(&self) -> PluginResult<GuiSize>;

    /// Called from CLAP `gui.can_resize`. `[main-thread]`
    fn can_resize(&self) -> bool;

    /// Called from CLAP `gui.get_resize_hints`. `[main-thread]`
    fn resize_hints(&self) -> Option<GuiResizeHints>;

    /// Called from CLAP `gui.adjust_size`. `[main-thread]`
    fn adjust_size(&self, size: GuiSize) -> PluginResult<GuiSize>;

    /// Called from CLAP `gui.set_size`. `[main-thread]`
    fn set_size(&self, size: GuiSize) -> PluginResult<()>;

    /// Called from CLAP `gui.set_parent`. `[main-thread]`
    fn set_parent(&self, window: HostWindow) -> PluginResult<()>;

    /// Called from CLAP `gui.show`. `[main-thread]`
    fn show(&self) -> PluginResult<()>;

    /// Called from CLAP `gui.hide`. `[main-thread]`
    fn hide(&self) -> PluginResult<()>;
}
