import { register } from "node:module";

// The sources use extensionless relative imports for bundlers, while Node's
// type stripping only loads explicit `.ts` paths. Tests resolve those imports
// to `.ts` files instead of changing the import style of the sources.
register(
  "data:text/javascript," +
    encodeURIComponent(`
export async function resolve(specifier, context, nextResolve) {
  try {
    return await nextResolve(specifier, context);
  } catch (error) {
    if (error?.code !== "ERR_MODULE_NOT_FOUND" || !specifier.startsWith(".")) {
      throw error;
    }
    return nextResolve(specifier + ".ts", context);
  }
}
`),
);
