import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Grouping } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { GroupingService } from './grouping.service';

/** The service on `fake`, signed in. */
function service(fake: FakeSession): GroupingService {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  return TestBed.inject(GroupingService);
}

/** An answer the test gives when it chooses, and the way to give it. */
function held(): { readonly answer: Promise<unknown>; readonly give: (value: unknown) => void } {
  let give: (value: unknown) => void = () => undefined;
  const answer = new Promise((resolve) => {
    give = resolve;
  });
  return { answer, give };
}

describe('GroupingService', () => {
  it('groups by project until it reads the person’s choice', async () => {
    const fake = new FakeSession().answer('grouping/get', { grouping: 'status' });
    const grouping = service(fake);
    expect(grouping.grouping()).toBe('project');
    await grouping.read();
    expect(grouping.grouping()).toBe('status');
    expect(fake.paramsOf('grouping/get')).toEqual([{}]);
  });

  it('leaves the threads as they are when the choice cannot be read', async () => {
    const fake = new FakeSession().answer(
      'grouping/get',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'the store is unreachable' }),
    );
    const grouping = service(fake);
    await grouping.read();
    expect(grouping.grouping()).toBe('project');
    expect(grouping.problem()).toBeNull();
  });

  it('shows a choice at once and keeps what the server answers', async () => {
    let kept: Grouping = 'project';
    const fake = new FakeSession().answer('grouping/set', (params) => {
      kept = (params as { readonly grouping: Grouping }).grouping;
      return { grouping: kept };
    });
    const grouping = service(fake);
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
    const grouping = service(fake);
    await grouping.choose('status');
    expect(grouping.grouping()).toBe('project');
    expect(grouping.problem()).toBe('The store is read-only.');
  });

  it('never lets a read begun before a choice undo it', async () => {
    const read = held();
    const grouping = service(new FakeSession().answer('grouping/get', () => read.answer));
    const reading = grouping.read();
    await grouping.choose('status');
    read.give({ grouping: 'project' });
    await reading;
    expect(grouping.grouping()).toBe('status');
  });

  it('sends choices one after another, so the server keeps the last, and shows the last', async () => {
    const answers = [held(), held()];
    const fake = new FakeSession().answer('grouping/set', () => answers.shift()?.answer ?? {});
    const [first, second] = answers;
    const grouping = service(fake);
    const status = grouping.choose('status');
    const project = grouping.choose('project');
    await settled();
    // The second waits for the first's answer.
    expect(fake.paramsOf('grouping/set')).toEqual([{ grouping: 'status' }]);
    first?.give({ grouping: 'status' });
    await status;
    // The first's answer does not move the switch back while the second is on its way.
    expect(grouping.grouping()).toBe('project');
    await settled();
    expect(fake.paramsOf('grouping/set')).toEqual([
      { grouping: 'status' },
      { grouping: 'project' },
    ]);
    second?.give({ grouping: 'project' });
    await project;
    expect(grouping.grouping()).toBe('project');
  });

  it('when none of the choices is kept, shows what the server last kept', async () => {
    const fake = new FakeSession()
      .answer('grouping/get', { grouping: 'project' })
      .answer(
        'grouping/set',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'the store is read-only' }),
      );
    const grouping = service(fake);
    await grouping.read();
    await Promise.all([grouping.choose('status'), grouping.choose('project')]);
    expect(grouping.grouping()).toBe('project');
    await Promise.all([grouping.choose('project'), grouping.choose('status')]);
    expect(grouping.grouping()).toBe('project');
    expect(grouping.problem()).toBe('The store is read-only.');
    // Read again, as the side nav opens for the next person, it says nothing of before.
    await grouping.read();
    expect(grouping.problem()).toBeNull();
  });
});

/** Lets the calls already made reach the fake. */
async function settled(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}
