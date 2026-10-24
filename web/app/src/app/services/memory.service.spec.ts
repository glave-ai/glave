import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Remembered } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { MemoryService } from './memory.service';

const MEMORY: Remembered = {
  memory: 'm1',
  version: 1,
  scope: 'personal',
  text: 'Prefers short answers',
  saved_at: '2026-10-11T10:00:00.000Z',
};

describe('MemoryService', () => {
  it('lists, edits and forgets memories', async () => {
    const fake = new FakeSession().answer('memory/list', { memories: [MEMORY] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const memory = TestBed.inject(MemoryService);
    await memory.refresh();
    expect(memory.state()).toBe('ready');
    expect(memory.memories()).toEqual([MEMORY]);
    await memory.edit('m1', 'Prefers brief answers');
    await memory.forget('m1');
    expect(fake.paramsOf('memory/edit')).toEqual([{ memory: 'm1', text: 'Prefers brief answers' }]);
    expect(fake.paramsOf('memory/forget')).toEqual([{ memory: 'm1' }]);
  });

  it('deletes the chat a memory was saved in once it is forgotten, when the server names one', async () => {
    const saved: Remembered = { ...MEMORY, source: { thread: 't1', seq: 3 } };
    const fake = new FakeSession()
      .answer('memory/list', { memories: [saved] })
      .answer('memory/forget', { memories: [saved] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const memory = TestBed.inject(MemoryService);
    await memory.forget('m1', true);
    expect(fake.paramsOf('memory/forget')).toEqual([{ memory: 'm1' }]);
    expect(fake.paramsOf('thread/delete')).toEqual([{ thread: 't1' }]);
  });

  it("forgets all the same when the chat cannot go, and says why in the server's words", async () => {
    const saved: Remembered = { ...MEMORY, source: { thread: 't1', seq: 3 } };
    const running = new ApiError({ code: ErrorCode.TURN_RUNNING, message: 'a turn is running' });
    const fake = new FakeSession()
      .answer('memory/list', { memories: [] })
      .answer('memory/forget', { memories: [saved] })
      .answer('thread/delete', running);
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const memory = TestBed.inject(MemoryService);
    await memory.forget('m1', true);
    expect(memory.memories()).toEqual([]);
    expect(memory.error()).toBe('a turn is running');
  });

  it('says when memory is off for the server', async () => {
    const fake = new FakeSession().answer(
      'memory/list',
      new ApiError({ code: ErrorCode.MEMORY_OFF, message: 'off' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const memory = TestBed.inject(MemoryService);
    await memory.refresh();
    expect(memory.state()).toBe('off');
    expect(memory.error()).toBeNull();
  });
});
