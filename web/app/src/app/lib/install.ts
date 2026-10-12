/**
 * Chromium's `beforeinstallprompt` event: the browser offers to install the app, and the page
 * shows that offer when the person asks. Not in the DOM's types, since only Chromium has it.
 */
export interface InstallPrompt extends Event {
  prompt(): Promise<unknown>;
  readonly userChoice: Promise<{ readonly outcome: 'accepted' | 'dismissed' }>;
}

/** Whether `event` is Chromium's install prompt. */
export function isInstallPrompt(event: Event): event is InstallPrompt {
  return (
    'prompt' in event &&
    typeof event.prompt === 'function' &&
    'userChoice' in event &&
    event.userChoice instanceof Promise
  );
}

/**
 * How this browser installs the app: `prompt`, from the browser's own prompt (Chromium);
 * `safari-mac` or `safari-ios`, by hand from Safari's menus (Add to Dock, Add to Home Screen);
 * or `none`: installed already, or no way the page knows of.
 */
export type InstallOffer = 'prompt' | 'safari-mac' | 'safari-ios' | 'none';

/** Whether `userAgent` is Safari's own, not another browser that names Safari in its own. */
export function isSafari(userAgent: string): boolean {
  return (
    userAgent.includes('Safari/') &&
    !/Chrome|Chromium|CriOS|FxiOS|EdgiOS|Edg\/|OPR\/|Android/u.test(userAgent)
  );
}

/** How to offer the app's install, from what the page knows. */
export function installOffer(installed: boolean, prompt: boolean, userAgent: string): InstallOffer {
  if (installed) {
    return 'none';
  }
  if (prompt) {
    return 'prompt';
  }
  if (!isSafari(userAgent)) {
    return 'none';
  }
  return /iPhone|iPad|iPod/u.test(userAgent) ? 'safari-ios' : 'safari-mac';
}
