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

/** What the turn's card asked to show in full, in order. */
let shown: (string | null)[] = [];

async function render(files: readonly ChangedFile[]): Promise<HTMLElement> {
  shown = [];
  TestBed.configureTestingModule({
    providers: [
      {
        provide: ThreadPanelService,
        useValue: { showChanges: (path: string | null) => shown.push(path) },
      },
    ],
  });
  const fixture = TestBed.createComponent(TurnChangesComponent);
  fixture.componentRef.setInput('files', files);
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

  it('shows a file’s changes in full from its row, and every file’s from the last', async () => {
    const root = await render(FILES);
    root.querySelectorAll<HTMLButtonElement>('[data-testid="turn-changes-file"]')[1]?.click();
    root.querySelector<HTMLButtonElement>('[data-testid="turn-changes-more"]')?.click();
    expect(shown).toEqual(['crates/computer/src/local/worktree/lock.rs', null]);
  });

  it('says nothing of more files when they all fit', async () => {
    const root = await render(FILES.slice(0, 1));
    expect(root.querySelector('[data-testid="turn-changes-count"]')?.textContent).toBe('1 file');
    expect(root.querySelector('[data-testid="turn-changes-more"]')).toBeNull();
  });
});
