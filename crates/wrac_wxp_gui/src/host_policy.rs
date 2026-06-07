use wrac_host_context::{HostContext, HostFamily, PluginFormat};

use crate::dpi::HostGuiSizeUnit;

#[derive(Debug, Clone)]
pub(crate) struct HostGuiPolicy {
    context: HostContext,
}

impl HostGuiPolicy {
    pub(crate) fn new(context: HostContext) -> Self {
        Self { context }
    }

    pub(crate) fn should_async_resync_bounds_after_set_size(&self) -> bool {
        self.context.host.family == HostFamily::SteinbergCubase
            && self.context.plugin_format == PluginFormat::Vst3
    }

    pub(crate) fn needs_cubase_vst3_scale_correction(&self) -> bool {
        self.context.host.family == HostFamily::SteinbergCubase
            && self.context.plugin_format == PluginFormat::Vst3
    }

    pub(crate) fn host_size_unit(&self) -> HostGuiSizeUnit {
        // macOS wrapper formats expose Cocoa/NSView geometry at the CLAP GUI boundary.
        // Treating those logical coordinates as physical pixels would divide the child
        // WebView bounds by the scale factor and clip the editor to the top-left area.
        if cfg!(target_os = "macos")
            && matches!(
                self.context.plugin_format,
                PluginFormat::Vst3 | PluginFormat::Au | PluginFormat::Aax
            )
        {
            HostGuiSizeUnit::LogicalPoints
        } else {
            HostGuiSizeUnit::PhysicalPixels
        }
    }
}

#[cfg(test)]
mod tests {
    use wrac_host_context::{DetectedHost, HostContext, HostFamily, PluginFormat};

    use super::*;

    fn context(family: HostFamily, plugin_format: PluginFormat) -> HostContext {
        HostContext {
            host: DetectedHost {
                family,
                display_name: "Test Host".to_string(),
                process_name: "Test Host".to_string(),
                process_path: String::new(),
                version: None,
            },
            plugin_format,
        }
    }

    #[test]
    fn cubase_vst3_policy_is_scoped_to_cubase_vst3() {
        let cubase_vst3 =
            HostGuiPolicy::new(context(HostFamily::SteinbergCubase, PluginFormat::Vst3));
        assert!(cubase_vst3.should_async_resync_bounds_after_set_size());
        assert!(cubase_vst3.needs_cubase_vst3_scale_correction());

        let cubase_au = HostGuiPolicy::new(context(HostFamily::SteinbergCubase, PluginFormat::Au));
        assert!(!cubase_au.should_async_resync_bounds_after_set_size());
        assert!(!cubase_au.needs_cubase_vst3_scale_correction());
    }

    #[test]
    fn host_size_unit_uses_logical_points_for_macos_wrappers() {
        let policy = HostGuiPolicy::new(context(HostFamily::Unknown, PluginFormat::Au));
        assert_eq!(
            policy.host_size_unit(),
            if cfg!(target_os = "macos") {
                HostGuiSizeUnit::LogicalPoints
            } else {
                HostGuiSizeUnit::PhysicalPixels
            }
        );
    }
}
