# WRAC Template Code Review Checklist

> 日本語版: [code-review-checklist-ja.md](code-review-checklist-ja.md)

Use this checklist for code review of products built from this template. It
only lists risks that reviewers can easily miss and that the compiler and CI do
not reliably prove.

## Audio Thread Realtime Safety

**Review:** Whether code reachable from the audio processor satisfies realtime
requirements and does not access project/editor state, GUI notifications, file
I/O, or other non-realtime services.

**Why:** Allocation guards such as assert_no_alloc catch only part of the
problem: memory allocation. They do not catch issues such as blocking locks.

## Saved State Compatibility

**Review:** Whether changes to released `SavedState` schemas are covered by
automated migration tests for older DAW projects and presets.

**Why:** Human review alone is not reliable enough for saved-state
compatibility. Current save/load tests can prove the latest schema round-trips,
but they do not automatically prove that older saved states still recall as
intended after a schema change.
