import { LiveAnnouncer } from '@angular/cdk/a11y';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type ScheduleInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { ProjectsService } from '../../services/projects.service';
import { READER_ZONE } from '../../services/zone.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { record, schedule, thread } from '../../testing/fixtures';
import { RepeatingListComponent } from './repeating-list.component';

/** A schedule that ran last night and runs again the night after next. */
const RAN: ScheduleInfo = {
  ...schedule('s1', '2030-11-18T02:00:00.000Z'),
  last: { due: '2030-11-16T02:00:00.000Z', status: 'ended', outcome: 'completed', said: 'Done' },
};
const PAUSED: ScheduleInfo = {
  ...schedule('s2', null),
  name: 'Lisbon flight prices',
  created_at: '2026-10-12T10:00:00.000Z',
  thread: null,
  paused: true,
};

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}

/** The list, its calls answered by `fake`, the side nav's lists read first when `listed`. */
async function render(
  fake: FakeSession,
  listed = false,
): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ComponentFixture<RepeatingListComponent>;
}> {
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake),
      { provide: READER_ZONE, useValue: 'Europe/Lisbon' },
      { provide: LiveAnnouncer, useValue: { announce: () => Promise.resolve() } },
    ],
  });
  if (listed) {
    await TestBed.inject(ProjectsService).refresh();
  }
  const fixture = TestBed.createComponent(RepeatingListComponent);
  await settle();
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function all(root: ParentNode, testId: string): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(`[data-testid="${testId}"]`)];
}

/** Each row's parts of when it runs, as it shows them. */
function lines(root: HTMLElement): readonly (readonly string[])[] {
  return all(root, 'repeating-line').map((line) =>
    all(line, 'repeating-part').map((part) => part.textContent),
  );
}

describe('RepeatingListComponent', () => {
  it('lists each schedule by name, opening its chat, with when it repeats, runs and ran', async () => {
    const fake = new FakeSession().answer('schedule/list', { schedules: [PAUSED, RAN] });
    const { root } = await render(fake);
    expect(all(root, 'repeating-heading')[0]?.textContent.trim()).toBe('Repeating');
    const names = all(root, 'repeating-name');
    // Oldest first, whatever order the server lists them in.
    expect(names.map((name) => name.textContent.trim())).toEqual([
      'Audit the dependencies',
      'Lisbon flight prices',
    ]);
    expect(names[0]?.getAttribute('href')).toBe('/t/0192f5c0-0000-7000-8000-0000000000b1');
    // One made before schedules had chats has no chat to open.
    expect(names[1]?.tagName).toBe('SPAN');
    const [ran, paused] = lines(root);
    expect(ran?.[0]).toBe('Every day at 2:00 am');
    expect(ran?.[1]).toMatch(/^Next: .*2:00 AM$/u);
    expect(ran?.[2]).toMatch(/^Last: .*2:00 AM, done$/u);
    expect(paused).toEqual(['Every day at 2:00 am', 'Paused']);
    expect(all(root, 'repeating-pause').map((each) => each.textContent.trim())).toEqual([
      'Pause',
      'Resume',
    ]);
    await expectAccessible(root);
  });

  it('opens with the schedules the side nav’s chats carry, until its own list is read', async () => {
    const fake = new FakeSession()
      .answer('project/list', { projects: [record('p1', 'glave')] })
      .answer('thread/list', { threads: [thread('t1', 'Audit', { schedule: RAN })] });
    const answer = fake.held('schedule/list');
    const { root, fixture } = await render(fake, true);
    expect(all(root, 'repeating-name').map((name) => name.textContent.trim())).toEqual([
      'Audit the dependencies',
    ]);
    answer({ schedules: [RAN, PAUSED] });
    await settle();
    await fixture.whenStable();
    expect(all(root, 'repeating-schedule')).toHaveLength(2);
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
    expect(lines(root)[0]?.[1]).toBe('Paused');
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
    expect(document.activeElement).toBe(all(root, 'repeating-heading')[0]);
  });

  it('shows nothing without schedules', async () => {
    const { root } = await render(new FakeSession().answer('schedule/list', { schedules: [] }));
    expect(all(root, 'repeating')).toHaveLength(0);
  });
});
