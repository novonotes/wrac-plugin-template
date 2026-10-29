# WRAC Gain GUI

> English version: [README.md](README.md)

WRAC Gain サンプルプラグインの Vite/TypeScript WebView フロントエンドです。

プラグインのデバッグビルドは、このパッケージを Vite 開発サーバーから読み込みます。リリースビルドはプラグイン crate の `build.rs` から `wrac_build` がパッケージ化する `dist` 出力を使用します。

このパッケージは製品 UI とそのコマンド・イベントのスキーマを担当します。DOM、スタイル、パラメーターの表示、プラグインが公開する契約が含まれます。製品コマンドの呼び出しや通知チャネルを含む Rust との通信は、すべて `@novonotes/wrac-frontend-runtime` を経由します。runtime はログ転送、ホストへのフォーカス復帰、ネイティブカーソルの橋渡し、リサイズ処理など、DAW 上の WebView に共通する動作も担当します。製品のスキーマは担当しません。

## コマンド

```sh
npm install
npm run dev
npm run build
```
