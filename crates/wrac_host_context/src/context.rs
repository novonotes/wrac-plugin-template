//! Per-instance host context: the detected process host plus identity reported later.

use std::sync::{Arc, PoisonError, RwLock};

use super::identity::{host_from_wrapper_if_needed, resolve_au_host};
use super::{DetectedHost, PluginFormat, SystemContext, detect_host};

/// Host identity, process details, and wrapper format inferred from runtime context.
///
/// The host family is intentionally diagnostic/mechanical context, not policy. Callers
/// decide which workaround or compatibility decision to apply from this value so product
/// behavior does not accumulate inside the detection table.
///
/// [`HostContext::host`] is the only way to read the host identity. AU hosts may identify
/// themselves after instance creation, so the identity is resolved on each read instead of
/// being frozen in a field. Clones share that late identity: a product that keeps a clone
/// from instance creation still observes the host that the wrapper reported later.
#[derive(Clone, Debug)]
pub struct HostContext {
    process_host: DetectedHost,
    au_host_identifier: Arc<RwLock<Option<String>>>,
    pub plugin_format: PluginFormat,
    pub system: SystemContext,
}

/// Write access to the AU host identifier shared by one [`HostContext`] and its clones.
///
/// Returned separately from the context so that only the code that receives host metadata
/// from the wrapper can change the identity; product code receives read-only contexts.
#[derive(Debug)]
pub struct AuHostIdentifierWriter {
    identifier: Arc<RwLock<Option<String>>>,
}

impl AuHostIdentifierWriter {
    /// Records the identifier from the AU host, replacing any earlier one.
    ///
    /// `[non-realtime & thread-safe]`
    pub fn set(&self, identifier: Option<String>) {
        // The slot holds a plain value with no invariant to break, so a writer that
        // panicked while holding the lock leaves nothing to repair.
        *self
            .identifier
            .write()
            .unwrap_or_else(PoisonError::into_inner) = identifier;
    }
}

impl HostContext {
    /// Detects the context of the current process for one plugin instance.
    ///
    /// Create one context per instance: the returned writer updates every clone of it.
    pub fn detect_current(clap_host_name: Option<&str>) -> (Self, AuHostIdentifierWriter) {
        let plugin_format = PluginFormat::detect(clap_host_name.unwrap_or_default());
        let host = host_from_wrapper_if_needed(detect_host(), plugin_format, clap_host_name);
        Self::new(host, plugin_format, SystemContext::detect())
    }

    /// Builds a context from an already detected process host.
    pub fn new(
        process_host: DetectedHost,
        plugin_format: PluginFormat,
        system: SystemContext,
    ) -> (Self, AuHostIdentifierWriter) {
        let au_host_identifier = Arc::new(RwLock::new(None));
        (
            Self {
                process_host,
                au_host_identifier: au_host_identifier.clone(),
                plugin_format,
                system,
            },
            AuHostIdentifierWriter {
                identifier: au_host_identifier,
            },
        )
    }

    /// Returns the current host, including an identity the AU host reported after creation.
    ///
    /// Validation and scanner processes keep their process identity even when an
    /// application hosts them, because product policy depends on the actual process role.
    /// Non-AU contexts return without locking.
    ///
    /// `[non-realtime & thread-safe]`
    pub fn host(&self) -> DetectedHost {
        if self.plugin_format != PluginFormat::Au || self.process_host.is_validation_or_scan_host()
        {
            return self.process_host.clone();
        }
        let identifier = self
            .au_host_identifier
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        resolve_au_host(&self.process_host, identifier.as_deref())
    }

    /// Keeps validation / scanner process classification as the WRAC-owned source of truth.
    ///
    /// Product code should consume this result from the `PluginInstanceContext::host_context`
    /// passed during plugin instance creation instead of duplicating process-name checks.
    pub fn is_validation_or_scan_host(&self) -> bool {
        self.process_host.is_validation_or_scan_host()
    }
}
