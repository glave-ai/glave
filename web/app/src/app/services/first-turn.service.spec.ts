import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type TurnStartParams } from '@glave/client';

import { FakeSession, FakeTurn, provideFakeSession } from '../testing/fake-session';
import { FirstTurnService } from './first-turn.service';

function settled(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe('FirstTurnService', () => {
  it('sends a first turn with a key, says why it did not go, and sends it again with that key', async () => {
    const refused = new ApiError({
      code: ErrorCode.QUEUE_FULL,
      message: 'Too many messages are waiting',
    });
    const cut = new FakeTurn();
    cut.fail(new TypeError('Failed to fetch'));
    const fake = new FakeSession().turn(cut).turn(refused);
    TestBed.configureTestingModule({ providers: [...provideFakeSession(fake)] });
    const service = TestBed.inject(FirstTurnService);
    service.start({ thread: 't1', prompt: 'Plan' });
    service.start({ thread: 't2', prompt: 'Tidy' });
    await settled();
    expect([service.of('t1')?.failed, service.of('t2')?.failed]).toEqual([
      'lost',
      { said: 'Too many messages are waiting' },
    ]);
    service.again('t1');
    await settled();
    expect(service.of('t1')?.failed).toBeNull();
    service.forget('t1');
    expect([service.of('t1'), service.of('t2')?.params.prompt]).toEqual([null, 'Tidy']);
    const keys = fake
      .paramsOf('turn/start')
      .map((params) => (params as TurnStartParams)._meta?.idempotency_key);
    expect(keys[0]).toMatch(/^[0-9a-f]{32}$/u);
    expect(keys[2]).toBe(keys[0]);
    expect(new Set(keys).size).toBe(2);
  });
});
