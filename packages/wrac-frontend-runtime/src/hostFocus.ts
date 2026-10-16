import type { WracFrontendRuntime } from "./runtime";

export type HostFocusRestorer = (target?: EventTarget | null) => void;
export type EditableElementPredicate = (target: EventTarget | null) => boolean;

export type HostFocusRestorerOptions = {
  /**
   * Decides which focused elements keep keyboard input in the GUI. Defaults to
   * `defaultIsEditableElement`. A GUI that owns keys outside text fields (e.g.
   * Delete in a custom editor) extends the default instead of replacing it.
   */
  isEditableElement?: EditableElementPredicate;
};

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
  options: HostFocusRestorerOptions = {},
): HostFocusRestorer {
  const isEditableElement =
    options.isEditableElement ?? defaultIsEditableElement;
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
