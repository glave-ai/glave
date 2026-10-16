import { TestBed } from '@angular/core/testing';
import { ApiError, type CredentialInfo, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { CredentialsService } from './credentials.service';

const HOST: CredentialInfo = {
  id: '01900000-0000-7000-8000-000000000401',
  forge: 'gitea',
  url: 'https://code.example.com',
  login: 'ana',
  hint: 'wxyz',
};

describe('CredentialsService', () => {
  it('connects a host with a token, then reads the accounts again', async () => {
    let connected = false;
    const fake = new FakeSession()
      .answer('credential/connect', () => {
        connected = true;
        return { credential: HOST };
      })
      .answer('credential/list', () => ({
        credentials: connected ? [HOST] : [],
        device: true,
        install: 'https://github.com/apps/glave/installations/new',
      }));
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const credentials = TestBed.inject(CredentialsService);
    await credentials.connect('https://code.example.com', 'gta_token');
    expect(fake.paramsOf('credential/connect')).toEqual([
      { url: 'https://code.example.com', token: 'gta_token' },
    ]);
    expect(credentials.accounts()).toEqual([HOST]);
    expect(credentials.device()).toBe(true);
    expect(credentials.install()).toBe('https://github.com/apps/glave/installations/new');
  });

  it('passes a refusal on, keeping nothing', async () => {
    const refused = new ApiError({ code: ErrorCode.HOST, message: 'no' });
    const fake = new FakeSession()
      .answer('credential/connect', refused)
      .answer('credential/list', { credentials: [], device: false });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const credentials = TestBed.inject(CredentialsService);
    await expect(credentials.connect('https://code.example.com', 'bad')).rejects.toBe(refused);
    expect(credentials.accounts()).toEqual([]);
  });

  it('waits for a sign-in until GitHub approves it, or until told to stop', async () => {
    let polls = 0;
    const github = { ...HOST, forge: 'github', url: 'https://github.com' } as const;
    const fake = new FakeSession()
      .answer('credential/device/start', {
        device: {
          flow: 'f1',
          code: 'WDJB-MJHT',
          url: 'https://github.com/login/device',
          expires_in: 900,
        },
      })
      .answer('credential/device/poll', () => {
        polls += 1;
        return polls < 3 ? { flow: 'f1' } : { credential: github };
      })
      .answer('credential/list', { credentials: [github], device: true });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const credentials = TestBed.inject(CredentialsService);
    const device = await credentials.start();
    expect(device.code).toBe('WDJB-MJHT');
    expect(await credentials.finish(device.flow, () => false)).toEqual(github);
    expect(polls).toBe(3);
    expect(credentials.accounts()).toEqual([github]);
    expect(await credentials.finish(device.flow, () => true)).toBeNull();
    expect(polls).toBe(3);
  });

  it('removes an account by its id', async () => {
    const fake = new FakeSession().answer('credential/list', { credentials: [], device: false });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    await TestBed.inject(CredentialsService).remove(HOST.id);
    expect(fake.paramsOf('credential/remove')).toEqual([{ credential: HOST.id }]);
  });
});
