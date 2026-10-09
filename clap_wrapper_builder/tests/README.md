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

The test first saves and restores ClassInfo before AU Initialize, then verifies
that idle applies a host edit before the first render.
It then inspects serialized CLAP state after rendering, detecting changes hidden
by the AU parameter cache. It checks pre-recall edits, subsequent edits, and
preservation of pending edits after malformed AU dictionaries or rejected CLAP
payloads. For plugins that notify ClassInfo during load, it also renders on a
separate thread while load is paused in a property listener, queues a new edit,
and verifies that the edit survives subsequent recall completion. That check
reports SKIP when the plugin does not provide the notification. Use a runner
timeout: holding the render lock across load causes this check to deadlock.

Recall bookkeeping is internal to the AU owner. Loading runs without the
render/flush lock. Parameter events are deferred during loading, while MIDI
continues. Successful load supersedes pre-recall value events only; modulation
and later edits remain queued. Failed load preserves all pending edits. This
assumes state recall replaces saved parameter values; it does not make an
underlying plugin's failed, partially applied load transactional, or guarantee
uninterrupted audio processing in every plugin/host combination.
