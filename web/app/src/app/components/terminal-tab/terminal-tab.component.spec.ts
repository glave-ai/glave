import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { TERMINAL_EMULATOR } from '../terminal-screen/terminal-screen.component';
import { TerminalTabComponent } from './terminal-tab.component';

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round += 1) {
    TestBed.tick();
    await new Promise((resolve) => setTimeout(resolve));
  }
}

function texts(root: HTMLElement, testId: string): readonly string[] {
  return Array.from(root.querySelectorAll(`[data-testid="${testId}"]`)).map((each) =>
    each.textContent.trim(),
  );
}

async function render(fake: FakeSession): Promise<HTMLElement> {
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake.answer('thread/events', { events: [] })),
      ThreadService,
      // No emulator loads: the screen's own tests draw one.
      { provide: TERMINAL_EMULATOR, useValue: () => new Promise(() => undefined) },
    ],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(TerminalTabComponent);
  await settle();
  return fixture.nativeElement as HTMLElement;
}

describe('TerminalTabComponent', () => {
  it('opens the person’s own shell in the thread’s folder, says whose it is, and opens another', async () => {
    let opened = 0;
    const fake = new FakeSession()
      .answer('terminal/list', { terminals: [] })
      .answer('terminal/open', () => {
        opened += 1;
        return { terminal: { id: `t${String(opened)}`, folder: '/work/glave' } };
      })
      .answer('terminal/close', {});
    const root = await render(fake);
    expect(texts(root, 'terminal-folder')).toEqual(['/work/glave']);
    expect(texts(root, 'terminal-who')).toEqual(["Your shell, not the agent's"]);
    expect(root.querySelectorAll('app-terminal-screen')).toHaveLength(1);
    expect(root.querySelector('[data-testid="terminal-choices"]')).toBeNull();
    await expectAccessible(root);

    root.querySelector<HTMLButtonElement>('[data-testid="terminal-new"]')?.click();
    await settle();
    expect(texts(root, 'terminal-choices-label')).toEqual(['Terminals']);
    expect(texts(root, 'terminal-choices-option')).toEqual(['1', '2']);
    const choices = Array.from(
      root.querySelectorAll<HTMLInputElement>('[data-testid="terminal-choices-option"] input'),
    );
    expect(choices.map((choice) => choice.checked)).toEqual([false, true]);
    await expectAccessible(root);
    choices[0]?.click();
    await settle();
    expect(choices.map((choice) => choice.checked)).toEqual([true, false]);
    root.querySelector<HTMLButtonElement>('[data-testid="terminal-close"]')?.click();
    await settle();
    const closed = fake.calls.filter((call) => call.method === 'terminal/close');
    expect(closed.map((call) => call.params)).toEqual([{ thread: 'thread-1', terminal: 't1' }]);
    expect(root.querySelector('[data-testid="terminal-choices"]')).toBeNull();
  });

  it('says why no terminal opens, in the server’s words', async () => {
    const fake = new FakeSession()
      .answer('terminal/list', { terminals: [] })
      .answer('terminal/open', () => {
        throw new Error('unreachable');
      });
    const root = await render(fake);
    expect(texts(root, 'terminal-problem')).toEqual([
      'Something went wrong. Try again in a moment.',
    ]);
    expect(root.querySelector('app-terminal-screen')).toBeNull();
    await expectAccessible(root);
  });
});
