import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession } from '../../testing/fake-session';
import { ENDPOINT, FakePushBrowser, device, offered, pushingTab } from '../../testing/push';
import { settle } from '../../testing/links';
import { PushBannerComponent } from './push-banner.component';

async function render(
  browser: FakePushBrowser | null,
  pushes = true,
  fake = new FakeSession(),
): Promise<{
  readonly fixture: ComponentFixture<PushBannerComponent>;
  readonly root: HTMLElement;
}> {
  await pushingTab(fake, browser?.browser ?? null, pushes);
  const fixture = TestBed.createComponent(PushBannerComponent);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement };
}

function find(root: ParentNode, testId: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
}

/** Whether the banner is dismissed: there, unseen, and out of reach. */
function gone(root: ParentNode): boolean {
  const banner = find(root, 'push-banner');
  return banner !== null && getComputedStyle(banner).visibility === 'hidden' && banner.inert;
}

describe('PushBannerComponent', () => {
  afterEach(() => {
    localStorage.clear();
  });

  it('offers notifications where the server pushes and this browser can, asking nothing yet', async () => {
    const browser = new FakePushBrowser('default');
    const { root } = await render(browser);
    expect(find(root, 'push-banner')?.textContent).toContain(
      'Get a notification when glave needs you, even with this tab closed.',
    );
    expect(find(root, 'push-banner-on')?.textContent.trim()).toBe('Turn on');
    expect(find(root, 'push-banner-not-now')?.textContent.trim()).toBe('Not now');
    expect(browser.asked).toEqual([]);
    await expectAccessible(root);
  });

  it('keeps its room once dismissed, unseen, so nothing under it moves', async () => {
    // How it lies on the page the end-to-end test measures; here, what keeps it in its place.
    const { fixture, root } = await render(new FakePushBrowser('default'));
    const room = root.querySelector<HTMLElement>('.room');
    expect(getComputedStyle(room ?? root).position).toBe('sticky');
    find(root, 'push-banner-not-now')?.click();
    await settle(fixture);
    expect(root.querySelector('.room')).toBe(room);
    expect(gone(root)).toBe(true);
  });

  it('remembers Not now in this browser', async () => {
    const { fixture, root } = await render(new FakePushBrowser('default'));
    find(root, 'push-banner-not-now')?.click();
    await settle(fixture);
    expect(gone(root)).toBe(true);
    TestBed.resetTestingModule();
    const again = await render(new FakePushBrowser('default'));
    expect(find(again.root, 'push-banner')).toBeNull();
  });

  it('turns them on from the click: the browser asks, then this browser is subscribed', async () => {
    const browser = new FakePushBrowser('default');
    const fake = new FakeSession()
      .answer('push/get', { push: offered() })
      .answer('push/subscribe', { push: offered([device('d1', 'A browser', true)]) });
    const { fixture, root } = await render(browser, true, fake);
    find(root, 'push-banner-on')?.click();
    expect(browser.asked).toEqual(['asked']);
    await settle(fixture);
    expect(browser.registered).toEqual(['/push-worker.js?fallback=glave%20needs%20you']);
    expect(browser.made.map((options) => options.userVisibleOnly)).toEqual([true]);
    expect(fake.paramsOf('push/subscribe')).toEqual([
      { endpoint: ENDPOINT, p256dh: 'browser-key', auth: 'browser-auth', device: 'A browser' },
    ]);
    expect(gone(root)).toBe(true);
  });

  it('goes once the person says no, subscribing nothing', async () => {
    const browser = new FakePushBrowser('default', 'denied');
    const fake = new FakeSession();
    const { fixture, root } = await render(browser, true, fake);
    find(root, 'push-banner-on')?.click();
    await settle(fixture);
    expect(gone(root)).toBe(true);
    expect(browser.registered).toEqual([]);
    expect(fake.paramsOf('push/subscribe')).toEqual([]);
  });

  it('says what went wrong when this browser could not subscribe', async () => {
    const browser = new FakePushBrowser('default');
    const fake = new FakeSession().answer('push/get', { push: offered() });
    browser.browser.workers.register = () => Promise.reject(new Error('refused'));
    const { fixture, root } = await render(browser, true, fake);
    find(root, 'push-banner-on')?.click();
    await settle(fixture);
    expect(find(root, 'push-banner-problem')?.textContent).toContain(
      'This browser could not turn notifications on.',
    );
    await expectAccessible(root);
  });

  it('draws nothing where the server pushes none, the browser takes none, or the person decided', async () => {
    for (const [browser, pushes] of [
      [new FakePushBrowser('default'), false],
      [null, true],
      [new FakePushBrowser('granted'), true],
      [new FakePushBrowser('denied'), true],
    ] as const) {
      const { root } = await render(browser, pushes);
      expect(find(root, 'push-banner')).toBeNull();
      TestBed.resetTestingModule();
    }
  });
});
