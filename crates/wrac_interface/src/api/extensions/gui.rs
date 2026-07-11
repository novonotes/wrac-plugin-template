use crate::{GuiApi, GuiConfig, GuiResizeHints, GuiSize, HostWindow, PluginResult};

/// Query surface for CLAP GUI API support.
pub trait PluginGuiApiSupportExtension: Send + Sync + 'static {
    /// Called from CLAP `gui.is_api_supported`.
    ///
    /// `[realtime-safe & thread-safe]`
    fn is_api_supported(&self, api: GuiApi, is_floating: bool) -> bool;
}

/// CLAP GUI query surface.
pub trait PluginGuiQueryExtension: Send + Sync + 'static {
    /// Called from CLAP `gui.get_preferred_api`.
    ///
    /// `[thread-safe]`
    fn preferred_api(&self) -> Option<GuiConfig>;

    /// Called from CLAP `gui.get_size`.
    ///
    /// `[thread-safe]`
    fn get_size(&self) -> PluginResult<GuiSize>;

    /// Called from CLAP `gui.can_resize`.
    ///
    /// `[thread-safe]`
    fn can_resize(&self) -> bool;

    /// Called from CLAP `gui.get_resize_hints`.
    ///
    /// `[thread-safe]`
    fn resize_hints(&self) -> Option<GuiResizeHints>;

    /// Called from CLAP `gui.adjust_size`.
    ///
    /// `[thread-safe]`
    fn adjust_size(&self, size: GuiSize) -> PluginResult<GuiSize>;
}

/// CLAP GUI lifecycle / native UI surface.
pub trait PluginGuiMainThreadExtension: 'static {
    /// Called from CLAP `gui.create`.
    ///
    /// `[main-thread]`
    fn create(&self, configuration: GuiConfig) -> PluginResult<()>;

    /// Called from CLAP `gui.destroy` or plugin destruction.
    ///
    /// `[main-thread]`
    fn destroy(&self);

    /// Called from CLAP `gui.set_scale`.
    ///
    /// `[main-thread]`
    fn set_scale(&self, scale: f64) -> PluginResult<()>;

    /// Called from CLAP `gui.set_size`.
    ///
    /// `[main-thread]`
    fn set_size(&self, size: GuiSize) -> PluginResult<()>;

    /// Called from CLAP `gui.set_parent`.
    ///
    /// `[main-thread]`
    fn set_parent(&self, window: HostWindow) -> PluginResult<()>;

    /// Called from CLAP `gui.show`.
    ///
    /// `[main-thread]`
    fn show(&self) -> PluginResult<()>;

    /// Called from CLAP `gui.hide`.
    ///
    /// `[main-thread]`
    fn hide(&self) -> PluginResult<()>;
}

/// CLAP GUI extension exposed to the adapter.
///
/// Query methods and main-thread lifecycle methods are split so product code can
/// implement thread-safe host queries separately from native UI operations.
pub trait PluginGuiExtension: Send + Sync + 'static {
    /// `[default]`
    fn api_support(&self) -> &(dyn PluginGuiApiSupportExtension + Send + Sync);

    /// `[default]`
    fn query(&self) -> &(dyn PluginGuiQueryExtension + Send + Sync);

    /// `[default]`
    fn main_thread(&self) -> &dyn PluginGuiMainThreadExtension;
}
