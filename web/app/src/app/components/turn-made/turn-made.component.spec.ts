import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { MadeFile } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { TurnMadeComponent } from './turn-made.component';

const PLAN: MadeFile = {
  kind: 'pdf',
  name: 'Lisbon plan',
  file_name: 'Lisbon plan.pdf',
  media_type: 'application/pdf',
  bytes: 619,
  sha256: 'c'.repeat(64),
  parts: '1 page',
  previews: true,
  kept: 'Kept with this chat',
  sentence: 'Made a PDF: Lisbon plan (1 page)',
};

/** The files `files` drawn, unseen while only `room`, with `note` under them. */
async function render(
  files: readonly MadeFile[],
  room: boolean,
  note: string | null,
): Promise<HTMLElement> {
  // The chat's panel as the cards see it: nothing open in it.
  const panel = { file: signal(null), visible: signal(false), openFile: () => undefined };
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(new FakeSession()),
      ThreadService,
      { provide: ThreadPanelService, useValue: panel },
    ],
  });
  const fixture = TestBed.createComponent(TurnMadeComponent);
  fixture.componentRef.setInput('files', files);
  fixture.componentRef.setInput('room', room);
  fixture.componentRef.setInput('note', note);
  await settle(fixture);
  return fixture.nativeElement as HTMLElement;
}

describe('TurnMadeComponent', () => {
  it('draws a card for each file, then what they leave out in the server’s words', async () => {
    const note = '1 more file was made than a chat keeps: it stays on the computer';
    const root = await render([PLAN], false, note);
    const cards = root.querySelectorAll('[data-testid="made-file"]');
    expect(cards).toHaveLength(1);
    expect(root.querySelector('[data-testid="turn-made-note"]')?.textContent).toBe(note);
    await expectAccessible(root);
  });

  it('keeps each card’s room, unseen, and says nothing when nothing is left out', async () => {
    const root = await render([PLAN], true, null);
    expect(root.querySelector('app-made-file')?.classList.contains('room')).toBe(true);
    expect(root.querySelector('[data-testid="turn-made-note"]')).toBeNull();
  });
});
