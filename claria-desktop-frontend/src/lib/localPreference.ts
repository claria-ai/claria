export function isNoticeDismissed(key: string): boolean {
  try {
    return window.localStorage?.getItem(key) === "true";
  } catch {
    return false;
  }
}

export function dismissNotice(key: string): void {
  try {
    window.localStorage?.setItem(key, "true");
  } catch {
    // A locked-down webview may not expose persistent storage. Hiding the
    // notice still works for the current component lifetime.
  }
}
