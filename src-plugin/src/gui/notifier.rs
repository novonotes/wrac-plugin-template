use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use novonotes_run_loop::{RunLoop, RunLoopSender};
use parking_lot::Mutex;
use serde_json::json;
use wxp::Channel;

use crate::plugin::parameter_value_text;
use crate::state::EditorPage;

/// WebView 側へ GUI state を push する通知口。通知タイミングは呼び出し元が決める。
pub(crate) struct GuiStateNotifier {
    next_subscription_id: AtomicU64,
    subscriptions: Mutex<HashMap<GuiSubscriptionId, GuiSubscription>>,
}

/// WebView 側 subscriber 1 つぶんの登録情報。
///
/// `kind` (何の stream か) と `channel` (送り先) を分けて持つことで、parameter /
/// meter / analyzer などを個別に購読・解除でき、古い cleanup が別の購読を
/// 巻き込んで消す事故も防げる。
#[derive(Clone)]
struct GuiSubscription {
    kind: GuiSubscriptionKind,
    // 通知を UI thread に戻すための run loop sender。
    sender: RunLoopSender,
    // WebView 側 JS の subscriber に値を送る channel。
    channel: Channel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GuiSubscriptionId(u64);

impl GuiSubscriptionId {
    pub(crate) fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

/// 購読の種類。meter や analyzer の stream を足すときは variant を追加し、
/// `notify_*` でその variant の subscription にだけ配信する
/// (Channel を増やさず種別で振り分ける設計)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiSubscriptionKind {
    Parameters,
    EditorPage,
}

impl GuiStateNotifier {
    pub(super) fn new() -> Self {
        Self {
            next_subscription_id: AtomicU64::new(1),
            subscriptions: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn subscribe_parameters(&self, channel: Channel) -> GuiSubscriptionId {
        self.subscribe(GuiSubscriptionKind::Parameters, channel)
    }

    pub(crate) fn subscribe_editor_page(&self, channel: Channel) -> GuiSubscriptionId {
        self.subscribe(GuiSubscriptionKind::EditorPage, channel)
    }

    fn subscribe(&self, kind: GuiSubscriptionKind, channel: Channel) -> GuiSubscriptionId {
        // id は wxp の Channel id とは独立に採番。transport と購読 lifecycle を
        // 別々に管理するため。
        let id = GuiSubscriptionId(self.next_subscription_id.fetch_add(1, Ordering::Relaxed));
        self.subscriptions.lock().insert(
            id,
            GuiSubscription {
                kind,
                sender: RunLoop::sender(),
                channel,
            },
        );
        id
    }

    pub(crate) fn unsubscribe(&self, id: GuiSubscriptionId) {
        self.subscriptions.lock().remove(&id);
    }

    pub(crate) fn clear_subscriptions(&self) {
        self.subscriptions.lock().clear();
    }

    pub(crate) fn notify_parameter(&self, parameter_id: u32, value: f32) {
        self.notify(
            GuiSubscriptionKind::Parameters,
            parameter_payload(parameter_id, value),
        );
    }

    pub(crate) fn notify_editor_page(&self, editor_page: EditorPage) {
        self.notify(
            GuiSubscriptionKind::EditorPage,
            editor_page_payload(editor_page),
        );
    }

    fn notify(&self, kind: GuiSubscriptionKind, payload: serde_json::Value) {
        // 送り先が notifier を再入しても deadlock しないよう、配信対象を
        // clone してから lock を離す。
        let subscriptions: Vec<_> = self
            .subscriptions
            .lock()
            .values()
            .filter(|subscription| subscription.kind == kind)
            .cloned()
            .collect();
        if subscriptions.is_empty() {
            // GUI が開いていなければ送り先がないので何もしない。
            return;
        }

        for subscription in subscriptions {
            let payload = payload.clone();
            // WebView channel は GUI runtime と同じ UI thread でしか触れない。
            // host/audio thread から直接送ると thread affinity を破るので、
            // 必ず run loop に戻してから channel に渡す。
            subscription.sender.send(move || {
                let _ = subscription.channel.send(payload);
            });
        }
    }
}

/// WebView へ送る JSON payload。TypeScript 側はこの形を期待する。
/// 新しい parameter でも payload の形は変えず `parameterId` で振り分ける。
pub(crate) fn parameter_payload(parameter_id: u32, value: f32) -> serde_json::Value {
    json!({
        "type": "parameter-value",
        "parameterId": parameter_id,
        "value": value,
        "text": parameter_value_text(parameter_id, value as f64).unwrap_or_else(|_| value.to_string()),
    })
}

pub(crate) fn editor_page_payload(editor_page: EditorPage) -> serde_json::Value {
    json!({
        "type": "editor-page",
        "page": editor_page.as_str(),
    })
}
