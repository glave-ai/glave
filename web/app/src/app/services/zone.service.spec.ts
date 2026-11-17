import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { ZoneService } from './zone.service';

describe('ZoneService', () => {
  it('gives the server the browser’s time zone where schedules run, once', async () => {
    const fake = new FakeSession().answer('server/discover', { schedules: true });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const zones = TestBed.inject(ZoneService);
    await Promise.all([zones.give(), zones.give()]);
    await zones.give();
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    expect(fake.paramsOf('zone/set')).toEqual([{ zone }]);
  });

  it('gives it again at the next asking when it could not, and takes the refusal quietly', async () => {
    const fake = new FakeSession()
      .answer('server/discover', { schedules: true })
      .answer('zone/set', new ApiError({ code: ErrorCode.INTERNAL, message: 'down' }));
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const zones = TestBed.inject(ZoneService);
    await expect(zones.give()).resolves.toBeUndefined();
    fake.answer('zone/set', {});
    await zones.give();
    await zones.give();
    expect(fake.paramsOf('zone/set')).toHaveLength(2);
  });

  it('gives none where schedules do not run', async () => {
    const fake = new FakeSession();
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    await TestBed.inject(ZoneService).give();
    expect(fake.paramsOf('zone/set')).toEqual([]);
  });
});
