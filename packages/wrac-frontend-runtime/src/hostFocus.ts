import type { WracFrontendRuntime } from "./runtime";

export type HostFocusRestorer = (target?: EventTarget | null) => void;

export function isEditableElement(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}

export function createHostFocusRestorer(
  runtime: WracFrontendRuntime,
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
