# xtask_workflow

`xtask_workflow` provides dependency-ordered workflow primitives for
repository-local `cargo xtask` crates.

It intentionally does not know about WRAC, audio plugins, plugin formats, or
build commands. Product repositories define their own task enum, task labels,
task executor, and dependency graph while reusing this crate for stable
topological ordering, cycle detection, task IDs, task status, failure policy,
dry-run output, dependency-skip behavior, and result summaries.

The executor accepts caller-provided label and run closures. Those closures are
not stored in graph nodes, so task semantics remain visible in each repository's
local task enum.
