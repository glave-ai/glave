import { LiveAnnouncer } from '@angular/cdk/a11y';
import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type ScheduleInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { fakeMedia } from '@glave/glaze/testing/media';

import { NavService } from '../../services/nav.service';
import { PanelService } from '../../services/panel.service';
import { READER_ZONE } from '../../services/zone.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { schedule } from '../../testing/fixtures';
import { ThreadHeaderComponent } from './thread-header.component';

interface Options {
  readonly narrow?: boolean;
  readonly project?: string | null;
  readonly panel?: boolean;
  readonly schedule?: ScheduleInfo;
  readonly fake?: FakeSession;
}

/** A schedule's chat's schedule, which runs again in years to come. */
const REPEATS = schedule('s1', '2030-11-18T02:00:00.000Z');

async function render(options: Options = {}): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<ThreadHeaderComponent>>;
  readonly renamed: () => number;
}> {
  localStorage.clear();
  fakeMedia(options.narrow ?? false);
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(options.fake ?? new FakeSession()),
      { provide: READER_ZONE, useValue: 'Europe/Lisbon' },
      { provide: LiveAnnouncer, useValue: { announce: () => Promise.resolve() } },
    ],
  });
  const fixture = TestBed.createComponent(ThreadHeaderComponent);
  fixture.componentRef.setInput('title', 'Fix the flaky test');
  fixture.componentRef.setInput('project', 'project' in options ? options.project : 'glave');
  fixture.componentRef.setInput('schedule', options.schedule ?? null);
  fixture.componentRef.setInput('panel', options.panel ?? true);
  let renames = 0;
  fixture.componentInstance.rename.subscribe(() => (renames += 1));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture, renamed: () => renames };
}

function find(root: HTMLElement, testId: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
}

