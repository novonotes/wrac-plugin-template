# Main Thread Hook 方針メモ

## 背景

CoreDevice / CompositeDevice 系の製品では、プラグインインスタンス生成時に
「ホストまたは wrapper が main thread として扱う thread」を製品コード側へ知らせたい。

この hook は、JUCE が `ScopedJuceInitialiser_GUI` / `ScopedRunLoop` を置いている
プラグイン API 呼び出しコンテキストと揃える必要がある。単に CLAP factory
`create_plugin()` の中で製品側 hook を呼ぶだけだと、VST3 などで host が後段の
`initialize()` を main thread 以外から呼ぶ場合に、製品コードが期待する thread とずれる。

一方で、`clap-wrapper` や `wrac_clap_adapter` が `novonotes_run_loop::RunLoop` を直接知ると、
WRAC Gain のように RunLoop を必要としない製品まで Linux headless CI の GTK 初期化などに
巻き込まれる。

そのため、共通層の責務は「main thread lifecycle を通知する」までに限定し、
`RunLoop::init()` などの具体的な処理は製品コードへ寄せる。

## 基本方針

- 現在の PR ブランチは捨て、`origin/main` から小さく積み直す前提にする。
- `clap-wrapper` は `novonotes_run_loop` を知らない。
- `wrac_clap_adapter` も `novonotes_run_loop` を直接参照しない。
- `wrac_clap_adapter` は generic な CLAP factory extension として main-thread attach/detach を公開する。
- `clap-wrapper` は、その extension を JUCE と同じ wrapper API 呼び出し位置で呼ぶ。
- `clap-wrapper` の既存 CLAP plugin instance lifecycle は変更しない。
- native CLAP は wrapper を通らないため、`wrac_clap_adapter` の CLAP factory `create_plugin()` で同等の attach/detach を行う。
- `attach_main_thread()` / `detach_main_thread()` は通知専用 hook とし、plugin instance 生成の成否を直接返さない。
- main-thread 準備の失敗で plugin instance 生成を拒否するかどうかは製品側 factory が判断する。
- `detach_main_thread()` は、対応する attach が呼ばれた plugin object lifecycle の終端で呼ぶ。
- WRAC Gain は default no-op のままにし、既存の RunLoop 初期化方針を維持する。
- XDevice / CompositeDevice 系は製品 entry 側で `RunLoop::init()` と guard 保持を実装する。

## API 案

`PluginEntry` に最小限の hook だけを追加する。

```rust
pub trait PluginEntry: Sync {
    fn init(&self, context: EntryContext<'_>) -> PluginResult<()> {
        let _ = context;
        Ok(())
    }

    fn deinit(&self) {}

    fn plugin_factory(&self) -> Option<&dyn PluginFactory>;

    /// Host/wrapper が plugin main thread として扱う thread を製品コードへ通知する。
    ///
    /// この hook 自体は plugin instance 生成の成否を返さない。
    /// main-thread 準備に失敗した製品は entry / factory 側に状態を記録し、
    /// 後続の `PluginFactory::create_plugin()` で instance 生成を拒否する。
    fn attach_main_thread(&self) {}

    /// 対応する `attach_main_thread()` と対になる解除処理。
    ///
    /// teardown 中の回復策がないため、失敗は返さず製品側で必要に応じてログする。
    fn detach_main_thread(&self) {}
}
```

`attach_main_thread()` に `HostContext` や format 情報は渡さない。現時点で必要なのは
「この thread が plugin main thread である」という capability だけであり、format 別分岐は
`PluginFactory::create_plugin(..., PluginCoreContext)` 側で扱える。API 面積を増やすのは、
製品側で attach 時点の format 分岐が必要になった時だけにする。

## CLAP Extension 案

`clap-wrapper` からは Rust trait を直接呼べないため、`wrac_clap_adapter` が CLAP factory
extension として公開する。名前は RunLoop 固有にしない。

```c
#define WRAC_PLUGIN_MAIN_THREAD_HOOK "com.novonotes.wrac.plugin-main-thread-hook/0"

typedef struct wrac_plugin_main_thread_hook {
    void (*attach_main_thread)(const struct wrac_plugin_main_thread_hook *hook);
    void (*detach_main_thread)(const struct wrac_plugin_main_thread_hook *hook);
} wrac_plugin_main_thread_hook_t;
```

adapter 側はこの extension を `PluginEntry::attach_main_thread()` /
`PluginEntry::detach_main_thread()` へ forwarding する。adapter は attach の成功/失敗状態を持たない。
wrapper / adapter は、plugin object lifecycle ごとに attach と detach が一対になるよう呼び出す。

## JUCE 調査結果

JUCE / clap-juce-extensions が main-thread hook を置いている plugin API は以下。
ここでの「API 入口」は JUCE 内部の class/function 名ではなく、host または plugin SDK から見える
format API の method / callback 名を指す。

