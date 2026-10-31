/**
 * Whether a page at `hostname` can make and use passkeys: only at a name, since WebAuthn takes
 * no address (`127.0.0.1`, `[::1]`) as a relying party. Apart from `passkeys.ts`, as the session
 * asks it on every page and the rest is needed only where passkeys are made and used.
 */
export function passkeysAt(hostname: string): boolean {
  return !hostname.startsWith('[') && !/^\d+(?:\.\d+){3}$/u.test(hostname);
}
