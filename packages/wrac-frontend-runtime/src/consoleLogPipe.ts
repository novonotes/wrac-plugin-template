import type { NativeLogData, NativeLogEntry, NativeLogLevel } from "./runtime";

type ConsoleMethodName = "debug" | "log" | "info" | "warn" | "error";

const consoleMethodLevels = {
  debug: "debug",
  log: "info",
  info: "info",
  warn: "warn",
  error: "error",
} as const satisfies Record<ConsoleMethodName, NativeLogLevel>;

let consoleLogPipeInstalled = false;

export function installConsoleLogPipe(
  writeToLog: (entry: NativeLogEntry) => Promise<unknown> | void,
): void {
  if (consoleLogPipeInstalled) {
    return;
  }
  consoleLogPipeInstalled = true;

  const originalConsole = {
    debug: console.debug.bind(console),
    log: console.log.bind(console),
    info: console.info.bind(console),
    warn: console.warn.bind(console),
    error: console.error.bind(console),
  } satisfies Record<ConsoleMethodName, (...args: unknown[]) => void>;

  for (const methodName of Object.keys(
    consoleMethodLevels,
  ) as ConsoleMethodName[]) {
    console[methodName] = (...args: unknown[]) => {
      originalConsole[methodName](...args);
      try {
        const result = writeToLog({
          level: consoleMethodLevels[methodName],
          message: formatConsoleArgs(args),
        });
        if (result instanceof Promise) {
          void result.catch(() => undefined);
        }
      } catch {
        // Logging must never break GUI behavior.
      }
    };
  }
}

function formatConsoleArgs(args: unknown[]): string {
  if (args.length === 0) {
    return "";
  }
  return args.map((arg) => formatConsoleValue(arg, new WeakSet())).join(" ");
}

function formatConsoleValue(value: unknown, seen: WeakSet<object>): string {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  if (typeof value === "bigint") {
    return `${value.toString()}n`;
  }
  if (typeof value === "symbol") {
    return value.toString();
  }
  if (typeof value === "function") {
    return `[Function ${value.name || "anonymous"}]`;
  }
  if (value === null || value === undefined) {
    return String(value);
  }
  if (value instanceof Error) {
    return value.stack || `${value.name}: ${value.message}`;
  }
  return stringifyObject(value, seen);
}

function stringifyObject(value: object, seen: WeakSet<object>): string {
  try {
    return JSON.stringify(value, createJsonReplacer(seen));
  } catch {
    return Object.prototype.toString.call(value);
  }
}

function createJsonReplacer(seen: WeakSet<object>) {
  return (_key: string, value: unknown): NativeLogData | undefined => {
    if (typeof value === "bigint") {
      return `${value.toString()}n`;
    }
    if (typeof value === "function") {
      return `[Function ${value.name || "anonymous"}]`;
    }
    if (typeof value === "symbol") {
      return value.toString();
    }
    if (value instanceof Error) {
      return {
        name: value.name,
        message: value.message,
        stack: value.stack ?? null,
      };
    }
    if (value && typeof value === "object") {
      if (seen.has(value)) {
        return "[Circular]";
      }
      seen.add(value);
    }
    if (
      value === undefined ||
      value === null ||
      typeof value === "string" ||
      typeof value === "number" ||
      typeof value === "boolean" ||
      Array.isArray(value) ||
      typeof value === "object"
    ) {
      return value as NativeLogData | undefined;
    }
    return String(value);
  };
}
