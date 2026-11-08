import { TestBed } from '@angular/core/testing';

import { keyBytes } from '../lib/push';
import { FakeSession } from '../testing/fake-session';
import { FakePushBrowser, device, offered, pushingTab } from '../testing/push';
import { PushService, WORKER } from './push.service';

/** Lets the service's promises settle. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
}

async function service(fake: FakeSession, browser: FakePushBrowser): Promise<PushService> {
  await pushingTab(fake, browser.browser);
  return TestBed.inject(PushService);
}

describe('PushService', () => {
  afterEach(() => {
    localStorage.clear();
    vi.unstubAllGlobals();
  });

  it('names its worker through the one Trusted Types policy the page allows', async () => {
    const made: string[] = [];
    const browser = new FakePushBrowser('default');
    const fake = new FakeSession()
      .answer('push/get', { push: offered() })
      .answer('push/subscribe', { push: offered() });
    const push = await service(fake, browser);
    // Once the test's module is made, which Angular compiles under Trusted Types of its own.
    vi.stubGlobal('trustedTypes', {
      createPolicy: (name: string, rules: { createScriptURL(url: string): string }) => {
        made.push(name);
        return { createScriptURL: (url: string) => `trusted:${rules.createScriptURL(url)}` };
      },
    });
    await push.turnOn();
    await push.turnOn();
    expect(browser.registered).toEqual([`trusted:${WORKER}`, `trusted:${WORKER}`]);
    expect(made).toEqual(['glave-push']);
  });

  it('replaces a subscription made with a key the server no longer signs with', async () => {
    const browser = new FakePushBrowser('granted');
    browser.holding(keyBytes('CQkJ'));
    const fake = new FakeSession()
      .answer('push/get', { push: offered() })
      .answer('push/subscribe', { push: offered([device('d1', 'A browser', true)]) });
    const push = await service(fake, browser);
    await push.turnOn();
    await settle();
    expect(browser.unsubscribed).toHaveLength(1);
    expect(browser.made).toHaveLength(1);
    expect(push.on()).toBe(true);
  });

  it('keeps which browser is this one when the server answers without marking it', async () => {
    const browser = new FakePushBrowser('granted');
    const mine = device('d1', 'A browser', true);
    const fake = new FakeSession()
      .answer('push/get', { push: offered([mine]) })
      .answer('push/set', { push: offered([{ ...mine, current: false }]) });
    const push = await service(fake, browser);
    await push.load();
    await push.choose(false, true);
    expect(push.on()).toBe(true);
  });
});
