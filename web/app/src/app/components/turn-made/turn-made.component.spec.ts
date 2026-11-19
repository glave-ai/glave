import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { MadeFile } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import { type MadeShown, NO_MADE, madeCards } from '../../lib/made';
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

/** What a turn shows of the files it made, drawn. */
async function render(made: MadeShown): Promise<HTMLElement> {
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
  fixture.componentRef.setInput('made', made);
  await settle(fixture);
  return fixture.nativeElement as HTMLElement;
}

describe('TurnMadeComponent', () => {
  it('draws a card for each file, then what they leave out in the server’s words', async () => {
    const note = '1 more file was made than a chat keeps: it stays on the computer';
    const root = await render({ ...NO_MADE, files: madeCards([PLAN]), note });
    const cards = root.querySelectorAll('[data-testid="made-file"]');
    expect(cards).toHaveLength(1);
    expect(root.querySelector('app-made-file')?.classList.contains('room')).toBe(false);
    expect(root.querySelector('[data-testid="turn-made-note"]')?.textContent).toBe(note);
    await expectAccessible(root);
  });

  it('keeps the room of the cards and the note not described yet, unseen, after those drawn', async () => {
    const root = await render({
      files: madeCards([PLAN]),
      room: madeCards([{ ...PLAN, file_name: 'deck.pptx' }]),
      note: ' ',
      noteRoom: true,
    });
    const rooms = [...root.querySelectorAll('app-made-file')].map((card) =>
      card.classList.contains('room'),
    );
    expect(rooms).toEqual([false, true]);
    expect(root.querySelector('[data-testid="turn-made-note"]')?.classList.contains('room')).toBe(
      true,
    );
  });

  it('says nothing when nothing is left out', async () => {
    const root = await render({ ...NO_MADE, files: madeCards([PLAN]) });
    expect(root.querySelector('[data-testid="turn-made-note"]')).toBeNull();
  });

  it('shows 3 files whole, with no row to fold them', async () => {
    const root = await render({ ...NO_MADE, files: madeCards(plans(3)) });
    expect(root.querySelectorAll('[data-testid="made-file"]')).toHaveLength(3);
    expect(root.querySelector('[data-testid="turn-made-fold"]')).toBeNull();
  });

  it('shows the first 3 of 4 or more, then a row that unfolds the rest and folds them back', async () => {
    const root = await render({ ...NO_MADE, files: madeCards(plans(5)) });
    const fold = root.querySelector<HTMLButtonElement>('[data-testid="turn-made-fold"]');
    const titles = (): readonly string[] =>
      [...root.querySelectorAll('[data-testid="made-file-title"]')].map(
        (title) => title.textContent,
      );
    expect(titles()).toEqual(['Plan 1', 'Plan 2', 'Plan 3']);
    expect(fold?.textContent.trim()).toBe('2 more files');
    expect(fold?.getAttribute('aria-expanded')).toBe('false');
    await expectAccessible(root);

    fold?.click();
    await settle();
    expect(titles()).toEqual(['Plan 1', 'Plan 2', 'Plan 3', 'Plan 4', 'Plan 5']);
    expect(fold?.textContent.trim()).toBe('Show fewer');
    expect(fold?.getAttribute('aria-expanded')).toBe('true');
    await expectAccessible(root);

    fold?.click();
    await settle();
    expect(titles()).toEqual(['Plan 1', 'Plan 2', 'Plan 3']);
    expect(fold?.getAttribute('aria-expanded')).toBe('false');
  });

  it('says one more file in the singular, and counts the cards not described yet', async () => {
    const [first, ...rest] = plans(4);
    const root = await render({
      ...NO_MADE,
      files: madeCards(first === undefined ? [] : [first]),
      room: madeCards(rest),
    });
    const fold = root.querySelector('[data-testid="turn-made-fold"]');
    expect(fold?.textContent.trim()).toBe('1 more file');
    expect(root.querySelectorAll('app-made-file')).toHaveLength(3);
  });

  it('keeps the row’s room, unseen, until the server describes a file', async () => {
    const root = await render({ ...NO_MADE, room: madeCards(plans(4)) });
    const fold = root.querySelector('[data-testid="turn-made-fold"]');
    expect(fold?.classList.contains('room')).toBe(true);
  });
});

/** `count` PDFs a turn made, each named for its place: Plan 1, Plan 2, … */
function plans(count: number): MadeFile[] {
  return Array.from({ length: count }, (_, at) => ({
    ...PLAN,
    name: `Plan ${String(at + 1)}`,
    file_name: `plan-${String(at + 1)}.pdf`,
    sha256: String(at + 1)
      .repeat(64)
      .slice(0, 64),
  }));
}
