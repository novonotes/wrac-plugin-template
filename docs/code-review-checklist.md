# WRAC Template Code Review Checklist

> 日本語版: [code-review-checklist-ja.md](code-review-checklist-ja.md)

Use this checklist for code review of products built from this template. It
only lists risks that reviewers can easily miss and that the compiler and CI do
not reliably prove.

## Audio Thread Realtime Safety

**Review:** Whether code reachable from the audio processor satisfies realtime
requirements and does not access project/editor state, GUI notifications, file
I/O, or other non-realtime services.

Also check logging in realtime paths. Code reachable from the audio callback,
parameter/event application performed by that callback, host process callbacks,
or realtime query methods must not use regular `log::*` macros. When
realtime-path logging is unavoidable, use realtime-safe `wrac_log::rtwarn!` /
`wrac_log::rtdebug!` instead.

**Why:** Allocation guards such as assert_no_alloc catch only part of the
problem: memory allocation. They do not catch issues such as blocking locks.

## Saved State Compatibility

**Review:** Whether changes to released `SavedState` schemas are covered by
automated migration tests for older DAW projects and presets.

**Why:** Human review alone is not reliable enough for saved-state
compatibility. Current save/load tests can prove the latest schema round-trips,
but they do not automatically prove that older saved states still recall as
intended after a schema change.

## Synchronous Plugin ABI Waiting

**Review:** Lightweight parameter and port queries use synchronous snapshots without
blocking. Lifecycle and state callbacks that bridge asynchronous work choose the wait
strategy from the caller thread, never wait on an audio thread, and leave no pending
work after destruction returns.

**Why:** Wrapper formats may invoke control callbacks from either the run-loop thread
or a background thread. Unconditionally waiting for a run-loop hop can deadlock, while
returning from destruction before cleanup completes can leave code running after unload.
