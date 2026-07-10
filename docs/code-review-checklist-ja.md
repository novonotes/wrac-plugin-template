# WRAC Template コードレビュー・チェックリスト

> English version: [code-review-checklist.md](code-review-checklist.md)

このチェックリストは、このテンプレートから作られた製品のコードレビューで使います。コンパイラ、CI では確実に証明できず、レビュワーが見落としやすいリスクだけを載せています。

## オーディオスレッドのリアルタイム安全性

**確認すること:** audio processor から到達可能なコードがリアルタイム要件を満たすように書かれているか。project/editor state、GUI 通知、File IO、その他の非リアルタイムサービスへアクセスしていないか。

realtime 経路のログも確認する。audio callback、その callback 内で行われる
parameter / event 適用、host process callback から同期的に呼ばれる処理、
realtime query method から到達可能なコードでは、通常の `log::*` macro を使わない。
realtime 経路でログが必要な場合は、realtime-safe な `wrac_log::rtwarn!` /
`wrac_log::rtdebug!` を使う。

**理由:** assert_no_alloc のような allocation guard が検出できる問題はメモリアロケーションに関する一部だけです。blocking lock などの問題は検出できません。

## 保存状態の後方互換性

**確認すること:** リリース済みの `SavedState` schema を変更する場合に、古い DAW project や preset に対する、マイグレーションの自動テストが書かれているか。

**理由:** 保存状態の互換性は、人間のレビューだけでは信頼性が足りません。現在の save/load test は最新 schema の round-trip を証明できますが、schema 変更後に古い保存状態が意図通り recall されることまでは自動的に証明しません。

## 同期 Plugin ABI の待機

**確認すること:** parameter / port などの軽量 query が同期 snapshot から応答し、待機しないこと。
lifecycle / state callback で非同期処理を待つ場合は caller thread に応じた方式を使い、audio
thread では待たず、destroy の復帰後に未完了処理を残さないこと。

**理由:** wrapper format は control callback を RunLoop thread と background thread のどちらからも
呼び得ます。無条件に RunLoop への移送を待つとデッドロックし、cleanup 前に destroy から復帰すると
unload 後まで処理が残る可能性があります。
