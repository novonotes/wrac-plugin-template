//! Private CLAP host callback proxies implementing the product-facing `Host*` contracts.

mod audio_ports;
mod gui;
mod latency;
mod lifecycle;
mod note_ports;
mod params;
mod state;
mod tail;

pub(crate) use audio_ports::HostAudioPortsProxy;
pub(crate) use gui::HostGuiProxy;
pub(crate) use latency::HostLatencyProxy;
pub(crate) use lifecycle::HostLifecycleProxy;
pub(crate) use note_ports::HostNotePortsProxy;
pub(crate) use params::HostParamsProxy;
pub(crate) use state::HostStateProxy;
pub(crate) use tail::HostTailFactory;
