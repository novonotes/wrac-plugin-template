use wrac_clap_adapter::PluginLatencyExtension;

pub(super) struct WracGainLatencyExtension;

impl PluginLatencyExtension for WracGainLatencyExtension {
    fn latency_frames(&self) -> u32 {
        // WRAC Gain applies sample-local gain only, so reporting zero keeps wrapper
        // latency queries explicit without inventing delay compensation.
        0
    }
}
