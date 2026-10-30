# AU state recall regression

> 日本語版: [README_JA.md](README_JA.md)

Checks parameter edits around AUv2 state recall, including failed loads and
concurrent rendering. On macOS, build an AUv2 plugin with the current wrapper;
it must expose deterministic CLAP state and a writable parameter, and accept
stereo float32 audio. Compile and run the test against the bundle:

```sh
clang++ -std=c++17 -Wall -Wextra -Werror \
  -framework AudioToolbox -framework CoreFoundation \
  clap_wrapper_builder/tests/au_state_recall.cpp -o /tmp/au-state-recall
/tmp/au-state-recall '/path/to/Your Plugin.component'
```

Run with a timeout: a regression can deadlock the test. The concurrent recall
check reports SKIP if the plugin does not notify ClassInfo during load.

# AU latency restart regression

Checks the AUv2 wrapper's response to a latency change and restart request while
rendering. On macOS, build the template's WRAC Gain AUv2 plugin, then compile and
run the test against its bundle:

```sh
clang++ -std=c++17 -Wall -Wextra -Werror \
  -framework AudioToolbox -framework CoreFoundation \
  clap_wrapper_builder/tests/au_latency_restart.cpp -o /tmp/au-latency-restart
/tmp/au-latency-restart '/path/to/WRAC Gain.component'
```

The silence-buffer check reports SKIP if no silent block is observed during the test.

# VST3 latency cache regression

On Windows, build the template's WRAC Gain VST3 with the current wrapper. This
host checks inactive latency queries before the first activation, after activation
without an active query, during an offline-mode switch, and after latency changes.
It also checks that an inactive query is followed by a latency-change notification.

```powershell
cmake -S clap_wrapper_builder/tests/vst3_latency_cache -B target/vst3-latency-cache-test
cmake --build target/vst3-latency-cache-test --config Release --target vst3_latency_cache
& 'target/vst3-latency-cache-test/Release/vst3_latency_cache.exe' 'path/to/WRAC Gain.vst3'
```
