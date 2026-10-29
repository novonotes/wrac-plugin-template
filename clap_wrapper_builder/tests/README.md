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

The test inspects serialized CLAP state after rendering, detecting changes hidden
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

# AU latency restart regression

Checks the AUv2 wrapper's handling of `host.request_restart()` for a latency change,
using the template plugin (WRAC Gain), whose saved state carries its delay setting as
`latency_frames`. Build the AU, then compile and run:

```sh
clang++ -std=c++17 -Wall -Wextra -Werror \
  -framework AudioToolbox -framework CoreFoundation \
  clap_wrapper_builder/tests/au_latency_restart.cpp -o /tmp/au-latency-restart
/tmp/au-latency-restart '/path/to/WRAC Gain.component'
```

The test restores states that select a different delay, which makes the plugin request
a restart. Before the run loop runs, it checks that the audio already follows the new
delay while the old latency is still reported: the plugin applies the change itself
and only the report waits for the restart. While another thread keeps rendering, it
runs the main run loop so the
wrapper's idle timer can reactivate the plugin, then checks that the AU reports the new
`kAudioUnitProperty_Latency`, that latency listeners were notified, and that an impulse
comes out delayed by exactly the reported latency. The change is driven by state rather
than a parameter because the delay setting is not a host parameter.
