use serde_json::json;
use wxp::WxpCommandHandler;

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};

/// Registers the native cursor bridge command used by WebView frontends.
///
/// The frontend remains CSS-driven: it sends a normalized `cursorIntent` derived from
/// `getComputedStyle(...).cursor`, and this bridge applies the closest native cursor
/// only on platforms that need it.
pub fn register_native_cursor_bridge_commands(command_handler: &WxpCommandHandler) {
    command_handler.register_sync("apply_native_cursor", move |ctx| {
        let cursor_intent = ctx
            .arg::<String>("cursorIntent")
            .map_err(|e| e.to_string())?;
        let reason = ctx.arg::<String>("reason").unwrap_or_default();
        let applied = apply_native_cursor(&cursor_intent, &reason);
        Ok::<_, String>(json!({ "ok": true, "applied": applied }))
    });
}

#[cfg(target_os = "macos")]
fn apply_native_cursor(cursor_intent: &str, reason: &str) -> bool {
    let intent = CursorIntent::from_wire(cursor_intent);
    let result = intent.apply();
    log::debug!(
        "native cursor bridge: reason={} cursor_intent={} native={} fidelity={} applied={}",
        reason,
        cursor_intent,
        result.native,
        result.fidelity,
        result.applied
    );
    result.applied
}

#[cfg(not(target_os = "macos"))]
fn apply_native_cursor(cursor_intent: &str, reason: &str) -> bool {
    log::debug!(
        "native cursor bridge: skipped unsupported platform reason={reason} cursor_intent={cursor_intent}"
    );
    false
}

#[cfg(target_os = "macos")]
enum CursorIntent {
    Alias,
    AllScroll,
    Arrow,
    Cell,
    ColumnResize,
    ContextMenu,
    Copy,
    Crosshair,
    EdgeResize(ResizeEdge),
    Grab,
    Grabbing,
    Help,
    IBeam,
    Move,
    NoDrop,
    None,
    NotAllowed,
    PointingHand,
    RowResize,
    VerticalIBeam,
    Wait,
    ZoomIn,
    ZoomOut,
    Unknown,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum ResizeEdge {
    E,
    W,
    N,
    S,
    Ne,
    Nw,
    Se,
    Sw,
    Ew,
    Ns,
    Nesw,
    Nwse,
}

#[cfg(target_os = "macos")]
struct CursorApplyResult {
    applied: bool,
    native: &'static str,
    fidelity: &'static str,
}

#[cfg(target_os = "macos")]
static CURSOR_HIDDEN_BY_BRIDGE: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
impl CursorIntent {
    fn from_wire(cursor_intent: &str) -> Self {
        match cursor_intent {
            "alias" => Self::Alias,
            "all-scroll" => Self::AllScroll,
            "arrow" => Self::Arrow,
            "cell" => Self::Cell,
            "col-resize" => Self::ColumnResize,
            "context-menu" => Self::ContextMenu,
            "copy" => Self::Copy,
            "crosshair" => Self::Crosshair,
            "e-resize" => Self::EdgeResize(ResizeEdge::E),
            "ew-resize" => Self::EdgeResize(ResizeEdge::Ew),
            "grab" => Self::Grab,
            "grabbing" => Self::Grabbing,
            "help" => Self::Help,
            "move" => Self::Move,
            "n-resize" => Self::EdgeResize(ResizeEdge::N),
            "ne-resize" => Self::EdgeResize(ResizeEdge::Ne),
            "nesw-resize" => Self::EdgeResize(ResizeEdge::Nesw),
            "no-drop" => Self::NoDrop,
            "none" => Self::None,
            "not-allowed" => Self::NotAllowed,
            "ns-resize" => Self::EdgeResize(ResizeEdge::Ns),
            "nw-resize" => Self::EdgeResize(ResizeEdge::Nw),
            "nwse-resize" => Self::EdgeResize(ResizeEdge::Nwse),
            "pointer" => Self::PointingHand,
            "progress" => Self::Wait,
            "row-resize" => Self::RowResize,
            "s-resize" => Self::EdgeResize(ResizeEdge::S),
            "se-resize" => Self::EdgeResize(ResizeEdge::Se),
            "sw-resize" => Self::EdgeResize(ResizeEdge::Sw),
            "text" => Self::IBeam,
            "vertical-text" => Self::VerticalIBeam,
            "w-resize" => Self::EdgeResize(ResizeEdge::W),
            "wait" => Self::Wait,
            "zoom-in" => Self::ZoomIn,
            "zoom-out" => Self::ZoomOut,
            _ => Self::Unknown,
        }
    }

