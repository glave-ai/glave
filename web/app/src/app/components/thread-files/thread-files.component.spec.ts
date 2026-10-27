import { TestBed } from '@angular/core/testing';
import type { ChangedFile } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { ThreadFilesComponent } from './thread-files.component';

/** What the server says the thread changed: two changes to one file, and another file. */
const CHANGED: readonly ChangedFile[] = [
  {
    path: 'notes.md',
    added: 2,
    removed: 1,
    changes: [
      { kind: 'wrote', lines: [{ kind: 'added', text: 'Hello' }] },
      {
        kind: 'edited_every',
        lines: [
          { kind: 'removed', text: 'Hello' },
          { kind: 'added', text: 'Hi' },
        ],
      },
    ],
  },
  {
    path: 'todo.md',
    added: 1,
    removed: 0,
    changes: [{ kind: 'wrote', lines: [{ kind: 'added', text: '- one' }] }],
  },
];

const NOTE = 'Changes made by commands or other tools are not listed.';

async function render(path: string | null = null, note = NOTE): Promise<HTMLElement> {
  const fixture = TestBed.createComponent(ThreadFilesComponent);
  fixture.componentRef.setInput('files', CHANGED);
  fixture.componentRef.setInput('path', path);
  fixture.componentRef.setInput('note', note);
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

describe('ThreadFilesComponent', () => {
  it('shows one file open on the server’s diffs, marked by sign as well as colour', async () => {
    const root = await render('notes.md');
    const files = root.querySelectorAll('[data-testid="file"]');
    expect(files).toHaveLength(1);
    expect((files[0] as HTMLDetailsElement | undefined)?.open).toBe(true);
    expect(files[0]?.querySelector('.path')?.textContent).toBe('notes.md');
    expect(files[0]?.querySelector('.plus')?.textContent).toBe('+2');
    expect(files[0]?.querySelector('.minus')?.textContent).toBe('−1');
    expect(files[0]?.textContent).toContain('2 lines added, 1 removed');
    const changes = files[0]?.querySelectorAll('[data-testid="file-change"]');
    expect(changes?.[0]?.querySelector('.note')?.textContent).toBe('Wrote the whole file');
    expect(changes?.[1]?.querySelector('.note')?.textContent).toBe('Edited every match');
    const lines = Array.from(changes?.[1]?.querySelectorAll('.line') ?? []);
    expect(lines.map((line) => [line.className.includes('removed'), line.textContent])).toEqual([
      [true, '-Hello'],
      [false, '+Hi'],
    ]);
    expect(root.querySelector('[data-testid="files-note"]')?.textContent).toBe(NOTE);
    await expectAccessible(root);
  });

  it('shows every file the thread changed, without a path, and no note before it is read', async () => {
    const root = await render(null, '');
    const paths = Array.from(root.querySelectorAll('[data-testid="file"] .path'));
    expect(paths.map((path) => path.textContent)).toEqual(['notes.md', 'todo.md']);
    expect(root.querySelector('[data-testid="files-note"]')).toBeNull();
  });
});
