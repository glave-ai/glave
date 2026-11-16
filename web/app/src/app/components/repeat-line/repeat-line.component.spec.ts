import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { schedule } from '../../testing/fixtures';
import { RepeatLineComponent } from './repeat-line.component';

const CHAT = '0192f5c0-0000-7000-8000-0000000000b1';

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}

async function render(
  fake: FakeSession,
  thread: string,
): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<RepeatLineComponent>>;
}> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(RepeatLineComponent);
  fixture.componentRef.setInput('thread', thread);
  await settle();
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function find(root: HTMLElement, testId: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
}

describe('RepeatLineComponent', () => {
  it('says when a schedule’s chat repeats and runs next, and pauses it', async () => {
    const fake = new FakeSession()
      .answer('tasks/list', { tasks: [] })
      .answer('schedule/list', { schedules: [schedule('s1', '2026-10-12T02:00:00.000Z')] });
    const { root } = await render(fake, CHAT);
    const line = find(root, 'thread-repeats')?.textContent ?? '';
    expect(line).toContain('Every day at 2:00 am · Next:');
    await expectAccessible(root);
    find(root, 'thread-repeats-pause')?.click();
    await settle();
    expect(fake.paramsOf('schedule/pause')).toEqual([{ schedule: 's1', paused: true }]);
  });

  it('removes it once the person says yes', async () => {
    const fake = new FakeSession()
      .answer('tasks/list', { tasks: [] })
      .answer('schedule/list', { schedules: [schedule('s1', null)] });
    const { root, fixture } = await render(fake, CHAT);
    find(root, 'thread-repeats-remove')?.click();
    await fixture.whenStable();
    find(document.body, 'thread-repeats-confirm-ok')?.click();
    await settle();
    expect(fake.paramsOf('schedule/delete')).toEqual([{ schedule: 's1' }]);
  });

  it('shows nothing in any other chat', async () => {
    const fake = new FakeSession()
      .answer('tasks/list', { tasks: [] })
      .answer('schedule/list', { schedules: [schedule('s1', null)] });
    const { root } = await render(fake, 'another');
    expect(find(root, 'thread-repeats')).toBeNull();
  });
});
