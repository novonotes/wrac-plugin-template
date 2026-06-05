# WRAC Template Code Review Checklist

> English version: [code-review-checklist.md](code-review-checklist.md)

このチェックリストは、このテンプレートから作られた製品のコードレビューで使います。コンパイラ、CI、`cargo xtask validate` では確実に証明できず、レビュワーが見落としやすい、このテンプレート固有のリスクだけを載せています。

## Realtime Store Boundaries

- **確認すること:** audio processor から到達可能なコードが、project/editor state store、GUI notifier、host GUI/state handle、logging setup、その他の non-realtime service に誤って到達できないか。
  **理由:** このテンプレートは、realtime parameter state と project/editor state を意図的に分離しています。allocation guard が検出できる realtime risk は一部だけです。audio thread からの blocking lock、host callback、non-realtime service access までは検出できません。

## Saved State Compatibility

- **確認すること:** リリース済みの `SavedState` schema を変更する場合に、古い DAW project や preset に対する migration test または compatibility test が書かれているか。
  **理由:** serialized state compatibility は、人間のレビューだけでは信頼性が足りません。現在の save/load test は最新 schema の round-trip を証明できますが、schema 変更後に古い serialized state が意図通り recall されることまでは自動的に証明しません。
