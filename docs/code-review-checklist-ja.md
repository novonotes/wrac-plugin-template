# WRAC Template コードレビュー・チェックリスト

> English version: [code-review-checklist.md](code-review-checklist.md)

このチェックリストは、このテンプレートから作られた製品のコードレビューで使います。コンパイラ、CI では確実に証明できず、レビュワーが見落としやすいリスクだけを載せています。

## オーディオスレッドのリアルタイム安全性

**確認すること:** audio processor から到達可能なコードがリアルタイム要件を満たすように書かれているか。project/editor state、GUI 通知、File IO、その他の非リアルタイムサービスへアクセスしていないか。

**理由:** assert_no_alloc のような allocation guard が検出できる問題はメモリアロケーションに関する一部だけです。blocking lock などの問題は検出できません。

## 保存状態の後方互換性

**確認すること:** リリース済みの `SavedState` schema を変更する場合に、古い DAW project や preset に対する、マイグレーションの自動テストが書かれているか。

**理由:** 保存状態の互換性は、人間のレビューだけでは信頼性が足りません。現在の save/load test は最新 schema の round-trip を証明できますが、schema 変更後に古い保存状態が意図通り recall されることまでは自動的に証明しません。
