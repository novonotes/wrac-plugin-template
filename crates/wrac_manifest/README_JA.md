# wrac_manifest

> English: [README.md](README.md)

`wrac_manifest` は WRAC plugin manifest (`wrac-plugin.toml`) を typed Rust
metadata として parse します。WRAC の build script と xtask command が使う標準 manifest
schema の一次定義を持つ crate です。

## `wrac-plugin.toml`

WRAC plugin package は、repository が所有する `wrac-plugin.toml` で記述します。
この manifest には host-visible な製品 metadata と validation exception を書きます。

### 標準 field

- `schema_version`: WRAC manifest schema version。
- `[package]`: package-level metadata override。`version_source = "cargo"` は
  `Cargo.toml` を bundle / descriptor version の source として使います。
- `[bundle]`: bundle 内のすべての plugin product で共有する metadata。company name、
  bundle identifier、URL、description、copyright、`supported_formats` などを含みます。
- `[[plugins]]`: bundle から公開する host-visible な plugin product。各 entry は ID、name、
  CLAP feature、wrapper descriptor、必要に応じて AAX metadata を定義します。
- `[validation]`: production-readiness rule の無効化や external validator の skip filter
  など、validation exception を定義します。

### リポジトリ固有の拡張

WRAC は `wrac-plugin.toml` の未知 field / table を無視します。downstream repository は、
namespaced table に独自 metadata を置き、repository-local automation から parse できます。

```toml
[acme.ci]
validation_profile = "prototype"
```

WRAC は extension table の意味を解釈しません。repository automation は repository-specific
policy を明示的な WRAC command-line option に変換してください。
