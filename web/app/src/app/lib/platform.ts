/**
 * The authenticator a computer has of its own, by the name its system gives it, where glave
 * knows it: Touch ID on a Mac, Windows Hello on Windows; `null` elsewhere, where the app says
 * "a passkey".
 */
export type PlatformAuthenticator = 'touch-id' | 'windows-hello' | null;

/** The authenticator of the system `platform` names (`navigator.platform`, or the client hints'). */
export function platformAuthenticator(platform: string): PlatformAuthenticator {
  if (/^mac/iu.test(platform)) {
    return 'touch-id';
  }
  return /^win/iu.test(platform) ? 'windows-hello' : null;
}
