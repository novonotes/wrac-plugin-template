# src-plugin

## サブクレート

この src-plugin の実装は、`../crates/` にある、サブクレート群に依存しています。

- [wrac_clap_adapter](../crates/wrac_clap_adapter/README.md): PluginCore と extension の trait を定義し、その実装と CLAP の C ABI を接続します。
- [wrac_wxp_gui](../crates/wrac_wxp_gui/README.md): WXP を CLAP GUI として扱うための helper です。
