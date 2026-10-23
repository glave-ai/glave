import { platformAuthenticator } from './platform';

describe('platformAuthenticator', () => {
  it('names the system’s own: Touch ID on a Mac, Windows Hello on Windows, else none', () => {
    expect(platformAuthenticator('MacIntel')).toBe('touch-id');
    expect(platformAuthenticator('macOS')).toBe('touch-id');
    expect(platformAuthenticator('Win32')).toBe('windows-hello');
    expect(platformAuthenticator('Windows')).toBe('windows-hello');
    expect(platformAuthenticator('Linux x86_64')).toBeNull();
    expect(platformAuthenticator('')).toBeNull();
  });
});
