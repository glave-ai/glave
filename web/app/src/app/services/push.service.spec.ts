import { TestBed } from '@angular/core/testing';

import { keyBytes } from '../lib/push';
import { FakeSession } from '../testing/fake-session';
import { settle as settled } from '../testing/links';
import { ENDPOINT, FakePushBrowser, KEY, device, offered, pushingTab } from '../testing/push';
import { PushService, WORKER } from './push.service';
import { SESSION_CLIENT_FACTORY } from './session.service';

/** Where the service remembers notifications were turned on in this browser. */
const ON_HERE = 'glave.push.on';

/** The person's organizations: the one this tab works in, and another. */
const HERE = '019a0000-0000-7000-8000-0000000000aa';
const ELSEWHERE = '019a0000-0000-7000-8000-0000000000bb';

/** Lets what the service started on its own come to rest. */
async function settle(): Promise<void> {
  await settled({ whenStable: () => Promise.resolve() });
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

  it('stops this browser’s notifications as the person signs out: unsubscribed first, then forgotten', async () => {
    const browser = new FakePushBrowser('granted');
    browser.holding(keyBytes('AQIDBA'));
    const fake = new FakeSession()
      .answer('push/get', { push: offered([device('d1', 'A browser', true)]) })
      .answer('push/remove', { push: offered() });
    const push = await service(fake, browser);
    const asked: number[] = [];
    browser.onUnsubscribe = () => asked.push(fake.paramsOf('push/get').length);
    await push.signingOut();
    expect(asked).toEqual([0]);
    expect(fake.paramsOf('push/get')).toEqual([{ endpoint: ENDPOINT }]);
    expect(fake.paramsOf('push/remove')).toEqual([{ device: 'd1' }]);
    // With nothing subscribed here, nothing is asked.
    await push.signingOut();
    expect(fake.paramsOf('push/get')).toHaveLength(1);
  });

  it('leaves this browser unsubscribed as the person signs out, though the server can’t be reached', async () => {
    const browser = new FakePushBrowser('granted');
    browser.holding(keyBytes('AQIDBA'));
    const fake = new FakeSession().answer('push/get', () => {
      throw new TypeError('Failed to fetch');
    });
    const push = await service(fake, browser);
    await push.signingOut();
    expect(browser.unsubscribed).toEqual([ENDPOINT]);
    expect(fake.paramsOf('push/remove')).toEqual([]);
  });

  it('gives the server this browser again, quietly, when it was turned on here and the server lost it', async () => {
    localStorage.setItem(ON_HERE, 'true');
    const browser = new FakePushBrowser('granted');
    browser.holding(keyBytes(KEY));
    const fake = new FakeSession()
      .answer('push/get', { push: offered() })
      .answer('push/subscribe', { push: offered([device('d1', 'A browser', true)]) });
    const push = await service(fake, browser);
    await settle();
    expect(fake.paramsOf('push/subscribe')).toHaveLength(1);
    expect(push.on()).toBe(true);
    expect(push.problem()).toBeNull();
  });

  it('gives the server nothing again where the person turned them off, or the server still has it', async () => {
    for (const [on, kept] of [
      ['false', []],
      ['true', [device('d1', 'A browser', true)]],
    ] as const) {
      localStorage.setItem(ON_HERE, on);
      const browser = new FakePushBrowser('granted');
      browser.holding(keyBytes(KEY));
      const fake = new FakeSession().answer('push/get', { push: offered(kept) });
      await service(fake, browser);
      await settle();
      expect(fake.paramsOf('push/subscribe')).toEqual([]);
      TestBed.resetTestingModule();
    }
  });

  it('turned off, this browser is forgotten in every organization of the person’s', async () => {
    const browser = new FakePushBrowser('granted');
    browser.holding(keyBytes(KEY));
    const fake = new FakeSession()
      .answer('organization/list', {
        organizations: [
          { id: HERE, name: 'Personal', own: true, current: true },
          { id: ELSEWHERE, name: 'Acme', own: false },
        ],
      })
      .answer('push/get', { push: offered([device('d1', 'A browser', true)]) })
      .answer('push/remove', { push: offered() });
    // Calls in the other organization reach a server of their own, which keeps this browser.
    const elsewhere = new FakeSession()
      .answer('push/get', { push: offered([device('e1', 'A browser', true)]) })
      .answer('push/remove', { push: offered() });
    const client = (_: unknown, organization: () => string | null): FakeSession =>
      organization() === ELSEWHERE ? elsewhere : fake;
    await pushingTab(fake, browser.browser, true, [
      { provide: SESSION_CLIENT_FACTORY, useValue: client },
    ]);
    const push = TestBed.inject(PushService);
    await push.turnOff();
    expect(browser.unsubscribed).toEqual([ENDPOINT]);
    expect(fake.paramsOf('push/remove')).toEqual([{ device: 'd1' }]);
    expect(elsewhere.paramsOf('push/get')).toEqual([{ endpoint: ENDPOINT }]);
    expect(elsewhere.paramsOf('push/remove')).toEqual([{ device: 'e1' }]);
    expect(localStorage.getItem(ON_HERE)).toBe('false');
  });
});