    #[allow(deprecated)]
    fn apply(&self) -> CursorApplyResult {
        use objc2::sel;
        use objc2_app_kit::NSCursor;

        if !matches!(self, Self::None) {
            unhide_cursor_if_hidden_by_bridge();
        }

        match self {
            Self::Alias => set_cursor("drag-link", "good-semantic-fallback", || {
                NSCursor::dragLinkCursor().set();
            }),
            Self::AllScroll => unsupported_cursor("all-scroll"),
            Self::Arrow => set_cursor("arrow", "exact", || {
                NSCursor::arrowCursor().set();
            }),
            Self::Cell => unsupported_cursor("cell"),
            Self::ColumnResize => set_cursor("resize-left-right", "good-semantic-fallback", || {
                NSCursor::resizeLeftRightCursor().set();
            }),
            Self::ContextMenu => set_cursor("contextual-menu", "exact", || {
                NSCursor::contextualMenuCursor().set();
            }),
            Self::Copy => set_cursor("drag-copy", "good-semantic-fallback", || {
                NSCursor::dragCopyCursor().set();
            }),
            Self::Crosshair => set_cursor("crosshair", "exact", || {
                NSCursor::crosshairCursor().set();
            }),
            Self::EdgeResize(edge) => apply_edge_resize_cursor(*edge),
            Self::Grab => set_cursor("open-hand", "exact", || {
                NSCursor::openHandCursor().set();
            }),
            Self::Grabbing => set_cursor("closed-hand", "exact", || {
                NSCursor::closedHandCursor().set();
            }),
            Self::Help => unsupported_cursor("help"),
            Self::IBeam => set_cursor("ibeam", "exact", || {
                NSCursor::IBeamCursor().set();
            }),
            Self::Move => set_cursor("open-hand", "visual-fallback", || {
                NSCursor::openHandCursor().set();
            }),
            Self::NoDrop | Self::NotAllowed => {
                set_cursor("operation-not-allowed", "good-semantic-fallback", || {
                    NSCursor::operationNotAllowedCursor().set();
                })
            }
            Self::None => hide_cursor(),
            Self::PointingHand => set_cursor("pointing-hand", "exact", || {
                NSCursor::pointingHandCursor().set();
            }),
            Self::RowResize => set_cursor("resize-up-down", "good-semantic-fallback", || {
                NSCursor::resizeUpDownCursor().set();
            }),
            Self::VerticalIBeam => set_cursor("vertical-ibeam", "exact", || {
                NSCursor::IBeamCursorForVerticalLayout().set();
            }),
            Self::Wait => unsupported_cursor("wait"),
            Self::ZoomIn => apply_if_class_method_exists(sel!(zoomInCursor), "zoom-in", || {
                NSCursor::zoomInCursor().set();
            }),
            Self::ZoomOut => apply_if_class_method_exists(sel!(zoomOutCursor), "zoom-out", || {
                NSCursor::zoomOutCursor().set();
            }),
            Self::Unknown => unsupported_cursor("unknown"),
        }
    }
}

#[cfg(target_os = "macos")]
impl ResizeEdge {
    fn name(&self) -> &'static str {
        match self {
            Self::Ne => "ne-resize",
            Self::Nw => "nw-resize",
            Self::Se => "se-resize",
            Self::Sw => "sw-resize",
            Self::Nesw => "nesw-resize",
            Self::Nwse => "nwse-resize",
            Self::E => "e-resize",
            Self::W => "w-resize",
            Self::N => "n-resize",
            Self::S => "s-resize",
            Self::Ew => "ew-resize",
            Self::Ns => "ns-resize",
        }
    }
}

#[cfg(target_os = "macos")]
fn unsupported_cursor(_requested: &'static str) -> CursorApplyResult {
    objc2_app_kit::NSCursor::arrowCursor().set();
    CursorApplyResult {
        applied: true,
        native: "arrow",
        fidelity: "arrow-fallback",
    }
}

#[cfg(target_os = "macos")]
fn set_cursor(
    native: &'static str,
    fidelity: &'static str,
    apply: impl FnOnce(),
) -> CursorApplyResult {
    apply();
    CursorApplyResult {
        applied: true,
        native,
        fidelity,
    }
}

