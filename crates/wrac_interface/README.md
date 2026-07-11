# wrac_interface

`wrac_interface` defines the product-facing contracts for implementing a WRAC plugin.

Product DSP, state, parameter, port, and GUI implementations depend on this crate. The
`wrac_clap_adapter` crate implements the CLAP ABI boundary and consumes these contracts.
