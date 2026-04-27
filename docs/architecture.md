# Architecture Overview

> 日本語版: [architecture_JA.md](architecture_JA.md)

## Thread Model

A CLAP plugin operates on two primary threads.

```
┌─────────────────────────────────────────────────────────────────┐
│ Main thread (= RunLoop thread)                                  │
│  - GUI creation, destruction, and resize (gui.rs)               │
│  - Exposing parameter information (params.rs)                   │
│  - Command processing via WxpCommandHandler                     │
│  - State save and restore                                        │
│  - wxp WebView event processing                                  │
│  - Receiving tasks from other threads via RunLoopSender          │
│  - Channel::send() for Rust → JS notifications runs here        │
├─────────────────────────────────────────────────────────────────┤
│ Audio thread (real-time)                                        │
│  - Applies gain to samples in process() (audio.rs)              │
│  - Locking, memory allocation, and I/O are forbidden            │
└─────────────────────────────────────────────────────────────────┘
```

> **Note:** Because RunLoop is initialized on the main thread,
> the RunLoop thread and the main thread are the same in this plugin.
> `RunLoopSender` is used to post closures from other threads (such as
> the audio thread) to the main thread.

## Rust ↔ JavaScript Communication

```
JavaScript (main.ts)                    Rust (plugin.rs)
──────────────────                      ────────────────
invoke("set_gain", {value})  ──────►   WxpCommandHandler
                                        └─ register_sync("set_gain", ...)

Channel callback            ◄──────    RunLoopSender → Channel::send()
  └─ render(state)                      └─ notify_gui()
```

- **JS → Rust**: `invoke()` makes an RPC call to a command registered on the Rust side.
- **Rust → JS**: Push notifications via `Channel`. When the host changes a value through automation or similar, the change is dispatched to the main thread via `RunLoopSender`, and then sent to JS as JSON through `Channel::send()`.

## Parameter Change Flow

**UI → Host:**

```
1. User starts dragging a knob
2. JS: invoke("begin_parameter_gesture")
3. JS: invoke("set_gain", {value})          ← repeated while dragging
4. Rust: Updates AtomicF32 in SharedStateInner + sets the pending flag
5. Audio thread: process() reads the pending flag and notifies the host via output events
6. User finishes dragging
7. JS: invoke("end_parameter_gesture")
```

**Host → UI:**

```
1. Host changes a value via automation or similar
2. Rust: Receives a ParamValue from input events in process()
3. Rust: Updates AtomicF32 in SharedStateInner
4. Rust: notify_gui() → RunLoopSender → Channel::send()
5. JS: Channel callback invokes render(), updating the UI
```

## wxp Initialization Flow (CLAP Context)

The WebView is created inside the `set_parent()` callback (see `gui.rs` for the implementation).

1. `WebContext::new(data_dir)` — sets the user data directory. The `WebContext` must be kept alive for the lifetime of the WebView, so it is stored on `self`.
2. `wxp_clack::window::clack_to_wry_window_handle(&parent)` — converts the CLAP `Window` to a wry `WindowHandle`.
3. `WxpWebViewBuilder::new(&mut web_context)` — creates a builder and configures the command handler, URL, bounds, etc.
4. `.build_as_child(&parent_handle)` — obtains a `WebViewRef` and stores it on `self`.

In `destroy()`, `reset_webview()` clears the GUI notification channel, drops the `WebViewRef` to destroy the WebView, then drops the `WebContext`.

## Key Dependencies

| Crate | Role |
|-------|------|
| `clack-plugin` / `clack-extensions` | Rust bindings for the CLAP plugin API |
| `wxp` | WebView GUI framework (WxpWebViewBuilder, WxpCommandHandler, Channel) |
| `wxp_clack` | Utilities bridging wxp and CLAP (DPI conversion, window handle conversion) |
| `novonotes_run_loop` | Platform-abstracted event loop (RunLoop, RunLoopSender) |
| `wry` | WebView engine (used internally by wxp) |
| `@novonotes/webview-bridge` | JS-side communication library (invoke, Channel) |
