import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { schedule } from '../../testing/fixtures';
import { RepeatingListComponent } from './repeating-list.component';

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}

async function render(fake: FakeSession): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<RepeatingListComponent>>;
}> {
  TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
  const fixture = TestBed.createComponent(RepeatingListComponent);
  await settle();
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function all(root: HTMLElement, testId: string): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(`[data-testid="${testId}"]`)];
}

describe('RepeatingListComponent', () => {
  it('lists each schedule by name, linked to its chat, with when it repeats and runs', async () => {
    const paused = { ...schedule('s2', null), name: 'Lisbon flight prices', paused: true };
    const ran = {
      ...schedule('s1', '2026-10-12T02:00:00.000Z'),
      last: { status: 'skipped', due: '2026-10-11T02:00:00.000Z' },
    } as const;
    const fake = new FakeSession()
      .answer('tasks/list', { tasks: [] })
      .answer('schedule/list', { schedules: [ran, paused] });
    const { root, fixture } = await render(fake);
    const names = all(root, 'repeating-name');
    expect(names.map((name) => name.textContent.trim())).toEqual([
      'Audit the dependencies',
      'Lisbon flight prices',
    ]);
    expect(names[0]?.getAttribute('href')).toBe('/t/0192f5c0-0000-7000-8000-0000000000b1');
    const lines = all(root, 'repeating-line').map((line) => line.textContent);
    expect(lines[0]).toContain('Every day at 2:00 am · Next:');
    expect(lines[0]).toContain('skipped while the one before was still going');
    expect(lines[1]).toContain('Paused');
    await expectAccessible(root);
    all(root, 'repeating-pause')[0]?.click();
    all(root, 'repeating-resume')[0]?.click();
    await settle();
    expect(fake.paramsOf('schedule/pause')).toEqual([
      { schedule: 's1', paused: true },
      { schedule: 's2', paused: false },
    ]);
    all(root, 'repeating-remove')[1]?.click();
    await fixture.whenStable();
    document.querySelector<HTMLElement>('[data-testid="repeating-confirm-ok"]')?.click();
    await settle();
    expect(fake.paramsOf('schedule/delete')).toEqual([{ schedule: 's2' }]);
  });

  it('shows nothing without schedules', async () => {
    const fake = new FakeSession()
      .answer('tasks/list', { tasks: [] })
      .answer('schedule/list', { schedules: [] });
    const { root } = await render(fake);
    expect(all(root, 'repeating')).toHaveLength(0);
  });
});
