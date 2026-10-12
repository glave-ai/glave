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
