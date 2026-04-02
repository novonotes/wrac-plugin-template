//! WXP Example Gain Plugin
//!
//! wxp (WebView eXtension Platform) を使った CLAP オーディオプラグインのサンプル。
//! 入力信号にゲイン（音量倍率）を掛けるだけのシンプルなエフェクトプラグイン。
//!
//! ## モジュール構成
//! - `plugin` : プラグインの定義、共有状態、コマンドハンドラの登録
//! - `audio`  : リアルタイムオーディオ処理（オーディオスレッドで動作）
//! - `params` : CLAP パラメータの公開・ホストとのパラメータ同期
//! - `gui`    : wxp WebView による GUI の生成・リサイズ管理

mod audio;
mod gui;
mod params;
mod plugin;

use std::ffi::CStr;

use clack_plugin::{clack_export_entry, entry::prelude::*};
use plugin::WxpExampleGainPluginFactory;

/// CLAP プラグインのエントリーポイント。
/// ホスト（DAW）がプラグインの共有ライブラリをロードすると、まずこの型が生成される。
/// Entry はプラグインのライフサイクル全体を管理する最上位の構造体。
pub struct WxpExampleGainEntry {
    /// PluginFactoryWrapper は clack が提供するラッパーで、
    /// 自前の PluginFactoryImpl をホストに公開するために使う。
    plugin_factory: PluginFactoryWrapper<WxpExampleGainPluginFactory>,
}

impl Entry for WxpExampleGainEntry {
    /// ホストがプラグインをロードした直後に一度だけ呼ばれる。
    /// _bundle_path にはプラグインファイル（.clap）のパスが渡される。
    fn new(_bundle_path: &CStr) -> Result<Self, EntryLoadError> {
        // RunLoop の初期化。メインスレッド上で RunLoop を起動する。
        // wxp の WebView やコマンドハンドラはメインスレッド（= RunLoop）上で
        // 動作するため、プラグインの最初期に init() を呼ぶ必要がある。
        // init/deinit は参照カウント方式なので、複数回呼んでも安全。
        novonotes_run_loop::RunLoop::init().map_err(|_| EntryLoadError)?;

        Ok(Self {
            plugin_factory: PluginFactoryWrapper::new(WxpExampleGainPluginFactory::new()),
        })
    }

    /// ホストに対してこのプラグインが提供するファクトリを登録する。
    /// 1つの Entry が複数のプラグインファクトリを公開することも可能。
    fn declare_factories<'a>(&'a self, builder: &mut EntryFactories<'a>) {
        builder.register_factory(&self.plugin_factory);
    }
}

impl Drop for WxpExampleGainEntry {
    fn drop(&mut self) {
        // init() と対になる deinit()。Entry が破棄される＝プラグインがアンロードされる。
        novonotes_run_loop::RunLoop::deinit();
    }
}

// CLAP ホストがプラグインを検出するためのエントリーポイントシンボルをエクスポートする。
// このマクロにより `clap_entry` というグローバルシンボルが生成される。
clack_export_entry!(WxpExampleGainEntry);

/// 一部のホストは `clap_entry` シンボルを直接検出できないため、
/// 関数として明示的にエントリーディスクリプタを返すフォールバック。
#[unsafe(no_mangle)]
pub extern "C" fn get_clap_entry() -> EntryDescriptor {
    clap_entry
}
