import { passkeysAt } from './passkeys-at';

describe('passkeysAt', () => {
  it('are made at a name and never at an address', () => {
    for (const name of ['localhost', 'glave.test', 'box.tail1234.ts.net', '1.example']) {
      expect(passkeysAt(name)).toBe(true);
    }
    for (const address of ['127.0.0.1', '192.168.1.2', '[::1]', '[2001:db8::1]']) {
      expect(passkeysAt(address)).toBe(false);
    }
  });
});
