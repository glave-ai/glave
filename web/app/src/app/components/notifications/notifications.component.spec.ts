import { type ComponentFixture, TestBed } from '@angular/core/testing';
import type { PushInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession } from '../../testing/fake-session';
import { ENDPOINT, FakePushBrowser, device, offered, pushingTab } from '../../testing/push';
import { settle } from '../../testing/links';
import { NotificationsComponent } from './notifications.component';

const IPHONE_SAFARI =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1';
const SAFARI_MAC =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15';

async function render(
  fake: FakeSession,
  browser: FakePushBrowser | null,
  pushes = true,
): Promise<{
  readonly fixture: ComponentFixture<NotificationsComponent>;
  readonly root: HTMLElement;
}> {
  await pushingTab(fake, browser?.browser ?? null, pushes);
  const fixture = TestBed.createComponent(NotificationsComponent);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement };
}

function find<T extends HTMLElement = HTMLElement>(root: ParentNode, testId: string): T | null {
  return root.querySelector<T>(`[data-testid="${testId}"]`);
}

/** What each row of `testId` says, its lines and its button's word in order. */
function texts(root: ParentNode, testId: string): string[] {
  return [...root.querySelectorAll(`[data-testid="${testId}"]`)].map((each) =>
    [...each.querySelectorAll('p, button')].map((part) => part.textContent.trim()).join(' '),
  );
}

/** A server that keeps what it is told, starting from `info`. */
function keeping(start: PushInfo): FakeSession {
  let info = start;
  const answer = (): object => ({ push: info });
  return new FakeSession()
    .answer('push/get', answer)
    .answer('push/remove', (params) => {
      const { device: removed } = params as { device: string };
      if (info.kind === 'offered') {
        info = { ...info, devices: info.devices.filter((each) => each.id !== removed) };
      }
      return answer();
    })
    .answer('push/subscribe', () => {
      info = offered([device('d9', 'A browser', true)]);
      return answer();
    });
}

describe('NotificationsComponent', () => {
  afterEach(() => {
    localStorage.clear();
    vi.restoreAllMocks();
  });

  it('shows this browser off, and the person’s browsers', async () => {
    const fake = keeping(offered([device('d1', 'Safari on iPhone')]));
    const { root } = await render(fake, new FakePushBrowser('default'));
    expect(find(root, 'notifications-state')?.textContent).toBe(
      'This browser doesn’t get notifications.',
    );
    expect(find(root, 'notifications-switch')?.textContent.trim()).toBe('Turn on');
    expect(texts(root, 'notifications-device')).toEqual([
      'Safari on iPhone Added Nov 7, 2026 Remove',
    ]);
    await expectAccessible(root);
  });

  it('turns this browser on and off', async () => {
    const fake = keeping(offered());
    const browser = new FakePushBrowser('default');
    const { fixture, root } = await render(fake, browser);
    find(root, 'notifications-switch')?.click();
    await settle(fixture);
    expect(browser.asked).toEqual(['asked']);
    expect(fake.paramsOf('push/subscribe')).toEqual([
      { endpoint: ENDPOINT, p256dh: 'browser-key', auth: 'browser-auth', device: 'A browser' },
    ]);
    expect(find(root, 'notifications-state')?.textContent).toBe('This browser gets notifications.');
    expect(texts(root, 'notifications-device')[0]).toContain('This browser');
    find(root, 'notifications-switch')?.click();
    await settle(fixture);
    expect(browser.unsubscribed).toEqual([ENDPOINT]);
    expect(fake.paramsOf('push/remove')).toEqual([{ device: 'd9' }]);
    expect(find(root, 'notifications-state')?.textContent).toBe(
      'This browser doesn’t get notifications.',
    );
  });

  it('forgets a browser the person removes', async () => {
    const fake = keeping(offered([device('d1', 'Old laptop')]));
    const { fixture, root } = await render(fake, new FakePushBrowser('granted'));
    expect(find(root, 'notifications-remove')?.getAttribute('aria-label')).toBe(
      'Remove Old laptop',
    );
    find(root, 'notifications-remove')?.click();
    await settle(fixture);
    expect(fake.paramsOf('push/remove')).toEqual([{ device: 'd1' }]);
    expect(texts(root, 'notifications-device')).toEqual([]);
  });

  it('says why this browser can’t take them: blocked, none, or an iPhone or iPad outside the Home Screen', async () => {
    const blocked = await render(keeping(offered()), new FakePushBrowser('denied'));
    expect(find(blocked.root, 'notifications-blocked')?.textContent).toContain('blocked');
    expect(find(blocked.root, 'notifications-switch')).toBeNull();
    await expectAccessible(blocked.root);
    TestBed.resetTestingModule();
    const none = await render(keeping(offered()), null);
    expect(find(none.root, 'notifications-unsupported')).not.toBeNull();
    TestBed.resetTestingModule();
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(IPHONE_SAFARI);
    const phone = await render(keeping(offered()), null);
    expect(find(phone.root, 'notifications-home-screen')?.textContent).toContain('Home Screen');
    TestBed.resetTestingModule();
    // An iPad says it is a Mac, on a touch screen.
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(SAFARI_MAC);
    Object.defineProperty(navigator, 'maxTouchPoints', { configurable: true, get: () => 5 });
    onTestFinished(() => {
      Reflect.deleteProperty(navigator, 'maxTouchPoints');
    });
    const tablet = await render(keeping(offered()), null);
    expect(find(tablet.root, 'notifications-home-screen')).not.toBeNull();
  });

  it('draws nothing where the server pushes none', async () => {
    const { root } = await render(new FakeSession(), new FakePushBrowser('default'), false);
    expect(find(root, 'notifications')).toBeNull();
    expect(getComputedStyle(root).display).toBe('contents');
  });
});
