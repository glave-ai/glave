import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Remembered } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { MemoryComponent } from './memory.component';

const MEMORY: Remembered = {
  memory: 'm1',
  version: 1,
  scope: 'personal',
  text: 'Prefers short answers',
  saved_at: '2026-10-11T10:00:00.000Z',
};

async function settle(fixture: { whenStable: () => Promise<unknown> }): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
}

function click(root: HTMLElement, testId: string): void {
  root.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.click();
}

describe('MemoryComponent', () => {
  it('lists memories, edits one and forgets one after confirming', async () => {
    const fake = new FakeSession().answer('memory/list', { memories: [MEMORY] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(MemoryComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="memory"]')?.textContent).toContain(
      'Prefers short answers',
    );
    expect(root.querySelector('[data-testid="page-back"]')).not.toBeNull();
    await expectAccessible(root);

    click(root, 'memory-edit');
    await settle(fixture);
    const text = root.querySelector<HTMLTextAreaElement>('[data-testid="memory-text"]');
    expect(text?.value).toBe('Prefers short answers');
    if (text) {
      text.value = 'Prefers brief answers';
      text.dispatchEvent(new Event('input'));
    }
    click(root, 'memory-save');
    await settle(fixture);
    expect(fake.paramsOf('memory/edit')).toEqual([{ memory: 'm1', text: 'Prefers brief answers' }]);

    click(root, 'memory-forget');
    await settle(fixture);
    click(root, 'memory-forget-confirm-ok');
    await settle(fixture);
    expect(fake.paramsOf('memory/forget')).toEqual([{ memory: 'm1' }]);
  });

  it('offers to delete the chat a memory was saved in, and only then', async () => {
    const saved: Remembered = {
      ...MEMORY,
      memory: 'm2',
      text: 'Likes marigolds',
      source: { thread: 't1', seq: 3 },
    };
    const fake = new FakeSession().answer('memory/list', { memories: [MEMORY, saved] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(MemoryComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    const forgets = root.querySelectorAll<HTMLElement>('[data-testid="memory-forget"]');
    forgets[0]?.click();
    await settle(fixture);
    expect(root.querySelector('[data-testid="memory-forget-chat"]')).toBeNull();
    click(root, 'memory-forget-confirm-cancel');
    await settle(fixture);

    forgets[1]?.click();
    await settle(fixture);
    expect(root.querySelector('[data-testid="memory-forget-chat"]')?.textContent).toContain(
      'Also delete the chat it was saved in',
    );
    await expectAccessible(root);
    click(root, 'memory-forget-chat-input');
    await settle(fixture);
    click(root, 'memory-forget-confirm-ok');
    await settle(fixture);
    expect(fake.paramsOf('memory/forget')).toEqual([{ memory: 'm2', delete_thread: true }]);
  });

  it('will not save an empty memory', async () => {
    const fake = new FakeSession().answer('memory/list', { memories: [MEMORY] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(MemoryComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    click(root, 'memory-edit');
    await settle(fixture);
    const text = root.querySelector<HTMLTextAreaElement>('[data-testid="memory-text"]');
    if (text) {
      text.value = '';
      text.dispatchEvent(new Event('input'));
    }
    click(root, 'memory-save');
    await settle(fixture);
    expect(root.textContent).toContain('Write something');
    expect(fake.paramsOf('memory/edit')).toEqual([]);
  });

  it('says when memory is off', async () => {
    const fake = new FakeSession().answer(
      'memory/list',
      new ApiError({ code: ErrorCode.MEMORY_OFF, message: 'off' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(MemoryComponent);
    await settle(fixture);
    expect((fixture.nativeElement as HTMLElement).textContent).toContain('Memory is off');
  });
});
