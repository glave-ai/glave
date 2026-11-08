import { TestBed } from '@angular/core/testing';

import { keyBytes } from '../lib/push';
import { FakeSession } from '../testing/fake-session';
import { ENDPOINT, FakePushBrowser, device, offered, pushingTab } from '../testing/push';
import { PushService, WORKER } from './push.service';

async function service(fake: FakeSession, browser: FakePushBrowser): Promise<PushService> {
  await pushingTab(fake, browser.browser);
  return TestBed.inject(PushService);
}

describe('PushService', () => {
  afterEach(() => {
    localStorage.clear();
    vi.unstubAllGlobals();
  });

  it('names its worker, with what to show of a push it cannot read, through the one policy the page allows', async () => {
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
    const named = `trusted:${WORKER}?fallback=glave%20needs%20you`;
    expect(browser.registered).toEqual([named, named]);
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
    expect(browser.unsubscribed).toHaveLength(1);
    expect(browser.made).toHaveLength(1);
    expect(push.on()).toBe(true);
  });

  it('keeps which browser is this one when the server answers without marking it', async () => {
    const browser = new FakePushBrowser('granted');
    const mine = device('d1', 'A browser', true);
    const fake = new FakeSession()
      .answer('push/get', { push: offered([mine, device('d2', 'Old laptop')]) })
      .answer('push/remove', { push: offered([{ ...mine, current: false }]) });
    const push = await service(fake, browser);
    await push.load();
    await push.remove('d2');
    expect(push.on()).toBe(true);
  });

  it('stops this browser’s notifications as the person signs out, whatever comes of it', async () => {
    const browser = new FakePushBrowser('granted');
    browser.holding(keyBytes('AQIDBA'));
    const fake = new FakeSession()
      .answer('push/get', { push: offered([device('d1', 'A browser', true)]) })
      .answer('push/remove', { push: offered() });
    const push = await service(fake, browser);
    await push.signingOut();
    expect(fake.paramsOf('push/get')).toEqual([{ endpoint: ENDPOINT }]);
    expect(browser.unsubscribed).toEqual([ENDPOINT]);
    expect(fake.paramsOf('push/remove')).toEqual([{ device: 'd1' }]);
    // With nothing subscribed here, nothing is asked.
    await push.signingOut();
    expect(fake.paramsOf('push/get')).toHaveLength(1);
  });
});
