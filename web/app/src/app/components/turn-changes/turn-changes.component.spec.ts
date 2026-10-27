import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import type { ChangedFile } from '../../lib/files';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { TurnChangesComponent } from './turn-changes.component';

const FILES: readonly ChangedFile[] = [
  { path: 'crates/computer/src/local/worktree.rs', changes: [], added: 24, removed: 5 },
  { path: 'crates/computer/src/local/worktree/lock.rs', changes: [], added: 16, removed: 0 },
  { path: 'crates/harness/tests/worktrees.rs', changes: [], added: 2, removed: 2 },
];

/** What the turn's card asked to show in full, in order: whose files, and which. */
let shown: (readonly [readonly ChangedFile[] | null, string | null])[] = [];

async function render(
  files: readonly ChangedFile[],
  inputs: Readonly<Record<string, unknown>> = {},
): Promise<HTMLElement> {
  shown = [];
  TestBed.configureTestingModule({
    providers: [
      {
        provide: ThreadPanelService,
        useValue: {
          showChanges: (files: readonly ChangedFile[] | null, path: string | null) =>
            shown.push([files, path]),
        },
      },
    ],
  });
  const fixture = TestBed.createComponent(TurnChangesComponent);
  fixture.componentRef.setInput('files', files);
  for (const [name, value] of Object.entries(inputs)) {
    fixture.componentRef.setInput(name, value);
  }
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

function texts(root: HTMLElement, selector: string): readonly string[] {
  return Array.from(root.querySelectorAll(selector)).map((each) => each.textContent.trim());
}

describe('TurnChangesComponent', () => {
  it('says what changed: how many files, the lines added and removed, and each file', async () => {
    const root = await render(FILES);
    const card = root.querySelector('[data-testid="turn-changes"]');
    expect(document.getElementById(card?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      'What changed',
    );
    expect(root.querySelector('[data-testid="turn-changes-count"]')?.textContent).toBe('3 files');
    expect(texts(root, '.head .counts > span')).toEqual(['+42', '−7']);
    expect(texts(root, '.head .glz-visually-hidden')).toEqual(['42 lines added, 7 removed']);
    expect(texts(root, '.path')).toEqual(FILES.map((file) => file.path));
    expect(texts(root, '.row .counts > span')).toEqual(['+24', '−5', '+16', '−0', '+2', '−2']);
    // What a narrow card says after its first two rows.
    expect(root.querySelector('[data-testid="turn-changes-more"]')?.textContent.trim()).toBe(
      '1 more file',
    );
    await expectAccessible(root);
  });

  it('shows this turn’s changes in full: a file’s from its row, every one’s from the last', async () => {
    const root = await render(FILES);
    root.querySelectorAll<HTMLButtonElement>('[data-testid="turn-changes-file"]')[1]?.click();
    root.querySelector<HTMLButtonElement>('[data-testid="turn-changes-more"]')?.click();
    expect(shown).toEqual([
      [FILES, 'crates/computer/src/local/worktree/lock.rs'],
      [FILES, null],
    ]);
  });

  it('serves the panel under its own heading, a level down, with its own test hooks', async () => {
    const root = await render(FILES, { heading: 'Changes', level: 3, testId: 'panel-changes' });
    expect(root.querySelector('h3')?.textContent).toBe('Changes');
    expect(root.querySelector('h2')).toBeNull();
    expect(root.querySelectorAll('[data-testid="panel-changes-file"]')).toHaveLength(3);
    expect(root.querySelector('[data-testid="turn-changes"]')).toBeNull();
  });

  it('says nothing of more files when they all fit', async () => {
    const root = await render(FILES.slice(0, 1));
    expect(root.querySelector('[data-testid="turn-changes-count"]')?.textContent).toBe('1 file');
    expect(root.querySelector('[data-testid="turn-changes-more"]')).toBeNull();
  });
});
