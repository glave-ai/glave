import { TestBed } from '@angular/core/testing';

import { InstallService } from './install.service';

/** A stand-in for Chromium's install prompt, answering `outcome`. */
function installPrompt(
  outcome: 'accepted' | 'dismissed',
): Event & { readonly shown: () => number } {
  let shown = 0;
  return Object.assign(new Event('beforeinstallprompt', { cancelable: true }), {
    prompt: () => {
      shown += 1;
      return Promise.resolve();
    },
    userChoice: Promise.resolve({ outcome }),
    shown: () => shown,
  });
}

describe('InstallService', () => {
  it('offers nothing in a browser with no prompt of its own', () => {
    expect(TestBed.inject(InstallService).offer()).toBe('none');
  });

  it('keeps the browser’s prompt for the person, and offers nothing once installed', async () => {
    const service = TestBed.inject(InstallService);
    const prompt = installPrompt('accepted');
    window.dispatchEvent(prompt);
    expect(prompt.defaultPrevented).toBe(true);
    expect(service.offer()).toBe('prompt');
    await service.install();
    expect(prompt.shown()).toBe(1);
    expect(service.offer()).toBe('none');
    await service.install();
    expect(prompt.shown()).toBe(1);
  });

  it('offers again after a dismissed prompt only when the browser offers again', async () => {
    const service = TestBed.inject(InstallService);
    window.dispatchEvent(installPrompt('dismissed'));
    await service.install();
    expect(service.offer()).toBe('none');
    window.dispatchEvent(installPrompt('dismissed'));
    expect(service.offer()).toBe('prompt');
    window.dispatchEvent(new Event('appinstalled'));
    expect(service.offer()).toBe('none');
  });

  it('ignores events that are not the install prompt', () => {
    const service = TestBed.inject(InstallService);
    window.dispatchEvent(new Event('beforeinstallprompt'));
    expect(service.offer()).toBe('none');
  });
});
