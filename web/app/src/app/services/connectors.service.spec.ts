import { TestBed } from '@angular/core/testing';
import { ApiError, type ConnectorInfo, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { ConnectorsService } from './connectors.service';

const LINEAR: ConnectorInfo = {
  id: 'linear',
  name: 'Linear',
  company: 'linear',
  url: 'https://mcp.linear.app/mcp',
  categories: ['dev'],
  tags: ['issues'],
  returns: 'text',
  sign_in: 'dcr',
  description: 'Issues, projects and cycles.',
  docs: 'https://linear.app/docs/mcp',
  status: 'available',
};
const CATEGORIES = [{ id: 'dev', name: 'Developer tools' }];

describe('ConnectorsService', () => {
  it('lists the connectors, and keeps them, saying why, when they could not be listed', async () => {
    let refused = false;
    const fake = new FakeSession().answer('connector/list', () =>
      refused
        ? new ApiError({
            code: ErrorCode.NOT_AVAILABLE,
            message: 'connections are signed in to on the computer glave runs on, by its person',
          })
        : { categories: CATEGORIES, connectors: [LINEAR] },
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const connectors = TestBed.inject(ConnectorsService);
    expect(connectors.catalog()).toBeNull();
    await connectors.load();
    expect(connectors.catalog()).toEqual({ categories: CATEGORIES, connectors: [LINEAR] });
    refused = true;
    await connectors.load();
    expect(connectors.catalog()?.connectors).toEqual([LINEAR]);
    expect(connectors.problem()).toBe(
      'Connections are signed in to on the computer glave runs on, by its person.',
    );
  });

  it('connects at once, or signs in and waits, then lists them again; and disconnects', async () => {
    let polls = 0;
    let connected = false;
    const fake = new FakeSession()
      .answer('connector/list', () => ({
        categories: CATEGORIES,
        connectors: [{ ...LINEAR, status: connected ? 'connected' : 'available' }],
      }))
      .answer('connector/connect', (params) =>
        (params as { connector: string }).connector === 'linear'
          ? { sign_in: { flow: 'f1', url: 'https://auth.example.com/authorize' } }
          : { connector: { ...LINEAR, id: 'deepwiki', status: 'connected' } },
      )
      .answer('connection/poll', () => {
        polls += 1;
        connected = polls >= 2;
        return connected
          ? { connection: { url: LINEAR.url, status: 'signed_in' } }
          : { flow: 'f1' };
      })
      .answer('connector/disconnect', {});
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const connectors = TestBed.inject(ConnectorsService);
    expect(await connectors.connect('deepwiki')).toBeNull();
    const link = await connectors.connect('linear');
    expect(link?.url).toBe('https://auth.example.com/authorize');
    expect(await connectors.finish('f1', new AbortController().signal)).toBe(true);
    expect(connectors.catalog()?.connectors[0]?.status).toBe('connected');
    await connectors.disconnect('linear');
    expect(fake.paramsOf('connector/disconnect')).toEqual([{ connector: 'linear' }]);
  });
});
