# WRAC Gain GUI

Vite/TypeScript WebView frontend for the WRAC Gain example plugin.

Debug plugin builds load this package from the Vite dev server. Release builds
use the `dist` output packaged by `wrac_build` from the plugin crate's
`build.rs`.

This package owns only the product UI: DOM, styling, parameter presentation, and
calls into Rust commands exposed by the plugin. Shared DAW-hosted WebView
behavior such as log forwarding, host focus restoration, native cursor bridging,
and resize handling lives in `@novonotes/wrac-frontend-runtime`.

## Commands

```sh
npm install
npm run dev
npm run build
```
