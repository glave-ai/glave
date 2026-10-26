import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { event } from '../../testing/fixtures';
import { ThreadFilesComponent } from './thread-files.component';

const EDITED = [
  event(1, 'tool.call.requested', {
    call_id: 'c1',
    tool: 'write',
    arguments: { json: { path: 'notes.md', content: 'Hello\n' } },
  }),
  event(2, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
  event(3, 'tool.call.requested', {
    call_id: 'c2',
    tool: 'edit',
    arguments: { json: { path: 'notes.md', old_string: 'Hello', new_string: 'Hi' } },
  }),
  event(4, 'tool.call.completed', { call_id: 'c2', is_error: false, output: [] }),
  event(5, 'tool.call.requested', {
    call_id: 'c3',
    tool: 'write',
    arguments: { json: { path: 'todo.md', content: '- one\n' } },
  }),
  event(6, 'tool.call.completed', { call_id: 'c3', is_error: false, output: [] }),
];

async function render(path: string | null = null): Promise<HTMLElement> {
  const fake = new FakeSession().answer('thread/events', { events: EDITED });
  TestBed.configureTestingModule({ providers: [...provideFakeSession(fake), ThreadService] });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(ThreadFilesComponent);
  fixture.componentRef.setInput('path', path);
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

describe('ThreadFilesComponent', () => {
  it('shows one file open on its diffs, marked by sign as well as colour', async () => {
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
    expect(changes?.[1]?.querySelector('.note')?.textContent).toBe('Edited');
    const lines = Array.from(changes?.[1]?.querySelectorAll('.line') ?? []);
    expect(lines.map((line) => [line.className.includes('removed'), line.textContent])).toEqual([
      [true, '-Hello'],
      [false, '+Hi'],
    ]);
    expect(root.textContent).toContain('Changes made by commands');
    await expectAccessible(root);
  });

  it('shows every file the thread changed, without a path', async () => {
    const root = await render();
    const paths = Array.from(root.querySelectorAll('[data-testid="file"] .path'));
    expect(paths.map((path) => path.textContent)).toEqual(['notes.md', 'todo.md']);
  });
});
