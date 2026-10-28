import { BreakpointObserver, type BreakpointState } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import type { Notification, RawEvent, Update } from '@glave/client';
import { BehaviorSubject } from 'rxjs';

import { FakeSession, FakeTurn, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { PanelService } from './panel.service';
import { LIVE_THOUGHT, ThoughtService } from './thought.service';
import { ThreadService } from './thread.service';

/** The model's reply at `seq`, with its reasoning and what the server says it came to. */
function reasoned(seq: number, reasoning: string, ms: number): RawEvent {
  return event(seq, 'message.added', {
    role: 'assistant',
    parts: [
      { type: 'reasoning', text: reasoning },
      { type: 'text', text: 'Done.' },
    ],
    thought: { headline: reasoning, ms },
  });
}

const HISTORY = [
  event(1, 'turn.started', {}),
  event(2, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Stop it' }] }),
  reasoned(3, 'The port is 5173.', 4250),
  reasoned(4, 'It stopped.', 1000),
  event(5, 'turn.ended', { outcome: 'completed' }),
];

function recorded(stored: RawEvent): Notification {
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: 'thread-1', event: stored } };
}

function live(update: Update): Notification {
  return { jsonrpc: '2.0', method: 'turn/live', params: { thread: 'thread-1', update } };
}

function settled(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function make(
  fake: FakeSession,
  narrow = false,
): {
  readonly thoughts: ThoughtService;
  readonly thread: ThreadService;
  readonly panel: PanelService;
} {
  const screen = new BehaviorSubject<BreakpointState>({ matches: narrow, breakpoints: {} });
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake),
      { provide: BreakpointObserver, useValue: { observe: () => screen } },
      ThreadService,
      ThoughtService,
    ],
  });
  return {
    thoughts: TestBed.inject(ThoughtService),
    thread: TestBed.inject(ThreadService),
    panel: TestBed.inject(PanelService),
  };
}

describe('ThoughtService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('shows the reasoning of the line chosen in the panel, and no longer when chosen again', async () => {
    const { thoughts, thread, panel } = make(
      new FakeSession().answer('thread/events', { events: HISTORY }),
    );
    await thread.open('thread-1');
    panel.setShowing(false);
    expect(thoughts.shown()).toBeNull();
    thoughts.choose('message-3');
    expect(panel.showing()).toBe(true);
    expect(thoughts.shown()).toEqual({
      key: 'message-3',
      text: 'The port is 5173.',
      thought: { headline: 'The port is 5173.', ms: 4250 },
    });
    expect(thoughts.selected()).toBe('message-3');
    // Another line takes its place; with the panel hidden, none is selected.
    thoughts.choose('message-4');
    expect(thoughts.selected()).toBe('message-4');
    panel.setShowing(false);
    expect(thoughts.selected()).toBeNull();
    // Chosen with the panel hidden, it shows again; chosen once more, it goes.
    thoughts.choose('message-4');
    expect(thoughts.selected()).toBe('message-4');
    thoughts.choose('message-4');
    expect(thoughts.shown()).toBeNull();
    expect(thoughts.selected()).toBeNull();
  });

  it('opens the sheet on a narrow screen', async () => {
    const { thoughts, thread, panel } = make(
      new FakeSession().answer('thread/events', { events: HISTORY }),
      true,
    );
    await thread.open('thread-1');
    expect(panel.showing()).toBe(false);
    thoughts.choose('message-3');
    expect(panel.showing()).toBe(true);
  });

  it('follows the reasoning being written, then its message once recorded', async () => {
    const turn = new FakeTurn();
    const fake = new FakeSession().answer('thread/events', { events: HISTORY }).turn(turn);
    const { thoughts, thread } = make(fake);
    await thread.open('thread-1');
    const sending = thread.send('And now?');
    turn.send(recorded(event(6, 'turn.started', {})));
    turn.send(live({ kind: 'reasoning', text: '\nChecking again.\n\n' }));
    turn.send(live({ kind: 'thinking', headline: 'Checking again.' }));
    await settled();
    thoughts.choose(LIVE_THOUGHT);
    expect(thoughts.shown()).toEqual({
      key: LIVE_THOUGHT,
      text: 'Checking again.',
      thought: { headline: 'Checking again.', ms: null },
    });
    expect(thoughts.selected()).toBe(LIVE_THOUGHT);
    turn.send(live({ kind: 'thought', ms: 2000 }), recorded(reasoned(7, 'Checking again.', 2000)));
    turn.send(live({ kind: 'waiting' }));
    await settled();
    expect(thoughts.selected()).toBe('message-7');
    turn.send(recorded(event(8, 'turn.ended', { outcome: 'completed' })));
    turn.end();
    await sending;
  });

  it('starts again from nothing in each thread', async () => {
    const fake = new FakeSession().answer('thread/events', { events: HISTORY });
    const { thoughts, thread } = make(fake);
    await thread.open('thread-1');
    thoughts.choose('message-3');
    await thread.open('thread-2');
    expect(thoughts.shown()).toBeNull();
  });
});
