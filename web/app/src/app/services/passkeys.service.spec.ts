import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { unreached } from '../lib/text';
import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { AUTHENTICATOR, passkeyRefused, PasskeysService } from './passkeys.service';

const PHONE = { id: 'p1', name: 'Phone', added: '2026-10-20T09:00:00.000Z' };

function service(fake: FakeSession, made: unknown = null): PasskeysService {
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake),
      {
        provide: AUTHENTICATOR,
        useValue: {
          create: () => Promise.resolve(made),
          get: () => Promise.resolve(null),
        },
      },
    ],
  });
  return TestBed.inject(PasskeysService);
}

describe('PasskeysService', () => {
  it('lists the passkeys, and removes one, listing them again', async () => {
    let kept = [PHONE];
    const fake = new FakeSession()
      .answer('passkey/list', () => ({ passkeys: kept }))
      .answer('passkey/remove', () => {
        kept = [];
        return {};
      });
    const keys = service(fake);
    expect(await keys.load()).toBeNull();
    expect(keys.passkeys()).toEqual([PHONE]);
    expect(await keys.remove('p1')).toBeNull();
    expect(keys.passkeys()).toEqual([]);
  });

  it('says why a passkey was not added, and why nothing could be listed', async () => {
    const fake = new FakeSession().answer('passkey/add_options', {
      ceremony: 'c1',
      options: { publicKey: { challenge: 'AQID', user: { id: 'AQID' } } },
    });
    const keys = service(fake);
    expect(await keys.add()).toBe(passkeyRefused());
    expect(fake.paramsOf('passkey/add')).toEqual([]);
    fake.answer('passkey/list', new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'no' }));
    expect(await keys.load()).toBe('no');
    fake.answer('passkey/list', () => {
      throw new TypeError('offline');
    });
    expect(await keys.load()).toBe(unreached());
  });
});
