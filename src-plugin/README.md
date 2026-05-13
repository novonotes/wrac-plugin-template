# src-plugin

`src-plugin` は WRAC Gain 固有の plugin crate です。

この crate には gain parameter、state serialization、audio processor、WebView の画面内容と command handler だけを置きます。CLAP C ABI、host callback、raw window handle 変換、run loop 上の GUI runtime 保持は下位 crate の責務です。

## 依存境界

- `wrac_clap_adapter`: CLAP entry point と extension callback の adapter
- `wrac_wxp_gui`: wxp WebView を CLAP GUI として扱う helper
- `src-plugin`: gain plugin 固有の DSP、state、GUI command

製品固有コードから unsafe をなくすことを優先します。platform handle や CLAP pointer が必要になった場合は、まず adapter/helper 側の境界として表現できるかを検討してください。
