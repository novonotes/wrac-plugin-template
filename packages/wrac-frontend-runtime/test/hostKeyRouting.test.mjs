import assert from "node:assert/strict";
import test from "node:test";
import { installHostKeyRouting } from "../src/hostKeyRouting.ts";
import { createWracFrontendRuntime } from "../src/runtime.ts";

// Minimal window/document pair with the DOM dispatch order the routing relies
// on: window capture, document capture, document bubble, window bubble.
function createFakeDom() {
  const createTarget = () => {
    const listeners = [];
    return {
      listeners,
      addEventListener(type, listener, options) {
        const capture = options === true || options?.capture === true;
        listeners.push({ type, listener, capture });
      },
      removeEventListener(type, listener, options) {
        const capture = options === true || options?.capture === true;
        const index = listeners.findIndex(
          (entry) =>
            entry.type === type &&
            entry.listener === listener &&
            entry.capture === capture,
        );
        if (index >= 0) listeners.splice(index, 1);
      },
    };
  };
  const dom = {
    window: createTarget(),
    document: createTarget(),
  };
  dom.window.setTimeout = globalThis.setTimeout;
  dom.document.activeElement = null;
  dom.documentHasFocus = true;
  dom.document.hasFocus = () => dom.documentHasFocus;

  dom.dispatch = (type, target, init = {}) => {
    const event = {
      type,
      target,
      key: "",
      code: "",
      keyCode: 0,
      isComposing: false,
      defaultPrevented: false,
      propagationStopped: false,
      immediateStopped: false,
      preventDefault() {
        this.defaultPrevented = true;
      },
      stopPropagation() {
        this.propagationStopped = true;
      },
      stopImmediatePropagation() {
        this.propagationStopped = true;
        this.immediateStopped = true;
      },
      ...init,
    };
    const phases = [
      [dom.window, true],
      [dom.document, true],
      [dom.document, false],
      [dom.window, false],
    ];
    for (const [currentTarget, capture] of phases) {
      for (const entry of [...currentTarget.listeners]) {
        if (entry.type !== type || entry.capture !== capture) continue;
        entry.listener(event);
        if (event.immediateStopped) return event;
      }
      if (event.propagationStopped) return event;
    }
    return event;
  };
  return dom;
}

function setup() {
  const dom = createFakeDom();
  globalThis.window = dom.window;
  globalThis.document = dom.document;
  const focusCalls = [];
  const runtime = createWracFrontendRuntime({
    async invoke(command) {
      focusCalls.push(command);
      return { ok: true };
    },
    createChannel(onMessage) {
      return { onmessage: onMessage };
    },
  });
  const routing = installHostKeyRouting(runtime);
  return { dom, focusCalls, routing };
}

const nextTask = () => new Promise((resolve) => setTimeout(resolve, 0));
const element = (tagName, props = {}) => ({ tagName, ...props });

test("returns focus to the host after GUI operations so Space reaches the host", async () => {
  const { dom, focusCalls, routing } = setup();
  const knob = element("DIV");
  const select = element("SELECT");

  dom.dispatch("pointerup", knob);
  // A committed select choice also returns focus to the host.
  dom.document.activeElement = select;
  dom.dispatch("change", select);
  await nextTask();

  assert.deepEqual(focusCalls, ["focus_host_window", "focus_host_window"]);
  routing.dispose();
});

test("keeps a native select open until its choice is committed", async () => {
  const { dom, focusCalls, routing } = setup();
  const select = element("SELECT");
  dom.document.activeElement = select;

  dom.dispatch("pointerup", select);
  await nextTask();
  assert.deepEqual(focusCalls, []);

  dom.dispatch("change", select);
  await nextTask();
  assert.deepEqual(focusCalls, ["focus_host_window"]);
  routing.dispose();
});

test("drops Space on a non-editable element instead of scrolling or opening it", async () => {
  const { dom, focusCalls, routing } = setup();
  const select = element("SELECT");
  dom.document.activeElement = select;

  const event = dom.dispatch("keydown", select, { key: " ", code: "Space" });
  await nextTask();

  assert.equal(event.defaultPrevented, true);
  assert.deepEqual(focusCalls, ["focus_host_window"]);
  routing.dispose();
});

test("keeps focus in the GUI while typing into a text field", async () => {
  const { dom, focusCalls, routing } = setup();
  const input = element("INPUT", { type: "number" });
  dom.document.activeElement = input;

  const space = dom.dispatch("keydown", input, { key: " ", code: "Space" });
  dom.dispatch("pointerup", input);
  await nextTask();

  assert.equal(space.defaultPrevented, false);
  assert.deepEqual(focusCalls, []);
  routing.dispose();
});

test("returns focus after a text field closes unless the host took focus", async () => {
  const { dom, focusCalls, routing } = setup();
  const input = element("INPUT", { type: "text" });
  const body = element("BODY");

  // Enter followed by blur: focus stays in the WebView, so it goes back to the host.
  dom.document.activeElement = body;
  dom.dispatch("focusout", input, { relatedTarget: null });
  await nextTask();
  await nextTask();
  assert.deepEqual(focusCalls, ["focus_host_window"]);

  // A click into the host's own UI: focus already left the WebView.
  dom.documentHasFocus = false;
  dom.dispatch("focusout", input, { relatedTarget: null });
  await nextTask();
  await nextTask();
  assert.deepEqual(focusCalls, ["focus_host_window"]);
  routing.dispose();
});

test("consumes the confirming Enter of a text field through its keyup", () => {
  const { dom, routing } = setup();
  const input = element("INPUT", { type: "text" });

  const keydown = dom.dispatch("keydown", input, { key: "Enter" });
  const keyup = dom.dispatch("keyup", input, { key: "Enter" });

  assert.equal(keydown.defaultPrevented, true);
  assert.equal(keyup.propagationStopped, true);
  routing.dispose();
});

test("leaves IME confirmation Enter to the browser", () => {
  const { dom, routing } = setup();
  const input = element("INPUT", { type: "text" });

  const keydown = dom.dispatch("keydown", input, {
    key: "Enter",
    keyCode: 229,
    isComposing: true,
  });

  assert.equal(keydown.defaultPrevented, false);
  routing.dispose();
});
