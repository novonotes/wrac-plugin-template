# WRAC Gain GUI

Vite/TypeScript WebView frontend for the WRAC Gain example plugin.

Debug plugin builds load this package from the Vite dev server. Release builds
use the `dist` output packaged by `wrac_build` from the plugin crate's
`build.rs`.

This package owns the product UI and its command and event schemas: DOM, styling,
parameter presentation, and contracts exposed by the plugin. All communication
with Rust, including product command invocation and notification channels, goes
through `@novonotes/wrac-frontend-runtime`. The runtime also owns shared
DAW-hosted WebView behavior such as log forwarding, host focus restoration,
native cursor bridging, and resize handling, but it does not own product schemas.

## Commands

```sh
npm install
npm run dev
npm run build
```
