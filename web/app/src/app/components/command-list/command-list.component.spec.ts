import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { TICK_MS } from '../../services/clock';
import { CommandsService } from '../../services/commands.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { command, event } from '../../testing/fixtures';
import { CommandListComponent } from './command-list.component';

const DEV = 'npm run dev -- --port 4200 --host 127.0.0.1 --open';

async function render(fake: FakeSession): Promise<HTMLElement> {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), ThreadService, CommandsService],
  });
  fake.answer('thread/events', {
    events: [event(1, 'command.started', { command: 'c1', script: DEV })],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(CommandListComponent);
  await settle();
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
}

function all(root: HTMLElement, testId: string): readonly HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(`[data-testid="${testId}"]`));
}

describe('CommandListComponent', () => {
  beforeEach(() => {
    localStorage.clear();
    vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
    vi.setSystemTime(Date.parse('2026-10-11T10:02:05.000Z'));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('lists what runs, how long it has run, ticking, and stops one', async () => {
    let answer: (value: object) => void = () => undefined;
    const fake = new FakeSession()
      .answer('command/list', {
        commands: [command('c1', DEV), command('c2', 'cargo watch', { started_at: null })],
      })
      .answer(
        'command/stop',
        () =>
          new Promise((resolve) => {
            answer = resolve;
          }),
      );
    const root = await render(fake);
    const text = all(root, 'command-text');
    expect(text.map((each) => each.textContent)).toEqual([DEV, 'cargo watch']);
    // One line; it opens to the whole command and the server's sentence about it.
    const more = all(root, 'command-more')[0] as HTMLDetailsElement | undefined;
    expect(more?.open).toBe(false);
    expect(more?.querySelector('summary')?.contains(text[0] ?? null)).toBe(true);
    expect(more?.querySelector('.whole')?.textContent).toBe(DEV);
    expect(all(root, 'command-summary')[0]?.textContent).toContain('Still running');
    // Only a recorded start says how long it has run.
    expect(all(root, 'command-time').map((each) => each.textContent)).toEqual(['2m 05s']);
    vi.advanceTimersByTime(TICK_MS);
    await settle();
    expect(all(root, 'command-time')[0]?.textContent).toBe('2m 06s');
    await expectAccessible(root);

    const stop = all(root, 'command-stop')[0];
    expect(stop?.getAttribute('aria-label')).toBe(`Stop ${DEV}`);
    // A quiet button, no colour of danger: stopping destroys nothing.
    expect(stop?.className).not.toContain('danger');
    stop?.click();
    await settle();
    expect(stop?.getAttribute('aria-label')).toBe(`Stopping ${DEV}…`);
    expect((stop as HTMLButtonElement | undefined)?.disabled).toBe(true);
    expect(all(root, 'command')[0]?.getAttribute('aria-busy')).toBe('true');
    await expectAccessible(root);
    answer({});
    await settle();
    expect(all(root, 'command-text').map((each) => each.textContent)).toEqual(['cargo watch']);
    expect(fake.paramsOf('command/stop')).toEqual([{ thread: 'thread-1', command: 'c1' }]);
  });

  it("shows a refusal on its row in the server's words", async () => {
    const fake = new FakeSession()
      .answer('command/list', { commands: [command('c1', DEV)] })
      .answer(
        'command/stop',
        new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'you may not stop it' }),
      );
    const root = await render(fake);
    all(root, 'command-stop')[0]?.click();
    await settle();
    expect(all(root, 'command-refusal').map((each) => each.textContent)).toEqual([
      'You may not stop it.',
    ]);
    expect(all(root, 'command-refusal')[0]?.getAttribute('role')).toBe('alert');
    expect(all(root, 'command')).toHaveLength(1);
    await expectAccessible(root);
  });

  it('says why it could not read them, in plain words', async () => {
    const fake = new FakeSession().answer('command/list', { commands: [] });
    const root = await render(fake);
    expect(all(root, 'command')).toHaveLength(0);
    fake.answer(
      'command/list',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'the computer did not answer' }),
    );
    await TestBed.inject(CommandsService).refresh();
    await settle();
    expect(all(root, 'commands-error')[0]?.textContent).toBe('The computer did not answer.');
    await expectAccessible(root);
  });
});
