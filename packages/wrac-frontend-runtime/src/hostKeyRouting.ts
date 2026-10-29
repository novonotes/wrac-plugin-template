import {
  createHostFocusRestorer,
  defaultIsEditableElement,
  type EditableElementPredicate,
  type HostFocusRestorer,
} from "./hostFocus";
import { installImeKeyIsolation } from "./imeKeyIsolation";
import type { WracFrontendRuntime } from "./runtime";

// Decides where keyboard input goes between the WebView GUI and the host (DAW).
//
// There is no path that forwards a key the WebView received to the host: the
// VST3 onKeyDown/onKeyUp of the wrapper are stubs and WXP has no API to hand a
// key back. Instead of forwarding, keys are kept out of the WebView by
// returning focus to the host whenever an operation ends. Without this, a single
// click on the GUI makes Space scroll the page or open a focused `<select>`
// instead of starting host playback.
//
// Contract: the GUI keeps keyboard input only while an editable element is
// focused (`defaultIsEditableElement` unless overridden). Every other key, Space in
// particular, belongs to the host. A GUI element that needs Space for its own
// keyboard operation must call preventDefault on it.

export type HostKeyRoutingOptions = {
  /**
   * Decides which focused elements keep keyboard input in the GUI. Defaults to
   * `defaultIsEditableElement`. A GUI that owns keys outside text fields (e.g.
   * Delete in a custom editor) extends the default instead of replacing it.
   */
  isEditableElement?: EditableElementPredicate;
};

export type HostKeyRouting = {
  /**
   * Returns focus to the host unless an editable element is focused. Call it
   * after closing a text field programmatically (blur the field first, because
   * the call is a no-op while the field still has focus), or when a GUI-owned
   * operation ends without a pointerup on the document.
   */
  restoreHostFocus: (target?: EventTarget | null) => void;
  dispose: () => void;
};

/**
 * Installs the document-level key routing. Call it once at startup, before
 * other window or document key listeners are added, so that IME isolation runs
 * first.
 */
export function installHostKeyRouting(
  runtime: WracFrontendRuntime,
  options: HostKeyRoutingOptions = {},
): HostKeyRouting {
  const isEditableElement =
    options.isEditableElement ?? defaultIsEditableElement;
  const restoreHostFocus = createHostFocusRestorer(runtime, isEditableElement);
  const disposers = [
    installImeKeyIsolation(),
    restoreHostFocusAfterPointerUp(restoreHostFocus),
    restoreHostFocusAfterChoice(restoreHostFocus, isEditableElement),
    restoreHostFocusAfterEditing(restoreHostFocus, isEditableElement),
    returnStraySpaceToHost(restoreHostFocus, isEditableElement),
    consumeEditableEnterKeys(isEditableElement),
  ];
  return {
    restoreHostFocus,
    dispose() {
      for (const dispose of disposers) dispose();
    },
  };
}

// Document capture covers GUI operations wherever the element sits in the DOM
// (overlays, resize grips, portals). A native select can deliver pointerup
// while its popup is still open, so returning focus here would dismiss it.
function restoreHostFocusAfterPointerUp(
  restoreHostFocus: HostFocusRestorer,
): () => void {
  const handlePointerUp = (event: PointerEvent) => {
    if ((event.target as Element | null)?.tagName === "SELECT") return;
    restoreHostFocus(event.target);
  };
  document.addEventListener("pointerup", handlePointerUp, true);
  return () => document.removeEventListener("pointerup", handlePointerUp, true);
}

// A committed `change` marks the end of a native select operation, and covers
// keyboard changes to checkboxes and radios as well.
function restoreHostFocusAfterChoice(
  restoreHostFocus: HostFocusRestorer,
  isEditableElement: EditableElementPredicate,
): () => void {
  const handleChange = (event: Event) => {
    if (isEditableElement(event.target)) return;
    restoreHostFocus(event.target);
  };
  document.addEventListener("change", handleChange, true);
  return () => document.removeEventListener("change", handleChange, true);
}

// Text fields are usually closed by code (Enter or Escape followed by blur),
// which leaves focus on the body without any pointerup. Focus is returned once
// it has left the editable element, but only while the document still has
// focus: when the user clicked into the host's own UI, focus already left the
// WebView and must not be taken back.
function restoreHostFocusAfterEditing(
  restoreHostFocus: HostFocusRestorer,
  isEditableElement: EditableElementPredicate,
): () => void {
  const handleFocusOut = (event: FocusEvent) => {
    if (
      !isEditableElement(event.target) ||
      isEditableElement(event.relatedTarget)
    ) {
      return;
    }
    window.setTimeout(() => {
      if (document.hasFocus()) restoreHostFocus();
    }, 0);
  };
  document.addEventListener("focusout", handleFocusOut, true);
  return () => document.removeEventListener("focusout", handleFocusOut, true);
}

// Focus can still remain in the GUI, e.g. after a `<select>` popup is dismissed
// without a choice or when the host gives the WebView focus on open. Space then
// scrolls the page or opens the focused control. That press cannot be forwarded,
// so it is dropped and focus goes back to the host for the next press.
//
// The listener runs in the bubble phase and skips events whose default was
// already prevented, so a GUI element that deliberately handles Space (and
// calls preventDefault) keeps working.
function returnStraySpaceToHost(
  restoreHostFocus: HostFocusRestorer,
  isEditableElement: EditableElementPredicate,
): () => void {
  const handleKeyDown = (event: KeyboardEvent) => {
    if (
      event.key !== " " ||
      event.defaultPrevented ||
      isEditableElement(event.target)
    ) {
      return;
    }
    event.preventDefault();
    restoreHostFocus(event.target);
  };
  document.addEventListener("keydown", handleKeyDown);
  return () => document.removeEventListener("keydown", handleKeyDown);
}

// Prevents the Enter that confirms a text field from also firing as a host
// shortcut. Some hosts (confirmed in Pro Tools) receive the keyup even when the
// keydown was consumed, so the matching keyup of a consumed keydown is consumed
// as well.
function consumeEditableEnterKeys(
  isEditableElement: EditableElementPredicate,
): () => void {
  const consumedKeys = new Set<string>();

  const isEditableEnter = (event: KeyboardEvent) =>
    // IME confirmation is left to the browser; preventDefault would block it.
    // WebKit reports the Enter right after compositionend with keyCode 229 too.
    event.key === "Enter" &&
    !event.isComposing &&
    event.keyCode !== 229 &&
    isEditableElement(event.target);
  const consume = (event: KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();
  };

  const handleKeyDown = (event: KeyboardEvent) => {
    if (!isEditableEnter(event)) return;
    consumedKeys.add(event.key);
    consume(event);
  };
  const handleKeyPress = (event: KeyboardEvent) => {
    if (!isEditableEnter(event)) return;
    consume(event);
  };
  const handleKeyUp = (event: KeyboardEvent) => {
    // Only keyups whose keydown was consumed are paired and consumed.
    if (!consumedKeys.delete(event.key)) return;
    consume(event);
  };

  document.addEventListener("keydown", handleKeyDown);
  document.addEventListener("keypress", handleKeyPress);
  document.addEventListener("keyup", handleKeyUp, true);
  return () => {
    document.removeEventListener("keydown", handleKeyDown);
    document.removeEventListener("keypress", handleKeyPress);
    document.removeEventListener("keyup", handleKeyUp, true);
  };
}
