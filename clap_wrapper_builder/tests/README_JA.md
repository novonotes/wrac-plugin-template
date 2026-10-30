# AU 状態復元の回帰テスト

> English version: [README.md](README.md)

AUv2 の状態復元前後のパラメーター編集を、ロード失敗時や並行レンダリング時も含めて確認します。
macOS で現在のラッパーを使って AUv2 プラグインをビルドしてください。プラグインは決定的な
CLAP 状態と書き込み可能なパラメーターを公開し、ステレオ float32 音声を受け付ける必要があります。
テストをコンパイルし、バンドルを指定して実行します。

```sh
clang++ -std=c++17 -Wall -Wextra -Werror \
  -framework AudioToolbox -framework CoreFoundation \
  clap_wrapper_builder/tests/au_state_recall.cpp -o /tmp/au-state-recall
/tmp/au-state-recall '/path/to/Your Plugin.component'
```

不具合によってデッドロックする可能性があるため、タイムアウトを設定して実行してください。
プラグインがロード中に ClassInfo を通知しない場合、並行復元の確認は SKIP と報告されます。

# AU レイテンシー変更時の再起動の回帰テスト

レンダリング中のレイテンシー変更と再起動要求に対する AUv2 ラッパーの動作を確認します。
macOS でテンプレートの WRAC Gain の AUv2 をビルドし、テストをコンパイルしてバンドルを指定します。

```sh
clang++ -std=c++17 -Wall -Wextra -Werror \
  -framework AudioToolbox -framework CoreFoundation \
  clap_wrapper_builder/tests/au_latency_restart.cpp -o /tmp/au-latency-restart
/tmp/au-latency-restart '/path/to/WRAC Gain.component'
```

テスト中に無音ブロックが観測されなかった場合、無音バッファーの確認は SKIP と報告されます。

# VST3 レイテンシーのキャッシュの回帰テスト

Windows で現在のラッパーを使ってテンプレートの WRAC Gain VST3 をビルドします。
初回有効化前、有効化中に問い合わせず停止した後、オフライン処理への切り替え時、
レイテンシー変更後の停止中の問い合わせを確認します。停止中の問い合わせに続いて
レイテンシー変更通知が送られることも確認します。

```powershell
cmake -S clap_wrapper_builder/tests/vst3_latency_cache -B target/vst3-latency-cache-test
cmake --build target/vst3-latency-cache-test --config Release --target vst3_latency_cache
& 'target/vst3-latency-cache-test/Release/vst3_latency_cache.exe' 'path/to/WRAC Gain.vst3'
```
