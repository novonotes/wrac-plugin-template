import { defineConfig } from "vite";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parse } from "smol-toml";

type PluginMetadata = {
  pluginId: string;
  pluginName: string;
  companyName: string;
  version: string;
};

type CargoManifest = {
  package?: {
    version?: unknown;
    metadata?: {
      wrac?: {
        company_name?: unknown;
        plugins?: unknown;
      };
    };
  };
};

type WracPluginMetadata = {
  plugin_id?: unknown;
  plugin_name?: unknown;
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
  const manifest = parse(cargoToml) as unknown as CargoManifest;
  const wrac = manifest.package?.metadata?.wrac;
  const firstPlugin = Array.isArray(wrac?.plugins)
    ? (wrac.plugins[0] as WracPluginMetadata | undefined)
    : undefined;
  return {
    pluginId: requiredString(
      firstPlugin?.plugin_id,
      "package.metadata.wrac.plugins[0].plugin_id",
    ),
    pluginName: requiredString(
      firstPlugin?.plugin_name,
      "package.metadata.wrac.plugins[0].plugin_name",
    ),
    companyName: requiredString(
      wrac?.company_name,
      "package.metadata.wrac.company_name",
    ),
    version: requiredString(manifest.package?.version, "package.version"),
  };
}

function requiredString(value: unknown, key: string): string {
  if (typeof value === "string" && value.length > 0) {
    return value;
  }
  throw new Error(`Failed to read ${key} from src-plugin/Cargo.toml`);
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
