import { installOffer, isInstallPrompt, isSafari } from './install';

const SAFARI_MAC =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 15_6) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15';
const SAFARI_IOS =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1';
const CHROME =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36';
const CHROME_IOS =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/141.0 Mobile/15E148 Safari/604.1';
const FIREFOX = 'Mozilla/5.0 (X11; Linux x86_64; rv:143.0) Gecko/20100101 Firefox/143.0';

describe('isSafari', () => {
  it('knows Safari from the browsers that name it', () => {
    expect(isSafari(SAFARI_MAC)).toBe(true);
    expect(isSafari(SAFARI_IOS)).toBe(true);
    expect(isSafari(CHROME)).toBe(false);
    expect(isSafari(CHROME_IOS)).toBe(false);
    expect(isSafari(FIREFOX)).toBe(false);
  });
});

describe('installOffer', () => {
  it('offers nothing once installed', () => {
    expect(installOffer(true, true, CHROME)).toBe('none');
    expect(installOffer(true, false, SAFARI_MAC)).toBe('none');
  });

  it('offers the browser’s prompt when there is one, else Safari’s steps', () => {
    expect(installOffer(false, true, CHROME)).toBe('prompt');
    expect(installOffer(false, false, CHROME)).toBe('none');
    expect(installOffer(false, false, FIREFOX)).toBe('none');
    expect(installOffer(false, false, SAFARI_MAC)).toBe('safari-mac');
    expect(installOffer(false, false, SAFARI_IOS)).toBe('safari-ios');
  });
});

describe('isInstallPrompt', () => {
  it('knows the install prompt from other events', () => {
    const prompt = Object.assign(new Event('beforeinstallprompt'), {
      prompt: () => Promise.resolve(),
      userChoice: Promise.resolve({ outcome: 'accepted' }),
    });
    expect(isInstallPrompt(prompt)).toBe(true);
    expect(isInstallPrompt(new Event('beforeinstallprompt'))).toBe(false);
  });
});
