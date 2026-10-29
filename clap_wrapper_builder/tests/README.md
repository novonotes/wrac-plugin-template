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
