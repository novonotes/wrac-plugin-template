// IME keys reach not only the focused text field but also window-level Escape
// handlers and the Enter handling of host key routing. Stopping propagation in
// the first window capture listener isolates them from the application, while
// skipping preventDefault leaves composition itself to the browser. This keeps
// candidate confirmation and cancellation from turning into GUI or host actions,
// including in text fields and overlays added later.
export function installImeKeyIsolation(): () => void {
  let composing = false;
  const imeKeys = new Set<string>();

  const handleCompositionStart = () => {
    composing = true;
  };
  const handleCompositionEnd = () => {
    composing = false;
  };
  const isIme = (event: KeyboardEvent) =>
    composing || event.isComposing || event.keyCode === 229;
  const keyOf = (event: KeyboardEvent) => event.code || event.key;

  const handleKeyDown = (event: KeyboardEvent) => {
    if (!isIme(event)) return;
    imeKeys.add(keyOf(event));
    event.stopImmediatePropagation();
  };
  // WebKit may deliver compositionend in the middle of the confirming Enter.
  // The key is remembered until its keypress/keyup so the second half of the
  // confirmation does not reach the host or a form either.
  const handleKeyPress = (event: KeyboardEvent) => {
    if (isIme(event) || imeKeys.has(keyOf(event))) {
      event.stopImmediatePropagation();
    }
  };
  const handleKeyUp = (event: KeyboardEvent) => {
    const wasIme = imeKeys.delete(keyOf(event));
    if (isIme(event) || wasIme) event.stopImmediatePropagation();
  };
  // keyup is never delivered once focus leaves the window, so stale state is dropped.
  const handleBlur = () => {
    composing = false;
    imeKeys.clear();
  };

  window.addEventListener("compositionstart", handleCompositionStart, true);
  window.addEventListener("compositionend", handleCompositionEnd, true);
  window.addEventListener("keydown", handleKeyDown, true);
  window.addEventListener("keypress", handleKeyPress, true);
  window.addEventListener("keyup", handleKeyUp, true);
  window.addEventListener("blur", handleBlur);

  return () => {
    window.removeEventListener("compositionstart", handleCompositionStart, true);
    window.removeEventListener("compositionend", handleCompositionEnd, true);
    window.removeEventListener("keydown", handleKeyDown, true);
    window.removeEventListener("keypress", handleKeyPress, true);
    window.removeEventListener("keyup", handleKeyUp, true);
    window.removeEventListener("blur", handleBlur);
  };
}
