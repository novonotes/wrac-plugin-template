use parking_lot::RwLock;

use crate::interface::{DetectedHost, HostContext, HostIdentity};

pub(crate) struct HostIdentityProxy {
    initial: HostContext,
    identifier: RwLock<Option<String>>,
}

impl HostIdentityProxy {
    pub(crate) fn new(initial: HostContext) -> Self {
        Self {
            initial,
            identifier: RwLock::new(None),
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn set_au_identifier(&self, identifier: Option<String>) {
        // AU hosts can set or replace the identifier after plugin initialization.
        *self.identifier.write() = identifier;
    }
}

impl HostIdentity for HostIdentityProxy {
    fn current_host(&self) -> DetectedHost {
        self.initial
            .host_with_au_identifier(self.identifier.read().as_deref())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::interface::PluginFormat;
    use wrac_host_context::HostFamily;

    #[test]
    fn resolves_identifier_received_after_creation_without_notifications() {
        let mut context = HostContext::detect_current(None);
        context.plugin_format = PluginFormat::Au;
        context.host = wrac_host_context::detect_host_from_path("/unknown-host");
        let proxy = HostIdentityProxy::new(context);
        assert_eq!(proxy.current_host().family, HostFamily::Unknown);
        proxy.set_au_identifier(Some("com.apple.garageband".into()));
        assert_eq!(proxy.current_host().family, HostFamily::AppleGarageBand);
        proxy.set_au_identifier(Some("com.apple.logic.pro".into()));
        assert_eq!(proxy.current_host().family, HostFamily::AppleLogic);
        proxy.set_au_identifier(None);
        assert_eq!(proxy.current_host().family, HostFamily::Unknown);
    }
}
