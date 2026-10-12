import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { ServerService } from './server.service';

const INFO = {
  name: 'glave',
  version: '0.1.0',
  protocols: ['2026-10'],
  approval: 'ask',
  approvals: ['ask'],
} as const;

describe('ServerService', () => {
  it('asks the server about itself once, and says its approval mode', async () => {
    const fake = new FakeSession().answer('server/discover', INFO);
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const server = TestBed.inject(ServerService);
    expect(server.approval()).toBeNull();
    await Promise.all([server.load(), server.load()]);
    expect(server.info()).toEqual(INFO);
    expect(server.approval()).toBe('ask');
    expect(fake.paramsOf('server/discover')).toHaveLength(1);
  });

  it('asks again after a failure', async () => {
    const fake = new FakeSession().answer(
      'server/discover',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'down' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const server = TestBed.inject(ServerService);
    await server.load();
    expect(server.info()).toBeNull();
    fake.answer('server/discover', INFO);
    await server.load();
    expect(server.approval()).toBe('ask');
  });
});
