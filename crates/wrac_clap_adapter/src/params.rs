use std::collections::VecDeque;

use clap_sys::ext::params::{CLAP_EXT_PARAMS, CLAP_PARAM_RESCAN_VALUES, clap_host_params};
use clap_sys::host::clap_host;
use parking_lot::Mutex;

use crate::{
    HostParamsEditNotifier, InputEvents, OutputEvent, OutputEvents, ParamGestureEvent,
    ParamValueEvent, PluginParamsExtension,
};

/// Queue that holds UI-originated parameter edits until the host can receive them.
///
/// The CLAP output queue only exists during `flush()`/`process()` callbacks. Letting
/// the GUI construct CLAP events directly would mean holding pointers beyond the
/// callback lifetime, so the adapter stores only semantic information and converts to
/// CLAP events when the output queue becomes available.
pub(crate) struct ParameterEditQueue {
    pending: Mutex<VecDeque<ParameterEditEvent>>,
    host_params: Option<HostParams>,
}

impl ParameterEditQueue {
    pub(crate) fn new(host: *const clap_host) -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
            host_params: host_params(host),
        }
    }

    pub(crate) unsafe fn apply_input_parameter_events(
        &self,
        parameters: &dyn PluginParamsExtension,
        events: &InputEvents<'_>,
    ) {
        if let Err(error) = parameters.apply_param_events(events.param_events()) {
            wrac_log::rtwarn!("parameter_edits.apply_input: parameter apply failed: {error}");
        }
    }

    pub(crate) fn drain_output_parameter_events(&self, events: &mut OutputEvents<'_>) {
        // Avoid waiting on the UI thread from the audio callback. If the queue is
        // momentarily busy, defer to the next flush/process. request_flush to the host
        // was already issued when the edit was enqueued.
        let Some(mut pending) = self.pending.try_lock() else {
            wrac_log::rtdebug!(
                "parameter_edits.drain: pending queue try_lock failed; retrying later"
            );
            return;
        };

        while let Some(event) = pending.pop_front() {
            if !push_parameter_edit(events, event) {
                // The CLAP output queue is host-owned and may reject events when full
                // or during a no-buffer flush. Discarding an unsent edit would drop an
                // automation gesture, so preserve ordering and defer to the next
                // flush/process.
                pending.push_front(event);
                break;
            }
        }
    }

    fn push(&self, event: ParameterEditEvent) {
        self.pending.lock().push_back(event);
        // Issue request_flush after enqueuing. Some hosts will not call `flush()`
        // without this notification, causing UI edits to never reach the automation lane.
        self.request_flush();
    }

    fn request_flush(&self) {
        let Some(params) = self.host_params else {
            log::debug!("parameter_edits.request_flush: host params extension unavailable");
            return;
        };

        if let Some(request_flush) = params.request_flush {
            unsafe {
                request_flush(params.host);
            }
        } else {
            log::debug!("parameter_edits.request_flush: host request_flush callback unavailable");
        }
    }

    pub(crate) fn rescan_values(&self) {
        let Some(params) = self.host_params else {
            log::debug!("parameter_edits.rescan_values: host params extension unavailable");
            return;
        };

        if let Some(rescan) = params.rescan {
            unsafe {
                rescan(params.host, CLAP_PARAM_RESCAN_VALUES);
            }
        } else {
            log::debug!("parameter_edits.rescan_values: host rescan callback unavailable");
        }
    }
}

impl HostParamsEditNotifier for ParameterEditQueue {
    fn begin_edit(&self, param_id: u32) {
        self.push(ParameterEditEvent::Begin { param_id });
    }

    fn update_edit(&self, param_id: u32, value: f64) {
        self.push(ParameterEditEvent::Update { param_id, value });
    }

    fn end_edit(&self, param_id: u32) {
        self.push(ParameterEditEvent::End { param_id });
    }
}

#[derive(Clone, Copy)]
enum ParameterEditEvent {
    Begin { param_id: u32 },
    Update { param_id: u32, value: f64 },
    End { param_id: u32 },
}

fn push_parameter_edit(events: &mut OutputEvents<'_>, event: ParameterEditEvent) -> bool {
    match event {
        ParameterEditEvent::Begin { param_id } => {
            events.try_push(OutputEvent::ParamGestureBegin(ParamGestureEvent {
                time: 0,
                param_id,
            }))
        }
        ParameterEditEvent::Update { param_id, value } => {
            events.try_push(OutputEvent::ParamValue(ParamValueEvent {
                time: 0,
                param_id,
                value,
                note_id: -1,
                port_index: -1,
                channel: -1,
                key: -1,
            }))
        }
        ParameterEditEvent::End { param_id } => {
            events.try_push(OutputEvent::ParamGestureEnd(ParamGestureEvent {
                time: 0,
                param_id,
            }))
        }
    }
}

#[derive(Clone, Copy)]
struct HostParams {
    host: *const clap_host,
    rescan: Option<unsafe extern "C" fn(host: *const clap_host, flags: u32)>,
    request_flush: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

// The instance lifetime of the host pointer is the minimal unavoidable assumption of the
// CLAP ABI. Product-facing usage is limited to `request_flush()`; adapter-internal
// `rescan_values()` is called only after state load, where CLAP gives the callback a
// main-thread contract.
unsafe impl Send for HostParams {}
unsafe impl Sync for HostParams {}

fn host_params(host: *const clap_host) -> Option<HostParams> {
    if host.is_null() {
        return None;
    }

    unsafe {
        let get_extension = (*host).get_extension?;
        let params = get_extension(host, CLAP_EXT_PARAMS.as_ptr()) as *const clap_host_params;
        if params.is_null() {
            return None;
        }
        Some(HostParams {
            host,
            rescan: (*params).rescan,
            request_flush: (*params).request_flush,
        })
    }
}
