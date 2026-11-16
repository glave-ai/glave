import { LiveAnnouncer } from '@angular/cdk/a11y';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type ScheduleInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { READER_ZONE } from '../../services/zone.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { schedule } from '../../testing/fixtures';
import { RepeatingListComponent } from './repeating-list.component';

/** A schedule that ran last night and runs again the night after next. */
const RAN: ScheduleInfo = {
  ...schedule('s1', '2030-11-18T02:00:00.000Z'),
  last: { due: '2030-11-16T02:00:00.000Z', status: 'ended', outcome: 'completed', said: 'Done' },
};
const PAUSED: ScheduleInfo = {
  ...schedule('s2', null),
  name: 'Lisbon flight prices',
  thread: null,
  paused: true,
};

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}

async function render(
  fake: FakeSession,
  schedules = true,
): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ComponentFixture<RepeatingListComponent>;
}> {
  fake.answer('server/discover', { schedules }).answer('tasks/list', { tasks: [] });
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake),
      { provide: READER_ZONE, useValue: 'Europe/Lisbon' },
      { provide: LiveAnnouncer, useValue: { announce: () => Promise.resolve() } },
    ],
  });
  const fixture = TestBed.createComponent(RepeatingListComponent);
  await settle();
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function all(root: ParentNode, testId: string): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(`[data-testid="${testId}"]`)];
}

describe('RepeatingListComponent', () => {
  it('lists each schedule by name, opening its chat, with when it repeats, runs and ran', async () => {
    const fake = new FakeSession().answer('schedule/list', { schedules: [RAN, PAUSED] });
    const { root } = await render(fake);
    expect(root.querySelector('h2')?.textContent).toBe('Repeating');
    const names = all(root, 'repeating-name');
    expect(names.map((name) => name.textContent.trim())).toEqual([
      'Audit the dependencies',
      'Lisbon flight prices',
    ]);
    expect(names[0]?.getAttribute('href')).toBe('/t/0192f5c0-0000-7000-8000-0000000000b1');
    // One made before schedules had chats has no chat to open.
    expect(names[1]?.tagName).toBe('SPAN');
    const lines = all(root, 'repeating-line').map((line) => line.textContent);
    expect(lines[0]).toMatch(
      /^Every day at 2:00 am · Next: .*2:00 AM · Last: .*2:00 AM \(Done\)$/u,
    );
    expect(lines[1]).toBe('Every day at 2:00 am · Paused');
    expect(all(root, 'repeating-pause').map((each) => each.textContent.trim())).toEqual([
      'Pause',
      'Resume',
    ]);
    await expectAccessible(root);
  });

  it('keeps a row as the server answers its change, and says in its words what it refused', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [RAN, PAUSED] })
      .answer('schedule/pause', { schedule: { ...RAN, paused: true } });
    const { root, fixture } = await render(fake);
    all(root, 'repeating-pause')[0]?.click();
    await settle();
    await fixture.whenStable();
    expect(fake.paramsOf('schedule/pause')).toEqual([{ schedule: 's1', paused: true }]);
    expect(all(root, 'repeating-line')[0]?.textContent).toMatch(/· Paused ·/u);
    fake.answer(
      'schedule/pause',
      new ApiError({ code: ErrorCode.NO_SCHEDULE, message: 'no such schedule' }),
    );
    all(root, 'repeating-pause')[1]?.click();
    await settle();
    await fixture.whenStable();
    const errors = all(root, 'repeating-error');
    expect(errors.map((error) => [error.textContent, error.getAttribute('role')])).toEqual([
      ['no such schedule', 'alert'],
    ]);
    expect(all(root, 'repeating-schedule')[1]?.contains(errors[0] ?? null)).toBe(true);
    await expectAccessible(root);
  });

  it('removes one once the person says yes, the keyboard on the list’s heading', async () => {
    const fake = new FakeSession().answer('schedule/list', { schedules: [RAN] });
    const { root, fixture } = await render(fake);
    all(root, 'repeating-remove')[0]?.click();
    await fixture.whenStable();
    all(root, 'repeating-confirm-ok')[0]?.click();
    await settle();
    await fixture.whenStable();
    expect(fake.paramsOf('schedule/delete')).toEqual([{ schedule: 's1' }]);
    expect(all(root, 'repeating-schedule')).toHaveLength(0);
    expect(all(root, 'repeating-none')[0]?.textContent).toBe('Nothing repeats now.');
    expect(document.activeElement).toBe(root.querySelector('h2'));
  });

  it('shows nothing without schedules, or where schedules do not run', async () => {
    const none = await render(new FakeSession().answer('schedule/list', { schedules: [] }));
    expect(all(none.root, 'repeating')).toHaveLength(0);
    TestBed.resetTestingModule();
    const there = new FakeSession().answer('schedule/list', { schedules: [RAN] });
    const off = await render(there, false);
    expect(all(off.root, 'repeating')).toHaveLength(0);
  });
});
