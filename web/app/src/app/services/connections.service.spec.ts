import { TestBed } from '@angular/core/testing';
import { ApiError, type ConnectionInfo, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { ConnectionsService } from './connections.service';

const DOCS: ConnectionInfo = {
  url: 'https://mcp.docs.example.com/mcp',
  name: 'docs',
  status: 'signed_out',
};

describe('ConnectionsService', () => {
  it('lists the connections, and says why when they could not be listed', async () => {
    let refused = false;
    const fake = new FakeSession().answer('connection/list', () =>
      refused
        ? new ApiError({
            code: ErrorCode.NOT_AVAILABLE,
            message: 'connections are signed in to on the computer glave runs on, by its person',
          })
        : { connections: [DOCS] },
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const connections = TestBed.inject(ConnectionsService);
    expect(connections.connections()).toBeNull();
    await connections.load();
    expect(connections.connections()).toEqual([DOCS]);
    expect(connections.problem()).toBeNull();

    refused = true;
    await connections.load();
    expect(connections.connections()).toEqual([DOCS]);
    expect(connections.problem()).toBe(
      'Connections are signed in to on the computer glave runs on, by its person.',
    );
  });

  it('signs in: waits while the server does, then lists the connections again', async () => {
    let polls = 0;
    const fake = new FakeSession()
      .answer('connection/start', {
        sign_in: { flow: 'f1', url: 'https://auth.example.com/authorize' },
      })
      .answer('connection/poll', () => {
        polls += 1;
        return polls < 3 ? { flow: 'f1' } : { connection: { ...DOCS, status: 'signed_in' } };
      })
      .answer('connection/list', () => ({
        connections: [{ ...DOCS, status: polls < 3 ? 'signed_out' : 'signed_in' }],
      }));
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const connections = TestBed.inject(ConnectionsService);
    const link = await connections.start(DOCS.url);
    expect(link.url).toBe('https://auth.example.com/authorize');
    expect(fake.paramsOf('connection/start')).toEqual([{ server: DOCS.url }]);
    await connections.finish(link.flow, new AbortController().signal);
    expect(fake.paramsOf('connection/poll')).toEqual([
      { flow: 'f1' },
      { flow: 'f1' },
      { flow: 'f1' },
    ]);
    expect(connections.connections()?.[0]?.status).toBe('signed_in');
  });

  it('stops waiting once told to, and passes a refusal on', async () => {
    const stop = new AbortController();
    const fake = new FakeSession().answer('connection/poll', () => {
      stop.abort();
      return { flow: 'f1' };
    });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const connections = TestBed.inject(ConnectionsService);
    await connections.finish('f1', stop.signal);
    expect(fake.paramsOf('connection/poll')).toHaveLength(1);
    expect(fake.paramsOf('connection/list')).toHaveLength(0);

    const declined = new ApiError({
      code: ErrorCode.HOST,
      message: 'glave was not signed in to docs: the sign-in was declined',
    });
    fake.answer('connection/poll', declined);
    await expect(connections.finish('f1', new AbortController().signal)).rejects.toBe(declined);
  });

  it('removes a sign-in, then lists the connections again', async () => {
    const fake = new FakeSession()
      .answer('connection/remove', {})
      .answer('connection/list', { connections: [DOCS] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const connections = TestBed.inject(ConnectionsService);
    await connections.remove(DOCS.url);
    expect(fake.paramsOf('connection/remove')).toEqual([{ server: DOCS.url }]);
    expect(connections.connections()).toEqual([DOCS]);
  });
});
