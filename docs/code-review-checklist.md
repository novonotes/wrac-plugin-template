# WRAC Template Code Review Checklist

> 日本語版: [code-review-checklist-ja.md](code-review-checklist-ja.md)

Use this checklist for code review of products built from this template. It
only lists template-specific risks that reviewers can easily miss and that the
compiler, CI, and `cargo xtask validate` do not reliably prove.

## Realtime Store Boundaries

- **Review:** Whether the audio processor can accidentally reach project/editor
  state stores, GUI notifiers, host GUI/state handles, logging setup, or other
  non-realtime services.
  **Why:** The template intentionally separates realtime parameter state from
  project/editor state. Allocation guards catch only part of the realtime risk;
  they do not catch blocking locks, host callbacks, or non-realtime service
  access from the audio thread.

## Saved State Compatibility

- **Review:** Whether changes to released `SavedState` schemas are covered by
  migration or compatibility tests for older DAW projects and presets.
  **Why:** Human review alone is not reliable enough for serialized state
  compatibility. Current save/load tests can prove the latest schema
  round-trips, but they do not automatically prove that older serialized states
  still recall as intended after a schema change.
