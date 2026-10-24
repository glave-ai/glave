import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Grouping } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { GroupingService } from './grouping.service';

/** The service, signed in, on `fake`, once what it read on starting has come back. */
async function started(fake: FakeSession): Promise<GroupingService> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const grouping = TestBed.inject(GroupingService);
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  return grouping;
}

describe('GroupingService', () => {
  it('groups by project until it reads the person’s choice', async () => {
    const fake = new FakeSession().answer('grouping/get', { grouping: 'status' });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const grouping = TestBed.inject(GroupingService);
    expect(grouping.grouping()).toBe('project');
    TestBed.tick();
    await new Promise((resolve) => setTimeout(resolve));
    expect(grouping.grouping()).toBe('status');
    expect(fake.paramsOf('grouping/get')).toEqual([{}]);
  });

  it('reads nothing while the tab is signed out', async () => {
    const fake = new FakeSession();
    TestBed.configureTestingModule({ providers: provideFakeSession(fake, null) });
    TestBed.inject(GroupingService);
    TestBed.tick();
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('grouping/get')).toEqual([]);
  });

  it('shows a choice at once and keeps what the server answers', async () => {
    let kept: Grouping = 'project';
    const fake = new FakeSession().answer('grouping/set', (params) => {
      kept = (params as { readonly grouping: Grouping }).grouping;
      return { grouping: kept };
    });
    const grouping = await started(fake);
    const choosing = grouping.choose('status');
    expect(grouping.grouping()).toBe('status');
    await choosing;
    expect(kept).toBe('status');
    expect(grouping.grouping()).toBe('status');
    expect(grouping.problem()).toBeNull();
  });

  it('puts the choice before back when the server keeps none, and says why', async () => {
    const fake = new FakeSession().answer(
      'grouping/set',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'the store is read-only' }),
    );
    const grouping = await started(fake);
    await grouping.choose('status');
    expect(grouping.grouping()).toBe('project');
    expect(grouping.problem()).toBe('The store is read-only.');
  });

  it('never lets a read begun before a choice undo it', async () => {
    let answer: (value: unknown) => void = () => undefined;
    const fake = new FakeSession().answer(
      'grouping/get',
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const grouping = TestBed.inject(GroupingService);
    TestBed.tick();
    await grouping.choose('status');
    answer({ grouping: 'project' });
    await new Promise((resolve) => setTimeout(resolve));
    expect(grouping.grouping()).toBe('status');
  });
});
