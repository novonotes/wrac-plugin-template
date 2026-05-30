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
  // To keep plugin name and company name changes in a single place,
  // the GUI reads metadata from the same src-plugin/Cargo.toml as the Rust descriptor.
  // Hardcoding values on the frontend side easily causes the host-visible plugin name
  // and the About display to diverge.
  const cargoToml = readFileSync(
    resolve(__dirname, "../src-plugin/Cargo.toml"),
    "utf8",
  );
  const versionMatch = cargoToml.match(/^\s*version\s*=\s*"([^"]+)"\s*$/m);
  if (!versionMatch) {
    throw new Error(
      "Failed to read version from src-plugin/Cargo.toml",
    );
  }
  return {
    pluginId: readFirstPluginMetadataString(cargoToml, "plugin_id"),
    pluginName: readFirstPluginMetadataString(cargoToml, "plugin_name"),
    companyName: readWracMetadataString(cargoToml, "company_name"),
    version: versionMatch[1],
  };
}

function readFirstPluginMetadataString(cargoToml: string, key: string): string {
  let inSection = false;
  for (const rawLine of cargoToml.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith("[") && line.endsWith("]")) {
      inSection = line === "[[package.metadata.wrac.plugins]]";
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
    `Failed to read package.metadata.wrac.plugins.${key} from src-plugin/Cargo.toml`,
  );
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
    `Failed to read package.metadata.wrac.${key} from src-plugin/Cargo.toml`,
  );
}

export default defineConfig({
  define: {
    // Including the plugin-specific name in the constant identifier would require
    // updating the TypeScript identifier every time a new plugin is created from the template.
    // Only the contents are swapped via metadata.
    __WRAC_PLUGIN_METADATA__: JSON.stringify(readCargoMetadata()),
  },
  server: {
    // Debug plugins load the WebView from 127.0.0.1. Vite's default `localhost`
    // may bind only to the IPv6 loopback in some environments, causing a mismatch
    // with the address the WebView inside the DAW resolves to, which can result in a black screen.
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
