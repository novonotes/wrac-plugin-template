use std::{collections::VecDeque, ffi::c_char};

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
        for event in events.parameter_values() {
            if let Err(error) = parameters.apply_param_value(event) {
                wrac_log::rtwarn!(
                    "parameter_edits.apply_input: parameter apply failed param_id={} value={} error={error}",
                    event.param_id,
                    event.value
                );
            }
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
        // VST3 hosts expect UI-originated automation gestures through component-handler
        // edit callbacks. The private wrapper extension lets adapters provide that path
        // without changing the plugin-facing parameter API or the native CLAP event path.
        if let Some(direct_edit) = self.host_params.and_then(|params| params.direct_edit)
            && direct_edit.try_push(event)
        {
            return;
        }

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
    direct_edit: Option<HostDirectParameterEdit>,
}

// The instance lifetime of the host pointer is the minimal unavoidable assumption of the
// CLAP ABI. These callbacks are used from non-audio UI/control paths; realtime automation
// continues to use CLAP input/output events in `process()`.
unsafe impl Send for HostParams {}
unsafe impl Sync for HostParams {}

#[derive(Clone, Copy)]
struct HostDirectParameterEdit {
    host: *const clap_host,
    begin_edit: unsafe extern "C" fn(host: *const clap_host, param_id: u32) -> bool,
    update_edit: unsafe extern "C" fn(host: *const clap_host, param_id: u32, value: f64) -> bool,
    end_edit: unsafe extern "C" fn(host: *const clap_host, param_id: u32) -> bool,
}

unsafe impl Send for HostDirectParameterEdit {}
unsafe impl Sync for HostDirectParameterEdit {}

impl HostDirectParameterEdit {
    fn try_push(self, event: ParameterEditEvent) -> bool {
        unsafe {
            match event {
                ParameterEditEvent::Begin { param_id } => (self.begin_edit)(self.host, param_id),
                ParameterEditEvent::Update { param_id, value } => {
                    (self.update_edit)(self.host, param_id, value)
                }
                ParameterEditEvent::End { param_id } => (self.end_edit)(self.host, param_id),
            }
        }
    }
}

#[repr(C)]
struct WracClapHostParameterEdit {
    begin_edit: Option<unsafe extern "C" fn(host: *const clap_host, param_id: u32) -> bool>,
    update_edit:
        Option<unsafe extern "C" fn(host: *const clap_host, param_id: u32, value: f64) -> bool>,
    end_edit: Option<unsafe extern "C" fn(host: *const clap_host, param_id: u32) -> bool>,
}

const WRAC_CLAP_EXT_HOST_PARAMETER_EDIT: *const c_char =
    c"com.novonotes.wrac.host-parameter-edit/1".as_ptr();

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
        let direct_edit = host_direct_parameter_edit(
            host,
            get_extension(host, WRAC_CLAP_EXT_HOST_PARAMETER_EDIT),
        );
        Some(HostParams {
            host,
            rescan: (*params).rescan,
            request_flush: (*params).request_flush,
            direct_edit,
        })
    }
}

unsafe fn host_direct_parameter_edit(
    host: *const clap_host,
    extension: *const std::ffi::c_void,
) -> Option<HostDirectParameterEdit> {
    if extension.is_null() {
        return None;
    }

    let direct = extension as *const WracClapHostParameterEdit;
    Some(HostDirectParameterEdit {
        host,
        begin_edit: unsafe { (*direct).begin_edit? },
        update_edit: unsafe { (*direct).update_edit? },
        end_edit: unsafe { (*direct).end_edit? },
    })
}
