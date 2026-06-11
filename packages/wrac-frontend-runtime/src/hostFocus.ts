import type { WracFrontendRuntime } from "./runtime";

export type HostFocusRestorer = (target?: EventTarget | null) => void;
export type EditableElementPredicate = (target: EventTarget | null) => boolean;

export type HostFocusRestorerOptions = {
  isEditableElement?: EditableElementPredicate;
};

export function defaultIsEditableElement(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
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
