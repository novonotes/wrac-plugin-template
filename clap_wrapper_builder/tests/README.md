# AU state recall regression

On macOS, build an AUv2 plugin with the current wrapper, then compile and run the
native host below. It loads the bundle directly, without installing it or opening
an editor. The plugin must expose deterministic CLAP state and a writable
parameter, and accept stereo float32 audio.

```sh
clang++ -std=c++17 -Wall -Wextra -Werror \
  -framework AudioToolbox -framework CoreFoundation \
  clap_wrapper_builder/tests/au_state_recall.cpp -o /tmp/au-state-recall
/tmp/au-state-recall '/path/to/Your Plugin.component'
```

The test queues a host parameter before state recall and inspects the serialized
CLAP payload after the first render. Reading only `AudioUnitGetParameter` would
miss the regression: the AU cache can contain the restored value while the
pending event changes the plugin's internal state. It also verifies that edits
made after successful recall remain effective and that failed recall reports an
error without discarding pending host parameters.
