# AU 状態復元の回帰テスト

> English version: [README.md](README.md)

macOS で現在のラッパーを使って AUv2 プラグインをビルドし、以下のネイティブホストをコンパイルして実行します。バンドルをインストールしたりエディターを開いたりせずに直接読み込みます。プラグインは決定的な CLAP 状態と書き込み可能なパラメーターを公開し、ステレオ float32 音声を受け付ける必要があります。

```sh
clang++ -std=c++17 -Wall -Wextra -Werror \
  -framework AudioToolbox -framework CoreFoundation \
  clap_wrapper_builder/tests/au_state_recall.cpp -o /tmp/au-state-recall
/tmp/au-state-recall '/path/to/Your Plugin.component'
```

テストはレンダリング後のシリアライズされた CLAP 状態を調べ、AU パラメーターキャッシュに隠れた変更を検出します。復元前と復元後の編集、および不正な AU 辞書や拒否された CLAP ペイロードの後に保留中の編集が保持されることを確認します。ロード中に ClassInfo を通知するプラグインでは、プロパティリスナーでロードを一時停止した間に別スレッドでレンダリングし、新しい編集をキューに入れ、復元完了後も保持されることを確認します。通知がない場合、この確認は SKIP と報告されます。ロード中に render lock を保持するとデッドロックするため、実行にはタイムアウトを設定してください。

復元の管理は AU owner 内部で行います。ロードは render/flush lock を持たずに実行します。ロード中のパラメーターイベントは延期され、MIDI は継続します。ロード成功時は復元前の値イベントだけが置き換えられ、モジュレーションと後続の編集はキューに残ります。ロード失敗時は保留中の編集をすべて保持します。これは状態復元が保存されたパラメーター値を置き換えることを前提としており、プラグイン内部で途中まで適用されたロード失敗をトランザクション化したり、すべてのプラグインとホストの組み合わせで音声処理が途切れないことを保証したりするものではありません。

# AU レイテンシー変更時の再起動の回帰テスト

保存状態に `latency_frames` としてディレイ設定を持つテンプレートプラグイン（WRAC Gain）を使い、レイテンシー変更時の `host.request_restart()` を AUv2 ラッパーが処理する方法を確認します。AU をビルドし、以下をコンパイルして実行します。

```sh
clang++ -std=c++17 -Wall -Wextra -Werror \
  -framework AudioToolbox -framework CoreFoundation \
  clap_wrapper_builder/tests/au_latency_restart.cpp -o /tmp/au-latency-restart
/tmp/au-latency-restart '/path/to/WRAC Gain.component'
```

テストは異なるディレイを選択する状態を復元し、プラグインに再起動を要求させます。run loop の実行前に、古いレイテンシーが報告されている間も音声には新しいディレイが適用されていることを確認します。プラグイン自体が変更を適用し、報告だけが再起動を待つためです。別スレッドでレンダリングを続けながらメインの run loop を実行し、ラッパーの idle timer にプラグインを再アクティブ化させます。その後、AU が新しい `kAudioUnitProperty_Latency` を報告し、レイテンシーリスナーが通知され、インパルスが報告されたレイテンシーだけ遅延して出力されることを確認します。ディレイ設定はホストパラメーターではないため、パラメーターではなく状態によって変更します。

並行レンダリング中には、無音とマークされたすべてのブロックの両チャンネルのサンプルがゼロであることも確認します。無音ブロックが観測されなかった場合は SKIP と報告します。このテストはすべての再起動とレンダリングが重なることを保証しません。
