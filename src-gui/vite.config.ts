import { defineConfig } from "vite";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

type PluginMetadata = {
  pluginId: string;
  pluginName: string;
  companyName: string;
  version: string;
};

function readCargoMetadata(): PluginMetadata {
  // テンプレート利用者が plugin 名や company 名を変える場所を 1 箇所に絞るため、
  // GUI も Rust descriptor と同じ src-plugin/Cargo.toml の metadata を読む。
  // フロント側に直書きすると、host に見える plugin 名と About 表示がずれやすい。
  const cargoToml = readFileSync(
    resolve(__dirname, "../src-plugin/Cargo.toml"),
    "utf8",
  );
  const versionMatch = cargoToml.match(/^\s*version\s*=\s*"([^"]+)"\s*$/m);
  if (!versionMatch) {
    throw new Error(
      "src-plugin/Cargo.toml から version を取得できませんでした",
    );
  }
  return {
    pluginId: readWracMetadataString(cargoToml, "plugin_id"),
    pluginName: readWracMetadataString(cargoToml, "plugin_name"),
    companyName: readWracMetadataString(cargoToml, "company_name"),
    version: versionMatch[1],
  };
}

function readWracMetadataString(cargoToml: string, key: string): string {
  let inSection = false;
  for (const rawLine of cargoToml.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith("[") && line.endsWith("]")) {
      inSection = line === "[package.metadata.wrac]";
      continue;
    }
    if (!inSection) {
      continue;
    }
    const match = line.match(new RegExp(`^${key}\\s*=\\s*"([^"]+)"\\s*$`));
    if (match) {
      return match[1];
    }
  }
  throw new Error(
    `src-plugin/Cargo.toml から package.metadata.wrac.${key} を取得できませんでした`,
  );
}

export default defineConfig({
  define: {
    // plugin 固有名を定数名に含めると、テンプレートから別 plugin を作るたびに
    // TypeScript 側の識別子まで変更が必要になる。中身だけ metadata として差し替える。
    __WRAC_PLUGIN_METADATA__: JSON.stringify(readCargoMetadata()),
  },
  server: {
    // Debug plugin は WebView から 127.0.0.1 を読む。Vite の default `localhost`
    // だと環境によって IPv6 loopback だけに bind され、DAW 内 WebView の解決先と
    // ずれて黒画面になり得る。
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