describe('ThreadHeaderComponent', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('shows the title and its project, and renames from the title', async () => {
    const { root, renamed } = await render();
    expect(root.querySelector('h1[data-testid="thread-title"]')?.textContent.trim()).toBe(
      'Fix the flaky test',
    );
    expect(find(root, 'thread-meta')?.textContent.trim()).toBe('glave');
    // No loud status: the side nav and the card in the thread say what needs the person.
    expect(root.querySelector('glz-badge')).toBeNull();
    expect(find(root, 'page-menu')).toBeNull();
    await expectAccessible(root);
    find(root, 'thread-rename')?.click();
    expect(renamed()).toBe(1);
  });

  it('says no project for a chat', async () => {
    const { root } = await render({ project: null });
    expect(find(root, 'thread-meta')).toBeNull();
  });

  it('shows and hides the thread’s panel from a pressed toggle, and has none without one', async () => {
    const { root } = await render();
    const button = find(root, 'thread-panel');
    const panel = TestBed.inject(PanelService);
    expect(button?.getAttribute('aria-label')).toBe("Show or hide the chat's panel");
    expect(button?.getAttribute('aria-pressed')).toBe('true');
    expect(button?.hasAttribute('aria-expanded')).toBe(false);
    expect(button?.getAttribute('aria-keyshortcuts')).toMatch(/^(Meta|Control)\+\.$/u);
    expect(button?.classList).not.toContain('touch');
    button?.click();
    expect(panel.showing()).toBe(false);
    TestBed.resetTestingModule();
    const chat = await render({ panel: false });
    expect(find(chat.root, 'thread-panel')).toBeNull();
  });

  it('on a phone, opens the side bar and the panel’s sheet from buttons a finger’s size', async () => {
    const { root, fixture } = await render({ narrow: true });
    const menu = find(root, 'page-menu');
    const button = find(root, 'thread-panel');
    expect(menu?.classList).toContain('touch');
    expect(button?.classList).toContain('touch');
    // It opens a sheet over the page: expanded or not, never pressed.
    expect(button?.getAttribute('aria-expanded')).toBe('false');
    expect(button?.hasAttribute('aria-pressed')).toBe(false);
    await expectAccessible(root);
    const nav = TestBed.inject(NavService);
    const wasOpen = nav.open();
    menu?.click();
    expect(nav.open()).toBe(!wasOpen);
    button?.click();
    await fixture.whenStable();
    expect(TestBed.inject(PanelService).showing()).toBe(true);
    expect(button?.getAttribute('aria-expanded')).toBe('true');
  });

  it('says when a schedule’s chat repeats and runs next, and pauses it, in its line', async () => {
    const fake = new FakeSession().answer('schedule/pause', {
      schedule: { ...REPEATS, paused: true },
    });
    const { root, fixture } = await render({ project: null, schedule: REPEATS, fake });
    const meta = find(root, 'thread-meta');
    expect(find(root, 'thread-repeats')?.textContent.trim()).toMatch(
      /^Every day at 2:00 am · Next: .*2:00 AM$/u,
    );
    // Its actions sit in the line, beside a wide page.
    expect(meta?.contains(find(root, 'thread-repeats-pause'))).toBe(true);
    expect(find(root, 'thread-repeats-menu-trigger')).toBeNull();
    await expectAccessible(root);
    find(root, 'thread-repeats-pause')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(fake.paramsOf('schedule/pause')).toEqual([{ schedule: 's1', paused: true }]);
    // The answer shows at once; the summary the server sends next is what stays.
    expect(find(root, 'thread-repeats')?.textContent.trim()).toBe('Every day at 2:00 am · Paused');
    fixture.componentRef.setInput('schedule', { ...REPEATS });
    await fixture.whenStable();
    expect(find(root, 'thread-repeats')?.textContent).toContain('Next:');
  });

  it('says in the server’s words what it refused a change with, under the bar', async () => {
    const fake = new FakeSession().answer(
      'schedule/pause',
      new ApiError({ code: ErrorCode.NO_SCHEDULE, message: 'no such schedule' }),
    );
    const { root, fixture } = await render({ schedule: REPEATS, fake });
    find(root, 'thread-repeats-pause')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    const error = find(root, 'thread-repeats-error');
    expect(error?.getAttribute('role')).toBe('alert');
    expect(error?.textContent).toBe('no such schedule');
    expect(root.querySelector('header')?.contains(error)).toBe(false);
    await expectAccessible(root);
  });

  it('takes the line away once the schedule is removed, the keyboard on the title', async () => {
    const { root, fixture } = await render({ schedule: REPEATS });
    find(root, 'thread-repeats-remove')?.click();
    await fixture.whenStable();
    root.querySelector<HTMLElement>('[data-testid="thread-repeats-confirm-ok"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(find(root, 'thread-repeats')).toBeNull();
    expect(find(root, 'thread-meta')?.textContent.trim()).toBe('glave');
    expect(document.activeElement).toBe(find(root, 'thread-title'));
  });

  it('on a phone, says only when it repeats, its actions and the panel under a ⋯ in the panel’s place', async () => {
    const { root, fixture } = await render({ narrow: true, schedule: REPEATS });
    expect(find(root, 'thread-repeats')?.textContent.trim()).toBe('Every day at 2:00 am');
    expect(find(root, 'thread-repeats-pause')).toBeNull();
    expect(find(root, 'thread-panel')).toBeNull();
    const trigger = find(root, 'thread-repeats-menu-trigger');
    expect(find(root, 'thread-meta')?.contains(trigger)).toBe(false);
    expect(trigger?.getAttribute('aria-label')).toBe('Options for Audit the dependencies');
    await expectAccessible(root);
    trigger?.click();
    await fixture.whenStable();
    const items = [
      ...document.querySelectorAll<HTMLElement>('[data-testid="thread-repeats-menu-action"]'),
    ];
    expect(items.map((item) => item.textContent.trim())).toEqual([
      'Pause',
      'Remove',
      "Show or hide the chat's panel",
    ]);
    const panel = TestBed.inject(PanelService);
    const before = panel.showing();
    items[2]?.click();
    await fixture.whenStable();
    expect(panel.showing()).toBe(!before);
  });
});
