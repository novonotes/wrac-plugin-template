//! WRAC Gain plugin —— このテンプレートの読み始めの crate。
//!
//! 最小構成の gain (音量) plugin です。`src-plugin` には製品固有のロジック
//! (parameter / state / DSP / GUI) だけを置き、CLAP ABI や FFI の面倒な不変条件は
//! 別 crate `wrac_clap_adapter` に閉じ込めてあります。プラグインを作るときは
//! 基本的にこの crate の各ファイルを書き換えていきます。
//!
//! ファイル構成:
//! - `plugin.rs`   : host から見える plugin の契約。parameter / state / `PluginCore`。
//! - `state.rs`    : audio / GUI / host で共有する lock-free な state。
//! - `audio.rs`    : audio thread 上で動く DSP (このサンプルでは gain を掛けるだけ)。
//! - `gui.rs`      : WebView ベースの GUI runtime。
//! - `commands.rs` : WebView frontend から呼べる Rust command。
//!
//! ログは `log` facade 経由。`logging.rs` は debug build 用の簡易 logger で、
//! 製品では独自 logger に差し替える前提です。

// debug build では allocator を差し替え、audio thread での allocation を
// 即座に検出する (使い方は audio.rs の process() を参照)。
#[cfg(debug_assertions)]
use assert_no_alloc::*;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC_DISABLER: AllocDisabler = AllocDisabler;

mod audio;
mod commands;
mod gui;
mod logging;
mod plugin;
mod state;

// CLAP entry point を export する。C ABI / factory / lifecycle は adapter が生成するので、
// ここでは「どんな plugin か」(descriptor) と「core の作り方」(create) を渡すだけ。
wrac_clap_adapter::export_clap_plugin! {
    descriptor: crate::plugin::PLUGIN_DESCRIPTOR,
    create: crate::plugin::create_plugin_core,
}
