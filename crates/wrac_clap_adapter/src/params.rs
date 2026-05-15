use std::collections::VecDeque;

use clap_sys::ext::params::{CLAP_EXT_PARAMS, CLAP_PARAM_RESCAN_VALUES, clap_host_params};
use clap_sys::host::clap_host;
use parking_lot::Mutex;

use crate::{
    HostParameterEditNotifier, InputEvents, OutputEvent, OutputEvents, ParameterGestureEvent,
    ParameterValueEvent, PluginParameters,
};

/// UI 由来の parameter edit を host が受け取れるまで保持する queue。
///
/// CLAP output queue は `flush()`/`process()` callback 中しか存在しない。GUI に
/// 直接 CLAP event を作らせると callback lifetime 越えの pointer を握ることになるので、
/// adapter は意味情報だけ保存し、出力 queue が来た時点で CLAP event へ変換する。
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
        parameters: &dyn PluginParameters,
        events: &InputEvents<'_>,
    ) {
        for event in events.parameter_values() {
            if let Err(error) = parameters.apply_parameter_value(event) {
                log::warn!(
                    "parameter_edits.apply_input: parameter apply failed parameter_id={} value={} error={error}",
                    event.parameter_id,
                    event.value
                );
            }
        }
    }

    pub(crate) fn drain_output_parameter_events(&self, events: &mut OutputEvents<'_>) {
        // audio callback 上で UI thread と待ち合わないため、queue が一瞬 busy なら次回
        // flush/process へ回す。host への request_flush は edit 追加時点で発行済み。
        let Some(mut pending) = self.pending.try_lock() else {
            log::debug!("parameter_edits.drain: pending queue try_lock failed; retrying later");
            return;
        };

        while let Some(event) = pending.pop_front() {
            if !push_parameter_edit(events, event) {
                // CLAP output queue は host 所有で、queue full や no-buffer flush では拒否され得る。
                // 送れなかった edit を捨てると automation gesture が欠けるため、順序を保って
                // 次回の flush/process に回す。
                pending.push_front(event);
                break;
            }
        }
    }

    fn push(&self, event: ParameterEditEvent) {
        self.pending.lock().push_back(event);
        // request_flush は queue 追加後に出す。host によってはこの通知がないと
        // `flush()` を呼ばないため、UI edit を automation lane へ返す機会を失う。
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

impl HostParameterEditNotifier for ParameterEditQueue {
    fn begin_edit(&self, parameter_id: u32) {
        self.push(ParameterEditEvent::Begin { parameter_id });
    }

    fn update_edit(&self, parameter_id: u32, value: f64) {
        self.push(ParameterEditEvent::Update {
            parameter_id,
            value,
        });
    }

    fn end_edit(&self, parameter_id: u32) {
        self.push(ParameterEditEvent::End { parameter_id });
    }
}

#[derive(Clone, Copy)]
enum ParameterEditEvent {
    Begin { parameter_id: u32 },
    Update { parameter_id: u32, value: f64 },
    End { parameter_id: u32 },
}

fn push_parameter_edit(events: &mut OutputEvents<'_>, event: ParameterEditEvent) -> bool {
    match event {
        ParameterEditEvent::Begin { parameter_id } => {
            events.try_push(OutputEvent::ParamGestureBegin(ParameterGestureEvent {
                time: 0,
                parameter_id,
            }))
        }
        ParameterEditEvent::Update {
            parameter_id,
            value,
        } => events.try_push(OutputEvent::ParamValue(ParameterValueEvent {
            time: 0,
            parameter_id,
            value,
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
        })),
        ParameterEditEvent::End { parameter_id } => {
            events.try_push(OutputEvent::ParamGestureEnd(ParameterGestureEvent {
                time: 0,
                parameter_id,
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

// host pointer の instance lifetime は CLAP ABI で避けられない最小前提です。wrapper
// 固有の thread/order は信じず、adapter 内では `request_flush()` だけに用途を限定する。
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
