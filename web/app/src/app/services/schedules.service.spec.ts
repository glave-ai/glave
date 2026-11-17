import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { schedule } from '../testing/fixtures';
import { SchedulesService } from './schedules.service';

describe('SchedulesService', () => {
  it('pauses, resumes and removes a schedule, answering as the server has it', async () => {
    const fake = new FakeSession().answer('schedule/pause', (params) => {
      const { paused } = params as { readonly paused: boolean };
      return { schedule: { ...schedule('s1', null), paused } };
    });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const schedules = TestBed.inject(SchedulesService);
    expect((await schedules.pause('s1', true)).paused).toBe(true);
    expect((await schedules.pause('s1', false)).paused).toBe(false);
    await schedules.remove('s1');
    expect(fake.paramsOf('schedule/pause')).toEqual([
      { schedule: 's1', paused: true },
      { schedule: 's1', paused: false },
    ]);
    expect(fake.paramsOf('schedule/delete')).toEqual([{ schedule: 's1' }]);
  });

  it('throws what the server refused a change with', async () => {
    const fake = new FakeSession().answer(
      'schedule/delete',
      new ApiError({ code: ErrorCode.NO_SCHEDULE, message: 'no such schedule' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    await expect(TestBed.inject(SchedulesService).remove('s1')).rejects.toThrow('no such schedule');
  });
});
