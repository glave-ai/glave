import { TestBed } from '@angular/core/testing';
import {
  ApiError,
  ErrorCode,
  type Notification,
  type PlanInfo,
  type RawEvent,
} from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { PlanService } from './plan.service';
import { ThreadService } from './thread.service';

const PROPOSED: PlanInfo = {
  steps: [
    { step: 'Read how flags are parsed', status: 'pending' },
    { step: 'Add --verbose', status: 'pending' },
  ],
  proposed: true,
  sentence: 'Proposed plan: 2 steps',
};

const TICKING: PlanInfo = {
  steps: [
    { step: 'Read how flags are parsed', status: 'completed' },
    { step: 'Add --verbose', status: 'in_progress' },
  ],
  proposed: false,
  sentence: 'Plan: 1 of 2 steps done',
};

function recorded(raw: RawEvent): Notification {
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: raw.thread, event: raw } };
}

function make(fake: FakeSession): { readonly plans: PlanService; readonly thread: ThreadService } {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), ThreadService, PlanService],
  });
  return { plans: TestBed.inject(PlanService), thread: TestBed.inject(ThreadService) };
}

/** Runs the effects, lets the calls they made answer, and runs the effects of those answers. */
async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
}

describe('PlanService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('asks nothing of a thread that never had a plan', async () => {
    const fake = new FakeSession().answer('thread/events', {
      events: [event(1, 'turn.started', {}), event(2, 'turn.ended', { outcome: 'completed' })],
    });
    const { plans, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/plan')).toEqual([]);
    expect(plans.plan()).toBeNull();
  });

  it('reads the plan as the thread opens, and again as each change to it is recorded', async () => {
    const history = [
      event(1, 'turn.started', {}),
      event(2, 'plan.updated', { steps: [], proposed: true }),
      event(3, 'turn.ended', { outcome: 'completed' }),
    ];
    let plan = PROPOSED;
    const fake = new FakeSession()
      .answer('thread/events', { events: history })
      .answer('thread/plan', () => ({ plan }))
      .streams([
        recorded(event(4, 'plan.approved', {})),
        recorded(event(5, 'turn.started', {})),
        recorded(event(6, 'plan.updated', { steps: [] })),
        recorded(event(7, 'turn.ended', { outcome: 'completed' })),
      ]);
    const { plans, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/plan')).toEqual([{ thread: 'thread-1' }]);
    expect(plans.plan()).toEqual(PROPOSED);
    plan = TICKING;
    await thread.build();
    await settle();
    expect(fake.paramsOf('plan/build')).toEqual([{ thread: 'thread-1' }]);
    expect(plans.plan()).toEqual(TICKING);
  });

  it('an edit shows the plan the server kept, and a refusal says why', async () => {
    const edited: PlanInfo = {
      steps: [{ step: 'Add --verbose and -v', status: 'pending' }],
      proposed: true,
      sentence: 'Proposed plan: 1 step',
    };
    const fake = new FakeSession()
      .answer('thread/events', {
        events: [event(2, 'plan.updated', { steps: [], proposed: true })],
      })
      .answer('thread/plan', { plan: PROPOSED })
      .answer('plan/edit', { plan: edited });
    const { plans, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    await plans.edit(['Add --verbose and -v']);
    expect(fake.paramsOf('plan/edit')).toEqual([
      { thread: 'thread-1', steps: ['Add --verbose and -v'] },
    ]);
    expect(plans.plan()).toEqual(edited);
    const refused = 'thread 1 has no plan waiting for approval: plan mode proposes one';
    fake.answer('plan/edit', new ApiError({ code: ErrorCode.NO_PLAN, message: refused }));
    await plans.edit(['x']);
    expect(plans.error()).toBe(refused);
    expect(plans.busy()).toBe(false);
  });
});