#[cfg(target_os = "macos")]
fn hide_cursor() -> CursorApplyResult {
    if !CURSOR_HIDDEN_BY_BRIDGE.swap(true, Ordering::Relaxed) {
        objc2_app_kit::NSCursor::hide();
    }
    CursorApplyResult {
        applied: true,
        native: "hidden",
        fidelity: "exact",
    }
}

#[cfg(target_os = "macos")]
fn unhide_cursor_if_hidden_by_bridge() {
    if CURSOR_HIDDEN_BY_BRIDGE.swap(false, Ordering::Relaxed) {
        objc2_app_kit::NSCursor::unhide();
    }
}

#[cfg(target_os = "macos")]
fn class_method_exists(selector: objc2::runtime::Sel) -> bool {
    objc2::runtime::AnyClass::get(c"NSCursor")
        .and_then(|class| class.class_method(selector))
        .is_some()
}

#[cfg(target_os = "macos")]
fn apply_if_class_method_exists(
    selector: objc2::runtime::Sel,
    native: &'static str,
    apply: impl FnOnce(),
) -> CursorApplyResult {
    if !class_method_exists(selector) {
        return unsupported_cursor(native);
    }
    apply();
    CursorApplyResult {
        applied: true,
        native,
        fidelity: "exact",
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn apply_edge_resize_cursor(edge: ResizeEdge) -> CursorApplyResult {
    use objc2::sel;
    use objc2_app_kit::{NSCursor, NSCursorFrameResizeDirections, NSCursorFrameResizePosition};

    // The frame-resize cursor API is newer than the template's macOS 11 floor.
    // Check selector availability before calling it so older systems fall back safely.
    if class_method_exists(sel!(frameResizeCursorFromPosition:inDirections:)) {
        let position = match edge {
            ResizeEdge::E => NSCursorFrameResizePosition::Right,
            ResizeEdge::W => NSCursorFrameResizePosition::Left,
            ResizeEdge::N => NSCursorFrameResizePosition::Top,
            ResizeEdge::S => NSCursorFrameResizePosition::Bottom,
            ResizeEdge::Ne => NSCursorFrameResizePosition::TopRight,
            ResizeEdge::Nw => NSCursorFrameResizePosition::TopLeft,
            ResizeEdge::Se => NSCursorFrameResizePosition::BottomRight,
            ResizeEdge::Sw => NSCursorFrameResizePosition::BottomLeft,
            ResizeEdge::Ew => NSCursorFrameResizePosition::Right,
            ResizeEdge::Ns => NSCursorFrameResizePosition::Bottom,
            ResizeEdge::Nesw => NSCursorFrameResizePosition::TopRight,
            ResizeEdge::Nwse => NSCursorFrameResizePosition::BottomRight,
        };
        NSCursor::frameResizeCursorFromPosition_inDirections(
            position,
            NSCursorFrameResizeDirections::All,
        )
        .set();
        return CursorApplyResult {
            applied: true,
            native: "frame-resize",
            fidelity: "exact",
        };
    }

    match edge {
        ResizeEdge::E => set_cursor("resize-right", "good-semantic-fallback", || {
            NSCursor::resizeRightCursor().set();
        }),
        ResizeEdge::W => set_cursor("resize-left", "good-semantic-fallback", || {
            NSCursor::resizeLeftCursor().set();
        }),
        ResizeEdge::N => set_cursor("resize-up", "good-semantic-fallback", || {
            NSCursor::resizeUpCursor().set();
        }),
        ResizeEdge::S => set_cursor("resize-down", "good-semantic-fallback", || {
            NSCursor::resizeDownCursor().set();
        }),
        ResizeEdge::Ew => set_cursor("resize-left-right", "good-semantic-fallback", || {
            NSCursor::resizeLeftRightCursor().set();
        }),
        ResizeEdge::Ns => set_cursor("resize-up-down", "good-semantic-fallback", || {
            NSCursor::resizeUpDownCursor().set();
        }),
        ResizeEdge::Ne
        | ResizeEdge::Nw
        | ResizeEdge::Se
        | ResizeEdge::Sw
        | ResizeEdge::Nesw
        | ResizeEdge::Nwse => unsupported_cursor(edge.name()),
    }
}
