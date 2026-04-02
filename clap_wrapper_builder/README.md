# clap_wrapper_builder

`clap_wrapper_builder` は、`wxp-gain-example` の参照実装から
CLAP プラグインを VST3 / AUv2 / Standalone にラップするための補助ビルド環境です。

これは正式な安定 API ではありません。ルートの gain plugin などの
サンプル実装を支えるための例示用コードとして扱い、破壊的変更が入る可能性があります。

## 含まれるもの

- `build_wrapper_plugin.sh` - CLAP バンドルから VST3 / AUv2 ラッパーをビルド
- `build_wrapper_plugin_static.sh` - 静的ライブラリから VST3 / AUv2 / Standalone をビルド
- `install_wrapper_plugin.sh` - 生成済み VST3 をインストール
- `clap-wrapper` / `clap` / `vst3sdk` / `AudioUnitSDK` - 依存 SDK / ツールチェーン

## 想定用途

- `wxp-gain-example` の参照プラグインを複数フォーマットで配布する
- `xdevice-private` など別プロジェクトから参照実装として利用する

長期安定な公開インターフェースとしては扱わず、必要に応じて構成やスクリプトを見直します。