| Format | attach に対応する plugin API | detach に対応する plugin API | 根拠 |
|---|---|---|---|
| CLAP | `clap_plugin_factory.create_plugin(factory, host, plugin_id)` | `clap_plugin.destroy(plugin)` | `clap_create_plugin()` は product plugin 生成直前に `ScopedJuceInitialiser_GUI` を作る。`ClapJuceWrapper` は同じ initializer を member として保持する。 |
| VST3 | `IPluginFactory3::createInstance(FIDString cid, FIDString sourceIid, void** obj)` | `FUnknown::release()` による component/controller object の破棄 | `createInstance()` 内で `ScopedRunLoop` を作り、component/controller object を生成する。`IComponent::terminate()` は `releaseResources()` だけで object 破棄ではない。 |
| AUv2 | `AudioComponentPlugInInterface.Open` | `AudioComponentPlugInInterface.Close` | AUSDK factory は `Open = ComponentBase::AP_Open` / `Close = ComponentBase::AP_Close` を返す。`Initialize()` / `Cleanup()` は resource lifecycle。 |
| AAX | `kAAX_ProcPtrID_Create_EffectParameters` の `AAXCreateObjectProc` | effect parameters object の最終破棄 | `GetEffectDescriptions()` で `AAXCreateObjectProc` を登録し、その create proc が effect parameters object を作る。`Uninitialize()` は resource cleanup で object 破棄ではない。 |

## clap-wrapper 推奨呼び出し位置

`clap-wrapper` は `novonotes_run_loop` を知らず、WRAC adapter が公開する generic extension だけを呼ぶ。
呼び出し位置は JUCE の plugin object lifecycle に揃える。

| Format | `attach_main_thread()` を呼ぶ場所 | `detach_main_thread()` を呼ぶ場所 | 避ける場所 |
|---|---|---|---|
| native CLAP | adapter の `clap_plugin_factory.create_plugin` で product plugin core を作る直前 | adapter の `clap_plugin.destroy` で product plugin core 破棄後 | `clap_plugin.init()` / `deinit()` |
| VST3 | `ClapAsVst3::createInstance()` で wrapper object を作った直後に attach する。CLAP plugin instance creation は既存通り `initialize()` に残す。 | `FUnknown::release()` で到達する `ClapAsVst3` destructor。既存通り `terminate()` で CLAP plugin instance が破棄済みの場合も、ここで detach だけ行う。 | `IComponent::initialize()` / `IComponent::terminate()` |
| AUv2 | `AudioComponentPlugInInterface.Open` 経由の `WrapAsAUV2` construction、CLAP plugin instance を作る直前 | `AudioComponentPlugInInterface.Close` 経由の `WrapAsAUV2` destructor、CLAP plugin instance 破棄後 | `AudioUnitInitialize` / `AudioUnitUninitialize` / `Initialize()` / `Cleanup()` |
| AAX | `AAXCreateObjectProc` 経由の `ClapAsAAX` construction では empty shell に対して attach だけを済ませる。実際の CLAP plugin creation は `EffectInit()` 内なので、その前に attach 済みであることを要求する。 | effect parameters object の最終破棄で到達する `ClapAsAAX` destructor、CLAP plugin instance 破棄後 | `AAX_IACFEffectParameters::Initialize()` / `Uninitialize()` |

重要な補足は以下だけ残す。

- VST3 は `terminate()` 後に同じ object で `initialize()` されることがあるため、`terminate()` で CLAP plugin instance を destroy/detach しない。
- AAX は `Uninitialize()` が呼ばれても effect parameters object がまだ生きるため、detach は destructor 側に置く。
- native CLAP は `clap-wrapper` を通らないため、adapter 自身が同じ extension forwarding を `create_plugin` / `destroy` に適用する。
- VST3 / AAX とも、CLAP plugin instance creation の位置は既存実装から動かさない。JUCE に揃えるのは main-thread attach/detach の API context だけ。

## エラーハンドリング戦略

- `attach_main_thread()` / `detach_main_thread()` は失敗を返さない。
- `RunLoop::init()` など main-thread 準備が失敗した場合、製品側 entry / factory が状態を記録する。
- native CLAP / validator / xtask では、その状態を見て `PluginFactory::create_plugin()` が `None` を返す。
- VST3 / AUv2 / AAX では、既存の CLAP plugin creation failure 経路に従う。
- `detach_main_thread()` 中の回復不能な失敗は、製品側でログする。

## 差分を小さくする方針

- `clap-wrapper` に入れるのは generic extension lookup と wrapper lifecycle での attach/detach だけにする。
- extension 名・型名から `RunLoop` を消し、外部プロジェクト側の差分を WRAC 固有 runtime から切り離す。
- `wrac_clap_adapter` 側は `PluginEntry` hook と extension forwarding に限定する。
- `wrac_clap_adapter` / `clap-wrapper` に `novonotes_run_loop` 依存を持ち込まない。
- 製品側のみが `RunLoop::init()` と guard 管理を行う。
- CLAP plugin instance creation / termination / recreation の既存挙動は変えない。特に VST3 の `initialize()` / `terminate()`、AAX の `EffectInit()` には lifecycle 変更を入れない。
