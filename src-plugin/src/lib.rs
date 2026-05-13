//! WRAC Gain Plugin。
//!
//! このテンプレートを使うときの読み始めの crate です。シンプルな gain
//! (音量) plugin の最小実装が入っており、`src-plugin` には製品固有のロジック
//! (parameter / state / DSP / GUI) だけが置かれます。
//!
//! 公開される CLAP ABI (C 言語側から見える plugin entry point) は
//! `wrac_clap_adapter` という別 crate が提供してくれます。FFI や CLAP wrapper
//! まわりの面倒な不変条件はそちら側に閉じ込めてあるので、ここでは Rust 側で
//! 安全に書けるコードに集中できます。
//!
//! ファイル構成:
//! - `plugin.rs`   : plugin の中心。parameter / state save-restore / `PluginCore` 実装。
//! - `state.rs`    : audio / GUI / host で共有する lock-free な state。
//! - `audio.rs`  : audio thread 上で動く DSP (gain を掛けるだけ)。
//! - `gui.rs`    : WebView ベースの GUI runtime (HTML/JS で UI を作る)。
//! - `commands.rs` : WebView frontend から呼べる Rust command。

mod audio;
mod commands;
mod gui;
mod plugin;
mod state;

// CLAP entry point (`clap_entry`) を export する macro。
// adapter 側が C ABI / factory / lifecycle をすべて生成してくれるので、ここでは
// 「どんな plugin か」を表す descriptor と、core を生成する関数を渡すだけで済む。
wrac_clap_adapter::export_clap_plugin! {
    descriptor: crate::plugin::PLUGIN_DESCRIPTOR,
    create: crate::plugin::create_plugin_core,
}
