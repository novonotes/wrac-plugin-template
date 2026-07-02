# xtask_workflow

`xtask_workflow` provides dependency-ordered workflow primitives for
repository-local `cargo xtask` crates.

It intentionally does not know about WRAC, audio plugins, plugin formats, or
build commands. Product repositories define their own task enum, task labels,
executor, and dependency graph while reusing this crate for stable topological
ordering, cycle detection, task IDs, task status, and failure policy types.
