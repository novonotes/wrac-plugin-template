import assert from "node:assert/strict";
import test from "node:test";
import { createWracFrontendRuntime } from "../src/runtime.ts";

function createFakeTransport() {
  const calls = [];
  return {
    calls,
    transport: {
      async invoke(command, args) {
        calls.push({ command, args });
        return { command, args };
      },
      createChannel(onMessage) {
        return { onmessage: onMessage };
      },
    },
  };
}

test("uses the injected transport for commands and channels", async () => {
  const fake = createFakeTransport();
  const runtime = createWracFrontendRuntime(fake.transport);
  const onMessage = () => {};

  const channel = runtime.createChannel(onMessage);
  const response = await runtime.invoke("product_command", { channel });

  assert.equal(channel.onmessage, onMessage);
  assert.deepEqual(fake.calls, [
    { command: "product_command", args: { channel } },
  ]);
  assert.deepEqual(response, {
    command: "product_command",
    args: { channel },
  });
});

test("keeps WRAC host commands on the injected transport", async () => {
  const fake = createFakeTransport();
  const runtime = createWracFrontendRuntime(fake.transport);

  await runtime.writeToLog({ level: "info", message: "ready" });
  await runtime.focusHostWindow();
  await runtime.requestGuiResize({ width: 640, height: 480 });

  assert.deepEqual(fake.calls, [
    {
      command: "write_to_log",
      args: { entry: { level: "info", message: "ready" } },
    },
    { command: "focus_host_window", args: undefined },
    {
      command: "request_gui_resize",
      args: { request: { width: 640, height: 480 } },
    },
  ]);
});
