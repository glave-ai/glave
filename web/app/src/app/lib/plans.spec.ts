import { event } from '../testing/fixtures';
import { chatPlan, lastPlanning } from './plans';

describe('lastPlanning', () => {
  it('is the seq of the latest change to the plan, the model’s, the person’s or Build it', () => {
    expect(lastPlanning([])).toBeNull();
    expect(lastPlanning([event(1, 'turn.started', {}), event(2, 'turn.ended', {})])).toBeNull();
    const proposed = event(3, 'plan.updated', { steps: [], proposed: true });
    expect(lastPlanning([event(1, 'turn.started', {}), proposed, event(4, 'turn.ended', {})])).toBe(
      3,
    );
    const built = [proposed, event(5, 'plan.approved', {}), event(6, 'turn.started', {})];
    expect(lastPlanning(built)).toBe(5);
  });
});

describe('chatPlan', () => {
  const steps = [{ step: 'Run the tests', status: 'in_progress' }] as const;
  const proposed = { proposed: true, sentence: 'Proposed plan: 1 step', steps };
  const building = { proposed: false, sentence: 'Plan: 0 of 1 step done', steps };

  it('shows a proposal in the conversation, the panel shown or not: it waits for the person', () => {
    expect(chatPlan(proposed, true)).toBe(proposed);
    expect(chatPlan(proposed, false)).toBe(proposed);
  });

  it('leaves a plan being built to the panel while it shows, and to the card while it does not', () => {
    expect(chatPlan(building, true)).toBeNull();
    expect(chatPlan(building, false)).toBe(building);
    expect(chatPlan(null, false)).toBeNull();
  });
});
