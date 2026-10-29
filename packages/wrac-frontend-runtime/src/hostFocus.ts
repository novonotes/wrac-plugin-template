import type { WracFrontendRuntime } from "./runtime";

// Internal building blocks of `installHostKeyRouting`. Only the editable
// predicate is public, so that a GUI can extend it; focus restoration is
// obtained from `installHostKeyRouting` to keep a single entry point.

export type HostFocusRestorer = (target?: EventTarget | null) => void;
export type EditableElementPredicate = (target: EventTarget | null) => boolean;

// Input types that are operated like buttons or sliders rather than typed into.
// Once the pointer gesture or value change ends, keyboard input can go back to the host.
const NON_TEXT_INPUT_TYPES = new Set([
  "button",
  "checkbox",
  "color",
  "file",
  "hidden",
  "image",
  "radio",
  "range",
  "reset",
  "submit",
]);

/**
 * Returns true only for elements that accept typed text: text-like `<input>`,
 * `<textarea>`, and contenteditable elements. Keyboard focus stays in the GUI
 * only while one of these is focused.
 *
 * `<select>` is intentionally not editable. A select that keeps focus after a
 * choice turns the next Space press into "reopen the popup" instead of the
 * host's transport shortcut.
 *
 * Elements are identified by `tagName` instead of `instanceof` so that the
 * predicate does not depend on the realm's DOM constructors.
 */
export function defaultIsEditableElement(target: EventTarget | null): boolean {
  if (target === null || typeof target !== "object") {
    return false;
  }
  const element = target as Partial<HTMLInputElement>;
  if (element.isContentEditable === true) {
    return true;
  }
  switch (element.tagName) {
    case "TEXTAREA":
      return true;
    case "INPUT":
      return !NON_TEXT_INPUT_TYPES.has(element.type ?? "text");
    default:
      return false;
  }
}

export function createHostFocusRestorer(
  runtime: WracFrontendRuntime,
  isEditableElement: EditableElementPredicate,
): HostFocusRestorer {
  return (target?: EventTarget | null) => {
    if (
      isEditableElement(target ?? null) ||
      isEditableElement(document.activeElement)
    ) {
      return;
    }
    window.setTimeout(() => {
      if (isEditableElement(document.activeElement)) {
        return;
      }
      void runtime.focusHostWindow().catch(() => undefined);
    }, 0);
  };
}
